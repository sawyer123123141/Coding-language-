//! The sentences for what a call, a list operation or a string operation does: each candidate's
//! allocation and way to fail, checked against its arm in codegen and the runtime function it
//! calls.

use super::{Effect, Unknown, Walk, What};
use crate::range::summary::Part;
use crate::range::show::{expr_text, place_text, type_name};
use crate::range::MAX_ITEMS;
use crate::ir::*;

/// The limit on a list's or array's length, as the report writes it.
pub(super) fn limit() -> String {
    crate::range::show::number(MAX_ITEMS)
}

impl Walk<'_> {
    /// A closure literal is made: its code counts as called, unless nothing can call it.
    pub(super) fn closure(&mut self, e: &Expr, code: FuncIdx) {
        if !self.unused.contains(&(e as *const Expr as usize)) {
            self.push(e.line, What::Closure(code));
        }
    }

    /// The events of a statement that copies, makes an arena, frees one or sets a map key.
    pub(super) fn memory_stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::CopyFnEnv(p) => {
                self.place_walk(p);
                let at = self.place(p);
                self.alloc(0, Some(format!("copying the function value {at} makes a new environment for what it captured")), format!("copies the function value {at}"), "");
            }
            Stmt::CopyIface(p) => {
                self.place_walk(p);
                let at = self.place(p);
                self.alloc(0, Some(format!("copying the interface value {at} makes a new copy of its data")), format!("copies the interface value {at}"), "");
            }
            // (An arena the compiler pushes has no line: the scratch arena of a native function's text,
            // already part of the native's own item.)
            Stmt::PushArena(None, line) if *line != 0 => {
                self.alloc(*line, Some("`arena { }` makes a new arena".to_string()), "makes a new arena for `arena { }`".to_string(), "");
            }
            Stmt::PushArena(Some(p), line) if *line != 0 => {
                self.place_walk(p);
                let at = self.place(p);
                self.alloc(*line, Some(format!("`in {at} {{ }}` makes the arena the first time")), format!("enters `in {at} {{ }}`"), " and makes its arena the first time");
            }
            Stmt::FreeArena(p) => {
                self.place_walk(p);
                let at = self.place(p);
                let tail = " and fails if it is being allocated into";
                self.fail(0, Some(format!("freeing the arena at {at} fails if it is being allocated into")), format!("frees the arena at {at}"), tail);
            }
            Stmt::Dup(p) => {
                self.place_walk(p);
                let at = self.place(p);
                self.alloc(0, Some(format!("makes a copy of {at}")), format!("copies {at}"), "");
            }
            Stmt::MapSet(p, key, value, _) => {
                self.expr(key);
                self.expr(value);
                self.place_walk(p);
                let at = self.place(p);
                self.alloc(key.line, Some(format!("setting a key of {at} may grow the map")), format!("sets a key of {at}"), " and may grow the map");
                // A new key while looping over the map is a logic failure; too many keys is running
                // out of resources. The report prints them together.
                let logic = format!("setting a key of {at} fails if it adds a new key while looping over the map");
                self.fail_part(key.line, Some(logic), format!("sets a key of {at}"), " and fails if it adds a new key while looping over it", Part::Logic);
                let direct = format!("setting a key of {at} fails if it adds a new key while looping over the map, or if the map would hold more than 2,147,483,647 keys");
                let tail = " and fails if it adds a new key while looping over it, or if the map would hold more than 2,147,483,647 keys";
                self.fail_part(key.line, Some(direct), format!("sets a key of {at}"), tail, Part::Resource);
            }
            Stmt::Trap => {
                self.fail(0, Some("calling an interface method with no body stops the program".to_string()), "calls an interface method with no body".to_string(), "");
            }
            Stmt::Fail(msg) => {
                self.fail(0, Some(format!("stops the program with the error \"{msg}\"")), "stops the program with an error".to_string(), "");
            }
            _ => {}
        }
    }

    pub(super) fn call(&mut self, g: FuncIdx, line: u32) {
        let callee = &self.m.funcs[g.0 as usize];
        if callee.native.is_some() {
            self.unknown(line, Unknown::Native, format!("calls the native function `{}`", callee.name));
            self.effect(line, Effect::of_native(&callee.name), format!("calls `{}`", callee.name));
        } else if self.m.init != Some(g) {
            // (The constants are built once, before `main`: not a cost of the call.)
            self.push(line, What::Call(g));
        }
    }

    /// A call through the function value at `callee`, whose type is `pure` or not.
    pub(super) fn indirect(&mut self, callee: &Place, line: u32, pure: bool) {
        let name = place_text(self.m, self.f, callee, callee.path.len());
        let via = if name.starts_with("(...)") { "calls a function value".to_string() } else { format!("calls the function value `{name}`") };
        self.unknown(line, Unknown::Indirect, via.clone());
        if !pure {
            self.effect(line, Effect::ImpureFnValue, via);
        }
    }

    /// A call through the interface value at `recv` to `method` (the interface's own), which
    /// promises `keeps` and is `pure` or not.
    pub(super) fn virtual_call(&mut self, recv: &Place, line: u32, keeps: ContractSet, method: FuncIdx, pure: bool) {
        let via = format!("calls a method of the interface value `{}`", place_text(self.m, self.f, recv, recv.path.len()));
        self.unknown(line, Unknown::Virtual(keeps, method), via.clone());
        if !pure && !keeps.contains(ContractSet::DETERMINISTIC) {
            self.effect(line, Effect::ImpureMethod(method), via);
        }
    }

    pub(super) fn list_add(&mut self, p: &Place, line: u32) {
        let (xs, limit) = (self.place(p), limit());
        let fail_tail = format!(" and fails if the list would hold more than {limit} items");
        if let Some(name) = self.builtin_of(p.root).filter(|n| matches!(*n, "filter" | "map")) {
            let via = format!("builds a new list with `{name}`");
            self.alloc(line, Some(format!("`{name}` builds a new list")), via.clone(), "");
            self.fail_part(line, Some(format!("`{name}` fails if its list would hold more than {limit} items")), via, &fail_tail, Part::Resource);
            return;
        }
        self.alloc(line, Some(format!("`add` may grow {xs}")), format!("adds to {xs}"), " and may grow the list");
        self.fail_part(line, Some(format!("`add` fails if {xs} would hold more than {limit} items")), format!("adds to {xs}"), &fail_tail, Part::Resource);
    }

    pub(super) fn list_op(&mut self, op: ListOp, p: &Place, xs: &[Expr]) {
        let (list, limit) = (self.place(p), limit());
        let line = xs.first().map_or(0, |x| x.line);
        let index = quoted(xs.first().map_or(String::new(), |x| expr_text(self.m, self.f, x)), "the index");
        match op {
            ListOp::RemoveAt => {
                let direct = format!("`remove_at` fails if {index} is outside {list}");
                self.fail(line, Some(direct), format!("removes from {list}"), " and fails if the index is outside it");
            }
            ListOp::Insert => {
                self.alloc(line, Some(format!("`insert` may grow {list}")), format!("inserts into {list}"), " and may grow the list");
                // An index outside the list is a logic failure; too many items is running out of
                // resources. The report prints them together.
                let named = quoted(xs.first().map_or(String::new(), |x| self.expr_name(x)), "the index");
                let logic = format!("`insert` fails if {named} is outside {list}");
                self.fail_part(line, Some(logic), format!("inserts into {list}"), " and fails if the index is outside it", Part::Logic);
                let direct = format!("`insert` fails if {index} is outside {list} or {list} would hold more than {limit} items");
                let tail = format!(" and fails if the index is outside it or the list would hold more than {limit} items");
                self.fail_part(line, Some(direct), format!("inserts into {list}"), &tail, Part::Resource);
            }
            ListOp::Sort | ListOp::SortBy(_) => {
                let name = if xs.len() > 1 { "sort_by" } else { "sort" };
                self.alloc(line, Some(format!("`{name}` makes a buffer to sort {list} in")), format!("sorts {list} with `{name}`"), " and makes a buffer to sort in");
                // The comparison (or the key function) runs the key, which may allocate and fail.
                if let Some(Expr { kind: ExprKind::FnRef(compare), .. }) = xs.first() {
                    self.push(line, What::Call(*compare));
                }
            }
            ListOp::Clear | ListOp::Compact => {}
        }
    }

    /// The events of a string operation.
    pub(super) fn str_op(&mut self, op: StrOp, xs: &[Expr], line: u32) {
        let names: Vec<String> = xs.iter().map(|x| self.expr_name(x)).collect();
        let text = |i: usize| names.get(i).cloned().unwrap_or_default();
        let makes = |what: &str| (format!("`{what}` makes a new string"), format!("makes a new string with `{what}`"));
        let (direct, via) = match op {
            StrOp::At => (format!("`{}[{}]` makes a new string", text(0), text(1)), "indexes a string into a new string".to_string()),
            StrOp::Piece => ("cutting a piece out of a string makes a new string".to_string(), "cuts a piece out of a string".to_string()),
            StrOp::Slice => (format!("`{}.slice({}, {})` makes a new string", text(0), text(1), text(2)), "slices a string into a new string".to_string()),
            StrOp::Trim => makes("trim"),
            StrOp::Upper => makes("upper"),
            StrOp::Lower => makes("lower"),
            StrOp::Join => makes("join"),
            StrOp::Split => (format!("`{}.split({})` makes a new list", text(0), text(1)), "splits a string into a new list".to_string()),
            StrOp::Chars | StrOp::CharLen | StrOp::Find | StrOp::Starts | StrOp::Cmp | StrOp::Ends | StrOp::ToInt | StrOp::ToFloat => return,
        };
        self.alloc(line, Some(direct), via, "");
        match op {
            StrOp::At => {
                let direct = format!("`{}[{}]` fails if `{}` is outside `{}`", text(0), text(1), text(1), text(0));
                self.fail(line, Some(direct), "indexes a string".to_string(), " and fails if the index is outside it");
            }
            StrOp::Slice => {
                let direct = format!("`{}.slice({}, {})` fails unless the range is inside `{}`", text(0), text(1), text(2), text(0));
                self.fail(line, Some(direct), "slices a string".to_string(), " and fails unless the range is inside it");
            }
            StrOp::Split => {
                let sep = quoted(text(1), "the separator");
                let direct = format!("`{}.split({})` fails if {sep} is empty or the list would hold more than {} items", text(0), text(1), limit());
                let tail = format!(" and fails if the separator is empty or the list would hold more than {} items", limit());
                self.fail(line, Some(direct), "splits a string".to_string(), &tail);
            }
            _ => {}
        }
    }

}

