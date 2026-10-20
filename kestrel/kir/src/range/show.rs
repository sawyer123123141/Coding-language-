//! Expressions and ranges as the report writes them: Kestrel syntax from KIR (local names,
//! `xs.len`, `xs[i]`, `int(x)`), a value the compiler made with no name as `(...)`, and ranges as
//! "is exactly 5", "is 0 to 99", "is 0 or more", "is at most 10" or "can be any `int`".

use super::interval::{width, Interval};
use super::sites::CheckKind;
use crate::ir::*;

/// How many characters of an expression the report shows before cutting it with `...`.
const MAX_CHARS: usize = 40;

/// `e` in Kestrel syntax, cut at 40 characters.
pub fn expr_text(m: &Module, f: &Func, e: &Expr) -> String {
    let mut s = String::new();
    write_expr(m, f, e, &mut s);
    cut(s)
}

/// The place up to (not including) its step `upto`, in Kestrel syntax, cut at 40 characters.
pub fn place_text(m: &Module, f: &Func, p: &Place, upto: usize) -> String {
    let mut s = String::new();
    write_place(m, f, p, upto, &mut s);
    cut(s)
}

/// Why the check of `kind` on `e` may fail, after the kind's name: "`a * b` may exceed `int` (`a`
/// can be any `int`; `b` can be any `int`)". `operands` are `e`'s operands with their ranges; one
/// that is a literal gives no evidence, and none at all (a function the analysis gave up on) gives
/// no parentheses.
pub fn check_text(m: &Module, f: &Func, kind: CheckKind, e: &Expr, operands: &[(&Expr, Interval)]) -> String {
    let what = expr_text(m, f, e);
    let ty = operands.first().map_or(e.ty, |(x, _)| x.ty);
    let head = match kind {
        CheckKind::Overflow => format!("`{what}` may exceed `{}`", type_name(e.ty)),
        CheckKind::Shift => format!("`{what}` may shift by less than 0 or more than {}", width(ty) - 1),
        CheckKind::Division if operands.get(1).is_some_and(|(_, r)| !r.contains(0)) => {
            format!("`{what}` may divide the lowest `{}` by -1", type_name(ty))
        }
        CheckKind::Division => format!("`{what}` may divide by zero"),
        CheckKind::Narrowing => format!("`{what}` may not fit in `{}`", type_name(e.ty)),
        CheckKind::Bounds => format!("`{what}` may be outside its list"),
    };
    let evidence: Vec<String> = operands
        .iter()
        .filter(|(x, _)| !matches!(x.kind, ExprKind::Int(_)))
        .map(|(x, r)| format!("`{}` {}", expr_text(m, f, x), range_text(*r, x.ty)))
        .collect();
    if evidence.is_empty() {
        head
    } else {
        format!("{head} ({})", evidence.join("; "))
    }
}

/// Why the bounds check of step `i` of `p` may fail: "`out[i]` may be outside `out` (`i` is 0 or
/// more; nothing shows it is below `out.len`)". `index` is the index's range, when known.
pub fn bounds_text(m: &Module, f: &Func, p: &Place, i: usize, index: Option<Interval>) -> String {
    let (item, list) = (place_text(m, f, p, i + 1), place_text(m, f, p, i));
    let head = format!("`{item}` may be outside `{list}`");
    let Some(at) = p.path[i].at().filter(|at| !matches!(at.kind, ExprKind::Int(_))) else { return head };
    let Some(r) = index else { return head };
    let range = format!("`{}` {}", expr_text(m, f, at), range_text(r, at.ty));
    if r.lo >= 0 {
        format!("{head} ({range}; nothing shows it is below `{list}.len`)")
    } else {
        format!("{head} ({range})")
    }
}

fn cut(s: String) -> String {
    match s.char_indices().nth(MAX_CHARS) {
        Some((at, _)) => format!("{}...", &s[..at]),
        None => s,
    }
}

/// A type's name in the report: `int`, `i64`, `u8`.
pub fn type_name(ty: KTy) -> &'static str {
    match ty {
        KTy::U8 => "u8",
        KTy::I32 => "int",
        KTy::I64 => "i64",
        KTy::Bool => "bool",
        KTy::F32 => "float",
        KTy::F64 => "f64",
        _ => "value",
    }
}

/// The phrase after a value's name for the range `i` of a `ty` value.
pub fn range_text(i: Interval, ty: KTy) -> String {
    let t = Interval::of_type(ty);
    match (i.lo == t.lo, i.hi == t.hi) {
        _ if i.lo == i.hi => format!("is exactly {}", number(i.lo)),
        (true, true) => format!("can be any `{}`", type_name(ty)),
        (false, true) => format!("is {} or more", number(i.lo)),
        (true, false) => format!("is at most {}", number(i.hi)),
        (false, false) => format!("is {} to {}", number(i.lo), number(i.hi)),
    }
}

/// A number as the report writes it: commas from five digits (10,000; 9999).
pub fn number(n: i128) -> String {
    let digits = n.unsigned_abs().to_string();
    let grouped = if digits.len() < 5 {
        digits
    } else {
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(c);
        }
        out
    };
    if n < 0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

