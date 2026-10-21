//! The step after `compile` for a program with annotations: range analysis runs once, in place, in
//! the build's own mode (codegen then compiles the module as it is: see `Module::analysed`), and,
//! for `kestrel check`, each `@assume` that removes no check in either build gets a warning.
//!
//! A program with no `@prove` (and, when warnings are asked for, no `@assume`) is left exactly as
//! `compile` made it, so it costs nothing: codegen analyses a copy of it as before.

use kestrel_kir::range::{remove_checks, Analysis, Arith};
use kestrel_kir::Module;
use kestrel_syntax::Diagnostic;

use crate::{report, Compiled, DiagStyle};

/// What `prove` found.
pub struct Proved {
    /// False when the program must not run or be built.
    pub ok: bool,
    /// Its diagnostics, rendered (empty when there are none).
    pub text: String,
    /// The analysis of the module, when it was analysed: `kestrel check --report` reuses it.
    pub analysis: Option<Analysis>,
}

/// Analyses `c`'s module in place for a checked build (`checked`) or a `--release` one, when it has
/// a `@prove`, or, with `warn`, an `@assume`. With `warn` and an assumption it also analyses a copy
/// for the other build and warns for each assumption that removes no check in either.
pub fn prove(c: &mut Compiled, checked: bool, warn: bool, style: DiagStyle) -> Proved {
    let warn = warn && !c.module.assumptions.is_empty();
    if c.module.contracts.is_empty() && !warn {
        return Proved { ok: true, text: String::new(), analysis: None };
    }
    let arith = Arith::of(checked);
    // Copied before the analysis marks the module.
    let other = warn.then(|| c.module.clone());
    let analysis = remove_checks(&mut c.module, arith);
    let mut diags = Vec::new();
    if let Some(mut other) = other {
        let other_analysis = remove_checks(&mut other, Arith::of(!checked));
        diags.extend(useless_assumptions(&c.module, &[&analysis, &other_analysis]));
    }
    let text = if diags.is_empty() { String::new() } else { report(&c.sources, &diags, style) };
    let ok = !diags.iter().any(Diagnostic::is_error);
    Proved { ok, text, analysis: Some(analysis) }
}

/// A warning for each assumption that every analysed copy of it, in every analysis, used without
/// removing a check. One no analysis recorded (its function was given up on) gets none.
fn useless_assumptions(m: &Module, analyses: &[&Analysis]) -> Vec<Diagnostic> {
    let n = m.assumptions.len();
    let (mut seen, mut removes) = (vec![false; n], vec![false; n]);
    for u in analyses.iter().flat_map(|a| &a.assumptions) {
        for &id in &u.ids {
            seen[id as usize] = true;
            removes[id as usize] |= u.removed > 0;
        }
    }
    (0..n)
        .filter(|&i| seen[i] && !removes[i])
        .map(|i| {
            let info = &m.assumptions[i];
            Diagnostic::warning(info.span, format!("`@assume({})` removes no check; delete it", info.text))
        })
        .collect()
}
