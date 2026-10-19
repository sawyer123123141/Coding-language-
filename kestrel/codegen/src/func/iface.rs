//! Interface values: `{data, table}` in memory (see `KTy::Iface`). A borrowed value's data pointer
//! is its place's address; a boxed one's is a new block in the current arena holding a copy. Calls
//! go through the method table (see `tables`): the method's address is loaded from the table and
//! called with the data pointer where the method expects `self`.

use cranelift_codegen::ir::{types, AbiParam, InstBuilder, MemFlags, Signature, Value};
use cranelift_module::Module;
use kestrel_kir::{self as kir, IfaceData, KSig, Place, TableIdx, TABLE_COPY, TABLE_METHODS, TABLE_PRINT};

use super::{cl_type, FuncLowerer};

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// The address of method table `table`.
    fn table_addr(&mut self, table: TableIdx) -> Value {
        let gv = self.module.declare_data_in_func(self.shared.tables[table.0 as usize], self.b.func);
        self.b.ins().symbol_value(self.ptr, gv)
    }

    /// The address in word `word` of the table of the interface value at `addr`.
    fn table_entry(&mut self, addr: Value, word: u32) -> Value {
        let table = self.b.ins().load(self.ptr, MemFlags::trusted(), addr, 8);
        self.b.ins().load(self.ptr, MemFlags::trusted(), table, (word * 8) as i32)
    }

    /// Writes the interface value with table `table` and the concrete value `data` at `dest`. A
    /// boxed value is allocated (its concrete size, in the current arena) and filled before `dest`
    /// is written, since the value may read `dest`.
    pub(crate) fn make_iface_into(&mut self, table: TableIdx, data: &IfaceData, line: u32, dest: Value) -> Result<(), String> {
        let ptr = match data {
            IfaceData::Borrowed(p) => self.place_addr(p)?,
            IfaceData::Boxed(v) => {
                let (size, align) = self.shared.kir.size_align(v.ty);
                let one = self.b.ins().iconst(types::I64, 1);
                let size = self.b.ins().iconst(types::I64, i64::from(size.max(1)));
                let align = self.b.ins().iconst(types::I64, i64::from(align.max(1)));
                let (l, f) = self.loc_args(line);
                let block = self.call_id(self.shared.rt.alloc, &[one, size, align, l, f]).expect("returns a pointer");
                self.store_at(v, block, 0)?;
                block
            }
        };
        let table = self.table_addr(table);
        self.b.ins().store(MemFlags::trusted(), ptr, dest, 0);
        self.b.ins().store(MemFlags::trusted(), table, dest, 8);
        Ok(())
    }

    /// Calls method `slot` of the interface value at `recv` with the result pointer (if any), the
    /// value's data pointer, then `args`. The data pointer and the method are read before the
    /// arguments are evaluated. An aggregate result is written to `dest`, or to a temporary.
    pub(crate) fn call_virtual(&mut self, recv: &Place, slot: u32, args: &[kir::Arg], sig: &KSig, dest: Option<Value>) -> Result<Option<Value>, String> {
        let addr = self.place_addr(recv)?;
        let data = self.b.ins().load(self.ptr, MemFlags::trusted(), addr, 0);
        let code = self.table_entry(addr, TABLE_METHODS + slot);
        let mut vals = Vec::with_capacity(args.len() + 2);
        if let Some(r) = sig.ret.filter(|r| r.is_aggregate()) {
            vals.push(match dest {
                Some(d) => d,
                None => self.temp_addr(r),
            });
        }
        vals.push(data);
        vals.extend(self.call_args(args)?);
        let sigref = self.b.import_signature(self.method_sig(sig));
        let inst = self.b.ins().call_indirect(sigref, code, &vals);
        Ok(self.b.inst_results(inst).first().copied())
    }

    /// The signature of a method called through a table: the methods' own (see `signature`), a
    /// pointer to an aggregate result first, then `self` by pointer, then `sig`'s parameters.
    fn method_sig(&self, sig: &KSig) -> Signature {
        let mut out = Signature::new(self.module.isa().default_call_conv());
        if sig.ret.is_some_and(|r| r.is_aggregate()) {
            out.params.push(AbiParam::new(self.ptr));
        }
        out.params.push(AbiParam::new(self.ptr));
        for &(ty, by_ref) in &sig.params {
            out.params.push(AbiParam::new(if by_ref { self.ptr } else { cl_type(ty) }));
        }
        if let Some(r) = sig.ret.filter(|r| !r.is_aggregate()) {
            out.returns.push(AbiParam::new(cl_type(r)));
        }
        out
    }

    /// Prints the concrete value of the interface value at `addr`, through its table.
    pub(crate) fn print_iface(&mut self, addr: Value) {
        let data = self.b.ins().load(self.ptr, MemFlags::trusted(), addr, 0);
        let print = self.table_entry(addr, TABLE_PRINT);
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        sig.params.push(AbiParam::new(self.ptr));
        let sigref = self.b.import_signature(sig);
        self.b.ins().call_indirect(sigref, print, &[data]);
    }

    /// Gives the interface value at `p` its own copy of its concrete value, through the copy
    /// function in its table, which returns the copy's address.
    pub(crate) fn copy_iface(&mut self, p: &Place) -> Result<(), String> {
        let addr = self.place_addr(p)?;
        let data = self.b.ins().load(types::I64, MemFlags::trusted(), addr, 0);
        let copy = self.table_entry(addr, TABLE_COPY);
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let sigref = self.b.import_signature(sig);
        let inst = self.b.ins().call_indirect(sigref, copy, &[data]);
        let fresh = self.b.inst_results(inst)[0];
        self.b.ins().store(MemFlags::trusted(), fresh, addr, 0);
        Ok(())
    }
}
