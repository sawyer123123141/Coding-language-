//! The terms facts are about: tracked integer locals, lengths of stable places, and loops' hidden
//! ends. A place is stable when its path is fields, enum variants, optional payloads and indexes
//! that are an integer constant or a tracked local; `PlaceKey` is its hashable form. Each place
//! belongs to a variable, its home (a loop element belongs to its list's variable), and a write
//! of one place may change, or must change, another's length (see the spec's "What a write
//! forgets").

use std::collections::{HashMap, HashSet};

use crate::ir::*;
use crate::visit::{walk, Node};

/// The most terms a function's facts are kept over (the spec's pre-scan picks them).
pub const MAX_TERMS: usize = 64;

/// One step of a stable place's path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Step {
    Field(u32),
    Variant(u32, u32),
    Payload,
    /// An index that is this integer constant.
    Const(i64),
    /// An index that is this tracked local's value.
    Local(LocalIdx),
}

/// A stable place, hashable: the same root and path is the same place, as long as none of the
/// index locals changed.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlaceKey {
    pub root: LocalIdx,
    pub path: Vec<Step>,
}

impl PlaceKey {
    /// Whether one of the path's indexes is `l`.
    pub fn indexed_by(&self, l: LocalIdx) -> bool {
        self.path.contains(&Step::Local(l))
    }
}

/// Something facts are about.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Term {
    /// A tracked integer local.
    Local(LocalIdx),
    /// The length of the list or array at a stable place.
    Len(PlaceKey),
    /// A `for` range's end, worked out once each time the loop is entered: the `n`-th `ForRange`
    /// in walk order.
    Hidden(u32),
}

/// Whether `l` holds its own integer value (a `Value` or `Addressed` integer local): only those
/// have ranges and can be terms.
pub fn tracked(f: &Func, l: LocalIdx) -> bool {
    f.locals.get(l.0 as usize).is_some_and(|x| x.ty.is_int() && matches!(x.kind, LocalKind::Value | LocalKind::Addressed))
}

/// The tracked local whose value `e` is: a load of one, or a widening conversion of one.
pub fn local_value(f: &Func, e: &Expr) -> Option<LocalIdx> {
    match &e.kind {
        ExprKind::Load(p) if p.path.is_empty() && tracked(f, p.root) => Some(p.root),
        ExprKind::Cast(x) if x.ty.is_int() && e.ty.is_int() && !super::sites::is_narrowing(x.ty, e.ty) => local_value(f, x),
        _ => None,
    }
}

/// The stable form of the first `upto` steps of `p`, or `None` when one of them isn't stable.
pub fn key_of(f: &Func, p: &Place, upto: usize) -> Option<PlaceKey> {
    let mut path = Vec::with_capacity(upto);
    for proj in &p.path[..upto] {
        path.push(match proj {
            Proj::Field(i) => Step::Field(*i),
            Proj::Variant(v, i) => Step::Variant(*v, *i),
            Proj::Payload => Step::Payload,
            Proj::Index { at, .. } => match &at.kind {
                ExprKind::Int(c) => Step::Const(*c),
                _ => Step::Local(local_value(f, at)?),
            },
            Proj::MapKey { .. } | Proj::MapValue { .. } => return None,
        });
    }
    Some(PlaceKey { root: p.root, path })
}

/// The type of the value at the first `upto` steps of `p`.
pub fn place_ty(m: &Module, f: &Func, p: &Place, upto: usize) -> KTy {
    let mut ty = f.locals[p.root.0 as usize].ty;
    for proj in &p.path[..upto] {
        ty = match (proj, ty) {
            (Proj::Field(fi), KTy::Struct(s)) => m.layout(s).fields[*fi as usize].ty,
            (Proj::Variant(v, fi), KTy::Enum(e)) => m.enum_layout(e).variants[*v as usize].fields[*fi as usize].ty,
            (Proj::Index { .. }, KTy::Array(el) | KTy::List(el)) | (Proj::Payload, KTy::Optional(el)) => m.elem(el),
            (Proj::MapKey { .. }, KTy::Map(k, _)) => m.elem(k),
            (Proj::MapValue { .. }, KTy::Map(_, v)) => m.elem(v),
            _ => return KTy::I64,
        };
    }
    ty
}

/// The key of the list or array at the first `upto` steps of `p`, when it is stable (a string's
/// length is no term).
pub fn list_key(m: &Module, f: &Func, p: &Place, upto: usize) -> Option<PlaceKey> {
    match place_ty(m, f, p, upto) {
        KTy::List(_) | KTy::Array(_) => key_of(f, p, upto),
        _ => None,
    }
}

/// A step of a path as far as overlapping goes: every index is any index.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    Field(u32),
    Variant,
    Payload,
    Elem,
}

