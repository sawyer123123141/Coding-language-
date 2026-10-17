//! List methods (`add`, `remove_at`, `pop`, `insert`, `clear`, `has` and `find`), and the loop
//! check that keeps a list from changing length while a `for ... in` loop runs over it.

use kestrel_syntax::ast::{Arg, Expr, ExprKind};

use super::BodyChecker;
use crate::tables::{ListMethod, LocalId, Res};
use crate::ty::Ty;

/// A step into a place: a field's name, or `None` for an index (which may be any index).
pub(crate) type Step = Option<String>;

/// A `for ... in` loop over a list or an array that is being checked.
pub(crate) struct LoopList {
    /// The loop's item, which stands for one element of the list.
    pub(crate) item: LocalId,
    /// The variable the list belongs to, aliases followed.
    pub(crate) root: LocalId,
    /// The list's place inside `root`.
    pub(crate) path: Vec<Step>,
}

/// Whether the list at `changed` is the list at `looped`, or holds it: `changed` is `looped` or
/// a start of it, an index standing for any index. A list inside an element of the looped
/// list (`row` while looping over `grid`) is a different list, and is free to change.
pub(crate) fn covers(changed: &[Step], looped: &[Step]) -> bool {
    changed.len() <= looped.len() && changed.iter().zip(looped).all(|(a, b)| a.is_none() || b.is_none() || a == b)
}

/// How a list method is written, to tell `add` from the rest.
#[derive(Clone, Copy)]
enum Which {
    Add,
    Other(ListMethod),
}

impl<'a> BodyChecker<'a> {
    /// The root variable and steps of a place, with a loop's item standing for an element of its
    /// list. Call after the expression has been checked.
    pub(crate) fn list_place(&self, e: &Expr) -> Option<(LocalId, Vec<Step>)> {
        match &e.kind {
            ExprKind::Ident(_) => {
                let Some(&Res::Local(l)) = self.t.res.get(&e.id) else { return None };
                if let Some(x) = self.for_in_lists.iter().rev().find(|x| x.item == l) {
                    let mut path = x.path.clone();
                    path.push(None);
                    return Some((x.root, path));
                }
                // A `match` binding or `let ... else` view stands for a part of the place it reads.
                if let Some(p) = self.removing.views.get(&l) {
                    return Some(p.clone());
                }
                Some((self.t.local(l).alias.unwrap_or(l), Vec::new()))
            }
            ExprKind::Field { .. } if self.t.swizzles.contains_key(&e.id) => None,
            ExprKind::Field { base, .. } if matches!(self.t.ty(base.id), Ty::Array(_) | Ty::List(_) | Ty::Map(..) | Ty::Str) => None,
            ExprKind::Field { base, name, .. } => {
                let (root, mut path) = self.list_place(base)?;
                path.push(Some(name.clone()));
                Some((root, path))
            }
            ExprKind::Index { base, .. } => {
                let (root, mut path) = self.list_place(base)?;
                path.push(None);
                Some((root, path))
            }
            _ => None,
        }
    }

    /// The innermost enclosing `for ... in` loop (an index into `for_in_lists`) whose list is
    /// the one at `place`, or holds it: changing the length of `place` there would shorten,
    /// move or drop the items the loop is reading.
    pub(crate) fn overlapping_loop(&self, place: &Expr) -> Option<usize> {
        let (root, path) = self.list_place(place)?;
        self.for_in_lists.iter().rposition(|x| x.root == root && covers(&path, &x.path))
    }

    /// Pushes the loop over `iter`, whose item is `item`, if `iter` is a place.
    pub(crate) fn enter_list_loop(&mut self, item: LocalId, iter: &Expr) {
        if let Some((root, path)) = self.list_place(iter) {
            self.for_in_lists.push(LoopList { item, root, path });
        }
    }

