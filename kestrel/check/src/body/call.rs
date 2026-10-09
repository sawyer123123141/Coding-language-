use kestrel_syntax::ast::Expr;

use super::BodyChecker;
use crate::ty::Ty;

impl<'a> BodyChecker<'a> {
    pub(crate) fn check_call(&mut self, e: &Expr, _want: Option<Ty>) -> Ty {
        self.error(e.span, "function calls aren't supported yet");
        Ty::Error
    }

    pub(crate) fn check_discarded_call(&mut self, e: &Expr) -> Ty {
        self.check_call(e, None)
    }
}