/// Whether two steps at the same depth may be the same memory. Two variants of one enum share
/// bytes; any index may equal any other; steps of different kinds can't meet at one depth, but are
/// said to overlap anyway.
fn steps_meet(a: Shape, b: Shape) -> bool {
    match (a, b) {
        (Shape::Field(x), Shape::Field(y)) => x == y,
        _ => true,
    }
}

/// What each loop element (a `Bind` local) points into, for finding every place's home.
pub struct Homes {
    /// The place each bound local points at; `None` for a local bound to two different places.
    bound: HashMap<LocalIdx, Option<Place>>,
    /// The parameters of a `par for` body, which may point into one another (the caller passes
    /// pointers to its variables, and a variable may be a loop element of another).
    par_params: HashSet<LocalIdx>,
}

/// The variable a place belongs to, and its path from there.
#[derive(Clone, Debug)]
pub struct Full {
    /// `None`: it may be any variable's.
    home: Option<LocalIdx>,
    path: Vec<Shape>,
}

impl Homes {
    pub fn of(f: &Func, par_body: bool) -> Homes {
        let mut bound: HashMap<LocalIdx, Option<Place>> = HashMap::new();
        walk(&f.body, &mut |n| {
            // A loop element points into its list; a local `inline_small` moved a lent interface
            // argument into points at the lent place (a `var self` call through it changes that
            // place: see `writes.rs`).
            let (l, p, lent) = match n {
                Node::Stmt(Stmt::Bind(l, p)) => (l, p, false),
                Node::Stmt(Stmt::Let(l, Expr { kind: ExprKind::MakeIface { data: IfaceData::Borrowed(p), .. }, .. })) => (l, p, true),
                _ => return,
            };
            match bound.get(l) {
                None => {
                    bound.insert(*l, Some(p.clone()));
                }
                Some(Some(q)) if q == p => {}
                Some(_) => {
                    debug_assert!(lent, "`{}` is bound to two different places", f.locals[l.0 as usize].name);
                    bound.insert(*l, None);
                }
            }
        });
        let par_params = if par_body { f.params.iter().copied().collect() } else { HashSet::new() };
        Homes { bound, par_params }
    }

    /// The variable `root` belongs to: following each `Bind` to the place it points into.
    /// `None` when it may be any variable's (a local bound to two places).
    pub fn home(&self, root: LocalIdx) -> Option<LocalIdx> {
        self.full(root, Vec::new()).home
    }

    /// Whether `l` is `root` or a loop element `root` points into (directly or not): a new
    /// value of `l`, or pointing it elsewhere, changes what `root` is.
    pub fn through(&self, root: LocalIdx, l: LocalIdx) -> bool {
        let mut at = root;
        for _ in 0..64 {
            if at == l {
                return true;
            }
            match self.bound.get(&at) {
                Some(Some(p)) => at = p.root,
                Some(None) => return true,
                None => return false,
            }
        }
        true
    }

    fn full(&self, root: LocalIdx, tail: Vec<Shape>) -> Full {
        let mut path = tail;
        let mut at = root;
        for _ in 0..64 {
            match self.bound.get(&at) {
                None => return Full { home: Some(at), path },
                Some(None) => break,
                Some(Some(p)) => {
                    let mut front: Vec<Shape> = p.path.iter().map(shape).collect();
                    front.extend(path);
                    path = front;
                    at = p.root;
                }
            }
        }
        Full { home: None, path }
    }

    /// A place, from its home.
    pub fn place_full(&self, p: &Place) -> Full {
        self.full(p.root, p.path.iter().map(shape).collect())
    }

    /// A stable place, from its home.
    pub fn key_full(&self, k: &PlaceKey) -> Full {
        self.full(k.root, k.path.iter().map(step_shape).collect())
    }

    /// Whether writing the place `w` may change the length at `t`: they belong to the same
    /// variable, and `w`'s path is `t`'s or a beginning of it, step by step.
    pub fn meets(&self, w: &Full, t: &Full) -> bool {
        let (Some(wh), Some(th)) = (w.home, t.home) else { return true };
        if wh != th {
            // Two parameters of a `par for` body may point into each other.
            return self.par_params.contains(&wh) && self.par_params.contains(&th);
        }
        w.path.len() <= t.path.len() && w.path.iter().zip(&t.path).all(|(a, b)| steps_meet(*a, *b))
    }

    /// Whether writing the items of the place `w` (its length kept) may change the length at
    /// `t`: a length inside one of the items.
    pub fn inside(&self, w: &Full, t: &Full) -> bool {
        match (w.home, t.home) {
            (Some(_), Some(_)) => self.meets(w, t) && t.path.len() > w.path.len(),
            _ => true,
        }
    }

    /// Whether writing `written` may change `term`'s length. See `meets`.
    pub fn may_overlap(&self, written: &Place, term: &PlaceKey) -> bool {
        self.meets(&self.place_full(written), &self.key_full(term))
    }
}

