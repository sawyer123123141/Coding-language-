//! Strings: `.len`, `s[i]`, `a + b` and `for c in s`. Characters are counted by the runtime
//! (`StrOp`); `+` is an interpolation of its two sides, so it builds a new string in the current
//! arena, as `s += t` does.

use kestrel_check::StrMethod;
use kestrel_syntax::ast;

use super::FnLowerer;
use crate::ir::*;

fn load(l: LocalIdx, ty: KTy, line: u32) -> Expr {
    Expr { kind: ExprKind::Load(Place::local(l)), ty, line }
}

fn int(n: i64, line: u32) -> Expr {
    Expr { kind: ExprKind::Int(n), ty: KTy::I64, line }
}

fn add(a: Expr, b: Expr) -> Expr {
    let line = a.line;
    Expr { kind: ExprKind::Binary(BinOp::Add, Box::new(a), Box::new(b)), ty: KTy::I64, line }
}

/// The pieces a string value adds to a `Format`: an interpolation's own pieces (so a chain
/// `a + b + c` makes one string, not one per `+`), a literal as text, anything else as a value.
pub(crate) fn concat_pieces(e: Expr) -> Vec<PrintArg> {
    match e.kind {
        ExprKind::Format(parts) => parts,
        ExprKind::StrLit(s) => vec![PrintArg::Str(s)],
        _ => vec![PrintArg::Value(e)],
    }
}

impl<'a> FnLowerer<'a> {
    /// `s.len`: the characters, read as an `int` (checked when it doesn't fit).
    pub(crate) fn str_len(&mut self, base: &ast::Expr, line: u32) -> ExprKind {
        let s = self.expr(base);
        let chars = Expr { kind: ExprKind::Str(StrOp::Chars, vec![s]), ty: KTy::I64, line };
        ExprKind::Cast(Box::new(chars))
    }

    /// `s[i]`: a new one-character string; the runtime checks `i`.
    pub(crate) fn str_at(&mut self, base: &ast::Expr, index: &ast::Expr) -> ExprKind {
        let (s, i) = (self.expr(base), self.expr(index));
        ExprKind::Str(StrOp::At, vec![s, i])
    }

    /// A string method call (`has`, `find`, `slice`, `split` ...): the receiver, then the arguments.
    pub(crate) fn str_method(&mut self, method: StrMethod, receiver: &ast::Expr, args: &[ast::Arg], ty: KTy, line: u32) -> ExprKind {
        let mut operands = vec![self.expr(receiver)];
        operands.extend(args.iter().map(|a| self.expr(&a.value)));
        let op = match method {
            StrMethod::Has | StrMethod::Find => StrOp::Find,
            StrMethod::StartsWith => StrOp::Starts,
            StrMethod::EndsWith => StrOp::Ends,
            StrMethod::Slice => StrOp::Slice,
            StrMethod::Split => StrOp::Split,
            StrMethod::Upper => StrOp::Upper,
            StrMethod::Lower => StrOp::Lower,
            StrMethod::Trim => StrOp::Trim,
            StrMethod::ToInt => StrOp::ToInt,
            StrMethod::ToFloat => StrOp::ToFloat,
            StrMethod::Len | StrMethod::At => unreachable!("{method:?} is not a method call"),
        };
        match method {
            // `-1` means no match.
            StrMethod::Has => {
                let found = Expr { kind: ExprKind::Str(op, operands), ty: KTy::I64, line };
                ExprKind::Binary(BinOp::Ne, Box::new(found), Box::new(int(-1, line)))
            }
            StrMethod::Find => {
                let (result, at) = (self.temp(ty), self.temp(KTy::I64));
                let found = Expr { kind: ExprKind::Str(op, operands), ty: KTy::I64, line };
                let some = Expr { kind: ExprKind::Cast(Box::new(load(at, KTy::I64, line))), ty: KTy::I32, line };
                let some = Expr { kind: ExprKind::Some(Box::new(some)), ty, line };
                let hit = Expr { kind: ExprKind::Binary(BinOp::Ge, Box::new(load(at, KTy::I64, line)), Box::new(int(0, line))), ty: KTy::Bool, line };
                let pre = vec![
                    Stmt::Let(result, Expr { kind: ExprKind::NoneLit, ty, line }),
                    Stmt::Let(at, found),
                    Stmt::If { cond: hit, then_body: vec![Stmt::Assign(Place::local(result), some)], else_body: Vec::new() },
                ];
                Self::with_pre(pre, ExprKind::Load(Place::local(result)), ty, line)
            }
            _ => ExprKind::Str(op, operands),
        }
    }

