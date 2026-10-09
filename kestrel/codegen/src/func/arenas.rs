//! Arenas (`arena { }`, `in x { }`, dropping) and moves of owning values.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, MemFlags, Value};
use cranelift_module::Module;
use kestrel_kir::{self as kir, KTy, Place};

use super::FuncLowerer;

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    pub(crate) fn new_arena(&mut self) -> Value {
        self.call_id(self.shared.rt.arena_new, &[]).expect("returns an arena")
    }

    /// Makes an arena current: a fresh one, or the one at `place`, which is
    /// created there first if it's still null (a zero-filled value).
    pub(crate) fn push_arena(&mut self, place: Option<&Place>) -> Result<(), String> {
        let arena = match place {
            None => self.new_arena(),
            Some(p) => {
                let addr = self.place_addr(p)?;
                let a = self.b.ins().load(self.ptr, MemFlags::trusted(), addr, 0);
                let (create, done) = (self.b.create_block(), self.b.create_block());
                let result = self.b.append_block_param(done, self.ptr);
                let missing = self.b.ins().icmp_imm(IntCC::Equal, a, 0);
                self.b.ins().brif(missing, create, &[], done, &[a]);
                self.b.switch_to_block(create);
                self.b.set_cold_block(create);
                let fresh = self.new_arena();
                self.b.ins().store(MemFlags::trusted(), fresh, addr, 0);
                self.b.ins().jump(done, &[fresh]);
                self.b.switch_to_block(done);
                result
            }
        };
        self.call_id(self.shared.rt.arena_push, &[arena]);
        Ok(())
    }

    pub(crate) fn pop_arena(&mut self, free: bool) {
        let a = self.call_id(self.shared.rt.arena_pop, &[]).expect("returns the arena");
        if free {
            self.call_id(self.shared.rt.arena_free, &[a]);
        }
    }

    /// Frees the arena at `place` and nulls it, so dropping again does nothing.
    pub(crate) fn free_arena(&mut self, place: &Place) -> Result<(), String> {
        let addr = self.place_addr(place)?;
        let a = self.b.ins().load(self.ptr, MemFlags::trusted(), addr, 0);
        self.call_id(self.shared.rt.arena_free, &[a]);
        let null = self.b.ins().iconst(self.ptr, 0);
        self.b.ins().store(MemFlags::trusted(), null, addr, 0);
        Ok(())
    }

    /// Moves an owning aggregate out of `place` into `dest`, zeroing `place`.
    pub(crate) fn move_into(&mut self, place: &Place, ty: KTy, dest: Value) -> Result<(), String> {
        let src = self.place_addr(place)?;
        self.copy_aggregate(dest, src, ty);
        let (size, _) = self.shared.kir.size_align(ty);
        let zero = self.b.ins().iconst(types::I64, 0);
        let mut at = 0;
        while at + 8 <= size as i32 {
            self.b.ins().store(MemFlags::trusted(), zero, src, at);
            at += 8;
        }
        let zero8 = self.b.ins().iconst(types::I8, 0);
        while at < size as i32 {
            self.b.ins().store(MemFlags::trusted(), zero8, src, at);
            at += 1;
        }
        Ok(())
    }

    /// Moves a scalar owning value (an `Arena`) out of `place`, nulling it.
    pub(crate) fn move_scalar(&mut self, place: &Place) -> Result<Value, String> {
        let v = self.load_place(place)?;
        let null = self.b.ins().iconst(self.ptr, 0);
        self.store_place(place, null)?;
        Ok(v)
    }

    pub(crate) fn arena_stmt(&mut self, s: &kir::Stmt) -> Result<(), String> {
        match s {
            kir::Stmt::PushArena(p) => self.push_arena(p.as_ref())?,
            kir::Stmt::PopArena { free } => self.pop_arena(*free),
            kir::Stmt::FreeArena(p) => self.free_arena(p)?,
            other => unreachable!("{other:?} isn't an arena statement"),
        }
        Ok(())
    }
}
