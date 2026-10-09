mod arith;
mod expr;

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{
    types, AbiParam, Block, Function, InstBuilder, MemFlags, Signature, StackSlot, StackSlotData,
    StackSlotKind, TrapCode, Type, Value,
};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, DataId, FuncId, Module};
use kestrel_kir::{self as kir, KTy, LocalIdx, LocalKind};

use crate::rt::RtFuncs;
use crate::Options;

pub(crate) fn cl_type(t: KTy) -> Type {
    match t {
        KTy::Bool | KTy::U8 => types::I8,
        KTy::I32 => types::I32,
        KTy::I64 => types::I64,
        KTy::F32 => types::F32,
        KTy::F64 => types::F64,
    }
}

pub(crate) fn signature(f: &kir::Func, cc: CallConv, ptr: Type) -> Signature {
    let mut sig = Signature::new(cc);
    for p in &f.params {
        let local = &f.locals[p.0 as usize];
        let ty = if local.kind == LocalKind::RefParam { ptr } else { cl_type(local.ty) };
        sig.params.push(AbiParam::new(ty));
    }
    if let Some(r) = f.ret {
        sig.returns.push(AbiParam::new(cl_type(r)));
    }
    sig
}

/// String literal data, shared across all functions of a module.
#[derive(Default)]
pub(crate) struct Strings {
    ids: HashMap<String, DataId>,
}

/// Where a local's value lives.
#[derive(Clone, Copy)]
enum Slot {
    Var(Variable),
    Stack(StackSlot),
    /// Variable holding a pointer to the value.
    Ptr(Variable),
}

pub(crate) struct Shared<'a> {
    pub funcs: &'a [FuncId],
    pub rt: &'a RtFuncs,
    pub opts: Options,
}

pub(crate) struct FuncLowerer<'a, 'b, M: Module> {
    module: &'a mut M,
    strings: &'a mut Strings,
    shared: &'a Shared<'a>,
    b: FunctionBuilder<'b>,
    f: &'a kir::Func,
    ptr: Type,
    slots: Vec<Slot>,
    /// (continue target, break target) of each enclosing loop.
    loops: Vec<(Block, Block)>,
}

