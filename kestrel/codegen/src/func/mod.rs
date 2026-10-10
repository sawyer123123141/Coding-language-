mod aggregate;
mod arenas;
mod arith;
mod call;
mod collections;
mod divide;
mod expr;
mod maps;
mod par;
mod place;
mod print;

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{
    types, AbiParam, Block, Function, InstBuilder, Signature, StackSlot, StackSlotData, StackSlotKind, TrapCode,
    Type, Value,
};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataId, FuncId, Module};
use kestrel_kir::{self as kir, KTy, LocalIdx, LocalKind, Place};

use crate::rt::RtFuncs;
use crate::Options;

/// The register type of a scalar. Structs live in memory and have none.
pub(crate) fn cl_type(t: KTy) -> Type {
    match t {
        KTy::Bool | KTy::U8 => types::I8,
        KTy::I32 => types::I32,
        KTy::I64 => types::I64,
        KTy::F32 => types::F32,
        KTy::F64 => types::F64,
        KTy::Struct(_)
        | KTy::Enum(_)
        | KTy::Str
        | KTy::Array(_)
        | KTy::List(_)
        | KTy::Map(..)
        | KTy::Arena
        | KTy::Optional(_) => {
            unreachable!("aggregates live in memory, not registers")
        }
    }
}

/// A struct result is written through a pointer the caller passes first.
pub(crate) fn signature(f: &kir::Func, cc: CallConv, ptr: Type) -> Signature {
    let mut sig = Signature::new(cc);
    let ret = f.ret.filter(|r| !r.is_aggregate());
    if f.ret.is_some_and(|r| r.is_aggregate()) {
        sig.params.push(AbiParam::new(ptr));
    }
    for p in &f.params {
        let local = &f.locals[p.0 as usize];
        let ty = if local.kind == LocalKind::RefParam { ptr } else { cl_type(local.ty) };
        sig.params.push(AbiParam::new(ty));
    }
    if let Some(r) = ret {
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
    /// The thunk of each `par for` body, by `FuncIdx`.
    pub par_thunks: &'a HashMap<u32, FuncId>,
    pub rt: &'a RtFuncs,
    pub opts: Options,
    /// The whole program: struct layouts and callees' signatures.
    pub kir: &'a kir::Module,
}

pub(crate) struct FuncLowerer<'a, 'b, M: Module> {
    module: &'a mut M,
    strings: &'a mut Strings,
    shared: &'a Shared<'a>,
    b: FunctionBuilder<'b>,
    f: &'a kir::Func,
    ptr: Type,
    slots: Vec<Slot>,
    /// Where a struct result goes, for functions that return one.
    sret: Option<Value>,
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
    let mut params: Vec<Value> = b.block_params(entry).to_vec();
    let sret = if f.ret.is_some_and(|r| r.is_aggregate()) { Some(params.remove(0)) } else { None };

    let mut slots = Vec::with_capacity(f.locals.len());
    for (i, local) in f.locals.iter().enumerate() {
        let slot = match local.kind {
            LocalKind::Value => {
                let v = Variable::from_u32(i as u32);
                b.declare_var(v, cl_type(local.ty));
                Slot::Var(v)
            }
            LocalKind::RefParam => {
                let v = Variable::from_u32(i as u32);
                b.declare_var(v, ptr);
                Slot::Ptr(v)
            }
            LocalKind::Addressed => Slot::Stack(stack_slot(&mut b, shared.kir, local.ty)),
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

    let mut fl = FuncLowerer { module, strings, shared, b, f, ptr, slots, sret, loops: Vec::new() };
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

/// A stack slot big enough and aligned enough for a value of `ty`.
fn stack_slot(b: &mut FunctionBuilder, kir: &kir::Module, ty: KTy) -> StackSlot {
    let (size, align) = kir.size_align(ty);
    let data = StackSlotData::new(StackSlotKind::ExplicitSlot, size.max(1), align.trailing_zeros() as u8);
    b.create_sized_stack_slot(data)
}

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    fn local_ty(&self, l: LocalIdx) -> KTy {
        self.f.locals[l.0 as usize].ty
    }

    /// A fresh stack slot for an intermediate struct value; returns its address.
    pub(crate) fn temp_addr(&mut self, ty: KTy) -> Value {
        let ss = stack_slot(&mut self.b, self.shared.kir, ty);
        self.b.ins().stack_addr(self.ptr, ss, 0)
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
            kir::Stmt::Let(l, e) => {
                if e.ty.is_aggregate() {
                    let dest = self.place_addr(&Place::local(*l))?;
                    self.lower_into(e, dest)?;
                } else {
                    let v = self.expr(e)?;
                    self.store_place(&Place::local(*l), v)?;
                }
            }
            kir::Stmt::Assign(p, e) => self.assign(p, e)?,
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
                    Some(e) if e.ty.is_aggregate() => {
                        let dest = self.sret.expect("struct-returning function has a result pointer");
                        self.lower_into(e, dest)?;
                        self.b.ins().return_(&[]);
                    }
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
                self.call(*fi, args, None)?;
            }
            kir::Stmt::Print(args) => self.print(args, true)?,
            kir::Stmt::Write(args) => self.print(args, false)?,
            kir::Stmt::ListAdd(list, value) => self.list_add(list, value)?,
            kir::Stmt::Dup(p) => self.dup(p)?,
            kir::Stmt::MapSet(map, key, value) => self.map_set(map, key, value, key.line)?,
            kir::Stmt::MapUnlock(map, saved) => self.map_unlock(map, saved)?,
            kir::Stmt::ParFor { func, args, count } => self.par_for(*func, args, count)?,
            kir::Stmt::PushArena(_) | kir::Stmt::PopArena { .. } | kir::Stmt::FreeArena(_) => self.arena_stmt(s)?,
            kir::Stmt::Bind(l, p) => {
                let addr = self.place_addr(p)?;
                let Slot::Ptr(v) = self.slots[l.0 as usize] else { unreachable!("binds a reference local") };
                self.b.def_var(v, addr);
            }
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
        let place = Place::local(var);
        self.store_place(&place, start_v)?;
        let ty = self.local_ty(var);
        let (header, body_b, step, exit) =
            (self.b.create_block(), self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().jump(header, &[]);

        self.b.switch_to_block(header);
        let i = self.load_place(&place)?;
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
        let i = self.load_place(&place)?;
        let next = self.b.ins().iadd_imm(i, 1);
        self.store_place(&place, next)?;
        self.b.ins().jump(header, &[]);

        self.b.switch_to_block(exit);
        Ok(())
    }
}