/// A local's name as the source wrote it: the inliner's `callee.` prefix taken off; `(...)` for a
/// value the compiler made (`tmp3`, `discard`, `index`, an empty name).
fn local_name(m: &Module, f: &Func, l: LocalIdx) -> String {
    let name = &f.locals[l.0 as usize].name;
    let name = m
        .funcs
        .iter()
        .filter_map(|g| name.strip_prefix(g.name.as_str()).and_then(|rest| rest.strip_prefix('.')))
        .min_by_key(|rest| rest.len())
        .unwrap_or(name);
    let made = name.is_empty()
        || name.starts_with("__")
        || matches!(name, "discard" | "index" | "result" | "env" | "copy")
        || name.strip_prefix("tmp").is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
    if made {
        "(...)".to_string()
    } else {
        name.strip_suffix("_ref").unwrap_or(name).to_string()
    }
}

fn op_text(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Gt => ">",
        BinOp::Le => "<=",
        BinOp::Ge => ">=",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
    }
}

/// An operand of a binary operation: in parentheses when it is itself one.
fn write_operand(m: &Module, f: &Func, e: &Expr, s: &mut String) {
    let inner = match &e.kind {
        ExprKind::Proven(x) => x,
        _ => e,
    };
    if matches!(inner.kind, ExprKind::Binary(..) | ExprKind::And(..) | ExprKind::Or(..)) {
        s.push('(');
        write_expr(m, f, e, s);
        s.push(')');
    } else {
        write_expr(m, f, e, s);
    }
}

fn write_expr(m: &Module, f: &Func, e: &Expr, s: &mut String) {
    if s.len() > MAX_CHARS * 4 {
        return; // cut later anyway
    }
    match &e.kind {
        ExprKind::Int(n) => s.push_str(&super::sites::wrap_literal(e.ty, *n).to_string()),
        ExprKind::Bool(b) => s.push_str(if *b { "true" } else { "false" }),
        ExprKind::Load(p) | ExprKind::Move(p) => write_place(m, f, p, p.path.len(), s),
        ExprKind::Len(p) => {
            write_place(m, f, p, p.path.len(), s);
            s.push_str(".len");
        }
        ExprKind::Proven(x) => write_expr(m, f, x, s),
        ExprKind::Unary(op, x) => {
            s.push_str(match op {
                UnOp::Neg => "-",
                UnOp::Not => "not ",
                UnOp::BitNot => "~",
            });
            write_operand(m, f, x, s);
        }
        ExprKind::Binary(op, a, b) => {
            write_operand(m, f, a, s);
            s.push_str(&format!(" {} ", op_text(*op)));
            write_operand(m, f, b, s);
        }
        ExprKind::And(a, b) | ExprKind::Or(a, b) => {
            write_operand(m, f, a, s);
            s.push_str(if matches!(e.kind, ExprKind::And(..)) { " and " } else { " or " });
            write_operand(m, f, b, s);
        }
        ExprKind::Cast(x) => {
            s.push_str(type_name(e.ty));
            s.push('(');
            write_expr(m, f, x, s);
            s.push(')');
        }
        ExprKind::Call(fi, _) => {
            s.push_str(&m.funcs[fi.0 as usize].name);
            s.push_str("(...)");
        }
        ExprKind::RandInt(a, b) => {
            s.push_str("rand_int(");
            write_expr(m, f, a, s);
            s.push_str(", ");
            write_expr(m, f, b, s);
            s.push(')');
        }
        _ => s.push_str("(...)"),
    }
}

fn write_place(m: &Module, f: &Func, p: &Place, upto: usize, s: &mut String) {
    s.push_str(&local_name(m, f, p.root));
    let mut ty = f.locals[p.root.0 as usize].ty;
    for proj in p.path.iter().take(upto) {
        ty = match (proj, ty) {
            (Proj::Field(fi), KTy::Struct(st)) => {
                let field = &m.layout(st).fields[*fi as usize];
                s.push('.');
                s.push_str(&field.name);
                field.ty
            }
            (Proj::Variant(v, fi), KTy::Enum(en)) => {
                let field = &m.enum_layout(en).variants[*v as usize].fields[*fi as usize];
                s.push('.');
                s.push_str(&field.name);
                field.ty
            }
            (Proj::Payload, KTy::Optional(el)) => m.elem(el),
            (Proj::Index { at, .. }, KTy::Array(el) | KTy::List(el)) => {
                s.push('[');
                write_expr(m, f, at, s);
                s.push(']');
                m.elem(el)
            }
            (Proj::MapKey { .. }, KTy::Map(k, _)) => {
                s.push_str("[...]");
                m.elem(k)
            }
            (Proj::MapValue { .. }, KTy::Map(_, v)) => {
                s.push_str("[...]");
                m.elem(v)
            }
            _ => {
                s.push_str("(...)");
                return;
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_get_commas_from_five_digits() {
        assert_eq!(number(9999), "9999");
        assert_eq!(number(10000), "10,000");
        assert_eq!(number(-2147483648), "-2,147,483,648");
        assert_eq!(number(123456), "123,456");
    }

    #[test]
    fn ranges_read_as_phrases() {
        assert_eq!(range_text(Interval::exact(5), KTy::I32), "is exactly 5");
        assert_eq!(range_text(Interval::new(0, 99), KTy::I32), "is 0 to 99");
        assert_eq!(range_text(Interval::new(0, i32::MAX.into()), KTy::I32), "is 0 or more");
        assert_eq!(range_text(Interval::new(i32::MIN.into(), 10), KTy::I32), "is at most 10");
        assert_eq!(range_text(Interval::of_type(KTy::I32), KTy::I32), "can be any `int`");
        assert_eq!(range_text(Interval::new(0, 10), KTy::U8), "is at most 10");
    }
}
