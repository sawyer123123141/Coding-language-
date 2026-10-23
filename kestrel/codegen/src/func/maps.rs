//! Maps: a 64-byte header over entries and an index table, which the
//! runtime manages (`runtime/src/map.rs`). Lookups, adds and removals call
//! it; `len`, loops and values read the header and entries directly.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, InstBuilder, MemFlags, TrapCode, Value};
use cranelift_module::Module;
use kestrel_kir::{self as kir, KTy, MapLayout, MapOp, Place};
use kestrel_runtime::PANIC_MAP_CHANGED;

use super::FuncLowerer;

const ENTRIES: i32 = 0;
const COUNT: i32 = 8;
const USED: i32 = 16;
const LOCKS: i32 = 48;
const ARENA: i32 = 56;
/// Entry fields: the live flag, then the key.
const LIVE: i32 = 4;
const KEY: i64 = 8;

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    fn map_layout_of(&self, ty: KTy) -> MapLayout {
        let KTy::Map(k, v) = ty else { unreachable!("{ty:?} isn't a map") };
        self.shared.kir.map_layout(k, v)
    }

    fn load_i64(&mut self, addr: Value, offset: i32) -> Value {
        self.b.ins().load(types::I64, MemFlags::trusted(), addr, offset)
    }

    /// `map[K, V]()`: empty; it will grow in the current arena.
    pub(crate) fn map_new(&mut self, dest: Value) {
        let zero = self.b.ins().iconst(types::I64, 0);
        for offset in (0..ARENA).step_by(8) {
            self.b.ins().store(MemFlags::trusted(), zero, dest, offset);
        }
        let arena = self.call_id(self.shared.rt.current_arena, &[]).expect("returns the arena");
        self.b.ins().store(MemFlags::trusted(), arena, dest, ARENA);
    }

    /// Evaluates a key into a stack slot, whose address the runtime reads.
    fn key_slot(&mut self, key: &kir::Expr) -> Result<Value, String> {
        let addr = self.temp_addr(key.ty);
        if key.ty.is_aggregate() {
            self.lower_into(key, addr)?;
        } else {
            let v = self.expr(key)?;
            self.store_value(v, addr, 0);
        }
        Ok(addr)
    }

    /// The entry for `key` in the map at `map`, or null.
    fn map_find(&mut self, map: &Place, key: &kir::Expr) -> Result<(Value, MapLayout), String> {
        let l = self.map_layout_of(self.place_ty(map));
        let k = self.key_slot(key)?;
        let header = self.place_addr(map)?;
        let kind = self.b.ins().iconst(types::I32, l.key_kind as i64);
        let size = self.b.ins().iconst(types::I64, l.entry_size as i64);
        let entry = self.call_id(self.shared.rt.map_find, &[header, k, kind, size]).expect("returns a pointer");
        Ok((entry, l))
    }

    /// A map operation with a scalar result (`Get` builds an optional, in `map_get_into`).
    pub(crate) fn map_op(&mut self, op: MapOp, map: &Place, args: &[kir::Expr]) -> Result<Value, String> {
        Ok(match op {
            MapOp::Has => {
                let (entry, _) = self.map_find(map, &args[0])?;
                self.b.ins().icmp_imm(IntCC::NotEqual, entry, 0)
            }
            MapOp::Remove => {
                let l = self.map_layout_of(self.place_ty(map));
                let k = self.key_slot(&args[0])?;
                let header = self.place_addr(map)?;
                let kind = self.b.ins().iconst(types::I32, l.key_kind as i64);
                let size = self.b.ins().iconst(types::I64, l.entry_size as i64);
                self.call_id(self.shared.rt.map_remove, &[header, k, kind, size]).expect("returns a bool")
            }
            MapOp::Used => {
                let header = self.place_addr(map)?;
                self.load_i64(header, USED)
            }
            MapOp::Live => {
                let i = self.expr(&args[0])?;
                let i = self.widen_i64(i, args[0].ty);
                let header = self.place_addr(map)?;
                let size = self.map_layout_of(self.place_ty(map)).entry_size;
                self.entry_live(header, i, size)
            }
            MapOp::Lock => {
                let header = self.place_addr(map)?;
                let old = self.load_i64(header, LOCKS);
                let new = self.b.ins().iadd_imm(old, 1);
                self.b.ins().store(MemFlags::trusted(), new, header, LOCKS);
                old
            }
            MapOp::Get => unreachable!("`Get` builds an optional in memory"),
        })
    }

    /// Whether entry `i` is below `used` and still there (a bool).
    fn entry_live(&mut self, header: Value, i: Value, entry_size: u32) -> Value {
        let used = self.load_i64(header, USED);
        let in_range = self.b.ins().icmp(IntCC::UnsignedLessThan, i, used);
        let (check, done) = (self.b.create_block(), self.b.create_block());
        self.b.append_block_param(done, types::I8);
        let no = self.b.ins().iconst(types::I8, 0);
        self.b.ins().brif(in_range, check, &[], done, &[no]);
        self.b.switch_to_block(check);
        let entry = self.entry_ptr(header, i, entry_size);
        let live = self.b.ins().load(types::I8, MemFlags::trusted(), entry, LIVE);
        let live = self.b.ins().icmp_imm(IntCC::NotEqual, live, 0);
        self.b.ins().jump(done, &[live]);
        self.b.switch_to_block(done);
        self.b.block_params(done)[0]
    }

    fn entry_ptr(&mut self, header: Value, i: Value, entry_size: u32) -> Value {
        let entries = self.b.ins().load(self.ptr, MemFlags::trusted(), header, ENTRIES);
        let at = self.b.ins().imul_imm(i, entry_size as i64);
        self.b.ins().iadd(entries, at)
    }

    /// `m[key]` into the optional at `dest`: `none`, or a byte copy of the value.
    pub(crate) fn map_get_into(&mut self, map: &Place, key: &kir::Expr, ty: KTy, dest: Value) -> Result<(), String> {
        let KTy::Optional(v) = ty else { unreachable!("`Get` builds an optional") };
        let (entry, l) = self.map_find(map, key)?;
        let (found, missing, done) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().brif(entry, found, &[], missing, &[]);
        self.b.switch_to_block(missing);
        let zero = self.b.ins().iconst(types::I8, 0);
        self.b.ins().store(MemFlags::trusted(), zero, dest, 0);
        self.b.ins().jump(done, &[]);
        self.b.switch_to_block(found);
        let one = self.b.ins().iconst(types::I8, 1);
        self.b.ins().store(MemFlags::trusted(), one, dest, 0);
        let payload = self.offset(dest, self.shared.kir.payload_offset(v) as i32);
        let value = self.b.ins().iadd_imm(entry, l.val_off as i64);
        self.copy_value(payload, value, self.shared.kir.elem(v));
        self.b.ins().jump(done, &[]);
        self.b.switch_to_block(done);
        Ok(())
    }

    /// Copies a value of type `ty` (scalar or aggregate) from `src` to `dst`.
    fn copy_value(&mut self, dst: Value, src: Value, ty: KTy) {
        if ty.is_aggregate() {
            self.copy_aggregate(dst, src, ty);
        } else {
            let v = self.load_at(ty, src, 0);
            self.store_value(v, dst, 0);
        }
    }

    /// `m[key] = value`: the key, then the value, then the entry. With
    /// `copy_key` (a string that may be a read of a place), a key that adds a
    /// new entry gets its own copy of its bytes in the current arena, like any
    /// string stored somewhere new; a key already there keeps the bytes the
    /// map owns.
    pub(crate) fn map_set(&mut self, map: &Place, key: &kir::Expr, value: &kir::Expr, copy_key: bool, line: u32) -> Result<(), String> {
        let l = self.map_layout_of(self.place_ty(map));
        let k = self.key_slot(key)?;
        let v = if value.ty.is_aggregate() {
            let tmp = self.temp_addr(value.ty);
            self.lower_into(value, tmp)?;
            tmp
        } else {
            self.expr(value)?
        };
        let header = self.place_addr(map)?;
        let kind = self.b.ins().iconst(types::I32, l.key_kind as i64);
        let size = self.b.ins().iconst(types::I64, l.entry_size as i64);
        let align = self.b.ins().iconst(types::I64, l.entry_align as i64);
        let (line, file) = self.loc_args(line);
        let count = if copy_key { Some(self.load_i64(header, COUNT)) } else { None };
        let entry = self.call_id(self.shared.rt.map_insert, &[header, k, kind, size, align, line, file]).expect("returns a pointer");
        if let Some(before) = count {
            self.own_new_key(header, entry, before);
        }
        let slot = self.b.ins().iadd_imm(entry, l.val_off as i64);
        if value.ty.is_aggregate() {
            self.copy_aggregate(slot, v, value.ty);
        } else {
            self.store_value(v, slot, 0);
        }
        Ok(())
    }

    /// If the insert that returned `entry` added it (the map's live count
    /// grew past `before`), its string key gets its own bytes.
    fn own_new_key(&mut self, header: Value, entry: Value, before: Value) {
        let after = self.load_i64(header, COUNT);
        let added = self.b.ins().icmp(IntCC::NotEqual, after, before);
        let (copy, done) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(added, copy, &[], done, &[]);
        self.b.switch_to_block(copy);
        let key = self.b.ins().iadd_imm(entry, KEY);
        let ptr = self.b.ins().load(self.ptr, MemFlags::trusted(), key, 0);
        let len = self.load_i64(key, 8);
        let one = self.b.ins().iconst(types::I64, 1);
        let own = self.call_id(self.shared.rt.dup, &[ptr, len, one]).expect("returns a pointer");
        self.b.ins().store(MemFlags::trusted(), own, key, 0);
        self.b.ins().jump(done, &[]);
        self.b.switch_to_block(done);
    }

    /// Ends a loop over the map: the lock count goes back to what it was.
    pub(crate) fn map_unlock(&mut self, map: &Place, saved: &kir::Expr) -> Result<(), String> {
        let v = self.expr(saved)?;
        let header = self.place_addr(map)?;
        self.b.ins().store(MemFlags::trusted(), v, header, LOCKS);
        Ok(())
    }

    /// The address of entry `at`'s key (or value), for the map whose header
    /// is at `header`. A checked one panics unless `at` is below `used`.
    pub(crate) fn entry_part(&mut self, header: Value, ty: KTy, at: &kir::Expr, value: bool, checked: bool) -> Result<Value, String> {
        let l = self.map_layout_of(ty);
        let i = self.expr(at)?;
        let i = self.widen_i64(i, at.ty);
        if checked {
            let used = self.load_i64(header, USED);
            let bad = self.b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, i, used);
            let (fail, ok) = (self.b.create_block(), self.b.create_block());
            self.b.ins().brif(bad, fail, &[], ok, &[]);
            self.b.switch_to_block(fail);
            self.b.set_cold_block(fail);
            let kind = self.b.ins().iconst(types::I32, PANIC_MAP_CHANGED as i64);
            let (line, file) = self.loc_args(at.line);
            self.call_id(self.shared.rt.panic, &[kind, line, file]);
            self.b.ins().trap(TrapCode::unwrap_user(2));
            self.b.switch_to_block(ok);
        }
        let entry = self.entry_ptr(header, i, l.entry_size);
        Ok(self.b.ins().iadd_imm(entry, if value { l.val_off as i64 } else { KEY }))
    }

    /// Gives the map at `p` its own compact copy of its entries.
    pub(crate) fn map_dup(&mut self, p: &Place) -> Result<(), String> {
        let l = self.map_layout_of(self.place_ty(p));
        let header = self.place_addr(p)?;
        let size = self.b.ins().iconst(types::I64, l.entry_size as i64);
        let align = self.b.ins().iconst(types::I64, l.entry_align as i64);
        self.call_id(self.shared.rt.map_dup, &[header, size, align]);
        Ok(())
    }

    /// `a == b` / `a != b` on strings: same length and bytes.
    pub(crate) fn str_compare(&mut self, op: kir::BinOp, a: &kir::Expr, b: &kir::Expr) -> Result<Value, String> {
        let (x, y) = (self.temp_addr(KTy::Str), self.temp_addr(KTy::Str));
        self.lower_into(a, x)?;
        self.lower_into(b, y)?;
        let mut args = Vec::new();
        for s in [x, y] {
            args.push(self.b.ins().load(self.ptr, MemFlags::trusted(), s, 0));
            args.push(self.load_i64(s, 8));
        }
        let eq = self.call_id(self.shared.rt.str_eq, &args).expect("returns a bool");
        Ok(match op {
            kir::BinOp::Ne => self.b.ins().bxor_imm(eq, 1),
            _ => eq,
        })
    }
}