    /// Pops the loop `enter_list_loop` pushed, if it did.
    pub(crate) fn leave_list_loop(&mut self, item: LocalId) {
        if self.for_in_lists.last().is_some_and(|x| x.item == item) {
            self.for_in_lists.pop();
        }
    }

    /// The list a loop reads, as the loop's error names it: its variable and fields, up to the
    /// first index.
    pub(crate) fn loop_list_name(&self, i: usize) -> String {
        let x = &self.for_in_lists[i];
        let mut name = self.t.local(x.root).name.clone();
        for step in x.path.iter().map_while(Option::as_ref) {
            name.push_str(&format!(".{step}"));
        }
        name
    }

    /// An index argument: any integer type.
    pub(crate) fn check_int_index(&mut self, index: &Expr) {
        let ty = self.check_expr(index, None);
        let ty = self.reject_never(index, ty);
        if !ty.is_int() && ty != Ty::Error {
            let msg = format!("an index must be an integer, found {}", self.tn(ty));
            self.error(index.span, msg);
        }
    }

    /// `xs.add(v)`, `xs.remove_at(i)`, `xs.pop()`, `xs.insert(i, v)`, `xs.clear()`, `xs.has(v)`
    /// and `xs.find(v)`. Arrays only take the last two. The ones that change `xs` need it to be
    /// changeable.
    pub(crate) fn list_method(&mut self, e: &Expr, receiver: &Expr, recv_ty: Ty, args: &[Arg]) -> Ty {
        let ExprKind::MethodCall { name, name_span, .. } = &e.kind else { unreachable!("not a method call") };
        let elem = self.elem_of(recv_ty).expect("a collection type");
        let which = match name.as_str() {
            "add" => Which::Add,
            "remove_at" => Which::Other(ListMethod::RemoveAt),
            "pop" => Which::Other(ListMethod::Pop),
            "insert" => Which::Other(ListMethod::Insert),
            "clear" => Which::Other(ListMethod::Clear),
            "has" => Which::Other(ListMethod::Has),
            "find" => Which::Other(ListMethod::Find),
            "join" => Which::Other(ListMethod::Join),
            _ => {
                let help = match recv_ty {
                    // A slip for `remove_at`, or for the statement `remove b` in a loop.
                    Ty::List(_) if matches!(name.as_str(), "remove" | "erase") => {
                        let item = match args.first().map(|a| &a.value.kind) {
                            Some(ExprKind::Ident(b)) => b.clone(),
                            _ => "b".to_string(),
                        };
                        let xs = written(receiver);
                        format!("use `{xs}.remove_at(i)`, or `remove {item}` inside `for {item} in {xs}`")
                    }
                    Ty::List(_) => "lists have `add`, `remove_at`, `pop`, `insert`, `clear`, `has`, `find`, `join` and `.len`".to_string(),
                    _ => "arrays have `has`, `find`, `join` and `.len`".to_string(),
                };
                let msg = format!("`{}` has no method `{name}`", self.tn(recv_ty));
                self.error_help(*name_span, msg, help);
                self.check_loose_args(args);
                return Ty::Error;
            }
        };
        let changes = match which {
            Which::Add => true,
            Which::Other(m) => m.changes_receiver(),
        };
        if changes && matches!(recv_ty, Ty::Array(_)) {
            let msg = match which {
                Which::Add => format!("arrays have a fixed length; use a `list[{}]` to add items", self.tn(elem)),
                Which::Other(_) => format!("arrays have a fixed length; use a `list[{}]` if items come and go", self.tn(elem)),
            };
            self.error(*name_span, msg);
            self.check_loose_args(args);
            return Ty::Error;
        }
        let Which::Other(method) = which else { return self.list_add(e, receiver, args, elem) };
        if matches!(method, ListMethod::Has | ListMethod::Find) && !(elem.is_numeric() || matches!(elem, Ty::Bool | Ty::Str | Ty::Error)) {
            let msg = format!(
                "`{name}` needs items that `==` can compare (numbers, bools and strings); `{}` holds `{}`",
                self.tn(recv_ty),
                self.tn(elem)
            );
            self.error(*name_span, msg);
            self.check_loose_args(args);
            return Ty::Error;
        }
        if method == ListMethod::Join && !matches!(elem, Ty::Str | Ty::Error) {
            let msg = format!("`join` needs a list of strings, found `{}`", self.tn(recv_ty));
            self.error(*name_span, msg);
            self.check_loose_args(args);
            return Ty::Error;
        }
        let usage = match method {
            ListMethod::RemoveAt => ("`remove_at` takes one index", 1),
            ListMethod::Pop => ("`pop` takes no arguments", 0),
            ListMethod::Insert => ("`insert` takes an index and a value", 2),
            ListMethod::Clear => ("`clear` takes no arguments", 0),
            ListMethod::Has => ("`has` takes one value", 1),
            ListMethod::Find => ("`find` takes one value", 1),
            ListMethod::Join => ("`join` takes one value", 1),
        };
        if args.len() != usage.1 || args.iter().any(|a| a.label.is_some() || a.is_var) {
            self.error(e.span, usage.0);
            self.check_loose_args(args);
            return Ty::Error;
        }
        self.t.res.insert(e.id, Res::List(method));
        if method.changes_receiver() && self.receiver_root(receiver, name).is_some() {
            if let Some(i) = self.overlapping_loop(receiver) {
                let (list, item) = (self.loop_list_name(i), self.t.local(self.for_in_lists[i].item).name.clone());
                let msg = format!(
                    "can't change how many items `{list}` has while looping over it; use `remove {item}` to remove the loop's item, or change `{list}` after the loop"
                );
                self.error(e.span, msg);
            }
        }
        match method {
            ListMethod::RemoveAt => self.check_int_index(&args[0].value),
            ListMethod::Insert => {
                self.check_int_index(&args[0].value);
                self.check_element(&args[1].value, Some(elem));
                self.check_store(receiver, &args[1].value);
            }
            ListMethod::Has | ListMethod::Find => {
                self.check_value(&args[0].value, Some(elem));
            }
            ListMethod::Join => {
                self.check_value(&args[0].value, Some(Ty::Str));
            }
            ListMethod::Pop => self.check_pop_owner(e, receiver, elem),
            ListMethod::Clear => {}
        }
        match method {
            ListMethod::Pop => Ty::Optional(self.t.intern_elem(elem)),
            ListMethod::Find => Ty::Optional(self.t.intern_elem(Ty::Int)),
            // Lowering searches with `find`, whose `int?` needs its element type interned.
            ListMethod::Has => {
                self.t.intern_elem(Ty::Int);
                Ty::Bool
            }
            ListMethod::Join => Ty::Str,
            ListMethod::RemoveAt | ListMethod::Insert | ListMethod::Clear => Ty::Unit,
        }
    }

