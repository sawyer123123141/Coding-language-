# Language pass, part 1 (friction fixes) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make code that is safe but rejected today compile (moves on one branch, unwrapping owning optionals in place, struct and enum values in defaults and constants, `vec3()`), and make four confusing messages say what to do, including naming the file in runtime errors.

**Architecture:** The checker gets a path-following move state (`check/src/body/moves.rs`) and `let ... else` views (`check/src/body/let_else.rs`, plus a KIR branch). Constant expressions widen to struct and enum values, with a new graph check for defaults that build each other (`check/src/body/default_cycles.rs`). Runtime errors get the file through KIR's packed line, a per-file name record emitted by codegen, and one more argument on four runtime functions.

**Tech Stack:** Rust (Cargo workspace in `kestrel/`), Cranelift 0.116, the Kestrel runtime staticlib; tests are Rust unit/integration tests plus `.kes` program and error tests.

**Spec:** `docs/superpowers/specs/2026-10-12-language-pass-1-design.md` (read it first; this plan argues from it).

## Global Constraints

- Work on `main` (the user's standing approval); commit after each task; push at the end of each task.
- Every commit message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Keep source files under about 400 lines; split by responsibility.
- Error messages are plain sentences that say what to do.
- Never call a bare `link.exe` (Git's coreutils one shadows the MSVC linker).
- Run commands in `kestrel/`: `cargo test --workspace --no-fail-fast`, `cargo clippy --workspace --all-targets -- -D warnings`.
- Never open a visible window: window programs run with `KESTREL_HEADLESS=<frames>` or `KESTREL_HIDDEN=1`; never set `KESTREL_SMOKE_VISIBLE`.
- No program's output may change except the messages this plan names.
- Program tests are `kestrel/tests/programs/*.kes` with `// expect:`, `// expect-exit:`, `// expect-stderr:` lines; error tests are `kestrel/tests/errors/*.kes` with `// error <line>: <message>` lines (the harness compares line and message, not help).
- The `relocated` CLI test sometimes fails with "file in use" (OneDrive/antivirus); re-run it.

## Review Focus

1. A loop body that moves a local, gives it a new value, then `continue`s: the `continue` is a back edge and must see the new value (no error). Test in Task 1.
2. A `continue` in an inner loop after moving a local declared inside the outer loop's body (not outside the outer loop): no error at either loop. Test in Task 1.
3. A `let ... else` view inside a `par for` body over an item's optional field of a `let` list: compiles, reads in place, prints the right values on many threads. Test in Task 2.
4. An enum constant whose payload holds a list (a hoisted heap constant) used as a field default and in a function: built once, copied on store, prints right. Test in Task 3.
5. A runtime error raised on a `par for` pool thread names the file (the message is formatted on the pool thread, then re-raised). Test in Task 5 (`par_for_panic.kes` pin).

---

### Task 1: Moves follow the program's paths (spec part A)

**Files:**
- Create: `kestrel/check/src/body/moves.rs`
- Modify: `kestrel/check/src/body/mod.rs` (fields, `mod moves`, `While`, `ForRange`, `Break`/`Continue`)
- Modify: `kestrel/check/src/body/regions.rs` (move `consume` out; make `tail` `pub(crate)`)
- Modify: `kestrel/check/src/body/expr.rs:90-103` (`ident`), `:161-167` (`and`/`or`), `:275-323` (`check_if`)
- Modify: `kestrel/check/src/body/matching.rs` (`check_match`)
- Modify: `kestrel/check/src/body/place.rs:56-123` (`check_assign`)
- Modify: `kestrel/check/src/body/collections.rs` (`check_for_in` loop, `check_let_else` order)
- Modify: `kestrel/check/src/body/maps.rs:158-160` (map loop)
- Test: `kestrel/check/tests/moves.rs` (create), `kestrel/tests/errors/moves.kes` (create), `kestrel/codegen/tests/gpu.rs` (two cases)

**Interfaces:**
- Produces (used by Task 2): `pub(crate) struct MoveAt { span: Span, every: bool }`, `pub(crate) type Moved = HashMap<LocalId, MoveAt>`, `BodyChecker::moved: Moved`, `BodyChecker::consume(&mut self, v: &Expr)` now in `moves.rs` with a private `consume_on(&mut self, v: &Expr, every: bool)`, `BodyChecker::loop_body(&mut self, entry: Moved, body: &Block, exits_at_start: bool) -> bool`, `regions::tail(block: &Block) -> Option<&Expr>` (now `pub(crate)`).

- [ ] **Step 1: Write the failing checker tests**

Create `kestrel/check/tests/moves.rs`:

```rust
//! Moves follow the program's paths: a value moved on one branch is still there on the
//! other, a move followed by leaving the loop is fine, and giving a moved variable a new
//! value makes it usable again.

use kestrel_check::check;
use kestrel_syntax::parse;

/// `Bag` owns an arena, so it moves instead of copying.
const BAG: &str = "struct Bag {\n    mem: Arena = Arena()\n    n: int = 0\n}\nstruct Pair {\n    b: Bag\n}\npeek(o: Bag?) -> int {\n    1\n}\n";

fn errors(body: &str) -> Vec<String> {
    let src = format!("{BAG}{body}main() {{}}\n");
    let (p, d) = parse(&src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    check(&p).1.into_iter().map(|d| d.message).collect()
}

const MAYBE: &str = "`t` may have been moved earlier, so it can't be used here";
const MOVED: &str = "`t` was moved earlier, so it can't be used here";
const IN_LOOP: &str = "`t` is moved inside a loop, so the next time around it would already be gone";

fn none() -> Vec<String> {
    Vec::new()
}

#[test]
fn each_branch_may_move_the_same_value() {
    assert_eq!(errors("go(c: bool) {\n var a = list[Bag]()\n var b = list[Bag]()\n let t = Bag()\n if c { a.add(t) } else { b.add(t) }\n}\n"), none());
}

#[test]
fn a_branch_that_leaves_doesnt_move_for_the_rest() {
    assert_eq!(errors("go(c: bool, var keep: list[Bag]) {\n let t = Bag()\n if c {\n  keep.add(t)\n  return\n }\n keep.add(t)\n}\n"), none());
}

#[test]
fn leaving_the_loop_right_after_a_move_is_fine() {
    let body = "go(var keep: list[Bag]) {\n let t = Bag()\n for i from 0 to 3 {\n  if i == 1 {\n   keep.add(t)\n   break\n  }\n }\n}\n";
    assert_eq!(errors(body), none());
    // After the loop, `t` was moved only if the loop reached the `break`.
    let after = "go(var keep: list[Bag]) {\n let t = Bag()\n for i from 0 to 3 {\n  if i == 1 {\n   keep.add(t)\n   break\n  }\n }\n print(t.n)\n}\n";
    assert_eq!(errors(after), vec![MAYBE.to_string()]);
}

#[test]
fn a_new_value_makes_a_moved_variable_usable() {
    assert_eq!(errors("go(var keep: list[Bag]) {\n var t = Bag()\n keep.add(t)\n t = Bag()\n print(t.n)\n}\n"), none());
    let looped = "go(var done: list[Bag]) {\n var t = Bag()\n var i = 0\n while i < 3 {\n  done.add(t)\n  t = Bag()\n  i += 1\n }\n print(t.n)\n}\n";
    assert_eq!(errors(looped), none());
    // A `continue` is a way around the loop too; the new value is there before it.
    let cont = "go(var done: list[Bag], c: bool) {\n var t = Bag()\n var i = 0\n while i < 3 {\n  i += 1\n  done.add(t)\n  t = Bag()\n  if c {\n   continue\n  }\n }\n}\n";
    assert_eq!(errors(cont), none());
}

#[test]
fn a_local_of_the_loop_body_may_move_before_an_inner_continue() {
    let body = "go(var keep: list[Bag]) {\n for i from 0 to 3 {\n  let t = Bag()\n  keep.add(t)\n  for j from 0 to 2 {\n   if j == 0 {\n    continue\n   }\n  }\n }\n}\n";
    assert_eq!(errors(body), none());
}

#[test]
fn the_else_block_of_let_else_still_has_the_optional() {
    let body = "go(var keep: list[Bag]) {\n let o: Bag? = Bag()\n let x = o else {\n  print(peek(o))\n  return\n }\n keep.add(x)\n}\n";
    assert_eq!(errors(body), none());
}

#[test]
fn a_use_after_a_move_on_some_paths_is_an_error() {
    assert_eq!(errors("go(c: bool, var keep: list[Bag]) {\n let t = Bag()\n if c { keep.add(t) }\n print(t.n)\n}\n"), vec![MAYBE.to_string()]);
    assert_eq!(errors("go(c: bool) {\n let t = Bag()\n let ok = c and Pair(b: t).b.n > 0\n print(t.n)\n}\n"), vec![MAYBE.to_string()]);
    assert_eq!(errors("go(c: bool) {\n let t = Bag()\n let y = if c { t } else { Bag() }\n print(t.n)\n}\n"), vec![MAYBE.to_string()]);
}

#[test]
fn a_use_after_a_move_on_every_path_is_an_error() {
    assert_eq!(
        errors("go(c: bool, var a: list[Bag]) {\n let t = Bag()\n if c { a.add(t) } else { a.add(t) }\n print(t.n)\n}\n"),
        vec![MOVED.to_string()]
    );
    // Changing part of a moved variable is a use; only the whole variable gets a new value.
    assert_eq!(errors("go(var keep: list[Bag]) {\n var t = Bag()\n keep.add(t)\n t.n = 1\n}\n"), vec![MOVED.to_string()]);
    // The new value is checked first, so it can't use the moved variable.
    assert_eq!(
        errors("go(c: bool, var keep: list[Bag]) {\n var t = Bag()\n keep.add(t)\n t = if c { t } else { Bag() }\n}\n"),
        vec![MOVED.to_string()]
    );
}

#[test]
fn a_loop_that_goes_around_after_a_move_is_an_error() {
    assert_eq!(errors("go(var keep: list[Bag]) {\n let t = Bag()\n for i from 0 to 3 {\n  keep.add(t)\n }\n}\n"), vec![IN_LOOP.to_string()]);
    assert_eq!(
        errors("go(var keep: list[Bag]) {\n let t = Bag()\n for i from 0 to 3 {\n  if i == 1 {\n   keep.add(t)\n   continue\n  }\n }\n}\n"),
        vec![IN_LOOP.to_string()]
    );
    // A `while` condition runs every time around.
    assert_eq!(errors("go() {\n let t = Bag()\n var i = 0\n while Pair(b: t).b.n > i {\n  i += 1\n }\n}\n"), vec![IN_LOOP.to_string()]);
    // Reported once, though both loops go around.
    assert_eq!(
        errors("go(var keep: list[Bag]) {\n let t = Bag()\n for i from 0 to 2 {\n  for j from 0 to 2 {\n   keep.add(t)\n  }\n }\n}\n"),
        vec![IN_LOOP.to_string()]
    );
    // Leaving the inner loop isn't leaving the outer one.
    assert_eq!(
        errors("go(var keep: list[Bag]) {\n let t = Bag()\n for i from 0 to 2 {\n  for j from 0 to 2 {\n   keep.add(t)\n   break\n  }\n }\n}\n"),
        vec![IN_LOOP.to_string()]
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kestrel-check --test moves`
Expected: FAIL. `each_branch_may_move_the_same_value` gets `["`t` was moved earlier, so it can't be used here"]`; the `MAYBE` cases get `MOVED`; several loop cases get the loop error where none is expected.

- [ ] **Step 3: Create `moves.rs`**

Create `kestrel/check/src/body/moves.rs`:

```rust
//! Moves of owning values (ones that hold an `Arena` or a GPU object): which locals may have
//! been moved at each point of a function. The state follows the code's paths: each branch of
//! an `if` or `match` starts from the state before it, and a loop's body must leave every local
//! declared outside the loop as it found it, unless the path leaves the loop.

use std::collections::HashMap;

use kestrel_syntax::ast::{Block, Expr, ExprKind};
use kestrel_syntax::Span;

use super::regions::tail;
use super::{BodyChecker, Expect};
use crate::tables::{LocalId, Res};
use crate::ty::Ty;

/// Where a local was moved, and whether every path to this point moved it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MoveAt {
    pub(crate) span: Span,
    pub(crate) every: bool,
}

/// The owning locals that may have been moved at a point in the code.
pub(crate) type Moved = HashMap<LocalId, MoveAt>;

/// A loop being checked: the state where it starts, and the state at each of its `break`s.
pub(crate) struct LoopMoves {
    entry: Moved,
    breaks: Vec<Moved>,
}

/// The state where paths meet: a local is moved if some path moved it, and moved on every
/// path only if each one did. `None` when no path arrives.
pub(crate) fn merge(states: Vec<Moved>) -> Option<Moved> {
    let mut states = states.into_iter();
    let mut out = states.next()?;
    for s in states {
        for (l, at) in out.iter_mut() {
            if !s.get(l).is_some_and(|m| m.every) {
                at.every = false;
            }
        }
        for (l, at) in s {
            out.entry(l).or_insert(MoveAt { every: false, ..at });
        }
    }
    Some(out)
}

impl<'a> BodyChecker<'a> {
    /// The state after branches that each started from `before`, given each branch's type and
    /// its state at its end: the branches that can finish (not `Never`) meet here.
    pub(crate) fn join(&mut self, before: Moved, ends: Vec<(Ty, Moved)>) {
        let finishing = ends.into_iter().filter(|(ty, _)| *ty != Ty::Never).map(|(_, m)| m).collect();
        self.moved = merge(finishing).unwrap_or(before);
    }

    /// Checks `e`, which runs only sometimes (the right side of `and` / `or`): what it moves
    /// may have been moved afterwards.
    pub(crate) fn check_sometimes(&mut self, e: &Expr, want: Option<Ty>) -> Ty {
        let before = self.moved.clone();
        let ty = self.check_value(e, want);
        let after = std::mem::take(&mut self.moved);
        self.moved = merge(vec![before, after]).expect("two states");
        ty
    }

    /// Checks a loop's body, which runs any number of times. `entry` is the state where the
    /// loop starts (before a `while` loop's condition, which runs each time around);
    /// `exits_at_start` says whether the loop can end there (its condition false), not only at a
    /// `break`. Returns whether a `break` leaves the loop.
    pub(crate) fn loop_body(&mut self, entry: Moved, body: &Block, exits_at_start: bool) -> bool {
        self.loop_breaks.push(false);
        self.loop_moves.push(LoopMoves { entry, breaks: Vec::new() });
        if self.check_block(body, Expect::Discard) != Ty::Never {
            self.back_edge();
        }
        let broke = self.loop_breaks.pop().expect("pushed above");
        let frame = self.loop_moves.pop().expect("pushed above");
        let mut exits = frame.breaks;
        if exits_at_start {
            exits.push(frame.entry);
        }
        if let Some(after) = merge(exits) {
            self.moved = after;
        }
        broke
    }

    /// A `break`: the innermost loop ends here, with the current state.
    pub(crate) fn note_break(&mut self) {
        let now = self.moved.clone();
        if let Some(frame) = self.loop_moves.last_mut() {
            frame.breaks.push(now);
        }
    }

    /// The end of a loop's body, or a `continue`: the body runs again, so every local declared
    /// outside the loop that the loop moved must have a value again.
    pub(crate) fn back_edge(&mut self) {
        let Some(frame) = self.loop_moves.last() else { return };
        let depth = self.loop_breaks.len();
        let mut gone: Vec<(LocalId, Span)> = self
            .moved
            .iter()
            .filter(|(l, _)| !frame.entry.contains_key(l))
            .filter(|(l, _)| self.local_ctx.get(l).is_some_and(|&(_, loops)| loops < depth))
            .map(|(&l, at)| (l, at.span))
            .collect();
        gone.sort_by_key(|&(_, span)| span.start);
        for (l, span) in gone {
            // An inner loop may have reported this move already.
            if !self.loop_move_errors.insert(span) {
                continue;
            }
            let name = self.t.local(l).name.clone();
            self.error_help(
                span,
                format!("`{name}` is moved inside a loop, so the next time around it would already be gone"),
                "declare it inside the loop, give it a new value before the loop goes around, or leave the loop (`break` or `return`) right after moving it",
            );
        }
    }

    /// `v` is stored somewhere new. An owning value (one holding an `Arena` or a GPU object)
    /// can't be copied, only moved, and only whole variables move.
    pub(crate) fn consume(&mut self, v: &Expr) {
        self.consume_on(v, true);
    }

    /// `consume`, where `every` says whether every path through the enclosing `if` / `match`
    /// values reaches `v`.
    fn consume_on(&mut self, v: &Expr, every: bool) {
        let ty = self.t.ty(v.id);
        if !self.t.is_owning(ty) {
            return;
        }
        match &v.kind {
            ExprKind::If { then_block, else_block, .. } => {
                let tails: Vec<&Expr> = std::iter::once(then_block).chain(else_block.as_ref()).filter_map(tail).collect();
                let one = tails.len() == 1;
                for t in tails {
                    self.consume_on(t, every && one);
                }
            }
            ExprKind::Match { arms, .. } => {
                let tails: Vec<&Expr> = arms.iter().filter_map(|a| tail(&a.body)).collect();
                let one = tails.len() == 1;
                for t in tails {
                    self.consume_on(t, every && one);
                }
            }
            ExprKind::Ident(_) => {
                let Some(&Res::Local(l)) = self.t.res.get(&v.id) else { return };
                let info = self.t.local(l);
                let name = info.name.clone();
                if info.is_param {
                    let what = self.t.owns_what(ty);
                    let msg = format!("can't move parameter `{name}`: it belongs to the caller, and it holds {what}, so it can't be copied");
                    self.error(v.span, msg);
                } else if self.views.contains(&l) {
                    let what = self.t.owns_what(ty);
                    let msg = format!("can't move `{name}` out of its collection: it holds {what}, so it can't be copied");
                    self.error(v.span, msg);
                } else {
                    self.moved.insert(l, MoveAt { span: v.span, every });
                    self.t.moves.insert(v.id);
                }
            }
            // A part of a variable, or of a temporary (a call's result, a construction). `Enum.Variant`
            // is a field access that builds a value; it isn't a part of anything.
            ExprKind::Field { .. } | ExprKind::Index { .. } if !matches!(self.t.res.get(&v.id), Some(Res::Variant(..))) => {
                let what = self.t.owns_what(ty);
                let msg = format!("can't move this out: it holds {what}, so it can't be copied, and only whole variables move");
                let help = match self.place_root(v) {
                    Some(_) => "move the whole variable that holds it",
                    None => "use the part in place, or keep the whole value in a variable",
                };
                self.error_help(v.span, msg, help);
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 4: Take `consume` out of `regions.rs` and share `tail`**

In `kestrel/check/src/body/regions.rs`: delete the whole `/// \`v\` is stored somewhere new...` doc comment and `pub(crate) fn consume(&mut self, v: &Expr) { ... }` (lines 339-395, the last method of the `impl`), and change `fn tail(block: &Block) -> Option<&Expr> {` to `pub(crate) fn tail(block: &Block) -> Option<&Expr> {`. Update the file's first doc line to `//! The arena escape check: which arena a value's memory lives in (its region), and where`, second line `//! such a value may be stored. Moves are in \`moves\`.`

- [ ] **Step 5: Wire the state into `BodyChecker`**

In `kestrel/check/src/body/mod.rs`:
- Add `mod moves;` after `mod maps;`.
- Change the `use kestrel_syntax::{Diagnostic, Span};` line to keep both (it already imports `Span`).
- Replace the field
  ```rust
      /// Owning locals that were moved and can't be used again.
      moved: HashSet<LocalId>,
  ```
  with
  ```rust
      /// Owning locals that may have been moved by now, following the code's paths (see `moves`).
      moved: moves::Moved,
      /// The loops being checked, outermost first: where each started, and its `break`s.
      loop_moves: Vec<moves::LoopMoves>,
      /// Moves already reported as moved inside a loop, so an enclosing loop doesn't repeat them.
      loop_move_errors: HashSet<Span>,
  ```
- In `new`, replace `let (local_ctx, views, moved) = (HashMap::new(), HashSet::new(), HashSet::new());` with `let (local_ctx, views, moved) = (HashMap::new(), HashSet::new(), HashMap::new());`, add `let (loop_moves, loop_move_errors) = (Vec::new(), HashSet::new());`, and add `loop_moves, loop_move_errors` to the struct literal.
- Replace the `Stmt::While` arm with:
  ```rust
              Stmt::While { cond, body, .. } => {
                  // The condition runs each time around, so the loop starts before it.
                  let entry = self.moved.clone();
                  self.check_value(cond, Some(Ty::Bool));
                  let forever = matches!(cond.kind, ExprKind::Bool(true));
                  let broke = self.loop_body(entry, body, !forever);
                  // `while true` with no `break` can only be left by `return`.
                  if forever && !broke {
                      Ty::Never
                  } else {
                      Ty::Unit
                  }
              }
  ```
- In the `Stmt::ForRange` arm, replace
  ```rust
                  self.loop_breaks.push(false);
                  self.check_block(body, Expect::Discard);
                  self.loop_breaks.pop();
  ```
  with
  ```rust
                  let entry = self.moved.clone();
                  self.loop_body(entry, body, true);
  ```
- In the `Stmt::Break(span) | Stmt::Continue(span)` arm, just before its final `Ty::Never`, add:
  ```rust
                  if is_break {
                      self.note_break();
                  } else {
                      self.back_edge();
                  }
  ```

- [ ] **Step 6: Use the state for branches, `and`/`or`, assignment and the other loops**

In `kestrel/check/src/body/expr.rs`, `ident`: replace
```rust
            if self.moved.contains(&local) {
                self.error(e.span, format!("`{name}` was moved earlier, so it can't be used here"));
            }
```
with
```rust
            match self.moved.get(&local).copied() {
                Some(at) if at.every => self.error(e.span, format!("`{name}` was moved earlier, so it can't be used here")),
                Some(_) => self.error_help(
                    e.span,
                    format!("`{name}` may have been moved earlier, so it can't be used here"),
                    "it's moved on some paths to this line; give it a new value first",
                ),
                None => {}
            }
```
In `binary`, the `BinaryOp::And | BinaryOp::Or` arm: replace `self.check_value(rhs, Some(Ty::Bool));` with `self.check_sometimes(rhs, Some(Ty::Bool));`.

In `check_if`: after `self.check_value(cond, Some(Ty::Bool));` add `let before = self.moved.clone();`. In the `Expect::Discard` arm, after `let t1 = self.check_block(then_block, Expect::Discard);` add `let after_then = std::mem::replace(&mut self.moved, before.clone());`, and after the `let t2 = match else_block { ... };` add
```rust
                let after_else = std::mem::take(&mut self.moved);
                self.join(before, vec![(t1, after_then), (t2, after_else)]);
```
In the `Expect::Value(want)` arm (after the early return for a missing `else`), after `let t1 = self.check_block(then_block, Expect::Value(want));` add `let after_then = std::mem::replace(&mut self.moved, before.clone());`, and after `let t2 = self.check_block(else_block, Expect::Value(hint));` add
```rust
                let after_else = std::mem::take(&mut self.moved);
                self.join(before, vec![(t1, after_then), (t2, after_else)]);
```

In `kestrel/check/src/body/matching.rs`, `check_match`: after `let scrutinee_ty = self.check_expr(scrutinee, None);` add `let before = self.moved.clone();` and `let mut ends = Vec::with_capacity(arms.len());`. At the top of the `for arm in arms` loop body add `self.moved = before.clone();`. After `tys.push(ty);` add `ends.push((ty, std::mem::take(&mut self.moved)));`. After the loop (before `let missing`) add `self.join(before, ends);`.

In `kestrel/check/src/body/place.rs`, `check_assign`: replace `let target_ty = self.check_expr(target, None);` with
```rust
        // `t = value` gives a moved variable its value back; naming it here isn't a use.
        let reinit = match (&target.kind, op) {
            (ExprKind::Ident(name), AssignOp::Set) => self.lookup_local(name),
            _ => None,
        };
        let was = reinit.and_then(|l| self.moved.remove(&l));
        let target_ty = self.check_expr(target, None);
        if let (Some(l), Some(at)) = (reinit, was) {
            self.moved.insert(l, at);
        }
```
and at the end of the function, after `self.check_store(target, value);`, add
```rust
        if let Some(l) = reinit {
            self.moved.remove(&l);
        }
```

In `kestrel/check/src/body/collections.rs`, `check_for_in`: replace
```rust
        self.loop_breaks.push(false);
        self.check_block(body, Expect::Discard);
        self.loop_breaks.pop();
```
with
```rust
        let entry = self.moved.clone();
        self.loop_body(entry, body, true);
```
In `check_let_else`, move `self.consume(value);` from before the `if self.check_block(otherwise, ...)` to after that `if` block (the `else` block runs when the optional is `none`, before anything is unwrapped), with the comment `// The \`else\` block runs when \`value\` is \`none\`: nothing has been unwrapped or moved yet.` above the `if`.

In `kestrel/check/src/body/maps.rs`, the map loop: replace
```rust
        self.loop_breaks.push(false);
        self.check_block(body, Expect::Discard);
        self.loop_breaks.pop();
```
with
```rust
        let entry = self.moved.clone();
        self.loop_body(entry, body, true);
```
Remove any import that becomes unused (`Expect` in `maps.rs` or `collections.rs`, if nothing else uses it there).

- [ ] **Step 7: Run the checker tests**

Run: `cargo test -p kestrel-check`
Expected: PASS, including `moves`, `arenas` (its loop message is unchanged) and `gpu`.

- [ ] **Step 8: Add the error test and the drop-count cases**

Create `kestrel/tests/errors/moves.kes`:

```
struct Bag {
    mem: Arena = Arena()
    n: int = 0
}

go(c: bool) {
    var keep = list[Bag]()
    let t = Bag()
    if c {
        keep.add(t)
    }
    print(t.n)
    let u = Bag()
    for i from 0 to 3 {
        keep.add(u)
    }
}

main() {
    go(true)
}

// error 12: `t` may have been moved earlier, so it can't be used here
// error 15: `u` is moved inside a loop, so the next time around it would already be gone
```

In `kestrel/codegen/tests/gpu.rs`, in `every_way_out_of_a_scope_and_every_container_drops_each_object_once`, add these two entries at the end of `cases`:

```rust
        // A value moved on one path is dropped once, by whoever has it then.
        (
            "pick(c: bool) -> int {\n var keep = list[Tex]()\n let t = make(1)\n if c {\n  keep.add(t)\n }\n keep.len\n}\n\
             main() {\n print(pick(true), pick(false), dropped())\n}\n",
            "1 0 2\n",
        ),
        // A loop that moves its variable and gives it a new value each time around.
        (
            "main() {\n var keep = list[Tex]()\n var t = make(1)\n var i = 0\n while i < 3 {\n  keep.add(t)\n  t = make(i + 2)\n  i += 1\n }\n print(keep.len, t.w, dropped())\n}\n",
            "3 4 0\n",
        ),
```

- [ ] **Step 9: Run everything and lint**

Run: `cargo test --workspace --no-fail-fast` then `cargo clippy --workspace --all-targets -- -D warnings`
Expected: all pass (554 before this task, plus the new ones); clippy clean. Check `wc -l check/src/body/*.rs`: each under about 400.

- [ ] **Step 10: Commit**

```bash
git add kestrel/check kestrel/tests/errors/moves.kes kestrel/codegen/tests/gpu.rs
git commit -m "Let moves follow the program's paths: per branch, through loops that leave, and back after a new value"
git push
```

---

### Task 2: Views: no move under a reader, and `let ... else` reads in place (spec part B)

**Files:**
- Create: `kestrel/check/src/body/let_else.rs`
- Modify: `kestrel/check/src/body/collections.rs` (remove `check_let_else`)
- Modify: `kestrel/check/src/body/moves.rs` (`consume_on`'s `Ident` arm)
- Modify: `kestrel/check/src/body/mod.rs` (`mod let_else`, `let_guards` field, `check_block`)
- Modify: `kestrel/check/src/tables.rs` (`let_else_views`)
- Modify: `kestrel/kir/src/lower/collections.rs` (`let_else`, new `let_else_view`)
- Test: `kestrel/check/tests/views.rs` (create), `kestrel/codegen/tests/gpu.rs` (one case), `kestrel/tests/programs/let_else_views.kes` (create)

**Interfaces:**
- Consumes (Task 1): `BodyChecker::consume`, `consume_on`, `moved`.
- Produces: `pub let_else_views: HashSet<LocalId>` on `Tables` (checker → KIR); `pub(crate) struct LetGuard { view: LocalId, root: LocalId, depth: usize }`; `BodyChecker::let_guards: Vec<LetGuard>`.

- [ ] **Step 1: Write the failing checker tests**

Create `kestrel/check/tests/views.rs`:

```rust
//! Views read a place without copying it: nothing may move what a view reads, and
//! `let x = opt else` reads in place when `opt` is a place that can't change.

use kestrel_check::{check, Tables};
use kestrel_syntax::parse;

const HEAD: &str = "struct Bag {\n    mem: Arena = Arena()\n    n: int = 0\n}\nstruct Holder {\n    b: Bag?\n}\nenum Slot {\n    Empty\n    Full { b: Bag }\n}\npeek(o: Bag?) -> int {\n    1\n}\n";

fn run(body: &str) -> (Option<Tables>, Vec<String>) {
    let src = format!("{HEAD}{body}");
    let (p, d) = parse(&src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().map(|d| d.message).collect())
}

fn errors(body: &str) -> Vec<String> {
    run(body).1
}

fn none() -> Vec<String> {
    Vec::new()
}

#[test]
fn moving_a_matched_variable_while_an_arm_reads_it_is_an_error() {
    let body = "main() {\n var keep = list[Slot]()\n let s = Slot.Full(Bag())\n match s {\n  Full f {\n   keep.add(s)\n   print(f.b.n)\n  }\n  else { }\n }\n}\n";
    assert_eq!(errors(body), vec!["can't move `s` here: this `match` arm's `f` reads from it".to_string()]);
    // An arm with no binding reads nothing.
    let free = "main() {\n var keep = list[Slot]()\n let s = Slot.Full(Bag())\n match s {\n  Full {\n   keep.add(s)\n  }\n  else { }\n }\n}\n";
    assert_eq!(errors(free), none());
}

#[test]
fn let_else_unwraps_an_owning_parameter_field_or_self_in_place() {
    assert_eq!(errors("size(o: Bag?) -> int {\n let b = o else {\n  return 0\n }\n b.n\n}\nmain() {}\n"), none());
    assert_eq!(errors("main() {\n let h = Holder(b: Bag())\n let b = h.b else {\n  return\n }\n print(b.n)\n}\n"), none());
    let method = "struct Level {\n b: Bag?\n\n size(self) -> int {\n  let b = self.b else {\n   return 0\n  }\n  b.n\n }\n}\nmain() {}\n";
    assert_eq!(errors(method), none());
}

#[test]
fn a_whole_owning_local_still_moves() {
    let body = "main() {\n var keep = list[Bag]()\n let o: Bag? = Bag()\n let b = o else {\n  return\n }\n keep.add(b)\n print(peek(o))\n}\n";
    assert_eq!(errors(body), vec!["`o` was moved earlier, so it can't be used here".to_string()]);
}

#[test]
fn a_var_unwrap_still_moves_or_copies() {
    let body = "main() {\n let h = Holder(b: Bag())\n var b = h.b else {\n  return\n }\n}\n";
    assert_eq!(
        errors(body),
        vec!["can't move this out: it holds an `Arena`, so it can't be copied, and only whole variables move".to_string()]
    );
}

#[test]
fn a_view_and_what_it_reads_cant_move() {
    let field = "main() {\n var keep = list[Bag]()\n let h = Holder(b: Bag())\n let b = h.b else {\n  return\n }\n keep.add(b)\n}\n";
    assert_eq!(errors(field), vec!["can't move `b`: it's inside `h`, and it holds an `Arena`, so it can't be copied".to_string()]);
    let param = "take(o: Bag?, var keep: list[Bag]) {\n let b = o else {\n  return\n }\n keep.add(b)\n}\nmain() {}\n";
    assert_eq!(
        errors(param),
        vec!["can't move `b`: it's inside parameter `o`, which belongs to the caller, and it holds an `Arena`, so it can't be copied".to_string()]
    );
    let root = "main() {\n var keep = list[Holder]()\n let h = Holder(b: Bag())\n let b = h.b else {\n  return\n }\n keep.add(h)\n print(b.n)\n}\n";
    assert_eq!(errors(root), vec!["can't move `h` here: `b` reads from it".to_string()]);
    // After the block that declares the view, the root may move.
    let later = "main() {\n var keep = list[Holder]()\n let h = Holder(b: Bag())\n if true {\n  let b = h.b else {\n   return\n  }\n  print(b.n)\n }\n keep.add(h)\n}\n";
    assert_eq!(errors(later), none());
}

#[test]
fn only_heap_payloads_of_places_that_cant_change_are_views() {
    let views = |body: &str| {
        let (t, errs) = run(body);
        assert!(errs.is_empty(), "{errs:?}");
        t.unwrap().let_else_views.len()
    };
    assert_eq!(views("count(name: string?) -> int {\n let n = name else {\n  return 0\n }\n 1\n}\nmain() {}\n"), 1);
    assert_eq!(views("count(name: string?) -> int {\n var n = name else {\n  return 0\n }\n 1\n}\nmain() {}\n"), 0);
    assert_eq!(views("count(o: int?) -> int {\n let n = o else {\n  return 0\n }\n n\n}\nmain() {}\n"), 0);
    // A `var` root can change while the view reads it, so it's copied.
    assert_eq!(views("main() {\n var names: string? = \"a\"\n let n = names else {\n  return\n }\n print(n)\n}\n"), 0);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p kestrel-check --test views`
Expected: FAIL to compile (`let_else_views` doesn't exist on `Tables`). After Step 3 adds the field, the behaviour tests still fail (the param and field cases report "can't move parameter" / "can't move this out", the match case reports nothing).

- [ ] **Step 3: Add the table, the guard, and the `let ... else` checker file**

In `kestrel/check/src/tables.rs`, after the `locked_map_loops` field add:
```rust
    /// `let x = opt else` bindings that read the optional's payload in place (views), instead of
    /// moving or copying it. KIR maps uses of `x` to the payload's place and doesn't drop `x`.
    pub let_else_views: HashSet<LocalId>,
```

Create `kestrel/check/src/body/let_else.rs`:

```rust
//! `let x = opt else { ... }`: unwrapping an optional. When `opt` is a place that can't change,
//! `x` reads its payload in place (a view, like a loop's item) instead of moving or copying it.

use kestrel_syntax::ast::{Binding, Block, Expr, ExprKind, TypeExpr};

use super::{BodyChecker, Expect};
use crate::collect::resolve_type;
use crate::tables::{LocalId, Res};
use crate::ty::Ty;

/// A `let ... else` view in scope: `view` reads from inside `root` until the end of the block that
/// declares it, which is open while there are at least `depth` scopes.
#[derive(Clone)]
pub(crate) struct LetGuard {
    pub(crate) view: LocalId,
    pub(crate) root: LocalId,
    pub(crate) depth: usize,
}

impl<'a> BodyChecker<'a> {
    pub(crate) fn check_let_else(&mut self, is_var: bool, bind: &Binding, ty: Option<&TypeExpr>, value: &Expr, otherwise: &Block) {
        let ann = ty.map(|t| resolve_type(t, self.t, self.diags));
        let want = ann.map(|t| if t == Ty::Error { t } else { Ty::Optional(self.t.intern_elem(t)) });
        let found = self.check_value(value, want);
        let inner = match found {
            Ty::Optional(e) => self.t.elem(e),
            Ty::Error => Ty::Error,
            other => {
                let msg = format!("`let ... else` unwraps an optional (`T?`), but this is {}", self.tn(other));
                self.error(value.span, msg);
                Ty::Error
            }
        };
        // The `else` block runs when `value` is `none`: nothing has been unwrapped or moved yet.
        // It must leave, so what it moves doesn't flow past it.
        let before = self.moved.clone();
        if self.check_block(otherwise, Expect::Discard) != Ty::Never {
            self.error_help(otherwise.span, "the `else` block has to leave", "end it with `return`, `break` or `continue`");
        }
        self.moved = before;
        let root = if inner == Ty::Error || is_var { None } else { self.view_root(value, inner) };
        if root.is_none() {
            self.consume(value);
        }
        let l = self.declare(bind, inner, is_var, false, false);
        if let Some(root) = root {
            self.t.locals[l.0 as usize].alias = Some(root);
            self.views.insert(l);
            self.note_view(l, value);
            self.t.let_else_views.insert(l);
            self.let_guards.push(LetGuard { view: l, root, depth: self.scopes.len() });
        }
    }

    /// The variable `x` reads from when `let x = value else` can be a view: `value` is a place
    /// rooted at a variable that can't change, it isn't a `T` made into a `T?`, and its payload
    /// (`inner`) holds memory worth not copying. A whole owning variable moves instead, as with `let`.
    fn view_root(&self, value: &Expr, inner: Ty) -> Option<LocalId> {
        let worth = self.t.has_heap(inner) || self.t.is_owning(inner);
        if !worth || self.t.coerce.contains_key(&value.id) || !self.is_place(value) {
            return None;
        }
        let root = self.place_root(value)?;
        self.immutable(root)?;
        if let Some(&Res::Local(l)) = self.t.res.get(&value.id) {
            if self.t.is_owning(inner) && !self.t.local(l).is_param && !self.views.contains(&l) {
                return None;
            }
        }
        Some(root)
    }

    /// Whether `e` is a variable, or fields and array or list elements of one: memory a view can read.
    fn is_place(&self, e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Ident(_) => matches!(self.t.res.get(&e.id), Some(Res::Local(_))),
            ExprKind::Field { base, .. } => self.t.field_index.contains_key(&e.id) && self.is_place(base),
            ExprKind::Index { base, .. } => matches!(self.t.ty(base.id), Ty::Array(_) | Ty::List(_)) && self.is_place(base),
            _ => false,
        }
    }
}
```

In `kestrel/check/src/body/collections.rs`: delete `check_let_else` (its doc comment too) and remove imports it alone used (`TypeExpr`, `resolve_type`, `Block` if unused).

In `kestrel/check/src/body/mod.rs`:
- Add `mod let_else;` after `mod expr;`.
- Add the field after `match_guards`:
  ```rust
      /// `let ... else` views in scope: nothing may move what they read.
      let_guards: Vec<let_else::LetGuard>,
  ```
  and initialize it with `Vec::new()` in `new` (add `let_guards` to the struct literal).
- In `check_block`, replace `self.scopes.pop();` with
  ```rust
          self.scopes.pop();
          // The block's `let ... else` views end with it.
          let depth = self.scopes.len();
          self.let_guards.retain(|g| g.depth <= depth);
  ```

- [ ] **Step 4: Refuse moves under a reader**

In `kestrel/check/src/body/moves.rs`, `consume_on`'s `ExprKind::Ident(_)` arm: replace the `if info.is_param { ... } else if self.views.contains(&l) { ... } else { ... }` chain with:

```rust
                if info.is_param {
                    let what = self.t.owns_what(ty);
                    let msg = format!("can't move parameter `{name}`: it belongs to the caller, and it holds {what}, so it can't be copied");
                    self.error(v.span, msg);
                } else if let Some(g) = self.let_guards.iter().find(|g| g.view == l).cloned() {
                    let root = self.t.local(g.root);
                    let inside = if root.is_param {
                        format!("parameter `{}`, which belongs to the caller", root.name)
                    } else {
                        format!("`{}`", root.name)
                    };
                    let what = self.t.owns_what(ty);
                    self.error(v.span, format!("can't move `{name}`: it's inside {inside}, and it holds {what}, so it can't be copied"));
                } else if self.views.contains(&l) {
                    let what = self.t.owns_what(ty);
                    let msg = format!("can't move `{name}` out of its collection: it holds {what}, so it can't be copied");
                    self.error(v.span, msg);
                } else if let Some(g) = self.match_guards.iter().find(|g| g.root == l).cloned() {
                    let binding = self.t.local(g.binding).name.clone();
                    self.error_help(
                        v.span,
                        format!("can't move `{name}` here: this `match` arm's `{binding}` reads from it"),
                        format!("move it after the `match`, or copy what you need out of `{binding}` first"),
                    );
                } else if let Some(g) = self.let_guards.iter().find(|g| g.root == l).cloned() {
                    let view = self.t.local(g.view).name.clone();
                    self.error_help(v.span, format!("can't move `{name}` here: `{view}` reads from it"), format!("move it after the block that declares `{view}`"));
                } else {
                    self.moved.insert(l, MoveAt { span: v.span, every });
                    self.t.moves.insert(v.id);
                }
```

(Both `MatchGuard` and `LetGuard` derive `Clone`; cloning the found guard ends the borrow of `self` before the error is reported.)

- [ ] **Step 5: Run the checker tests**

Run: `cargo test -p kestrel-check`
Expected: PASS (`views`, `moves`, `arenas`, `gpu`, all others).

- [ ] **Step 6: Write the failing KIR/runtime tests**

In `kestrel/codegen/tests/gpu.rs`, add to the `cases` of `every_way_out_of_a_scope_and_every_container_drops_each_object_once`:

```rust
        // `let ... else` over a parameter or a field reads in place: its owner drops the object once.
        (
            "struct Frame {\n t: Tex?\n}\nwidth(o: Tex?) -> int {\n let t = o else {\n  return 0\n }\n t.w\n}\n\
             main() {\n let f = Frame(t: make(7))\n let t = f.t else {\n  return\n }\n print(width(f.t), t.w, dropped())\n}\n",
            "7 7 0\n",
        ),
```

Create `kestrel/tests/programs/let_else_views.kes`:

```
// expect: 3 7
// expect: 2 [7, 0]
// expect: none
// `let ... else` reads an optional parameter or field in place, even when it owns an arena.
struct Pool {
    mem: Arena = Arena()
    items: list[int] = []
}

struct Level {
    pool: Pool?

    size(self) -> int {
        let p = self.pool else {
            return 0
        }
        p.items.len
    }
}

total(o: Pool?) -> int {
    let p = o else {
        return -1
    }
    var sum = 0
    for v in p.items {
        sum += v
    }
    sum
}

struct Tagged {
    tag: string?
    n: int
}

main() {
    var pool = Pool()
    in pool.mem {
        pool.items = [1, 2, 4]
    }
    let level = Level(pool: pool)
    print(level.size(), total(level.pool))
    // A view inside a `par for` body, over an item's field of a list that can't change.
    let tagged = [Tagged(tag: "seven", n: 7), Tagged(tag: none, n: 9)]
    var lens = [0, 0]
    par for i, out in lens {
        let t = tagged[i].tag else {
            continue
        }
        if t == "seven" {
            out = 7
        } else {
            out = 1
        }
    }
    print(lens.len, lens)
    let empty = Level(pool: none)
    let p = empty.pool else {
        print("none")
        return
    }
    print(p.items.len)
}
```

(The `par for` part was run with today's compiler, where it copies: it prints `2 [7, 0]`. After this task `t` is a view, and the output must not change.)

- [ ] **Step 7: Run them to verify they fail**

Run: `cargo test -p kestrel-codegen --test gpu` and `cargo test -p kestrel-cli --test programs let_else_views`
Expected: FAIL. KIR still lowers the view as a `Let` + `declared`, so the GPU object is dropped twice (the drop count assertion fails) and the program may crash or print wrong values.

- [ ] **Step 8: Lower views in KIR**

In `kestrel/kir/src/lower/collections.rs`, add `use kestrel_check::LocalId;` to the imports, and at the start of `let_else` add:

```rust
        if let Some(&var) = self.t.bind_local.get(&bind.id) {
            if self.t.let_else_views.contains(&var) {
                return self.let_else_view(var, value, otherwise, out);
            }
        }
```

Then add, after `let_else`:

```rust
    /// `let x = opt else { ... }` where the checker made `x` a view (see
    /// `Tables::let_else_views`): uses of `x` read the payload of `opt` in place. Nothing is
    /// copied and `x` isn't dropped: whoever owns `opt` drops it.
    fn let_else_view(&mut self, var: LocalId, value: &ast::Expr, otherwise: &ast::Block, out: &mut Vec<Stmt>) {
        let line = self.line(value.span);
        // A copy here would drop an owning payload twice, so a view must be a place.
        let mut opt = self.place(value).expect("the checker makes views only of places");
        self.stabilize(&mut opt, out);
        let tag = Expr { kind: ExprKind::Tag(opt.clone()), ty: KTy::U8, line };
        let zero = Expr { kind: ExprKind::Int(0), ty: KTy::U8, line };
        let cond = Expr { kind: ExprKind::Binary(BinOp::Eq, Box::new(tag), Box::new(zero)), ty: KTy::Bool, line };
        let then_body = self.block_stmts(otherwise);
        out.push(Stmt::If { cond, then_body, else_body: Vec::new() });
        opt.path.push(Proj::Payload);
        self.elems.insert(var, opt);
    }
```

- [ ] **Step 9: Run everything and lint**

Run: `cargo test --workspace --no-fail-fast` then `cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS (`optionals.kes`, `par_for_strings.kes`, `promote_exits.kes`, `scalarize_payloads.kes` unchanged); clippy clean. `wc -l` on the touched files: under about 400.

- [ ] **Step 10: Commit**

```bash
git add kestrel/check kestrel/kir kestrel/codegen/tests/gpu.rs kestrel/tests/programs/let_else_views.kes
git commit -m "Read let-else payloads in place when the optional can't change, and refuse moves under a reader"
git push
```

---

### Task 3: Struct and enum values in defaults and constants, with a cycle check (spec part C)

**Files:**
- Create: `kestrel/check/src/body/default_cycles.rs`
- Modify: `kestrel/check/src/body/consts.rs` (`is_const_expr`, `names_used`, `check_const`; delete `builds_user_struct`, `uses_struct_constant`, `holds_user_struct`)
- Modify: `kestrel/check/src/body/structs.rs:10-37` (`check_field_defaults`, new `builds_owning`)
- Modify: `kestrel/check/src/body/mod.rs` (re-exports, `mod default_cycles`)
- Modify: `kestrel/check/src/lib.rs:34-37` (run the cycle check)
- Modify: `kestrel/check/src/collect.rs:240-252` (`default_allocates`)
- Modify: `kestrel/kir/src/lower/consts.rs:219-255` (`uses_in` follows method-call arguments)
- Test: `kestrel/check/tests/defaults.rs` (create), `kestrel/check/tests/structs.rs` (flip the ban tests), `kestrel/tests/errors/field_defaults.kes`, `kestrel/tests/programs/struct_defaults.kes` (create), and any test pinning the old constants message

**Interfaces:**
- Produces: `pub(crate) fn is_const_expr(e: &Expr, t: &Tables) -> bool` (the `user_structs` parameter is gone); `pub(crate) fn check_default_cycles(program: &Program, t: &Tables, diags: &mut Vec<Diagnostic>)`.

- [ ] **Step 1: Write the failing tests**

Create `kestrel/check/tests/defaults.rs`:

```rust
//! Field defaults and constants may be struct and enum values; defaults that build each other
//! forever are an error, and so is a default that builds a value owning an arena or GPU object.

use kestrel_check::check;
use kestrel_syntax::parse;

const HEAD: &str = "enum State {\n Idle\n Run\n}\nenum Shape {\n Dot\n Circle { r: float }\n}\nstruct Color {\n r: float\n g: float\n b: float\n}\nlet RED = Color(1, 0, 0)\nlet START = State.Idle\n";

fn errors(body: &str) -> Vec<String> {
    let src = format!("{HEAD}{body}");
    let (p, d) = parse(&src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    check(&p).1.into_iter().map(|d| d.message).collect()
}

fn none() -> Vec<String> {
    Vec::new()
}

const LOOP_HELP: &str = "; set the field when you build it, or start it empty (`[]`)";

#[test]
fn defaults_and_constants_may_be_struct_and_enum_values() {
    let body = "struct Player {\n tint: Color = RED\n state: State = START\n shape: Shape = Shape.Circle(2)\n mode: State = State.Run\n dir: vec2 = vec2(0, 1)\n}\nlet ORDER = [State.Idle, State.Run]\nmain() {\n print(Player(), ORDER)\n}\n";
    assert_eq!(errors(body), none());
}

#[test]
fn a_default_may_build_a_struct_that_holds_heap_memory() {
    let body = "struct Inventory {\n items: list[int] = []\n}\nstruct Player {\n inv: Inventory = Inventory()\n names: [string] = [\"a\"]\n}\nmain() {\n print(Player())\n}\n";
    assert_eq!(errors(body), none());
}

#[test]
fn a_default_cant_build_an_owning_value_except_an_arena() {
    assert_eq!(errors("struct Pool {\n mem: Arena = Arena()\n}\nmain() {}\n"), none());
    assert_eq!(
        errors("struct Pool {\n mem: Arena = Arena()\n}\nstruct World {\n pool: Pool = Pool()\n}\nmain() {}\n"),
        vec!["field defaults can't build a value that owns an `Arena` or a GPU object; set the field when you build it".to_string()]
    );
}

#[test]
fn defaults_that_build_each_other_forever_are_an_error() {
    assert_eq!(
        errors("struct Node {\n kids: list[Node] = [Node()]\n}\nmain() {}\n"),
        vec![format!("field defaults build each other in a loop: Node.kids -> Node.kids{LOOP_HELP}")]
    );
    assert_eq!(
        errors("struct Tree {\n kids: [Tree] = KIDS\n}\nlet KIDS = [Tree()]\nmain() {}\n"),
        vec![format!("field defaults build each other in a loop: Tree.kids -> KIDS -> Tree.kids{LOOP_HELP}")]
    );
    assert_eq!(
        errors("struct A {\n bs: list[B] = [B()]\n}\nstruct B {\n items: list[A] = [A()]\n}\nmain() {}\n"),
        vec![format!("field defaults build each other in a loop: A.bs -> B.items -> A.bs{LOOP_HELP}")]
    );
    // A field given at the construction isn't built from its default.
    assert_eq!(errors("struct Node {\n kids: list[Node] = [Node(kids: [])]\n}\nmain() {}\n"), none());
}

#[test]
fn defaults_must_be_constant_expressions() {
    assert_eq!(
        errors("f() -> int {\n 1\n}\nstruct P {\n x: int = f()\n}\nmain() {}\n"),
        vec!["field defaults can only use literals, operators, constants, struct and enum values, and pure built-ins like `sqrt`".to_string()]
    );
    assert_eq!(
        errors("f() -> int {\n 1\n}\nlet X = f()\nmain() {}\n"),
        vec!["constants can only use literals, operators, other constants, struct and enum values, and pure built-ins like `sqrt`".to_string()]
    );
}

#[test]
fn a_string_optional_default_from_a_constant_points_at_no_arena() {
    let body = "let NAME = \"n\"\nstruct S {\n name: string? = NAME\n}\nmain() {\n var keep = S()\n arena {\n  keep = S()\n }\n print(keep)\n}\n";
    assert_eq!(errors(body), none());
}
```

Update `kestrel/tests/errors/field_defaults.kes`'s comments (the program stays as it is) to:

```
// error 10: field defaults build each other in a loop: Node.kids -> Node.kids; set the field when you build it, or start it empty (`[]`)
// error 14: field defaults build each other in a loop: Tree.kids -> KIDS -> Tree.kids; set the field when you build it, or start it empty (`[]`)
// error 27: this memory is freed at the end of the `arena { }` block, but `keep` lives longer
```

(Line 6, `inner: Inner = Inner()`, now compiles.)

Create `kestrel/tests/programs/struct_defaults.kes`:

```
// expect: Player(tint: Color(r: 1, g: 0, b: 0), state: State.Idle, shape: Shape.Circle(r: 2), inv: Inventory(items: [1, 2]), loadout: Kit.Bag(ids: [3, 4]))
// expect: Kit.Bag(ids: [3, 4]) 2
// expect: [State.Idle, State.Run]
// Field defaults and constants may be struct and enum values, including ones that hold lists.
struct Color {
    r: float
    g: float
    b: float
}

enum State {
    Idle
    Run
}

enum Shape {
    Dot
    Circle { r: float }
}

enum Kit {
    Empty
    Bag { ids: list[int] }
}

struct Inventory {
    items: list[int] = [1, 2]
}

let RED = Color(1, 0, 0)
let LOADOUT = Kit.Bag([3, 4])
let ORDER = [State.Idle, State.Run]

struct Player {
    tint: Color = RED
    state: State = State.Idle
    shape: Shape = Shape.Circle(2)
    inv: Inventory = Inventory()
    loadout: Kit = LOADOUT
}

count(k: Kit) -> int {
    match k {
        Bag b {
            b.ids.len
        }
        else {
            0
        }
    }
}

main() {
    var p = Player()
    p.inv.items.add(9)
    print(Player())
    print(LOADOUT, count(LOADOUT))
    print(ORDER)
}
```

(The print format was checked against today's compiler: an enum value prints as `Kit.Bag(ids: [3, 4])` or `State.Idle`, a user struct as `Color(r: 1, g: 0, b: 0)`. The test also shows that `p.inv.items.add(9)` doesn't change a fresh `Player()`: each construction builds its own default.)

In `kestrel/check/tests/structs.rs`, the tests `field_defaults_cant_build_structs` and `field_defaults_cant_use_constants_that_hold_structs` (around lines 185-264) pin the old ban. Rewrite each case: a default building a plain or heap struct, or naming a struct constant, now has no error; keep only cases that are still errors (non-constant expressions) with the new message `field defaults can only use literals, operators, constants, struct and enum values, and pure built-ins like \`sqrt\``. Rename the two tests `field_defaults_may_build_structs` and `field_defaults_may_use_struct_constants`. Search for every other pin of the old texts and update it:

Run: `grep -rn "can't use struct values yet\|the one struct a default may build\|constants can only use literals, operators and other constants\|field defaults can only use literals, operators and constants" kestrel docs --include=*.rs --include=*.kes --include=*.md`

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p kestrel-check --test defaults` and `cargo test -p kestrel-cli --test errors` and `cargo test -p kestrel-cli --test programs struct_defaults`
Expected: FAIL (struct defaults are refused; enum constants are refused; no cycle message; old constants message).

- [ ] **Step 3: Widen constant expressions**

In `kestrel/check/src/body/consts.rs`, replace `is_const_expr`'s doc comment and body with:

```rust
/// Whether `e` is a constant expression: literals, operators, constants, pure built-ins, struct
/// and enum values, and array and map literals of those. Constants and field defaults are built
/// again where they're used, so both take these (field defaults also can't build a value that owns
/// an arena or a GPU object: see `check_field_defaults`; loops are found by `check_default_cycles`).
pub(crate) fn is_const_expr(e: &Expr, t: &Tables) -> bool {
    let all = |args: &[Arg]| args.iter().all(|a| is_const_expr(&a.value, t));
    // `State.Idle` and `Shape.Circle(1)`: an enum's values.
    let names_enum = |base: &Expr| matches!(&base.kind, ExprKind::Ident(n) if t.enum_by_name.contains_key(n));
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::None | ExprKind::Str(_) | ExprKind::Ident(_) => true,
        ExprKind::Call { name, args, .. } if name == "Arena" => args.is_empty(),
        // Pure built-ins (`sqrt(2.0)`) are fine; the random ones change state.
        ExprKind::Call { name, args, .. } if Builtin::global(name).is_some_and(Builtin::is_pure) => all(args),
        // A struct construction (`Color(1, 0, 0)`, `vec2(0, 1)`) is a value like any other.
        ExprKind::Call { name, args, .. } if t.struct_by_name.contains_key(name) => all(args),
        ExprKind::Field { base, .. } => names_enum(base),
        ExprKind::MethodCall { receiver, args, .. } => names_enum(receiver) && all(args),
        ExprKind::Unary { expr, .. } => is_const_expr(expr, t),
        ExprKind::Binary { lhs, rhs, .. } => is_const_expr(lhs, t) && is_const_expr(rhs, t),
        // Each use builds a fresh array, like any other value.
        ExprKind::ArrayLit(items) => items.iter().all(|i| is_const_expr(i, t)),
        ExprKind::MapLit(entries) => entries.iter().all(|(k, v)| is_const_expr(k, t) && is_const_expr(v, t)),
        ExprKind::TypeCall { args, .. } => all(args),
        ExprKind::Interp(_) | ExprKind::Index { .. } | ExprKind::Call { .. } | ExprKind::If { .. } | ExprKind::Match { .. } => false,
    }
}
```

Delete `builds_user_struct`, `uses_struct_constant` and `holds_user_struct` (and their doc comments), and the now-unused `BUILTIN_STRUCTS` import if nothing else in the file uses it.

In `names_used`, change the call arm to also follow an enum value's arguments:
```rust
        ExprKind::Call { args, .. } | ExprKind::TypeCall { args, .. } | ExprKind::MethodCall { args, .. } => {
            args.iter().for_each(|a| names_used(&a.value, out))
        }
```

In `check_const`, change `is_const_expr(&c.value, bc.t, true)` to `is_const_expr(&c.value, bc.t)` and the message to `"constants can only use literals, operators, other constants, struct and enum values, and pure built-ins like \`sqrt\`"`.

In `kestrel/check/src/body/mod.rs`, change the re-export line to:
```rust
pub(crate) use consts::{check_consts, is_const_expr};
pub(crate) use default_cycles::check_default_cycles;
```
and add `mod default_cycles;` after `mod consts;`. (Remove `builds_user_struct` / `uses_struct_constant` from any other `use`.)

- [ ] **Step 4: Rewrite `check_field_defaults`**

In `kestrel/check/src/body/structs.rs`, change the imports to `use super::{is_const_expr, BodyChecker};` and `use crate::tables::{LocalId, Res, StructId, Tables, BUILTIN_STRUCTS};` (drop what's unused), and replace `check_field_defaults` with:

```rust
/// Field defaults are constant expressions of the field's type (see `is_const_expr`), built again
/// at every construction that leaves the field out. They can't build a value that owns an arena or
/// a GPU object (other than a fresh `Arena()`).
pub(crate) fn check_field_defaults(program: &Program, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let structs = program.items.iter().filter_map(|it| match it {
        Item::Struct(s) => Some(s),
        _ => None,
    });
    for (sid, s) in structs.enumerate() {
        for f in &s.fields {
            let Some(default) = &f.default else { continue };
            let sid = sid + BUILTIN_STRUCTS as usize;
            let Some(field) = t.structs[sid].fields.iter().find(|g| g.name == f.name) else { continue };
            let ty = field.ty;
            let mut bc = BodyChecker::new(t, diags, None, Ty::Unit, false);
            if !is_const_expr(default, bc.t) {
                bc.error(default.span, "field defaults can only use literals, operators, constants, struct and enum values, and pure built-ins like `sqrt`");
            } else if bc.check_value(default, Some(ty)) != Ty::Error && builds_owning(default, bc.t) {
                bc.error(default.span, "field defaults can't build a value that owns an `Arena` or a GPU object; set the field when you build it");
            }
        }
    }
}

/// Whether the checked default `e` builds a value that owns an `Arena` or a GPU object, other
/// than a fresh `Arena()`.
fn builds_owning(e: &Expr, t: &Tables) -> bool {
    let any = |args: &[Arg]| args.iter().any(|a| builds_owning(&a.value, t));
    match &e.kind {
        ExprKind::Call { name, .. } if name == "Arena" => false,
        ExprKind::Call { args, .. } | ExprKind::MethodCall { args, .. } => t.is_owning(t.ty(e.id)) || any(args),
        // A constant is built again at each use, so naming an owning one builds one.
        ExprKind::Ident(_) => matches!(t.res.get(&e.id), Some(Res::Const(_))) && t.is_owning(t.ty(e.id)),
        ExprKind::TypeCall { args, .. } => any(args),
        ExprKind::Unary { expr, .. } => builds_owning(expr, t),
        ExprKind::Binary { lhs, rhs, .. } => builds_owning(lhs, t) || builds_owning(rhs, t),
        ExprKind::ArrayLit(items) => items.iter().any(|i| builds_owning(i, t)),
        ExprKind::MapLit(entries) => entries.iter().any(|(k, v)| builds_owning(k, t) || builds_owning(v, t)),
        _ => false,
    }
}
```

- [ ] **Step 5: Add the cycle check**

Create `kestrel/check/src/body/default_cycles.rs`:

```rust
//! Field defaults and constants are built again at each use, so ones that build each other would
//! never finish (`kids: list[Node] = [Node()]`, or through a constant). This finds such loops
//! before KIR, whose constant ordering and construction would otherwise recurse forever.

use std::collections::HashMap;

use kestrel_syntax::ast::{Expr, ExprKind, Item, Program};
use kestrel_syntax::{Diagnostic, Span};

use crate::tables::{ConstId, Res, StructId, Tables};

/// Something built from an expression at each use: a field's default, or a constant.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Node {
    Default(StructId, usize),
    Const(ConstId),
}

#[derive(Clone, Copy, PartialEq)]
enum Mark {
    /// On the stack: what it builds is being walked.
    Active,
    Done,
}

/// Reports each loop of defaults (and the constants they name) that build each other. Run it
/// after `check_field_defaults`, and only without earlier errors: it needs `ctor_args`.
pub(crate) fn check_default_cycles(program: &Program, t: &Tables, diags: &mut Vec<Diagnostic>) {
    let mut walk = Walk { program, t, marks: HashMap::new(), stack: Vec::new() };
    for (sid, s) in t.structs.iter().enumerate() {
        for fi in 0..s.fields.len() {
            walk.visit(Node::Default(StructId(sid as u32), fi), diags);
        }
    }
}

struct Walk<'a> {
    program: &'a Program,
    t: &'a Tables,
    marks: HashMap<Node, Mark>,
    /// The nodes being walked, outermost first.
    stack: Vec<Node>,
}

impl<'a> Walk<'a> {
    fn visit(&mut self, n: Node, diags: &mut Vec<Diagnostic>) {
        if self.marks.contains_key(&n) {
            return;
        }
        let Some(e) = self.expr(n) else { return };
        self.marks.insert(n, Mark::Active);
        self.stack.push(n);
        let mut next = Vec::new();
        self.builds(e, &mut next);
        for m in next {
            match self.marks.get(&m) {
                None => self.visit(m, diags),
                Some(Mark::Active) => self.report(m, diags),
                Some(Mark::Done) => {}
            }
        }
        self.stack.pop();
        self.marks.insert(n, Mark::Done);
    }

    /// The expression `n` is built from; `None` for a field with no default (or a built-in struct's).
    fn expr(&self, n: Node) -> Option<&'a Expr> {
        match n {
            Node::Default(sid, fi) => match self.program.items.get(self.t.strukt(sid).item)? {
                Item::Struct(decl) => decl.fields.get(fi)?.default.as_ref(),
                _ => None,
            },
            Node::Const(c) => match self.program.items.get(self.t.consts[c.0 as usize].item)? {
                Item::Const(decl) => Some(&decl.value),
                _ => None,
            },
        }
    }

    /// What building `e` also builds: the defaults of the fields a construction leaves out, and
    /// the constants it names.
    fn builds(&self, e: &'a Expr, out: &mut Vec<Node>) {
        match &e.kind {
            ExprKind::Ident(_) => {
                if let Some(&Res::Const(c)) = self.t.res.get(&e.id) {
                    out.push(Node::Const(c));
                }
            }
            ExprKind::Call { args, .. } => {
                if let (Some(&Res::Construct(sid)), Some(slots)) = (self.t.res.get(&e.id), self.t.ctor_args.get(&e.id)) {
                    out.extend(slots.iter().enumerate().filter(|(_, s)| s.is_none()).map(|(fi, _)| Node::Default(sid, fi)));
                }
                args.iter().for_each(|a| self.builds(&a.value, out));
            }
            ExprKind::MethodCall { args, .. } | ExprKind::TypeCall { args, .. } => args.iter().for_each(|a| self.builds(&a.value, out)),
            ExprKind::Unary { expr, .. } => self.builds(expr, out),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.builds(lhs, out);
                self.builds(rhs, out);
            }
            ExprKind::ArrayLit(items) => items.iter().for_each(|i| self.builds(i, out)),
            ExprKind::MapLit(entries) => entries.iter().for_each(|(k, v)| {
                self.builds(k, out);
                self.builds(v, out);
            }),
            _ => {}
        }
    }

    /// `back` is on the stack and the node on top builds it: a loop, reported where it closes.
    fn report(&self, back: Node, diags: &mut Vec<Diagnostic>) {
        let from = self.stack.iter().position(|&n| n == back).expect("an active node is on the stack");
        let path: Vec<String> = self.stack[from..].iter().chain([&back]).map(|&n| self.name(n)).collect();
        let span = self.expr(back).map_or(Span::default(), |e| e.span);
        let msg = format!(
            "field defaults build each other in a loop: {}; set the field when you build it, or start it empty (`[]`)",
            path.join(" -> ")
        );
        diags.push(Diagnostic::error(span, msg));
    }

    fn name(&self, n: Node) -> String {
        match n {
            Node::Default(sid, fi) => {
                let s = self.t.strukt(sid);
                format!("{}.{}", s.name, s.fields[fi].name)
            }
            Node::Const(c) => self.t.consts[c.0 as usize].name.clone(),
        }
    }
}
```

The `Tree` case's loop is reported at `KIDS -> Tree.kids`'s closing node: the walk starts at `Tree.kids`, goes to `KIDS`, whose `[Tree()]` leaves out `kids`, which is `Tree.kids` again (on the stack), so the path is `Tree.kids -> KIDS -> Tree.kids` and the span is `Tree.kids`'s default (line 14 of `field_defaults.kes`). If the struct order makes the walk start elsewhere and the path come out rotated, start the walk at each struct's fields in declaration order (as written above) and keep the test's expected text in that order.

In `kestrel/check/src/lib.rs`, after `body::check_field_defaults(program, &mut t, &mut diags);` add:

```rust
    // Defaults that build each other would recurse forever in KIR; this needs a clean check so far.
    if !diags.iter().any(|d| d.is_error()) {
        body::check_default_cycles(program, &t, &mut diags);
    }
```

- [ ] **Step 6: Look through optionals in `default_allocates`, and follow enum values in KIR's constant walk**

In `kestrel/check/src/collect.rs`, replace `default_allocates`'s body with:

```rust
    if !t.has_heap(ty) {
        return false;
    }
    // A `T?` default is a `T` or `none`.
    let inner = match ty {
        Ty::Optional(e) => t.elem(e),
        other => other,
    };
    match &default.kind {
        ExprKind::None | ExprKind::Str(_) => false,
        ExprKind::Ident(name) if inner == Ty::Str => !t.consts.iter().any(|c| &c.name == name),
        ExprKind::ArrayLit(items) if items.is_empty() => matches!(inner, Ty::List(_)),
        _ => true,
    }
```

In `kestrel/kir/src/lower/consts.rs`, `uses_in`: change `ast::ExprKind::TypeCall { args, .. } => all(args, out),` to `ast::ExprKind::TypeCall { args, .. } | ast::ExprKind::MethodCall { args, .. } => all(args, out),` (an enum value's payload may name a heap constant that must be built first).

- [ ] **Step 7: Run everything and lint**

Run: `cargo test --workspace --no-fail-fast` then `cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS; clippy clean; `collect.rs` still under 400 lines.

- [ ] **Step 8: Commit**

```bash
git add kestrel/check kestrel/kir kestrel/tests
git commit -m "Allow struct and enum values in field defaults and constants, and report defaults that build each other"
git push
```

---

### Task 4: Zero vectors and clearer construction errors (spec part D)

**Files:**
- Modify: `kestrel/check/src/body/structs.rs` (`check_construct` missing fields)
- Modify: `kestrel/check/src/body/call.rs:45-51` (unknown capitalized call)
- Modify: `kestrel/kir/src/lower/call.rs:194-198` (built-in struct's missing field is zero)
- Modify: `kestrel/syntax/src/parser/mod.rs` (helper), `stmt.rs` (`let_stmt`), `item.rs` (`struct_member`)
- Test: `kestrel/tests/programs/zero_vectors.kes` (create), `kestrel/tests/errors/value_in_type_place.kes` and `kestrel/tests/errors/construction.kes` (create), `kestrel/check/tests/gpu.rs:183` (message), `kestrel/check/tests/check.rs` (if it pins a multi-field "missing field")

**Interfaces:**
- Produces: nothing other tasks use.

- [ ] **Step 1: Write the failing tests**

Create `kestrel/tests/programs/zero_vectors.kes`:

```
// expect: vec2(x: 0, y: 0) vec3(x: 0, y: 0, z: 0)
// expect: Ball(velocity: vec3(x: 0, y: 0, z: 0), spin: vec2(x: 0, y: 0))
// expect: vec3(x: 0, y: 0, z: 0)
// `vec2()` and `vec3()` are zero vectors, like Godot's `Vector2()` and `Vector3()`.
let ORIGIN = vec3()

struct Ball {
    velocity: vec3 = vec3()
    spin: vec2 = vec2()
}

main() {
    let a = vec2()
    let b = vec3()
    print(a, b)
    print(Ball())
    print(ORIGIN)
}
```

The driver doesn't run the checker on a file with parse errors (`driver/src/lib.rs`, `compile_as`), so parse errors and checker errors go in separate files.

Create `kestrel/tests/errors/value_in_type_place.kes`:

```
struct Ball {
    velocity: vec3()
}

main() {
    var speed: vec3()
}

// error 2: a type goes after `:`, and the value after `=`
// error 6: a type goes after `:`, and the value after `=`
```

Create `kestrel/tests/errors/construction.kes`:

```
main() {
    let a = vec3(1)
    let b = vec2(x: 1)
    let c = Paddle(pos: vec2(0, 0))
}

// error 2: missing fields `y` and `z` for `vec3`
// error 3: missing field `y` for `vec2`
// error 4: unknown struct `Paddle`
```

If the parser's recovery after the struct field's error skips `main` (so only line 2 is reported in `value_in_type_place.kes`), make the struct member's recovery stop at the end of the field's line, the way `recover_stmt` does for statements, so both are reported.

In `kestrel/check/tests/gpu.rs:183`, change the expected `"unknown function \`GpuHandle\`"` to `"unknown struct \`GpuHandle\`"`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p kestrel-cli --test programs zero_vectors` and `cargo test -p kestrel-cli --test errors`
Expected: FAIL (`missing field \`x\` for \`vec3\`` three times; `expected \`=\`, found \`(\``; `unknown function \`Paddle\``).

- [ ] **Step 3: Implement**

In `kestrel/check/src/body/structs.rs`, `check_construct`: add `VEC2, VEC3` to the `crate::tables` import and replace the `for (fi, slot) in slots.iter().enumerate() { ... }` loop with:

```rust
        // `vec2()` and `vec3()` are zero vectors, like Godot's `Vector2()`.
        let vector = matches!(sid, VEC2 | VEC3);
        let zero_vector = vector && args.is_empty();
        let missing: Vec<String> = slots
            .iter()
            .enumerate()
            .filter(|&(fi, slot)| slot.is_none() && !info.fields[fi].has_default && !zero_vector)
            .map(|(fi, _)| format!("`{}`", info.fields[fi].name))
            .collect();
        if bound && !missing.is_empty() {
            let msg = match missing.as_slice() {
                [one] => format!("missing field {one} for `{}`", info.name),
                [init @ .., last] => format!("missing fields {} and {last} for `{}`", init.join(", "), info.name),
                [] => unreachable!("not empty"),
            };
            if vector {
                self.error_help(e.span, msg, format!("give every component, or none for a zero vector: `{}()`", info.name));
            } else {
                self.error(e.span, msg);
            }
            ok = false;
        }
```

In `kestrel/check/src/body/call.rs`, replace `self.error(*name_span, format!("unknown function \`{name}\`"));` with:

```rust
            if name.starts_with(|c: char| c.is_ascii_uppercase()) {
                self.error_help(
                    *name_span,
                    format!("unknown struct `{name}`"),
                    format!("declare it at the top level of a file (`struct {name} {{ ... }}`), or bring it in from a module with `use {{ {name} from m }}`"),
                );
            } else {
                self.error(*name_span, format!("unknown function `{name}`"));
            }
```

In `kestrel/kir/src/lower/call.rs`, `construct`: replace the `None => { ... }` arm with:

```rust
                None => match program.items.get(t.strukt(sid).item) {
                    Some(ast::Item::Struct(decl)) => {
                        self.value(decl.fields[fi].default.as_ref().expect("checker requires a default for a missing field"))
                    }
                    // `vec2()` and `vec3()`: a built-in vector's components start at zero.
                    _ => Expr { kind: ExprKind::Float(0.0), ty: super::kty(t.strukt(sid).fields[fi].ty), line: self.line(e.span) },
                },
```

In `kestrel/syntax/src/parser/mod.rs`, add `TypeExpr, TypeKind` to the `crate::ast` import, and add to the `impl`:

```rust
    /// `name: vec3()` where `=` should follow the type: the value was written where the type
    /// goes. `decl` is what comes before the `:` (`var velocity`, or a field's name).
    fn value_in_type_place<T>(&mut self, decl: &str, ty: &TypeExpr) -> PResult<T> {
        let mut d = Diagnostic::error(self.span(), "a type goes after `:`, and the value after `=`");
        if let TypeKind::Named { .. } = ty.kind {
            let t = type_text(ty);
            d = d.with_help(format!("write `{decl}: {t} = {t}()`"));
        }
        self.diags.push(d);
        Err(())
    }
```

and, outside the `impl`:

```rust
/// A type as written: `vec3`, `list[int]`, `[int]`, `Color?`.
fn type_text(t: &TypeExpr) -> String {
    match &t.kind {
        TypeKind::Named { name, args } if args.is_empty() => name.clone(),
        TypeKind::Named { name, args } => format!("{name}[{}]", args.iter().map(type_text).collect::<Vec<_>>().join(", ")),
        TypeKind::Array(inner) => format!("[{}]", type_text(inner)),
        TypeKind::Optional(inner) => format!("{}?", type_text(inner)),
    }
}
```

In `kestrel/syntax/src/parser/stmt.rs`, `let_stmt`: after the `let ty = ...;` line add

```rust
        if let (Some(t), true) = (&ty, self.at(&Tok::LParen)) {
            let kw = if is_var { "var" } else { "let" };
            return self.value_in_type_place(&format!("{kw} {}", bind.name), t);
        }
```

In `kestrel/syntax/src/parser/item.rs`, `struct_member`: after `let ty = self.type_expr()?;` add

```rust
        if self.at(&Tok::LParen) {
            return self.value_in_type_place(&name, &ty);
        }
```

- [ ] **Step 4: Run everything and lint**

Run: `cargo test --workspace --no-fail-fast` then `cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS; clippy clean. Fix any other test that pinned three separate "missing field" messages for one construction (search: `grep -rn "missing field" kestrel/check/tests kestrel/tests`).

- [ ] **Step 5: Commit**

```bash
git add kestrel/check kestrel/kir kestrel/syntax kestrel/tests
git commit -m "Make vec2() and vec3() zero, and say what to do for a missing struct, missing fields, or a value in a type's place"
git push
```

---

### Task 5: Runtime errors name the file (spec part E)

**Files:**
- Modify: `kestrel/syntax/src/span.rs` (`LineIndex::file`)
- Modify: `kestrel/kir/src/ir.rs` (`pack_line`, `unpack_line`, `LINE_BITS`, `NO_FILE`, `Module::files`, doc comments of `Expr::line` and `Stmt::SetLine`), `kestrel/kir/src/lib.rs` (re-export if `ir` items aren't already public at the root), `kestrel/kir/src/lower/mod.rs` (`line`, `Module { .. files: Vec::new() }`)
- Modify: `kestrel/driver/src/lib.rs:103-106` (fill `module.files`)
- Modify: `kestrel/runtime/src/panic.rs` (`FileName`, `of_file`, `kestrel_rt_call_file`, `fail_at_call`, `kestrel_rt_panic`, `kestrel_rt_panic_index`), `kestrel/runtime/src/arena.rs:170-176` (`kestrel_rt_alloc`), `kestrel/runtime/src/map.rs:241-262` (`kestrel_rt_map_insert`), and the runtime's own tests that call these
- Modify: `kestrel/codegen/src/rt.rs` (signatures, `call_file`, JIT symbol), `kestrel/codegen/src/globals.rs` (`define_file_names`), `kestrel/codegen/src/lib.rs:62-63`, `kestrel/codegen/src/func/mod.rs` (`Shared::files`), `kestrel/codegen/src/func/globals.rs` (`loc_args`, `set_line`), and the six call sites: `func/arith.rs:113-122` (`panic_if`), `func/place.rs:96-108` (`bounds_check`), `func/collections.rs:41-49` and `:56-63` (`array_new`, `array_lit`), `func/maps.rs:175-177` (`map_insert`), `func/maps.rs:229-231` (map changed)
- Test: `kestrel/cli/tests/modules.rs:107-115` (tighten, add exe and native variants), `kestrel/tests/programs/map_loop_add.kes:3`, `kestrel/tests/programs/par_for_panic.kes:2`, any pin of "elements is too large (line"
- Docs: `kestrel/godot/README.md:64`

**Interfaces:**
- Produces: `kestrel_kir::pack_line(file: usize, line: u32) -> u32`, `kestrel_kir::unpack_line(loc: u32) -> (u32, u32)` (line, file index), `kestrel_kir::NO_FILE: u32 = 1023`, `kir::Module::files: Vec<String>`; runtime `#[repr(C)] pub struct FileName { ptr: *const u8, len: i64 }` and `kestrel_rt_call_file: AtomicPtr<FileName>`; runtime signatures `kestrel_rt_panic(kind: i32, line: i32, file: *const FileName)`, `kestrel_rt_panic_index(index: i64, len: i64, line: i32, file: *const FileName)`, `kestrel_rt_alloc(count: i64, elem_size: i64, align: i64, line: i32, file: *const FileName)`, `kestrel_rt_map_insert(map, key, kind, entry_size, entry_align, line: i32, file: *const FileName)`.

- [ ] **Step 1: Write the failing tests**

In `kestrel/cli/tests/modules.rs`, replace `runtime_errors_report_the_line_in_their_own_file` with:

```rust
#[test]
fn runtime_errors_report_the_line_and_their_own_file() {
    let main = "use { at from geom }\nmain() {\n    print(at([1, 2], 5))\n}\n";
    let dir = project("modules_runtime", &[("main.kes", main), ("geom.kes", GEOM)]);
    // `xs[i]` is on line 24 of geom.kes; the check is inlined into `main`, and still names geom.kes.
    let want = "index 5 out of bounds for length 2 on line 24 of geom.kes";
    let out = kestrel(&["run"], &dir.join("main.kes"));
    assert_eq!(out.status.code(), Some(101));
    assert!(String::from_utf8_lossy(&out.stderr).contains(want), "{}", String::from_utf8_lossy(&out.stderr));
    // A built program carries the name too (only the file's name, not its folder).
    let exe = dir.join("main_runtime.exe");
    let built = kestrel(&["build", "-o", exe.to_str().unwrap()], &dir.join("main.kes"));
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let run = std::process::Command::new(&exe).output().expect("run the built program");
    assert_eq!(run.status.code(), Some(101));
    let err = String::from_utf8_lossy(&run.stderr);
    assert!(err.contains(want), "{err}");
    assert!(!err.contains(dir.to_str().unwrap()), "the built program mustn't carry the folder: {err}");
}

#[test]
fn a_native_error_names_the_module_file_that_called_it() {
    let shapes = "use draw\nuse { WHITE from draw }\n\npub dot() {\n    draw.circle(vec2(1, 1), 2, WHITE)\n}\n";
    let main = "use { dot from shapes }\nmain() {\n    dot()\n}\n";
    let dir = project("modules_native_error", &[("main.kes", main), ("shapes.kes", shapes)]);
    let out = kestrel(&["run"], &dir.join("main.kes"));
    assert_eq!(out.status.code(), Some(101));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("on line 5 of shapes.kes"), "{err}");
}
```

(`kestrel(args, file)` in this file appends the file after `args`, and the CLI takes `-o` anywhere, so `build -o <exe> main.kes` works. The native's message is "draw.circle was called before window.open" from `window/src/state.rs:121`, through `fail_at_call`. The native test opens no window: it fails before `window.open`, so it needs no `KESTREL_HEADLESS`.)

Update the program pins:
- `kestrel/tests/programs/map_loop_add.kes:3`: `// expect-stderr: can't add a key to a map while looping over it (line 13 of map_loop_add.kes)`
- `kestrel/tests/programs/par_for_panic.kes:2`: `// expect-stderr: index 8 out of bounds for length 8 on line 16 of par_for_panic.kes`
- Search for the arena one: `grep -rn "elements is too large" kestrel --include=*.kes --include=*.rs` and add ` of <file>.kes` inside its parentheses if a test pins it.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p kestrel-cli --test modules runtime_errors` and `cargo test -p kestrel-cli --test programs map_loop_add par_for_panic`
Expected: FAIL (messages have no file).

- [ ] **Step 3: Carry the file through syntax, KIR and the driver**

In `kestrel/syntax/src/span.rs`, add to `impl LineIndex`:

```rust
    /// Index of the file containing `offset`, in the order the files were given to `for_files`.
    pub fn file(&self, offset: u32) -> usize {
        self.files.iter().rposition(|(base, _)| *base <= offset).unwrap_or(0)
    }
```

In `kestrel/kir/src/ir.rs`, add near the top (after the imports):

```rust
/// Bits of a KIR line (`Expr::line`, `Stmt::SetLine`) that hold the line within its file; the
/// bits above hold the file's index in `Module::files`. Bit 31 stays clear.
pub const LINE_BITS: u32 = 21;
/// The file index that means "no name" (a file past the 1,023rd).
pub const NO_FILE: u32 = 1023;

/// A source position as KIR keeps it: the line within its file, and the file's index. A line
/// past 2,097,151 is clamped; a file index past 1,022 becomes `NO_FILE`.
pub fn pack_line(file: usize, line: u32) -> u32 {
    let file = u32::try_from(file).unwrap_or(NO_FILE).min(NO_FILE);
    (file << LINE_BITS) | line.min((1 << LINE_BITS) - 1)
}

/// The line and the file index of a packed KIR line (see `pack_line`).
pub fn unpack_line(loc: u32) -> (u32, u32) {
    (loc & ((1 << LINE_BITS) - 1), loc >> LINE_BITS)
}
```

Add to `pub struct Module`, after `main`:

```rust
    /// The source files' names (base names, like `geom.kes`), by the file index in each line
    /// (see `pack_line`). Empty when the compiler wasn't given names: runtime errors then give
    /// only the line.
    pub files: Vec<String>,
```

Update the doc comments of `Stmt::SetLine` and `Expr::line` to say the value is a packed line (see `pack_line`). Make sure `pack_line`, `unpack_line`, `NO_FILE` and `LINE_BITS` are reachable as `kestrel_kir::...` (check `kestrel/kir/src/lib.rs`: if it does `pub use ir::*;` nothing else is needed; otherwise add them to its re-exports).

In `kestrel/kir/src/lower/mod.rs`: change `line` to

```rust
    pub(crate) fn line(&self, span: Span) -> u32 {
        crate::ir::pack_line(self.lines.file(span.start), self.lines.line(span.start))
    }
```

and add `files: Vec::new()` to the `Module { ... }` literal in `lower` (the driver fills it).

In `kestrel/driver/src/lib.rs`, after `let mut module = kestrel_kir::lower(...);` (before `kestrel_kir::optimize`), add:

```rust
    // Runtime errors name the file by its base name only: a built program mustn't carry the folders.
    module.files = files.iter().map(|f| base_name(&f.path)).collect();
```

and at the end of the file:

```rust
/// `geom.kes` for `C:\game\geom.kes` (and `draw.kes` for a built-in module's `<builtin>/draw.kes`).
fn base_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}
```

- [ ] **Step 4: Take the name in the runtime**

In `kestrel/runtime/src/panic.rs`: change the import to `use std::sync::atomic::{AtomicI32, AtomicPtr, Ordering};` and add after `kestrel_rt_call_line`:

```rust
/// A source file's name as compiled code passes it to a runtime error: static data holding a
/// pointer to the name's bytes and its length. Compiled code passes null when it has no name.
#[repr(C)]
pub struct FileName {
    ptr: *const u8,
    len: i64,
}

/// " of geom.kes" for an error in that file, or nothing for null.
pub(crate) fn of_file(file: *const FileName) -> String {
    if file.is_null() {
        return String::new();
    }
    // SAFETY: compiled code passes a pointer to its static `FileName`, whose bytes are static too.
    let name = unsafe { std::slice::from_raw_parts((*file).ptr, (*file).len as usize) };
    format!(" of {}", String::from_utf8_lossy(name))
}

/// The file of the call whose line is in `kestrel_rt_call_line`, stored with it.
#[no_mangle]
#[allow(non_upper_case_globals)]
pub static kestrel_rt_call_file: AtomicPtr<FileName> = AtomicPtr::new(std::ptr::null_mut());
```

Change `fail_at_call`'s second arm to:
```rust
        line => fail(&format!("{msg} on line {line}{}", of_file(kestrel_rt_call_file.load(Ordering::Relaxed)))),
```
Change the two panic functions:
```rust
#[no_mangle]
pub extern "C" fn kestrel_rt_panic_index(index: i64, len: i64, line: i32, file: *const FileName) -> ! {
    fail(&format!("index {index} out of bounds for length {len} on line {line}{}", of_file(file)));
}

#[no_mangle]
pub extern "C" fn kestrel_rt_panic(kind: i32, line: i32, file: *const FileName) -> ! {
    // (the `what` match stays as it is)
    fail(&format!("{what} on line {line}{}", of_file(file)));
}
```

In `kestrel/runtime/src/arena.rs`, `kestrel_rt_alloc`: add the parameter `file: *const crate::panic::FileName` after `line: i32`, and the messages become `"negative array length {count} on line {line}{}"` and `"an array of {count} elements is too large (line {line}{})"`, each with `crate::panic::of_file(file)` as the extra argument.

In `kestrel/runtime/src/map.rs`, `kestrel_rt_map_insert`: add `file: *const crate::panic::FileName` after `line: i32`, and the message becomes `"can't add a key to a map while looping over it (line {line}{}); collect the keys in a list and change the map after the loop"` with `crate::panic::of_file(file)`.

Fix the runtime's own callers (they pass no file): `runtime/src/arena.rs:326`, `runtime/src/par.rs:305,319`, `runtime/src/print.rs:138` (`kestrel_rt_alloc(..., 0, std::ptr::null())`), and `runtime/src/map.rs:364,455` (`kestrel_rt_map_insert(..., 1, std::ptr::null())`). Run `grep -rn "kestrel_rt_alloc(\|kestrel_rt_map_insert(\|kestrel_rt_panic(\|kestrel_rt_panic_index(" kestrel --include=*.rs` and fix any other caller the same way.

- [ ] **Step 5: Emit the names and pass them in codegen**

In `kestrel/codegen/src/rt.rs`:
- Add the field `pub call_file: DataId,` (doc: `/// The runtime's \`kestrel_rt_call_file\`, the file of that call.`) after `call_line`, declare it beside `call_line` with `module.declare_data("kestrel_rt_call_file", Linkage::Import, true, false).map_err(|e| e.to_string())?`, and add `call_file,` to the struct literal.
- Change the four signatures: `panic: decl("kestrel_rt_panic", &[I32, I32, ptr], None)?`, `alloc: decl("kestrel_rt_alloc", &[I64, I64, I64, I32, ptr], Some(ptr))?`, `panic_index: decl("kestrel_rt_panic_index", &[I64, I64, I32, ptr], None)?`, `map_insert: decl("kestrel_rt_map_insert", &[ptr, ptr, I32, I64, I64, I32, ptr], Some(ptr))?`.
- Add to the JIT symbol list after `kestrel_rt_call_line`: `("kestrel_rt_call_file", std::ptr::addr_of!(panic::kestrel_rt_call_file).cast::<u8>()),`

In `kestrel/codegen/src/globals.rs`, add:

```rust
/// One `FileName` record per source file (see `kestrel_runtime::panic::FileName`): the name's
/// bytes, then a pointer to them and the length. A runtime error gets its file's record.
pub(crate) fn define_file_names<M: Module>(module: &mut M, m: &kir::Module) -> Result<Vec<DataId>, String> {
    let ptr_bytes = module.target_config().pointer_bytes() as usize;
    let mut ids = Vec::with_capacity(m.files.len());
    for name in &m.files {
        let bytes_id = module.declare_anonymous_data(false, false).map_err(|e| e.to_string())?;
        let mut bytes = DataDescription::new();
        // Zero-sized data objects aren't allowed; the length stays 0.
        bytes.define(if name.is_empty() { vec![0] } else { name.as_bytes().to_vec() }.into_boxed_slice());
        module.define_data(bytes_id, &bytes).map_err(|e| e.to_string())?;
        let id = module.declare_anonymous_data(false, false).map_err(|e| e.to_string())?;
        let mut record = DataDescription::new();
        let mut contents = vec![0u8; ptr_bytes + 8];
        contents[ptr_bytes..].copy_from_slice(&(name.len() as i64).to_le_bytes());
        record.define(contents.into_boxed_slice());
        let gv = module.declare_data_in_data(bytes_id, &mut record);
        record.write_data_addr(0, gv, 0);
        module.define_data(id, &record).map_err(|e| e.to_string())?;
        ids.push(id);
    }
    Ok(ids)
}
```

In `kestrel/codegen/src/func/mod.rs`, add to `Shared` after `globals`:
```rust
    /// Each source file's `FileName` record, by the file index in a KIR line.
    pub files: &'a [DataId],
```
In `kestrel/codegen/src/lib.rs`, after `let globals = globals::define_globals(module, m)?;` add `let files = globals::define_file_names(module, m)?;` and add `files: &files` to the `func::Shared { ... }` literal. (Check every other place that builds a `Shared`, e.g. tests or `dump`, and add the field there too.)

In `kestrel/codegen/src/func/globals.rs`, add `use cranelift_codegen::ir::Value;` to the imports and replace `set_line` with:

```rust
    /// A packed KIR line (see `kir::pack_line`) as runtime errors take it: the line, and a pointer
    /// to the file's `FileName` record (null when there is no name).
    pub(crate) fn loc_args(&mut self, loc: u32) -> (Value, Value) {
        let (line, file) = kir::unpack_line(loc);
        let line = self.b.ins().iconst(types::I32, i64::from(line));
        let record = (file != kir::NO_FILE).then(|| self.shared.files.get(file as usize).copied()).flatten();
        let file = match record {
            Some(id) => {
                let gv = self.module.declare_data_in_func(id, self.b.func);
                self.b.ins().symbol_value(self.ptr, gv)
            }
            None => self.b.ins().iconst(self.ptr, 0),
        };
        (line, file)
    }

    /// Stores the line and file in the runtime's `kestrel_rt_call_line` and `kestrel_rt_call_file`
    /// (see `Stmt::SetLine`).
    pub(crate) fn set_line(&mut self, loc: u32) {
        let (line, file) = self.loc_args(loc);
        for (data, value) in [(self.shared.rt.call_line, line), (self.shared.rt.call_file, file)] {
            let gv = self.module.declare_data_in_func(data, self.b.func);
            let addr = self.b.ins().symbol_value(self.ptr, gv);
            self.b.ins().store(MemFlags::trusted(), value, addr, 0);
        }
    }
```

(Use whatever name the file already uses for the KIR crate, `kir` or `kestrel_kir`.)

Then change the six call sites to pass the file:
- `func/arith.rs`, `panic_if`: replace `let l = self.b.ins().iconst(types::I32, line as i64);` and the call with `let (l, f) = self.loc_args(line);` and `self.call_id(self.shared.rt.panic, &[k, l, f]);`.
- `func/place.rs`, `bounds_check`: replace `let line = self.b.ins().iconst(types::I32, line as i64);` and the call with `let (l, f) = self.loc_args(line);` and `self.call_id(self.shared.rt.panic_index, &[i, len, l, f]);`.
- `func/collections.rs`, `array_new` and `array_lit`: replace `let l = self.b.ins().iconst(types::I32, line as i64);` with `let (l, f) = self.loc_args(line);` and pass `&[n, s, a, l, f]`.
- `func/maps.rs`, the insert: replace `let line = self.b.ins().iconst(types::I32, line as i64);` with `let (line, file) = self.loc_args(line);` and pass `&[header, k, kind, size, align, line, file]`.
- `func/maps.rs`, the map-changed panic: replace `let line = self.b.ins().iconst(types::I32, at.line as i64);` with `let (line, file) = self.loc_args(at.line);` and pass `&[kind, line, file]`.

- [ ] **Step 6: Run everything and lint**

Run: `cargo test --workspace --no-fail-fast` then `cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS. The 38 program pins that end in "on line N" still match (the file follows), `codegen/tests/jit.rs` and the KIR tests keep their messages and `Step::Line` values (one unnamed file, index 0). Then the Godot workspace: `cd kestrel/godot && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --no-fail-fast`.

- [ ] **Step 7: Update the add-on's README and commit**

In `kestrel/godot/README.md:64`, change "prints as a Godot error naming the line" to "prints as a Godot error naming the line and the file (`... on line 24 of world.kes (in \`crash\`)`)".

```bash
git add kestrel/syntax kestrel/kir kestrel/driver kestrel/runtime kestrel/codegen kestrel/cli/tests kestrel/tests kestrel/godot/README.md
git commit -m "Name the file in runtime errors: on line N of geom.kes"
git push
```

---

### Task 6: Notes, docs and the full check (spec part F)

**Files:**
- Modify: `kestrel/demos/voxels/world.kes:188-191` (comment)
- Modify: `docs/superpowers/plans/2026-10-09-core-2a-followups.md`, `docs/superpowers/plans/2026-10-10-handoff.md`, `docs/superpowers/specs/2026-10-10-window-design.md:137`, `docs/superpowers/specs/2026-10-12-language-pass-1-design.md` (add "As built")

- [ ] **Step 1: Correct the voxel demo's comment**

In `kestrel/demos/voxels/world.kes`, replace

```
// A chunk's mesh being built: MeshData plus a face whose corners get their own
// colours, like a Godot SurfaceTool with a colour per vertex. (It is a struct
// with a `var self` method because a pure function can't yet call a free
// function that fills one of its locals.)
```

with

```
// A chunk's mesh being built: MeshData plus a face whose corners get their own
// colours, like a Godot SurfaceTool with a colour per vertex.
```

- [ ] **Step 2: Update the follow-ups file**

In `docs/superpowers/plans/2026-10-09-core-2a-followups.md`:
- The entry "**A `pure` function can't call a helper that only changes a `var` argument rooted at its own local**": append "Resolved (found 2026-10-12): it has worked since 4073379 when the helper is `pure` (`tests/programs/pure_var.kes`); this note was written earlier the same day. A helper that isn't marked `pure` stays refused by design."
- "**`let x = opt else` always moves or copies.**": append "Done in the first language pass (2026-10-12): it reads in place when the optional is a place that can't change; `var`, a `var` root, or a whole owning local still move or copy."
- "**Runtime errors don't name the file**": append "Done in the first language pass: \"on line 24 of geom.kes\"."
- "**Struct-valued field defaults are unavailable**": append "Done in the first language pass: defaults and constants take struct and enum values (not owning ones), with a loop check."
- Add a section `## Found while building the first language pass (2026-10-12)` with whatever the tasks' reviews deferred (the controller fills it from the reviews' minor findings) and these known limits: views over a `var` root still copy; a moved-on-some-paths message can't point at the move (no line numbers or secondary labels in the checker); `var x: T` with no value is a syntax question put to the user.