/// The type of the place `p` in `f`.
pub(super) fn place_ty(m: &Module, f: &Func, p: &Place) -> KTy {
    let mut ty = f.locals[p.root.0 as usize].ty;
    for proj in &p.path {
        ty = match (proj, ty) {
            (Proj::Field(i), KTy::Struct(s)) => m.layout(s).fields[*i as usize].ty,
            (Proj::Variant(v, i), KTy::Enum(en)) => m.enum_layout(en).variants[*v as usize].fields[*i as usize].ty,
            (Proj::Payload, KTy::Optional(el)) => m.elem(el),
            (Proj::Index { .. }, KTy::Array(el) | KTy::List(el)) => m.elem(el),
            (Proj::MapKey { .. }, KTy::Map(k, _)) => m.elem(k),
            (Proj::MapValue { .. }, KTy::Map(_, v)) => m.elem(v),
            (Proj::Lane(_), KTy::F32X4 | KTy::I32X4) => ty.lane(),
            _ => return ty,
        };
    }
    ty
}

/// An element type as the report writes it.
pub(super) fn elem_text(m: &Module, ty: KTy) -> String {
    match ty {
        KTy::Struct(s) => m.layout(s).name.clone(),
        KTy::Enum(e) => m.enum_layout(e).name.clone(),
        KTy::Str => "string".to_string(),
        KTy::List(el) => format!("list[{}]", elem_text(m, m.elem(el))),
        KTy::Array(el) => format!("[{}]", elem_text(m, m.elem(el))),
        other => type_name(other).to_string(),
    }
}

/// `text` in backticks, or `unnamed` for a value the compiler made (`(...)`).
pub(super) fn quoted(text: String, unnamed: &str) -> String {
    if text.is_empty() || text == "(...)" {
        unnamed.to_string()
    } else {
        format!("`{text}`")
    }
}