    /// `xs.add(v)`: `xs` must be changeable. Adding while looping over `xs` is fine: the loop
    /// reads the length once.
    fn list_add(&mut self, e: &Expr, receiver: &Expr, args: &[Arg], elem: Ty) -> Ty {
        self.t.res.insert(e.id, Res::ListAdd);
        self.receiver_root(receiver, "add");
        match args {
            [a] if a.label.is_none() && !a.is_var => {
                self.check_element(&a.value, Some(elem));
                self.check_store(receiver, &a.value);
            }
            _ => {
                self.error(e.span, "`add` takes one value");
                self.check_loose_args(args);
            }
        }
        Ty::Unit
    }

    /// A kept `pop` of owning items moves the item out without copying it (a non-owning item is
    /// copied into the current arena, like an indexed read). So it can't take the item from
    /// inside a value that holds an `Arena`, or from a list the caller passed in: the item's
    /// memory may be in an owner's arena, which goes when the owner is replaced or cleared. Only
    /// a list rooted in a local of this function hands one out. A discarded `pop` drops the item
    /// at once.
    fn check_pop_owner(&mut self, e: &Expr, receiver: &Expr, elem: Ty) {
        if !self.t.is_owning(elem) || self.discarded == Some(e.id) {
            return;
        }
        let list = written(receiver);
        if let Some(owner) = self.arena_owner(receiver) {
            return self.error_help(
                e.span,
                format!("can't take an item out of `{list}` with `pop`: it may point into `{owner}`'s arena"),
                format!("read what you need with `{list}[i]`, then use `remove_at`"),
            );
        }
        // A list the caller passed in (a `var` parameter or `var self`, or anything reached
        // through one) may be inside an owner of the caller's, which it can replace while the
        // item lives.
        let from_caller = self.list_place(receiver).is_some_and(|(root, _)| self.t.local(root).is_param);
        if from_caller {
            self.error_help(
                e.span,
                format!("can't take an item out of `{list}` with `pop`: it may point into the caller's arena"),
                "return the index and let the caller use `pop` or `remove_at`",
            );
        }
    }