- [ ] **Step 3: Update the handoff and the window spec**

In `docs/superpowers/plans/2026-10-10-handoff.md`: under "Done", add a "**The first language pass**" entry (spec, plan, commits, what it added, test count, rulings pointer to the spec); in plan 2's ruling J, note that struct values in defaults arrived in the first language pass; replace the "Next" section's lead with: next is the list and string basics proposal (method names are the user's), then interfaces and closures with the `@` design alongside, then range analysis with `@prove`, hot reload, and the voxel race; keep the sub-project 5 candidates listed below that as the later queue. Update the "Status" line with the new test count.

In `docs/superpowers/specs/2026-10-10-window-design.md:137`, change the field-default sentence to say struct values are allowed since the first language pass (`tint: Color = draw.WHITE`).

In the language pass spec, add a closing `## As built (2026-10-12)` section listing what differed from the design (the controller writes it from the task reports).

- [ ] **Step 4: The full check, including the Godot add-on**

Run in `kestrel/`: `cargo test --workspace --no-fail-fast` and `cargo clippy --workspace --all-targets -- -D warnings`.
Run in `kestrel/godot/`: `cargo clippy --workspace --all-targets -- -D warnings`.
Rebuild the add-on with `kestrel/godot/build.ps1` (if Godot is open, the DLL is locked: ask the user to close it), then run the headless test: `& "C:\Users\sawye\Desktop\Godot_v4.7.2-stable_mono_win64\Godot_v4.7.2-stable_mono_win64\Godot_v4.7.2-stable_mono_win64_console.exe" --headless --path kestrel/godot/demo --script res://tests/test_bridge.gd`.
Expected: all pass; the Godot test reports 30 checks, 0 failures.

- [ ] **Step 5: Commit**

```bash
git add kestrel/demos/voxels/world.kes docs
git commit -m "Record the first language pass: notes, follow-ups, handoff"
git push
```