    /// `xs.join(sep)` on a list or array of strings.
    pub(crate) fn str_join(&mut self, list: &ast::Expr, sep: &ast::Expr) -> ExprKind {
        let (list, sep) = (self.expr(list), self.expr(sep));
        ExprKind::Str(StrOp::Join, vec![list, sep])
    }

    /// `a + b` on strings: the pieces of the two sides, in order.
    pub(crate) fn str_concat(&mut self, lhs: &ast::Expr, rhs: &ast::Expr) -> ExprKind {
        let (l, r) = (self.expr(lhs), self.expr(rhs));
        let mut parts = concat_pieces(l);
        parts.extend(concat_pieces(r));
        ExprKind::Format(parts)
    }

    /// `for i, c in s`: walks a snapshot of `s`'s header (a copy of its bytes when the body may
    /// change the place `s` is read from, since that could free them), so the loop sees the
    /// string as it was when it started. Each pass takes the character at byte `pos` as a new
    /// string `c`, and steps `pos` and the character count `k` before the body runs, so every
    /// way back to the condition (the end, `continue`) is already past the character.
    pub(crate) fn str_for_in(
        &mut self,
        index: Option<&ast::Binding>,
        var: &ast::Binding,
        iter: &ast::Expr,
        body: &ast::Block,
        out: &mut Vec<Stmt>,
    ) {
        let line = self.line(iter.span);
        let s = self.place_or_temp(iter, out, true);
        let (snap, pos, n, k) = (self.temp(KTy::Str), self.temp(KTy::I64), self.temp(KTy::I64), self.temp(KTy::I32));
        let c = self.bind_local(var);
        let body_stmts = self.loop_block(body);
        out.push(Stmt::Let(snap, Expr { kind: ExprKind::Load(s.clone()), ty: KTy::Str, line }));
        if self.may_move(&body_stmts, &s) {
            out.push(Stmt::Dup(Place::local(snap)));
        }
        out.push(Stmt::Let(pos, int(0, line)));
        out.push(Stmt::Let(k, Expr { kind: ExprKind::Int(0), ty: KTy::I32, line }));
        let text = || load(snap, KTy::Str, line);
        let mut pass = vec![
            Stmt::Let(n, Expr { kind: ExprKind::Str(StrOp::CharLen, vec![text(), load(pos, KTy::I64, line)]), ty: KTy::I64, line }),
            Stmt::Let(
                c,
                Expr { kind: ExprKind::Str(StrOp::Piece, vec![text(), load(pos, KTy::I64, line), load(n, KTy::I64, line)]), ty: KTy::Str, line },
            ),
            Stmt::Assign(Place::local(pos), add(load(pos, KTy::I64, line), load(n, KTy::I64, line))),
        ];
        if let Some(b) = index {
            pass.push(Stmt::Let(self.bind_local(b), load(k, KTy::I32, line)));
        }
        let next = Expr { kind: ExprKind::Binary(BinOp::Add, Box::new(load(k, KTy::I32, line)), Box::new(Expr { kind: ExprKind::Int(1), ty: KTy::I32, line })), ty: KTy::I32, line };
        pass.push(Stmt::Assign(Place::local(k), next));
        pass.extend(body_stmts);
        let bytes = Expr { kind: ExprKind::Len(Place::local(snap)), ty: KTy::I64, line };
        let more = Expr { kind: ExprKind::Binary(BinOp::Lt, Box::new(load(pos, KTy::I64, line)), Box::new(bytes)), ty: KTy::Bool, line };
        out.push(Stmt::While { cond: more, body: pass });
    }
}