    /// The outermost value holding an `Arena` that the place `e` is inside of (not `e` itself),
    /// as written. A loop's item or a view stands for the place it reads.
    fn arena_owner(&self, e: &Expr) -> Option<String> {
        match &e.kind {
            ExprKind::Field { .. } if self.t.swizzles.contains_key(&e.id) => None,
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => {
                self.arena_owner(base).or_else(|| self.has_arena_field(self.t.ty(base.id)).then(|| written(base)))
            }
            ExprKind::Ident(_) => {
                let (root, path) = self.list_place(e)?;
                let mut ty = self.t.local(root).ty;
                let mut name = self.t.local(root).name.clone();
                for step in &path {
                    if self.has_arena_field(ty) {
                        return Some(name);
                    }
                    ty = match step {
                        Some(f) => {
                            name.push_str(&format!(".{f}"));
                            self.field_ty(ty, f)?
                        }
                        None => {
                            name.push_str("[_]");
                            self.elem_of(ty)?
                        }
                    };
                }
                None
            }
            _ => None,
        }
    }

    /// Whether a value of `ty` has an `Arena` field of its own.
    fn has_arena_field(&self, ty: Ty) -> bool {
        match ty {
            Ty::Struct(s) => self.t.strukt(s).fields.iter().any(|f| f.ty == Ty::Arena),
            Ty::Variant(en, v) => self.t.enm(en).variants[v as usize].fields.iter().any(|f| f.ty == Ty::Arena),
            Ty::Enum(en) => self.t.enm(en).variants.iter().flat_map(|v| &v.fields).any(|f| f.ty == Ty::Arena),
            _ => false,
        }
    }

    /// The type of field `name` of a struct or variant.
    fn field_ty(&self, ty: Ty, name: &str) -> Option<Ty> {
        let fields = match ty {
            Ty::Struct(s) => &self.t.strukt(s).fields,
            Ty::Variant(en, v) => &self.t.enm(en).variants[v as usize].fields,
            _ => return None,
        };
        fields.iter().find(|f| f.name == name).map(|f| f.ty)
    }
}

/// A place as the user wrote it, for a message: `levels[0].items`. An index that isn't a
/// number or a name shows as `_`, and a call as `f(...)`.
fn written(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Ident(n) => n.clone(),
        ExprKind::Field { base, name, .. } => format!("{}.{name}", written(base)),
        ExprKind::Index { base, index } => {
            let at = match &index.kind {
                ExprKind::Int(n) => n.to_string(),
                ExprKind::Ident(n) => n.clone(),
                _ => "_".to_string(),
            };
            format!("{}[{at}]", written(base))
        }
        ExprKind::Call { name, .. } => format!("{name}(...)"),
        ExprKind::MethodCall { receiver, name, .. } => format!("{}.{name}(...)", written(receiver)),
        _ => "_".to_string(),
    }
}
