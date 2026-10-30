//! `checksum(x)`, `to_bytes(x)` and `first_difference(a, b)` in each copy of a generic (see
//! `copies`): the checker judges a placeholder's value as having no fault (see `snapshot`), so each
//! copy judges its argument with its own types.

use kestrel_syntax::ast::{Expr, ExprId, ExprKind};
use kestrel_syntax::{Diagnostic, Span};

use crate::body::{checksum_root, snapshot_verb};
use crate::builtins::Builtin;
use crate::generic::copies::{Ctx, Walk};
use crate::snapshot::snapshot_refusal;

/// A `checksum(x)` or `to_bytes(x)` call's argument and how it is written (see
/// `body::checksum_root`).
pub(super) type ChecksumArg = Option<(ExprId, Option<String>)>;

/// The argument of `e` when it calls `checksum` or `to_bytes` with one value, or the first of
/// `first_difference`'s two (reserved names, so no user function is called that).
pub(super) fn checksum_arg(e: &Expr) -> ChecksumArg {
    match &e.kind {
        ExprKind::Call { name, args, .. } if (name == "checksum" || name == "to_bytes") && args.len() == 1 => Some((args[0].value.id, checksum_root(&args[0].value))),
        // Both values have the first one's type.
        ExprKind::Call { name, args, .. } if name == "first_difference" && args.len() == 2 => Some((args[0].value.id, checksum_root(&args[0].value))),
        _ => None,
    }
}

impl Walk<'_> {
    /// `checksum(x)` or `to_bytes(x)` (built-in `b`) in a copy: the argument's type with the copy's
    /// types in it can be copied and checksummed (a placeholder has no fault when the generic is
    /// checked; see `snapshot`).
    pub(super) fn checksum_copy(&mut self, b: Builtin, arg: ExprId, root: Option<String>, span: Span, ctx: Ctx) -> Result<(), Diagnostic> {
        let written = self.t.ty(arg);
        if ctx.subst.is_none() || self.t.is_concrete(written) {
            return Ok(());
        }
        let ty = self.sub(written, ctx)?;
        let Some((msg, part)) = snapshot_refusal(self.t, ty, root.as_deref(), b.name(), snapshot_verb(b)) else { return Ok(()) };
        let copy = ctx.made_by.map(|f| format!("in the copy `{}`", self.t.func(f).name));
        let help = match (copy, part) {
            (Some(copy), Some(part)) => Some(format!("{copy}, {part}")),
            (copy, part) => copy.or(part),
        };
        let d = Diagnostic::error(span, msg);
        Err(match help {
            Some(h) => d.with_help(h),
            None => d,
        })
    }
}