pub(crate) fn lower_func<M: Module>(
    module: &mut M,
    strings: &mut Strings,
    shared: &Shared,
    func: &mut Function,
    fb_ctx: &mut FunctionBuilderContext,
    f: &kir::Func,
) -> Result<(), String> {
    let ptr = module.target_config().pointer_type();
    let mut b = FunctionBuilder::new(func, fb_ctx);
    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    b.seal_block(entry);
    let params: Vec<Value> = b.block_params(entry).to_vec();

    let mut slots = Vec::with_capacity(f.locals.len());
    for (i, local) in f.locals.iter().enumerate() {
        let ty = cl_type(local.ty);
        let slot = match local.kind {
            LocalKind::Value => {
                let v = Variable::from_u32(i as u32);
                b.declare_var(v, ty);
                Slot::Var(v)
            }
            LocalKind::RefParam => {
                let v = Variable::from_u32(i as u32);
                b.declare_var(v, ptr);
                Slot::Ptr(v)
            }
            LocalKind::Addressed => {
                let size = ty.bytes();
                let data = StackSlotData::new(StackSlotKind::ExplicitSlot, size, size.trailing_zeros() as u8);
                Slot::Stack(b.create_sized_stack_slot(data))
            }
        };
        slots.push(slot);
    }
    for (p, &val) in f.params.iter().zip(&params) {
        match slots[p.0 as usize] {
            Slot::Var(v) | Slot::Ptr(v) => b.def_var(v, val),
            Slot::Stack(ss) => {
                b.ins().stack_store(val, ss, 0);
            }
        }
    }

    let mut fl = FuncLowerer { module, strings, shared, b, f, ptr, slots, loops: Vec::new() };
    if fl.stmts(&f.body)? {
        if f.ret.is_some() {
            // The checker guarantees value-returning bodies don't fall off the end.
            fl.b.ins().trap(TrapCode::unwrap_user(1));
        } else {
            fl.b.ins().return_(&[]);
        }
    }
    fl.b.seal_all_blocks();
    fl.b.finalize();
    Ok(())
}

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    fn local_ty(&self, l: LocalIdx) -> KTy {
        self.f.locals[l.0 as usize].ty
    }

    pub(crate) fn load_local(&mut self, l: LocalIdx) -> Value {
        let ty = cl_type(self.local_ty(l));
        match self.slots[l.0 as usize] {
            Slot::Var(v) => self.b.use_var(v),
            Slot::Stack(ss) => self.b.ins().stack_load(ty, ss, 0),
            Slot::Ptr(v) => {
                let p = self.b.use_var(v);
                self.b.ins().load(ty, MemFlags::trusted(), p, 0)
            }
        }
    }

    fn store_local(&mut self, l: LocalIdx, val: Value) {
        match self.slots[l.0 as usize] {
            Slot::Var(v) => self.b.def_var(v, val),
            Slot::Stack(ss) => {
                self.b.ins().stack_store(val, ss, 0);
            }
            Slot::Ptr(v) => {
                let p = self.b.use_var(v);
                self.b.ins().store(MemFlags::trusted(), val, p, 0);
            }
        }
    }

    /// Address of a local passed to a `var` parameter.
    pub(crate) fn local_addr(&mut self, l: LocalIdx) -> Value {
        match self.slots[l.0 as usize] {
            Slot::Stack(ss) => self.b.ins().stack_addr(self.ptr, ss, 0),
            Slot::Ptr(v) => self.b.use_var(v),
            Slot::Var(_) => unreachable!("KIR marks every `var` argument Addressed"),
        }
    }

    /// Lowers statements; returns whether control can reach the end.
    fn stmts(&mut self, list: &[kir::Stmt]) -> Result<bool, String> {
        for s in list {
            if !self.stmt(s)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn stmt(&mut self, s: &kir::Stmt) -> Result<bool, String> {
        match s {
            kir::Stmt::Let(l, e) | kir::Stmt::Assign(l, e) => {
                let v = self.expr(e)?;
                self.store_local(*l, v);
            }
            kir::Stmt::If { cond, then_body, else_body } => return self.if_stmt(cond, then_body, else_body),
            kir::Stmt::While { cond, body } => self.while_loop(cond, body)?,
            kir::Stmt::ForRange { var, start, end, body } => self.for_range(*var, start, end, body)?,
            kir::Stmt::Break => {
                let (_, brk) = *self.loops.last().expect("break inside a loop");
                self.b.ins().jump(brk, &[]);
                return Ok(false);
            }
            kir::Stmt::Continue => {
                let (cont, _) = *self.loops.last().expect("continue inside a loop");
                self.b.ins().jump(cont, &[]);
                return Ok(false);
            }
            kir::Stmt::Return(value) => {
                match value {
                    Some(e) => {
                        let v = self.expr(e)?;
                        self.b.ins().return_(&[v]);
                    }
                    None => {
                        self.b.ins().return_(&[]);
                    }
                }
                return Ok(false);
            }
            kir::Stmt::Call(fi, args) => {
                self.call(*fi, args)?;
            }
            kir::Stmt::Print(args) => self.print(args)?,
        }
        Ok(true)
    }

    fn if_stmt(&mut self, cond: &kir::Expr, then_body: &[kir::Stmt], else_body: &[kir::Stmt]) -> Result<bool, String> {
        let c = self.expr(cond)?;
        let (then_b, else_b, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().brif(c, then_b, &[], else_b, &[]);
        self.b.switch_to_block(then_b);
        let then_falls = self.stmts(then_body)?;
        if then_falls {
            self.b.ins().jump(merge, &[]);
        }
        self.b.switch_to_block(else_b);
        let else_falls = self.stmts(else_body)?;
        if else_falls {
            self.b.ins().jump(merge, &[]);
        }
        self.b.switch_to_block(merge);
        if !then_falls && !else_falls {
            self.b.ins().trap(TrapCode::unwrap_user(1));
            return Ok(false);
        }
        Ok(true)
    }

    fn while_loop(&mut self, cond: &kir::Expr, body: &[kir::Stmt]) -> Result<(), String> {
        let (header, body_b, exit) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().jump(header, &[]);
        self.b.switch_to_block(header);
        let c = self.expr(cond)?;
        self.b.ins().brif(c, body_b, &[], exit, &[]);
        self.b.switch_to_block(body_b);
        self.loops.push((header, exit));
        let falls = self.stmts(body)?;
        self.loops.pop();
        if falls {
            self.b.ins().jump(header, &[]);
        }
        self.b.switch_to_block(exit);
        Ok(())
    }

    fn for_range(&mut self, var: LocalIdx, start: &kir::Expr, end: &kir::Expr, body: &[kir::Stmt]) -> Result<(), String> {
        let start_v = self.expr(start)?;
        let end_v = self.expr(end)?;
        self.store_local(var, start_v);
        let ty = self.local_ty(var);
        let (header, body_b, step, exit) =
            (self.b.create_block(), self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().jump(header, &[]);

        self.b.switch_to_block(header);
        let i = self.load_local(var);
        let cc = if ty.is_signed() { IntCC::SignedLessThan } else { IntCC::UnsignedLessThan };
        let more = self.b.ins().icmp(cc, i, end_v);
        self.b.ins().brif(more, body_b, &[], exit, &[]);

        self.b.switch_to_block(body_b);
        self.loops.push((step, exit));
        let falls = self.stmts(body)?;
        self.loops.pop();
        if falls {
            self.b.ins().jump(step, &[]);
        }

        // `i < end` held, so `i + 1` can't overflow.
        self.b.switch_to_block(step);
        let i = self.load_local(var);
        let next = self.b.ins().iadd_imm(i, 1);
        self.store_local(var, next);
        self.b.ins().jump(header, &[]);

        self.b.switch_to_block(exit);
        Ok(())
    }

    pub(crate) fn call(&mut self, fi: kir::FuncIdx, args: &[kir::Arg]) -> Result<Option<Value>, String> {
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(match a {
                kir::Arg::Value(e) => self.expr(e)?,
                kir::Arg::Ref(l) => self.local_addr(*l),
            });
        }
        let id = self.shared.funcs[fi.0 as usize];
        Ok(self.call_id(id, &vals))
    }

    pub(crate) fn call_id(&mut self, id: FuncId, args: &[Value]) -> Option<Value> {
        let fref = self.module.declare_func_in_func(id, self.b.func);
        let inst = self.b.ins().call(fref, args);
        self.b.inst_results(inst).first().copied()
    }

    fn print(&mut self, args: &[kir::PrintArg]) -> Result<(), String> {
        // Evaluate every argument before writing anything, so output (or a
        // panic) inside an argument can't interleave with this line.
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(match a {
                kir::PrintArg::Value(e) => Some(self.expr(e)?),
                kir::PrintArg::Str(_) => None,
            });
        }
        let rt = self.shared.rt;
        for (i, (a, val)) in args.iter().zip(vals).enumerate() {
            if i > 0 {
                self.call_id(rt.print_space, &[]);
            }
            match a {
                kir::PrintArg::Str(s) => {
                    let (p, n) = self.string(s)?;
                    self.call_id(rt.print_str, &[p, n]);
                }
                kir::PrintArg::Value(e) => {
                    let v = val.expect("value arguments are evaluated above");
                    match e.ty {
                        KTy::Bool => self.call_id(rt.print_bool, &[v]),
                        KTy::U8 => {
                            let w = self.b.ins().uextend(types::I64, v);
                            self.call_id(rt.print_i64, &[w])
                        }
                        KTy::I32 => {
                            let w = self.b.ins().sextend(types::I64, v);
                            self.call_id(rt.print_i64, &[w])
                        }
                        KTy::I64 => self.call_id(rt.print_i64, &[v]),
                        KTy::F32 => self.call_id(rt.print_f32, &[v]),
                        KTy::F64 => self.call_id(rt.print_f64, &[v]),
                    };
                }
            }
        }
        self.call_id(rt.print_newline, &[]);
        Ok(())
    }

    /// Pointer and length of a string literal's bytes.
    fn string(&mut self, s: &str) -> Result<(Value, Value), String> {
        let id = match self.strings.ids.get(s) {
            Some(&id) => id,
            None => {
                let id = self.module.declare_anonymous_data(false, false).map_err(|e| e.to_string())?;
                let mut desc = DataDescription::new();
                // Zero-sized data objects aren't allowed; the length stays 0.
                let bytes = if s.is_empty() { vec![0] } else { s.as_bytes().to_vec() };
                desc.define(bytes.into_boxed_slice());
                self.module.define_data(id, &desc).map_err(|e| e.to_string())?;
                self.strings.ids.insert(s.to_string(), id);
                id
            }
        };
        let gv = self.module.declare_data_in_func(id, self.b.func);
        let p = self.b.ins().symbol_value(self.ptr, gv);
        let n = self.b.ins().iconst(types::I64, s.len() as i64);
        Ok((p, n))
    }
}
