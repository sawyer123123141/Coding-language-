# Loop-Indexed Array Bounds Proof Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Elide the runtime array bounds check for the specific loop-indexed access pattern `bounds-heavy` uses (`let arr = [...]; let i = 0; while (i < N) { ...arr[i]...; i = i + 1; }`), without weakening any existing safety guarantee.

**Architecture:** A new pure-AST helper, `find_loop_bounds_proof`, recognizes one exact `while`-loop shape and returns `Option<(Symbol, i64)>` — the proven index variable and its literal upper bound. `codegen.rs` computes this proof when it starts compiling a `Stmt::While`'s body, pushes it onto a new parallel stack (alongside the existing `loop_stack`), and a new fast-path in `Index` codegen consults it: if the current index identifier matches the proven symbol and the target array's known static length covers the proven bound, the check is elided (same direct-load codegen as the existing literal-index fast path). Any shape mismatch anywhere leaves the proof `None` and every access falls through to today's unchanged runtime check.

**Tech Stack:** Rust, Cranelift (via the existing `codegen.rs` `FunctionBuilder`).

## Global Constraints

- No AST, parser, or `resolve.rs` changes — everything needed is already visible to `codegen.rs` (design doc's "Data flow" section).
- The recognized shape is exact and narrow (design doc's "Mechanism" section) — no partial credit, no heuristic fallback. Any deviation must produce `None`, never a best-effort guess.
- Scope is local `let`-array literals only (compile-time-known length) — not array parameters or the existing `where`-clause mechanism.
- No new compile errors: an unproven access is not rejected, it just keeps the existing runtime check (unchanged behavior).

---

### Task 1: `find_loop_bounds_proof` helper + unit tests

**Files:**
- Modify: `kestrelc/src/codegen.rs` (add the helper function and a new `#[cfg(test)] mod tests` block near the end of the file; this file currently has no test module, so this is the first one)

**Interfaces:**
- Produces: `fn find_loop_bounds_proof(prev: Option<&Stmt>, cond: &Expr, body: &[Stmt]) -> Option<(Symbol, i64)>` — a free function (not a method), placed just above `struct FnCodegen` (before line 1040) so Task 2 can call it as `find_loop_bounds_proof(prev, cond, body)` from inside `gen_stmt`.

This is pure AST analysis with no Cranelift dependency, so it's fully unit-testable in isolation before touching any codegen wiring.

- [ ] **Step 1: Write the failing tests**

Add this test module at the very end of `kestrelc/src/codegen.rs` (after the file's existing final `}`):

```rust
#[cfg(test)]
mod loop_bounds_proof_tests {
    use super::find_loop_bounds_proof;
    use crate::ast::{BinOp, Expr, ExprKind, Stmt};
    use crate::interner::intern;
    use crate::span::Span;

    fn sp() -> Span {
        Span { line: 1, col: 1, len: 1 }
    }

    fn ident(name: &str) -> Expr {
        Expr::new(ExprKind::Ident(intern(name)), sp())
    }

    fn num(n: i64) -> Expr {
        Expr::new(ExprKind::Num(n), sp())
    }

    fn let_stmt(name: &str, value: Expr) -> Stmt {
        Stmt::Let { name: intern(name), value, span: sp() }
    }

    fn assign(name: &str, value: Expr) -> Stmt {
        Stmt::Assign { name: intern(name), value, span: sp() }
    }

    fn lt(left: Expr, right: Expr) -> Expr {
        Expr::new(ExprKind::Binop { op: BinOp::Lt, left: Box::new(left), right: Box::new(right) }, sp())
    }

    fn add(left: Expr, right: Expr) -> Expr {
        Expr::new(ExprKind::Binop { op: BinOp::Add, left: Box::new(left), right: Box::new(right) }, sp())
    }

    fn index(target: &str, idx: &str) -> Expr {
        Expr::new(
            ExprKind::Index { target: Box::new(ident(target)), index: Box::new(ident(idx)) },
            sp(),
        )
    }

    fn increment(name: &str) -> Stmt {
        assign(name, add(ident(name), num(1)))
    }

    #[test]
    fn the_exact_bounds_heavy_shape_is_proven() {
        let prev = let_stmt("i", num(0));
        let cond = lt(ident("i"), num(20000));
        let body = vec![
            assign("total", index("arr", "i")),
            increment("i"),
        ];
        let proof = find_loop_bounds_proof(Some(&prev), &cond, &body);
        assert_eq!(proof, Some((intern("i"), 20000)));
    }

    #[test]
    fn missing_preceding_let_zero_is_not_proven() {
        let cond = lt(ident("i"), num(20000));
        let body = vec![assign("total", index("arr", "i")), increment("i")];
        assert_eq!(find_loop_bounds_proof(None, &cond, &body), None);
    }

    #[test]
    fn preceding_let_with_nonzero_initial_value_is_not_proven() {
        let prev = let_stmt("i", num(1));
        let cond = lt(ident("i"), num(20000));
        let body = vec![assign("total", index("arr", "i")), increment("i")];
        assert_eq!(find_loop_bounds_proof(Some(&prev), &cond, &body), None);
    }

    #[test]
    fn nested_if_in_body_is_not_proven() {
        let prev = let_stmt("i", num(0));
        let cond = lt(ident("i"), num(20000));
        let body = vec![
            Stmt::If {
                cond: ident("cond_flag"),
                then_block: vec![assign("total", index("arr", "i"))],
                else_block: None,
                span: sp(),
            },
            increment("i"),
        ];
        assert_eq!(find_loop_bounds_proof(Some(&prev), &cond, &body), None);
    }

    #[test]
    fn missing_increment_is_not_proven() {
        let prev = let_stmt("i", num(0));
        let cond = lt(ident("i"), num(20000));
        let body = vec![assign("total", index("arr", "i"))];
        assert_eq!(find_loop_bounds_proof(Some(&prev), &cond, &body), None);
    }

    #[test]
    fn increment_not_last_statement_is_not_proven() {
        let prev = let_stmt("i", num(0));
        let cond = lt(ident("i"), num(20000));
        let body = vec![
            increment("i"),
            assign("total", index("arr", "i")),
        ];
        assert_eq!(find_loop_bounds_proof(Some(&prev), &cond, &body), None);
    }

    #[test]
    fn extra_reassignment_of_index_is_not_proven() {
        let prev = let_stmt("i", num(0));
        let cond = lt(ident("i"), num(20000));
        let body = vec![
            assign("i", add(ident("i"), num(5))),
            assign("total", index("arr", "i")),
            increment("i"),
        ];
        assert_eq!(find_loop_bounds_proof(Some(&prev), &cond, &body), None);
    }

    #[test]
    fn condition_not_a_literal_bound_is_not_proven() {
        let prev = let_stmt("i", num(0));
        let cond = lt(ident("i"), ident("n"));
        let body = vec![assign("total", index("arr", "i")), increment("i")];
        assert_eq!(find_loop_bounds_proof(Some(&prev), &cond, &body), None);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib loop_bounds_proof_tests` from `kestrelc/`
Expected: FAIL to compile — `find_loop_bounds_proof` doesn't exist yet.

- [ ] **Step 3: Write the implementation**

Add this above `struct FnCodegen<'a> {` (immediately before line 1040 in the current file):

```rust
/// Attempts to prove `Some((idx, bound))` for a `while (idx < bound)`
/// loop: `idx` starts at exactly `0` (the statement immediately
/// preceding the loop, in the same block, must be `let idx = 0;`), the
/// condition is exactly `idx < N` for a literal `N`, the body contains
/// no nested control flow at all (flat `let`/assignment/expression-
/// statement/`print` only), and the body's *last* statement is exactly
/// `idx = idx + 1` with `idx` reassigned nowhere else in the body.
///
/// Returns `None` for any deviation from this exact shape -- no
/// partial credit, no heuristic fallback (see this file's Index
/// fast-path #3, and docs/superpowers/specs/2026-07-25-loop-indexed-
/// bounds-proof-design.md for the full soundness argument).
fn find_loop_bounds_proof(prev: Option<&Stmt>, cond: &Expr, body: &[Stmt]) -> Option<(Symbol, i64)> {
    let (idx, bound) = match &cond.kind {
        ExprKind::Binop { op: BinOp::Lt, left, right } => {
            let idx = match &left.kind {
                ExprKind::Ident(n) => *n,
                _ => return None,
            };
            let bound = match &right.kind {
                ExprKind::Num(n) => *n,
                _ => return None,
            };
            (idx, bound)
        }
        _ => return None,
    };

    match prev {
        Some(Stmt::Let { name, value, .. }) if *name == idx => match &value.kind {
            ExprKind::Num(0) => {}
            _ => return None,
        },
        _ => return None,
    }

    let (last, rest) = body.split_last()?;
    for s in rest {
        match s {
            Stmt::Let { .. } | Stmt::ExprStmt { .. } | Stmt::Print { .. } => {}
            Stmt::Assign { name, .. } if *name == idx => return None,
            Stmt::Assign { .. } => {}
            // Any nested control flow (If/While/RangeFor/Return/Break/
            // Continue) or FieldAssign bails -- the shape must be flat.
            _ => return None,
        }
    }

    match last {
        Stmt::Assign { name, value, .. } if *name == idx => match &value.kind {
            ExprKind::Binop { op: BinOp::Add, left, right } => {
                let l_ok = matches!(&left.kind, ExprKind::Ident(n) if *n == idx);
                let r_ok = matches!(&right.kind, ExprKind::Num(1));
                if l_ok && r_ok {
                    Some((idx, bound))
                } else {
                    None
                }
            }
            _ => None,
        },
        _ => None,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib loop_bounds_proof_tests` from `kestrelc/`
Expected: PASS, 8 tests.

- [ ] **Step 5: Commit**

```bash
git add kestrelc/src/codegen.rs
git commit -m "Add find_loop_bounds_proof: pure AST check for the loop-indexed bounds pattern"
```

---

### Task 2: Wire the proof through codegen (stack + fast-path)

**Files:**
- Modify: `kestrelc/src/codegen.rs`

**Interfaces:**
- Consumes: `find_loop_bounds_proof(prev: Option<&Stmt>, cond: &Expr, body: &[Stmt]) -> Option<(Symbol, i64)>` from Task 1.
- Produces: a new `FnCodegen` field `loop_bounds_stack: Vec<Option<(Symbol, i64)>>`, pushed/popped around `Stmt::While`'s body exactly like the existing `loop_stack`. Later tasks (none in this plan, but any future work extending this) would read `self.loop_bounds_stack.last()`.

This task has no new tests of its own — Task 3's integration tests are what verify it end-to-end. Fold in a `cargo build` check as this task's own verification step, per the task-sizing rule (the threading and the fast-path aren't independently reviewable — a stack with no consumer, or a fast-path with no proof feeding it, are both meaningless in isolation).

- [ ] **Step 1: Add the new field to `FnCodegen`**

In `kestrelc/src/codegen.rs`, find this block (around line 1085):

```rust
    loop_stack: Vec<(Block, Block)>,
}
```

Replace it with:

```rust
    loop_stack: Vec<(Block, Block)>,
    /// One entry per currently-active enclosing `while` loop, innermost
    /// last, mirroring `loop_stack`'s lifetime. `Some((idx, bound))`
    /// when `find_loop_bounds_proof` proved this loop's shape safe;
    /// `None` otherwise. Consulted by `gen_expr`'s `Index` arm (fast
    /// path #3) to decide whether an `arr[idx]` access inside this
    /// loop's body can skip the runtime bounds check.
    loop_bounds_stack: Vec<Option<(Symbol, i64)>>,
}
```

- [ ] **Step 2: Initialize the field where `FnCodegen` is constructed**

Find this block (around line 829-847, inside `compile_fn`):

```rust
            let mut fc = FnCodegen {
                builder,
                vars,
                fn_ids: &self.fn_ids,
                printf_id: self.printf_id,
                pmap_id: self.pmap_id,
                bounds_fail_id: self.bounds_fail_id,
                malloc_id: self.malloc_id,
                alloc_fail_id: self.alloc_fail_id,
                module: &mut self.module,
                str_cache: &mut self.str_cache,
                str_counter: &mut self.str_counter,
                where_info: &self.where_info,
                struct_table,
                my_where: self.where_info.get(&f.name),
                epilogue,
                cur_span: f.span,
                loop_stack: Vec::new(),
            };
```

Add `loop_bounds_stack: Vec::new(),` right after `loop_stack: Vec::new(),`:

```rust
                loop_stack: Vec::new(),
                loop_bounds_stack: Vec::new(),
            };
```

- [ ] **Step 3: Thread the preceding statement through `gen_block`/`gen_stmt`**

Find `gen_block` (around line 1105):

```rust
    fn gen_block(&mut self, stmts: &[Stmt]) -> CgResult<bool> {
        for s in stmts {
            if self.gen_stmt(s)? {
                return Ok(true); // rest of this block is unreachable
            }
        }
        Ok(false)
    }
```

Replace with:

```rust
    fn gen_block(&mut self, stmts: &[Stmt]) -> CgResult<bool> {
        for (i, s) in stmts.iter().enumerate() {
            let prev = if i > 0 { Some(&stmts[i - 1]) } else { None };
            if self.gen_stmt(s, prev)? {
                return Ok(true); // rest of this block is unreachable
            }
        }
        Ok(false)
    }
```

Find `gen_stmt`'s signature (around line 1276):

```rust
    fn gen_stmt(&mut self, s: &Stmt) -> CgResult<bool> {
```

Replace with:

```rust
    fn gen_stmt(&mut self, s: &Stmt, prev: Option<&Stmt>) -> CgResult<bool> {
```

`prev` is only used by the `Stmt::While` arm (Step 4 below) — every other arm ignores it, so no other match arm needs to change.

- [ ] **Step 4: Compute and push/pop the proof in the `While` arm**

Find this block (around line 1359-1379):

```rust
            Stmt::While { cond, body, .. } => {
                let header_blk = self.builder.create_block();
                let body_blk = self.builder.create_block();
                let after_blk = self.builder.create_block();

                self.builder.ins().jump(header_blk, &[]);

                self.builder.switch_to_block(header_blk);
                let c = self.gen_expr(cond)?;
                self.builder.ins().brif(c, body_blk, &[], after_blk, &[]);
                // header_blk is sealed after the body's back-edge is known.

                self.builder.switch_to_block(body_blk);
                // `continue` re-checks the condition directly (jumps to
                // header_blk, the same place the body's own normal
                // back-edge below jumps to) -- a `while` loop has no
                // separate step to preserve, unlike RangeFor below.
                self.loop_stack.push((header_blk, after_blk));
                let body_term = self.gen_block(body)?;
                self.loop_stack.pop();
```

Replace with:

```rust
            Stmt::While { cond, body, .. } => {
                let header_blk = self.builder.create_block();
                let body_blk = self.builder.create_block();
                let after_blk = self.builder.create_block();

                self.builder.ins().jump(header_blk, &[]);

                self.builder.switch_to_block(header_blk);
                let c = self.gen_expr(cond)?;
                self.builder.ins().brif(c, body_blk, &[], after_blk, &[]);
                // header_blk is sealed after the body's back-edge is known.

                self.builder.switch_to_block(body_blk);
                // `continue` re-checks the condition directly (jumps to
                // header_blk, the same place the body's own normal
                // back-edge below jumps to) -- a `while` loop has no
                // separate step to preserve, unlike RangeFor below.
                self.loop_stack.push((header_blk, after_blk));
                self.loop_bounds_stack.push(find_loop_bounds_proof(prev, cond, body));
                let body_term = self.gen_block(body)?;
                self.loop_bounds_stack.pop();
                self.loop_stack.pop();
```

- [ ] **Step 5: Add the Index fast-path #3**

Find this block (around line 1617-1636):

```rust
                // Proof-carrying fast path #2: this function has a
                // `where idx_param < N` clause tying exactly this
                // (array parameter, index parameter) pair together, and
                // this is exactly that access (`arr_param[idx_param]`).
                // Every call site to this function is required (see the
                // Call arm below) to prove the precondition before the
                // call is even allowed to compile — so by the time we're
                // generating code *inside* this function, the precondition
                // is already guaranteed, and the check would be redundant.
                if let (ExprKind::Ident(t), ExprKind::Ident(i)) = (&target.as_ref().kind, &index.as_ref().kind) {
                    if let Some(w) = self.my_where {
                        if t == &w.arr_param && i == &w.idx_param {
                            let (ptr, _len) = self.resolve_array(target)?;
                            let idx = self.gen_expr(index)?;
                            let offset = self.builder.ins().imul_imm(idx, 8);
                            let addr = self.builder.ins().iadd(ptr, offset);
                            return Ok(self.builder.ins().load(types::I64, MemFlags::new(), addr, 0));
                        }
                    }
                }

                let (ptr, len) = self.resolve_array(target)?;
```

Replace with:

```rust
                // Proof-carrying fast path #2: this function has a
                // `where idx_param < N` clause tying exactly this
                // (array parameter, index parameter) pair together, and
                // this is exactly that access (`arr_param[idx_param]`).
                // Every call site to this function is required (see the
                // Call arm below) to prove the precondition before the
                // call is even allowed to compile — so by the time we're
                // generating code *inside* this function, the precondition
                // is already guaranteed, and the check would be redundant.
                if let (ExprKind::Ident(t), ExprKind::Ident(i)) = (&target.as_ref().kind, &index.as_ref().kind) {
                    if let Some(w) = self.my_where {
                        if t == &w.arr_param && i == &w.idx_param {
                            let (ptr, _len) = self.resolve_array(target)?;
                            let idx = self.gen_expr(index)?;
                            let offset = self.builder.ins().imul_imm(idx, 8);
                            let addr = self.builder.ins().iadd(ptr, offset);
                            return Ok(self.builder.ins().load(types::I64, MemFlags::new(), addr, 0));
                        }
                    }
                }

                // Proof-carrying fast path #3: the innermost enclosing
                // `while` loop's own shape was proven safe by
                // find_loop_bounds_proof (pushed in the While arm above)
                // -- `idx` is provably `0 <= idx < bound` everywhere in
                // this loop's body, and this array's statically-known
                // length covers that bound.
                if let (ExprKind::Ident(t), ExprKind::Ident(i)) = (&target.as_ref().kind, &index.as_ref().kind) {
                    if let Some(Some((proven_idx, bound))) = self.loop_bounds_stack.last() {
                        if i == proven_idx {
                            if let Some(static_len) = self.static_array_len(target) {
                                if *bound as usize <= static_len {
                                    let (ptr, _len) = self.resolve_array(target)?;
                                    let idx = self.gen_expr(index)?;
                                    let offset = self.builder.ins().imul_imm(idx, 8);
                                    let addr = self.builder.ins().iadd(ptr, offset);
                                    return Ok(self.builder.ins().load(types::I64, MemFlags::new(), addr, 0));
                                }
                            }
                        }
                    }
                    let _ = t; // t isn't otherwise used in this fast path
                }

                let (ptr, len) = self.resolve_array(target)?;
```

- [ ] **Step 6: Build and confirm no compile errors**

Run: `cargo build --lib` from `kestrelc/`
Expected: builds cleanly. If there's a warning about `t` being unused in fast path #3, the `let _ = t;` line above already silences it — if a different warning appears, read it and fix the specific line it names before proceeding.

- [ ] **Step 7: Run the full existing test suite to confirm no regression**

Run: `cargo test --lib` from `kestrelc/`
Expected: PASS, same count as before this task (108 pre-existing plus the 8 from Task 1 = 116), 0 failures.

- [ ] **Step 8: Commit**

```bash
git add kestrelc/src/codegen.rs
git commit -m "Wire loop-bounds proof into codegen: new stack + Index fast-path #3"
```

---

### Task 3: Integration tests

**Files:**
- Modify: `kestrelc/tests/integration.rs` (append new tests; follows the exact existing pattern used by `statically_provable_out_of_bounds_index_is_a_compile_error` and the `where_clause_*` tests earlier in this file — `scratch_dir`, `fs::write`, `kestrelc_bin()`, `native_stdout`)

**Interfaces:**
- Consumes: `scratch_dir(name: &str) -> PathBuf`, `kestrelc_bin() -> PathBuf`, `native_stdout(run: &std::process::Output) -> String` — all already defined at the top of `kestrelc/tests/integration.rs` (lines 10-34).

- [ ] **Step 1: Write the positive test**

Append to `kestrelc/tests/integration.rs`:

```rust
#[test]
fn loop_indexed_access_with_a_provably_bounded_counter_elides_the_check_and_stays_correct() {
    // The exact bounds-heavy shape: a let-array literal indexed by a
    // while-loop counter whose bound matches the array's own length.
    // Also exercises the last valid index (i == len - 1), the boundary
    // a bug in this proof would most likely get wrong.
    let scratch = scratch_dir("loop_bounds_ok");
    let src_path = scratch.join("loop_bounds_ok.kes");
    fs::write(
        &src_path,
        r#"
        fn main() {
            let arr = [10, 20, 30, 40, 50];
            let total = 0;
            let i = 0;
            while (i < 5) {
                total = total + arr[i];
                i = i + 1;
            }
            print(total);
            print(arr[4]);
        }
        "#,
    )
    .unwrap();

    let out = Command::new(kestrelc_bin())
        .arg(&src_path)
        .current_dir(&scratch)
        .output()
        .expect("failed to run kestrelc");
    assert!(out.status.success(), "compile failed:\n{}", String::from_utf8_lossy(&out.stderr));

    let bin = scratch.join("loop_bounds_ok");
    let run = Command::new(&bin).output().expect("failed to run compiled binary");
    assert!(run.status.success(), "compiled binary exited with failure");
    assert_eq!(native_stdout(&run), "150\n50\n");
}

#[test]
fn two_sequential_loops_reusing_the_same_index_name_are_each_proven_independently() {
    // The second loop's proof must not leak from (or be blocked by) the
    // first -- each while loop is checked fresh against its own
    // immediately-preceding statement.
    let scratch = scratch_dir("loop_bounds_two_loops");
    let src_path = scratch.join("loop_bounds_two_loops.kes");
    fs::write(
        &src_path,
        r#"
        fn main() {
            let a = [1, 2, 3];
            let b = [10, 20, 30, 40];
            let sum_a = 0;
            let i = 0;
            while (i < 3) {
                sum_a = sum_a + a[i];
                i = i + 1;
            }
            let sum_b = 0;
            let i = 0;
            while (i < 4) {
                sum_b = sum_b + b[i];
                i = i + 1;
            }
            print(sum_a);
            print(sum_b);
        }
        "#,
    )
    .unwrap();

    let out = Command::new(kestrelc_bin())
        .arg(&src_path)
        .current_dir(&scratch)
        .output()
        .expect("failed to run kestrelc");
    assert!(out.status.success(), "compile failed:\n{}", String::from_utf8_lossy(&out.stderr));

    let bin = scratch.join("loop_bounds_two_loops");
    let run = Command::new(&bin).output().expect("failed to run compiled binary");
    assert!(run.status.success(), "compiled binary exited with failure");
    assert_eq!(native_stdout(&run), "6\n100\n");
}
```

- [ ] **Step 2: Run the new positive tests to verify they pass**

Run: `cargo test --test integration loop_indexed_access_with_a_provably_bounded_counter -- --nocapture` and `cargo test --test integration two_sequential_loops_reusing_the_same_index_name` from `kestrelc/`
Expected: both PASS.

- [ ] **Step 3: Write the genuinely-unsafe negative tests**

Append to `kestrelc/tests/integration.rs`:

```rust
#[test]
fn a_loop_index_reassigned_a_second_time_still_traps_instead_of_reading_garbage() {
    // idx is reassigned beyond the required `idx = idx + 1`, so it
    // genuinely exceeds the array partway through -- this shape must
    // fail find_loop_bounds_proof's "idx assigned nowhere else in the
    // body" check and fall back to the runtime check, which must still
    // catch the out-of-bounds access cleanly.
    let scratch = scratch_dir("loop_bounds_extra_reassign");
    let src_path = scratch.join("loop_bounds_extra_reassign.kes");
    fs::write(
        &src_path,
        r#"
        fn main() {
            let arr = [1, 2, 3, 4, 5];
            let total = 0;
            let i = 0;
            while (i < 5) {
                total = total + arr[i];
                i = i + 3;
                i = i + 1;
            }
            print(total);
        }
        "#,
    )
    .unwrap();

    let out = Command::new(kestrelc_bin())
        .arg(&src_path)
        .current_dir(&scratch)
        .output()
        .expect("failed to run kestrelc");
    assert!(out.status.success(), "compile failed:\n{}", String::from_utf8_lossy(&out.stderr));

    let bin = scratch.join("loop_bounds_extra_reassign");
    let run = Command::new(&bin).output().expect("failed to run compiled binary");
    assert!(!run.status.success(), "an eventually out-of-bounds access should not exit successfully");
    let stderr = String::from_utf8_lossy(&run.stderr).replace("\r\n", "\n");
    assert!(
        stderr.contains("out of bounds"),
        "expected a runtime bounds-check failure message, got:\n{stderr}"
    );
}

#[test]
fn a_loop_bound_exceeding_the_arrays_length_still_traps_instead_of_reading_garbage() {
    // The while condition's literal bound (6) exceeds arr's actual
    // static length (5) -- fast path #3's own `bound <= static_len`
    // check must refuse to elide here, so the runtime check must still
    // fire on the sixth (out-of-range) iteration.
    let scratch = scratch_dir("loop_bounds_over_length");
    let src_path = scratch.join("loop_bounds_over_length.kes");
    fs::write(
        &src_path,
        r#"
        fn main() {
            let arr = [1, 2, 3, 4, 5];
            let total = 0;
            let i = 0;
            while (i < 6) {
                total = total + arr[i];
                i = i + 1;
            }
            print(total);
        }
        "#,
    )
    .unwrap();

    let out = Command::new(kestrelc_bin())
        .arg(&src_path)
        .current_dir(&scratch)
        .output()
        .expect("failed to run kestrelc");
    assert!(out.status.success(), "compile failed:\n{}", String::from_utf8_lossy(&out.stderr));

    let bin = scratch.join("loop_bounds_over_length");
    let run = Command::new(&bin).output().expect("failed to run compiled binary");
    assert!(!run.status.success(), "reading past the array's actual length should not exit successfully");
    let stderr = String::from_utf8_lossy(&run.stderr).replace("\r\n", "\n");
    assert!(
        stderr.contains("out of bounds"),
        "expected a runtime bounds-check failure message, got:\n{stderr}"
    );
}
```

- [ ] **Step 4: Run the negative tests to verify they pass**

Run: `cargo test --test integration a_loop_index_reassigned_a_second_time` and `cargo test --test integration a_loop_bound_exceeding_the_arrays_length` from `kestrelc/`
Expected: both PASS.

- [ ] **Step 5: Write the shape-only regression test**

Append to `kestrelc/tests/integration.rs`:

```rust
#[test]
fn a_nested_if_around_the_access_still_compiles_and_produces_correct_output() {
    // find_loop_bounds_proof must refuse this shape (nested control
    // flow in the body), falling back to the runtime check -- since the
    // loop's own condition keeps this program genuinely safe either
    // way, this only proves no regression, not that the check was
    // actually retained (see the design doc's honest note on this
    // limitation).
    let scratch = scratch_dir("loop_bounds_nested_if");
    let src_path = scratch.join("loop_bounds_nested_if.kes");
    fs::write(
        &src_path,
        r#"
        fn main() {
            let arr = [1, 2, 3, 4, 5];
            let total = 0;
            let i = 0;
            while (i < 5) {
                if (i < 3) {
                    total = total + arr[i];
                }
                i = i + 1;
            }
            print(total);
        }
        "#,
    )
    .unwrap();

    let out = Command::new(kestrelc_bin())
        .arg(&src_path)
        .current_dir(&scratch)
        .output()
        .expect("failed to run kestrelc");
    assert!(out.status.success(), "compile failed:\n{}", String::from_utf8_lossy(&out.stderr));

    let bin = scratch.join("loop_bounds_nested_if");
    let run = Command::new(&bin).output().expect("failed to run compiled binary");
    assert!(run.status.success(), "compiled binary exited with failure");
    assert_eq!(native_stdout(&run), "6\n");
}
```

- [ ] **Step 6: Run the shape-only test to verify it passes**

Run: `cargo test --test integration a_nested_if_around_the_access` from `kestrelc/`
Expected: PASS.

- [ ] **Step 7: Run the entire test suite (unit + integration) to confirm no regression anywhere**

Run: `cargo test` from `kestrelc/`
Expected: PASS, all unit tests (116, per Task 2's count) plus all integration tests (existing count plus 5 new ones from this task), 0 failures.

- [ ] **Step 8: Commit**

```bash
git add kestrelc/tests/integration.rs
git commit -m "Add integration tests for the loop-indexed bounds proof"
```

---

## Self-Review Notes

- **Spec coverage:** Mechanism (Task 1's helper + Task 2's stack/fast-path), scope restriction to local let-literals (Task 1's `find_loop_bounds_proof` only ever looks at `Stmt`/`Expr`, never a `Fn`'s params or `where_clause`), soundness argument (encoded directly as the exact conditions in Task 1's implementation), data flow (no new files, no AST/parser/resolve.rs changes — confirmed, this plan only touches `codegen.rs` and the integration test file), testing (Task 3 covers positive, genuinely-unsafe negative, and shape-only regression, matching the design doc's three testing categories exactly, including its honest limitation note).
- **Placeholder scan:** no TBD/TODO; every step has complete, runnable code.
- **Type consistency:** `find_loop_bounds_proof`'s signature (`Option<(Symbol, i64)>`) is used identically in Task 1's tests, Task 2's Step 4 (`find_loop_bounds_proof(prev, cond, body)`) and Step 5 (`self.loop_bounds_stack.last()` yielding `&Option<Option<(Symbol, i64)>>`, destructured as `Some(Some((proven_idx, bound)))`), and the new field's declared type (`Vec<Option<(Symbol, i64)>>`) in Step 1 — all match.
