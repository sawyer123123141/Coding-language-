//! Method tables: one per (concrete type, interface) pair that an interface value is made from
//! (see `MethodTable`). A table holds the type's id, its copy function, its print function, then
//! the type's own methods in the interface's declaration order. The methods are called as they
//! are: their `self` is already a pointer, which is what an interface value holds.
//!
//! The print function, one per concrete type, writes the value it is given a pointer to. The copy
//! function, one per table, gives a copied interface value its own concrete value (see
//! `iface_copy`).

use std::collections::HashMap;

use kestrel_check::{IfaceId, Tables, Ty};
use kestrel_syntax::ast::Program;

use super::helpers::Helpers;
use super::kty;
use crate::ir::*;

/// The method tables made so far, and what they are made of.
#[derive(Default)]
pub(crate) struct IfaceTables {
    pub(crate) tables: Vec<MethodTable>,
    /// The concrete type of each table, by `TableIdx`.
    concrete: Vec<KTy>,
    by_pair: HashMap<(KTy, IfaceId), TableIdx>,
    /// Each concrete type's id: 1, 2, ... in the order types first get a table.
    type_ids: HashMap<KTy, u32>,
    /// The print function of each concrete type.
    prints: HashMap<KTy, FuncIdx>,
}

impl IfaceTables {
    /// The concrete type of the values table `table` serves.
    pub(crate) fn concrete(&self, table: TableIdx) -> KTy {
        self.concrete[table.0 as usize]
    }
}

impl Helpers {
    /// The method table of concrete type `ty` (a struct or enum that lists `iface`) as `iface`,
    /// made at its first use.
    pub(crate) fn table_for(&mut self, ty: Ty, iface: IfaceId, t: &Tables, program: &Program) -> TableIdx {
        let concrete = kty(ty);
        if let Some(&idx) = self.ifaces.by_pair.get(&(concrete, iface)) {
            return idx;
        }
        // An instance's methods are its declaration's generic originals: each is called as its
        // copy for the instance's types, which the collection pass made (one table per instance).
        let (own, args) = match ty {
            Ty::Struct(s) => (&t.strukt(s).methods, t.strukt(s).instance.map(|i| i.args)),
            Ty::Enum(e) => (&t.enm(e).methods, t.enm(e).instance.map(|i| i.args)),
            other => unreachable!("only structs and enums implement interfaces, not {other:?}"),
        };
        assert!(t.is_concrete(ty), "a generic type reached KIR unsubstituted; this is a compiler bug");
        let methods = t
            .iface(iface)
            .methods
            .iter()
            .map(|&m| {
                let name = &t.func(m).decl(program).name;
                let own = *own.get(name).unwrap_or_else(|| panic!("conformance guarantees a method `{name}`"));
                let f = match args {
                    Some(args) => *t.generic.fn_copies.get(&(own, args)).unwrap_or_else(|| {
                        panic!("the collection pass missed `{}.{name}` in a method table; this is a compiler bug", t.ty_name(ty))
                    }),
                    None => own,
                };
                FuncIdx(f.0)
            })
            .collect();
        let next_id = self.ifaces.type_ids.len() as u32 + 1;
        let type_id = *self.ifaces.type_ids.entry(concrete).or_insert(next_id);
        let print = self.iface_print_fn(concrete, t);
        let copy = self.iface_copy_fn(ty, iface, t);
        let idx = TableIdx(self.ifaces.tables.len() as u32);
        self.ifaces.tables.push(MethodTable { type_id, copy, print, methods });
        self.ifaces.concrete.push(concrete);
        self.ifaces.by_pair.insert((concrete, iface), idx);
        idx
    }

    /// `print(value)` without a newline for the concrete type `ty`, its value taken by pointer.
    fn iface_print_fn(&mut self, ty: KTy, t: &Tables) -> FuncIdx {
        if let Some(&f) = self.ifaces.prints.get(&ty) {
            return f;
        }
        self.ensure_print(ty, t);
        let idx = self.reserve();
        self.ifaces.prints.insert(ty, idx);
        let value = Expr { kind: ExprKind::Load(Place::local(LocalIdx(0))), ty, line: 0 };
        self.funcs[(idx.0 - self.base) as usize] = Func {
            name: format!("__kestrel_iface_print_{}", idx.0),
            params: vec![LocalIdx(0)],
            ret: None,
            locals: vec![Local { name: "value".into(), ty, kind: LocalKind::RefParam }],
            body: vec![Stmt::Write(vec![PrintArg::Value(value)])],
            is_pure: false,
            io: true,
            mutates_params: false,
            exported: false,
            lines: (0, 0),
            native: None,
        };
        idx
    }
}