/// Whether `written` is surely `term`'s place: the same root, the same path, the same index
/// locals (a term whose index local changed was forgotten then).
pub fn must_equal(f: &Func, written: &Place, term: &PlaceKey) -> bool {
    key_of(f, written, written.path.len()).is_some_and(|k| &k == term)
}

/// The variable `root` belongs to in `f` (`None`: it may be any variable's). See `Homes::home`.
pub fn home(f: &Func, root: LocalIdx) -> Option<LocalIdx> {
    Homes::of(f, false).home(root)
}

/// Whether writing `written` may change `term`'s length in `f`. See `Homes::may_overlap`.
pub fn may_overlap(f: &Func, written: &Place, term: &PlaceKey) -> bool {
    Homes::of(f, false).may_overlap(written, term)
}

fn shape(p: &Proj) -> Shape {
    match p {
        Proj::Field(i) => Shape::Field(*i),
        Proj::Variant(..) => Shape::Variant,
        Proj::Payload => Shape::Payload,
        Proj::Index { .. } | Proj::MapKey { .. } | Proj::MapValue { .. } => Shape::Elem,
    }
}

fn step_shape(s: &Step) -> Shape {
    match s {
        Step::Field(i) => Shape::Field(*i),
        Step::Variant(..) => Shape::Variant,
        Step::Payload => Shape::Payload,
        Step::Const(_) | Step::Local(_) => Shape::Elem,
    }
}

/// The spec's pre-scan: the terms that appear in an index, in a comparison, as a length, as a
/// loop's end, or as a copy of one of those (`let j = i + 1`), in the order they first appear,
/// at most `MAX_TERMS`.
pub fn pick(m: &Module, f: &Func) -> Vec<Term> {
    let mut p = Picker { m, f, order: Vec::new(), seen: HashSet::new() };
    let mut loops = 0;
    walk(&f.body, &mut |n| match n {
        Node::Stmt(Stmt::ForRange { var, start, end, .. }) => {
            p.add(Term::Hidden(loops));
            loops += 1;
            p.local(*var);
            p.linear(start);
            p.linear(end);
        }
        Node::Stmt(Stmt::SetLen(_, e)) => p.linear(e),
        Node::Expr(e) => match &e.kind {
            ExprKind::Len(place) => p.len(place, place.path.len()),
            ExprKind::Binary(op, l, r) if op.is_comparison() && l.ty.is_int() => {
                p.linear(l);
                p.linear(r);
            }
            ExprKind::ArrayNew(n) => p.linear(n),
            _ => {}
        },
        Node::Place(place, _) => {
            for (i, proj) in place.path.iter().enumerate() {
                if let Proj::Index { at, .. } = proj {
                    p.linear(at);
                    p.len(place, i);
                }
            }
        }
        Node::Stmt(_) => {}
    });
    // Copies: a local set from a linear form of terms, or one a term is set from.
    for _ in 0..8 {
        let before = p.order.len();
        walk(&f.body, &mut |n| {
            let (l, e) = match n {
                Node::Stmt(Stmt::Let(l, e)) => (*l, e),
                Node::Stmt(Stmt::Assign(pl, e)) if pl.path.is_empty() => (pl.root, e),
                _ => return,
            };
            if !tracked(f, l) {
                return;
            }
            let mut related = Picker { m, f, order: Vec::new(), seen: HashSet::new() };
            related.linear(e);
            if p.seen.contains(&Term::Local(l)) || related.order.iter().any(|t| p.seen.contains(t)) {
                p.local(l);
                for t in related.order {
                    p.add(t);
                }
            }
        });
        if p.order.len() == before {
            break;
        }
    }
    p.order.truncate(MAX_TERMS);
    p.order
}

struct Picker<'a> {
    m: &'a Module,
    f: &'a Func,
    order: Vec<Term>,
    seen: HashSet<Term>,
}

impl Picker<'_> {
    fn add(&mut self, t: Term) {
        if self.seen.insert(t.clone()) {
            self.order.push(t);
        }
    }

    fn local(&mut self, l: LocalIdx) {
        if tracked(self.f, l) {
            self.add(Term::Local(l));
        }
    }

    /// The length of the list at the first `upto` steps of `place`, and its index locals.
    fn len(&mut self, place: &Place, upto: usize) {
        if let Some(k) = list_key(self.m, self.f, place, upto) {
            for s in &k.path {
                if let Step::Local(l) = s {
                    self.local(*l);
                }
            }
            self.add(Term::Len(k));
        }
    }

    /// The terms in the parts of `e` that a linear form follows.
    fn linear(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Load(p) if p.path.is_empty() => self.local(p.root),
            ExprKind::Len(p) => self.len(p, p.path.len()),
            ExprKind::Cast(x) | ExprKind::Proven(x) | ExprKind::Seq(_, x) => self.linear(x),
            ExprKind::Binary(BinOp::Add | BinOp::Sub, a, b) | ExprKind::RandInt(a, b) => {
                self.linear(a);
                self.linear(b);
            }
            _ => {}
        }
    }
}
