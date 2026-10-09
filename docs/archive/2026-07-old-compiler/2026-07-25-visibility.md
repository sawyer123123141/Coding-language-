# Visibility (pub/private) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `pub`/private-by-default visibility for `fn`/`struct` declarations, enforced across module boundaries (both `use a, b from module;` and bare `use module;` + `module.fn()` access), leaving same-module access completely untouched.

**Architecture:** `pub` is a new prefix keyword in the same modifier slot as `pure` (`pub fn`, `pub struct`, `pub pure fn`). `Fn`/`StructDecl` each gain a `pub_: bool` field set by the parser. Enforcement lives entirely in `modules::merge_modules`: the existing `declared` table (already tracking each module's fn/struct names for from-import existence checks) also tracks each name's `pub_`-ness; a `from`-import of a private name is now a distinct compile error instead of silently succeeding, and a bare `use module;`'s qualified-call rename table skips private names, routing them into a new `denied` map that the (now fallible) AST rewrite pass consults to produce the same distinct error at the actual call site.

**Tech Stack:** Rust (kestrelc).

## Global Constraints

- Private by default; `pub` opts a `fn`/`struct` into cross-module visibility (from the design doc).
- `pub` gates both `from`-imports and bare qualified `module.fn()` calls uniformly (from the design doc).
- Visibility is declaration-level only, no per-field struct visibility (from the design doc).
- Same-module access is completely unaffected — no rename/visibility check applies to it at all (from the design doc).
- `pub` always precedes `pure` when both are present (`pub pure fn`) — `pure pub fn` is a parse error, not an alternate valid spelling (from the design doc).
- A private-access compile error must be distinct from the existing "not found in module" error, naming the item and its module (from the design doc).

---

### Task 1: `pub` keyword — lexer, AST, parser

**Files:**
- Modify: `kestrelc/src/lexer.rs` (new `Tok::Pub`, `"pub"` keyword)
- Modify: `kestrelc/src/ast.rs` (`Fn.pub_: bool`, `StructDecl.pub_: bool`)
- Modify: `kestrelc/src/parser.rs` (`parse_fn_decl`, `parse_struct_decl`, `parse_program`'s dispatch)
- Modify: `kestrelc/src/inline.rs` (no code change needed — its `Fn { ..f.clone() }` struct-update already carries the new field through; listed here only so the task's file list is complete)

**Interfaces:**
- Produces: `Fn.pub_: bool`, `StructDecl.pub_: bool` — every later task reads these two fields by name.

- [ ] **Step 1: Write the failing tests**

Add to `kestrelc/src/parser.rs`'s existing `#[cfg(test)] mod tests` block (append after the last test, `a_plain_field_access_without_a_call_still_parses_as_field`):

```rust
    #[test]
    fn a_bare_fn_declaration_is_private_by_default() {
        let program = parse(lex("fn helper() { print(1); }").unwrap()).unwrap();
        assert!(!program.fns[0].pub_);
    }

    #[test]
    fn pub_fn_is_marked_public() {
        let program = parse(lex("pub fn helper() { print(1); }").unwrap()).unwrap();
        assert!(program.fns[0].pub_);
    }

    #[test]
    fn pub_pure_fn_is_both_pub_and_pure() {
        let program = parse(lex("pub pure fn square(x: i64) -> i64 { return x * x; }").unwrap()).unwrap();
        assert!(program.fns[0].pub_);
        assert!(program.fns[0].pure);
    }

    #[test]
    fn pure_pub_fn_wrong_keyword_order_is_a_parse_error() {
        let result = parse(lex("pure pub fn square(x: i64) -> i64 { return x * x; }").unwrap());
        assert!(result.is_err(), "expected pure before pub to be a parse error");
    }

    #[test]
    fn a_bare_struct_declaration_is_private_by_default() {
        let program = parse(lex("struct Point { x: i64 }\nfn main() { print(1); }").unwrap()).unwrap();
        assert!(!program.structs[0].pub_);
    }

    #[test]
    fn pub_struct_is_marked_public() {
        let program = parse(lex("pub struct Point { x: i64 }\nfn main() { print(1); }").unwrap()).unwrap();
        assert!(program.structs[0].pub_);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd kestrelc && cargo test --lib parser::tests::a_bare_fn_declaration_is_private_by_default parser::tests::pub_fn_is_marked_public parser::tests::pub_pure_fn_is_both_pub_and_pure parser::tests::pure_pub_fn_wrong_keyword_order_is_a_parse_error parser::tests::a_bare_struct_declaration_is_private_by_default parser::tests::pub_struct_is_marked_public`
Expected: FAIL to compile — `pub_` field doesn't exist yet, `Tok::Pub` doesn't exist yet.

- [ ] **Step 3: Add `Tok::Pub` to the lexer**

In `kestrelc/src/lexer.rs`, find:

```rust
    For,
    From,
    To,
    Use,
```

Replace with:

```rust
    For,
    From,
    To,
    Use,
    Pub,
```

Find:

```rust
                "from" => Tok::From,
                "use" => Tok::Use,
```

Replace with:

```rust
                "from" => Tok::From,
                "use" => Tok::Use,
                "pub" => Tok::Pub,
```

- [ ] **Step 4: Add `pub_` to `Fn` and `StructDecl`**

In `kestrelc/src/ast.rs`, find `pub struct Fn {` and its fields (currently `name`, `pure`, `params`, `return_type`, `where_clause`, `body`, `span`). Add `pub_` right after `pure`:

```rust
pub struct Fn {
    pub name: Symbol,
    pub pure: bool,
    /// Cross-module visibility -- see docs/superpowers/specs/
    /// 2026-07-25-visibility-design.md. Private (`false`) by default;
    /// `pub` makes a function importable from another module (both
    /// `use fn_name from module;` and bare `use module;` +
    /// `module.fn_name(...)`). Never checked for same-module access,
    /// which this field has no effect on at all.
    pub pub_: bool,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub where_clause: Option<Expr>,
    pub body: Vec<Stmt>,
    pub span: Span,
}
```

Find `pub struct StructDecl {` and add `pub_` right after `name`:

```rust
pub struct StructDecl {
    pub name: Symbol,
    /// Same meaning as `Fn.pub_` above -- private by default, whole
    /// struct or nothing (no per-field visibility).
    pub pub_: bool,
    pub fields: Vec<Param>,
    pub span: Span,
}
```

- [ ] **Step 5: Parse `pub` in `parse_fn_decl` and `parse_struct_decl`**

In `kestrelc/src/parser.rs`, find:

```rust
    fn parse_struct_decl(&mut self) -> PResult<StructDecl> {
        let span = self.peek().span;
        self.expect(Tok::Struct)?;
        let name = self.expect_ident()?;
```

Replace with:

```rust
    fn parse_struct_decl(&mut self) -> PResult<StructDecl> {
        let span = self.peek().span;
        let pub_ = if self.at(&Tok::Pub) {
            self.advance();
            true
        } else {
            false
        };
        self.expect(Tok::Struct)?;
        let name = self.expect_ident()?;
```

Find (further down, still in `parse_struct_decl`):

```rust
        self.expect(Tok::RBrace)?;
        Ok(StructDecl { name, fields, span })
    }
```

Replace with:

```rust
        self.expect(Tok::RBrace)?;
        Ok(StructDecl { name, pub_, fields, span })
    }
```

Find:

```rust
    fn parse_fn_decl(&mut self) -> PResult<Fn> {
        let span = self.peek().span;
        let pure = if self.at(&Tok::Pure) {
            self.advance();
            true
        } else {
            false
        };
        self.expect(Tok::Fn)?;
```

Replace with:

```rust
    fn parse_fn_decl(&mut self) -> PResult<Fn> {
        let span = self.peek().span;
        // `pub` must precede `pure` when both are present (`pub pure
        // fn`) -- only this order is checked for here, so `pure pub
        // fn` falls through to `self.expect(Tok::Fn)` below and fails
        // as a real parse error, not a silently-accepted alternate
        // spelling.
        let pub_ = if self.at(&Tok::Pub) {
            self.advance();
            true
        } else {
            false
        };
        let pure = if self.at(&Tok::Pure) {
            self.advance();
            true
        } else {
            false
        };
        self.expect(Tok::Fn)?;
```

Find:

```rust
        let body = self.parse_block()?;
        Ok(Fn { name, pure, params, return_type, where_clause, body, span })
    }
```

Replace with:

```rust
        let body = self.parse_block()?;
        Ok(Fn { name, pure, pub_, params, return_type, where_clause, body, span })
    }
```

- [ ] **Step 6: Fix `parse_program`'s struct/fn dispatch to look past a leading `pub`**

In `kestrelc/src/parser.rs`, find:

```rust
        let mut fns = Vec::new();
        let mut structs = Vec::new();
        while !self.at(&Tok::Eof) {
            if self.at(&Tok::Struct) {
                structs.push(self.parse_struct_decl()?);
            } else {
                fns.push(self.parse_fn_decl()?);
            }
        }
        Ok(Program { fns, structs, uses })
```

Replace with:

```rust
        let mut fns = Vec::new();
        let mut structs = Vec::new();
        while !self.at(&Tok::Eof) {
            // `pub struct`/`pub fn` both start with `Tok::Pub`, so the
            // dispatch has to look one token past an optional leading
            // `pub` to tell which parser to call.
            let is_struct = if self.at(&Tok::Pub) {
                matches!(self.tokens.get(self.pos + 1).map(|t| &t.tok), Some(Tok::Struct))
            } else {
                self.at(&Tok::Struct)
            };
            if is_struct {
                structs.push(self.parse_struct_decl()?);
            } else {
                fns.push(self.parse_fn_decl()?);
            }
        }
        Ok(Program { fns, structs, uses })
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cd kestrelc && cargo test --lib parser:: 2>&1 | tail -20`
Expected: PASS, all `parser::` tests including the 6 new ones.

- [ ] **Step 8: Run the full test suite to confirm no regressions elsewhere**

Run: `cd kestrelc && cargo build 2>&1 | tail -100`
Expected: builds cleanly (this surfaces every other `Fn`/`StructDecl` literal construction site that needs the new field — there should be none besides `parser.rs`, since `inline.rs`'s only construction uses `..f.clone()` struct-update syntax, which already carries any field it doesn't explicitly list).

Run: `cd kestrelc && cargo test 2>&1 | tail -15`
Expected: PASS, same count as before this task plus 6, 0 failures.

- [ ] **Step 9: Commit**

```bash
git add kestrelc/src/lexer.rs kestrelc/src/ast.rs kestrelc/src/parser.rs
git commit -m "Add pub keyword: lexer, AST fields, parser support"
```

---

### Task 2: Enforce visibility in `modules::merge_modules`

**Files:**
- Modify: `kestrelc/src/modules.rs`

**Interfaces:**
- Consumes: `Fn.pub_: bool`, `StructDecl.pub_: bool` from Task 1.
- Produces: `merge_modules` now returns a distinct "is private to module" error (not the existing "not found in module" one) for a `from`-import or bare qualified call referencing a real but non-`pub` name. No new public functions — this task only changes `merge_modules`'s internal logic and the signatures of the module-private `rewrite_expr`/`rewrite_stmt`/`rewrite_fn_signature_and_body` helpers (all three become `-> Result<(), KestrelcError>`, each taking an additional `denied: &HashMap<Symbol, String>` parameter after their existing `rename` parameter). `rewrite_type` and `rewrite_struct` are unchanged (a struct field's declared *type* can never be a qualified-call-shaped symbol, since there's no parser syntax for a qualified type reference — only qualified function/struct-literal *calls* exist).

This task changes existing behavior: every module-using test fixture across `modules.rs` and `integration.rs` that calls a cross-module function/constructs a cross-module struct (but was written before `pub` existed) now needs `pub` added to that declaration, or it will fail with the new private-access error. This task's steps include fixing every one of those — found by re-running the full suite after the enforcement change and fixing each failure, but the exact fixtures needing it are enumerated below (Steps 6-7) so there's no guesswork.

- [ ] **Step 1: Write the failing tests (new private-access-error cases)**

Add to `kestrelc/src/modules.rs`'s `#[cfg(test)] mod tests` block, right after `a_bare_use_qualified_call_resolves_to_the_source_modules_qualified_name`'s closing `}`:

```rust
    #[test]
    fn a_from_import_of_a_private_function_is_a_distinct_compile_error() {
        let dir = scratch_dir("merge_from_private_fn");
        fs::write(dir.join("geometry.kes"), "fn square(x: i64) -> i64 { return x * x; }").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use square from geometry;\nfn main() { print(square(7)); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let err = merge_modules(&entry, discovered).unwrap_err();
        assert!(err.message.contains("private"), "got: {}", err.message);
        assert!(!err.message.contains("not found"), "private access should not read as \"not found\", got: {}", err.message);
    }

    #[test]
    fn a_bare_use_qualified_call_to_a_private_function_is_a_distinct_compile_error() {
        let dir = scratch_dir("merge_bare_use_private_fn");
        fs::write(dir.join("geometry.kes"), "fn square(x: i64) -> i64 { return x * x; }").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use geometry;\nfn main() { print(geometry.square(7)); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let err = merge_modules(&entry, discovered).unwrap_err();
        assert!(err.message.contains("private"), "got: {}", err.message);
    }

    #[test]
    fn a_from_import_of_a_private_struct_is_a_distinct_compile_error() {
        let dir = scratch_dir("merge_from_private_struct");
        fs::write(dir.join("shapes.kes"), "struct Point { x: i64, y: i64 }").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use Point from shapes;\nfn main() { let p = Point { x: 1, y: 2 }; print(p.x); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let err = merge_modules(&entry, discovered).unwrap_err();
        assert!(err.message.contains("private"), "got: {}", err.message);
    }

    #[test]
    fn a_pub_function_is_importable_via_from_import() {
        let dir = scratch_dir("merge_pub_fn_from_import");
        fs::write(dir.join("geometry.kes"), "pub fn square(x: i64) -> i64 { return x * x; }").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use square from geometry;\nfn main() { print(square(7)); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let merged = merge_modules(&entry, discovered).unwrap();
        find_fn(&merged, "geometry$square");
    }

    #[test]
    fn a_pub_function_is_importable_via_bare_use_qualified_call() {
        let dir = scratch_dir("merge_pub_fn_qualified_call");
        fs::write(dir.join("geometry.kes"), "pub fn square(x: i64) -> i64 { return x * x; }").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use geometry;\nfn main() { print(geometry.square(7)); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let merged = merge_modules(&entry, discovered).unwrap();
        find_fn(&merged, "geometry$square");
    }

    #[test]
    fn a_private_function_is_still_fully_usable_within_its_own_module() {
        let dir = scratch_dir("merge_private_same_module_ok");
        fs::write(dir.join("dummy.kes"), "fn noop() {}").unwrap();
        let entry = dir.join("main.kes");
        fs::write(&entry, "use dummy;\nfn helper() { print(1); }\nfn main() { helper(); }").unwrap();
        let discovered = discover_modules(&entry).unwrap();
        let merged = merge_modules(&entry, discovered).unwrap();
        find_fn(&merged, "main$helper");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd kestrelc && cargo test --lib modules:: 2>&1 | tail -60`
Expected: the 6 new tests FAIL (the private-error ones currently succeed instead of erroring; nothing enforces privacy yet).

- [ ] **Step 3: Thread `denied` through the rewrite pass**

In `kestrelc/src/modules.rs`, find:

```rust
/// Renames only the Symbol positions that can ever denote a
/// function/struct *name* -- `Call.name`, `StructLit.name`, and (the
/// one place a bare function name appears without being called)
/// `parallel_map`'s first argument. A plain `ExprKind::Ident` anywhere
/// else is always a local variable/parameter read, never a function
/// reference, so it's deliberately left untouched here -- renaming it
/// too would incorrectly rewrite a local variable that happens to
/// share a name with an imported/declared function.
fn rewrite_expr(e: &mut Expr, rename: &HashMap<Symbol, Symbol>) {
    match &mut e.kind {
        ExprKind::Num(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::Ident(_) => {}
        ExprKind::ArrayLit(elems) => {
            for el in elems {
                rewrite_expr(el, rename);
            }
        }
        ExprKind::Unary { expr, .. } => rewrite_expr(expr, rename),
        ExprKind::Binop { left, right, .. } => {
            rewrite_expr(left, rename);
            rewrite_expr(right, rename);
        }
        ExprKind::Index { target, index } => {
            rewrite_expr(target, rename);
            rewrite_expr(index, rename);
        }
        ExprKind::Call { name, args } => {
            let is_parallel_map = *name == crate::interner::well_known::parallel_map();
            if let Some(&q) = rename.get(name) {
                *name = q;
            }
            if is_parallel_map {
                if let Some(first) = args.first_mut() {
                    if let ExprKind::Ident(fn_name) = &mut first.kind {
                        if let Some(&q) = rename.get(fn_name) {
                            *fn_name = q;
                        }
                    }
                }
            }
            for a in args {
                rewrite_expr(a, rename);
            }
        }
        ExprKind::StructLit { name, fields } => {
            if let Some(&q) = rename.get(name) {
                *name = q;
            }
            for (_, v) in fields {
                rewrite_expr(v, rename);
            }
        }
        ExprKind::Field { target, .. } => rewrite_expr(target, rename),
    }
}
```

Replace with:

```rust
/// Renames only the Symbol positions that can ever denote a
/// function/struct *name* -- `Call.name`, `StructLit.name`, and (the
/// one place a bare function name appears without being called)
/// `parallel_map`'s first argument. A plain `ExprKind::Ident` anywhere
/// else is always a local variable/parameter read, never a function
/// reference, so it's deliberately left untouched here -- renaming it
/// too would incorrectly rewrite a local variable that happens to
/// share a name with an imported/declared function.
///
/// `denied` holds synthesized "module.name" symbols (see parser.rs's
/// qualified-call desugar) for names that exist in their source module
/// but aren't `pub` -- encountering one as a Call/StructLit name is a
/// compile error naming the module, distinct from the generic "unknown
/// function" error a genuinely nonexistent name gets instead (that one
/// is never in `rename` or `denied`, so it just falls through unchanged
/// to resolve.rs's existing check).
fn rewrite_expr(e: &mut Expr, rename: &HashMap<Symbol, Symbol>, denied: &HashMap<Symbol, String>) -> Result<(), KestrelcError> {
    match &mut e.kind {
        ExprKind::Num(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::Ident(_) => {}
        ExprKind::ArrayLit(elems) => {
            for el in elems {
                rewrite_expr(el, rename, denied)?;
            }
        }
        ExprKind::Unary { expr, .. } => rewrite_expr(expr, rename, denied)?,
        ExprKind::Binop { left, right, .. } => {
            rewrite_expr(left, rename, denied)?;
            rewrite_expr(right, rename, denied)?;
        }
        ExprKind::Index { target, index } => {
            rewrite_expr(target, rename, denied)?;
            rewrite_expr(index, rename, denied)?;
        }
        ExprKind::Call { name, args } => {
            let is_parallel_map = *name == crate::interner::well_known::parallel_map();
            if let Some(reason) = denied.get(name) {
                return Err(KestrelcError::internal(ErrorKind::Resolve, reason.clone()));
            }
            if let Some(&q) = rename.get(name) {
                *name = q;
            }
            if is_parallel_map {
                if let Some(first) = args.first_mut() {
                    if let ExprKind::Ident(fn_name) = &mut first.kind {
                        if let Some(reason) = denied.get(fn_name) {
                            return Err(KestrelcError::internal(ErrorKind::Resolve, reason.clone()));
                        }
                        if let Some(&q) = rename.get(fn_name) {
                            *fn_name = q;
                        }
                    }
                }
            }
            for a in args {
                rewrite_expr(a, rename, denied)?;
            }
        }
        ExprKind::StructLit { name, fields } => {
            if let Some(reason) = denied.get(name) {
                return Err(KestrelcError::internal(ErrorKind::Resolve, reason.clone()));
            }
            if let Some(&q) = rename.get(name) {
                *name = q;
            }
            for (_, v) in fields {
                rewrite_expr(v, rename, denied)?;
            }
        }
        ExprKind::Field { target, .. } => rewrite_expr(target, rename, denied)?,
    }
    Ok(())
}
```

Find:

```rust
fn rewrite_stmt(s: &mut Stmt, rename: &HashMap<Symbol, Symbol>) {
    match s {
        Stmt::Let { value, .. } | Stmt::Assign { value, .. } | Stmt::FieldAssign { value, .. } => {
            rewrite_expr(value, rename)
        }
        Stmt::Break { .. } | Stmt::Continue { .. } => {}
        Stmt::If { cond, then_block, else_block, .. } => {
            rewrite_expr(cond, rename);
            for st in then_block {
                rewrite_stmt(st, rename);
            }
            if let Some(eb) = else_block {
                for st in eb {
                    rewrite_stmt(st, rename);
                }
            }
        }
        Stmt::While { cond, body, .. } => {
            rewrite_expr(cond, rename);
            for st in body {
                rewrite_stmt(st, rename);
            }
        }
        Stmt::RangeFor { start, end, body, .. } => {
            rewrite_expr(start, rename);
            rewrite_expr(end, rename);
            for st in body {
                rewrite_stmt(st, rename);
            }
        }
        Stmt::Print { args, .. } => {
            for a in args {
                rewrite_expr(a, rename);
            }
        }
        Stmt::Return { value, .. } => {
            if let Some(v) = value {
                rewrite_expr(v, rename);
            }
        }
        Stmt::ExprStmt { expr, .. } => rewrite_expr(expr, rename),
    }
}
```

Replace with:

```rust
fn rewrite_stmt(s: &mut Stmt, rename: &HashMap<Symbol, Symbol>, denied: &HashMap<Symbol, String>) -> Result<(), KestrelcError> {
    match s {
        Stmt::Let { value, .. } | Stmt::Assign { value, .. } | Stmt::FieldAssign { value, .. } => {
            rewrite_expr(value, rename, denied)?
        }
        Stmt::Break { .. } | Stmt::Continue { .. } => {}
        Stmt::If { cond, then_block, else_block, .. } => {
            rewrite_expr(cond, rename, denied)?;
            for st in then_block {
                rewrite_stmt(st, rename, denied)?;
            }
            if let Some(eb) = else_block {
                for st in eb {
                    rewrite_stmt(st, rename, denied)?;
                }
            }
        }
        Stmt::While { cond, body, .. } => {
            rewrite_expr(cond, rename, denied)?;
            for st in body {
                rewrite_stmt(st, rename, denied)?;
            }
        }
        Stmt::RangeFor { start, end, body, .. } => {
            rewrite_expr(start, rename, denied)?;
            rewrite_expr(end, rename, denied)?;
            for st in body {
                rewrite_stmt(st, rename, denied)?;
            }
        }
        Stmt::Print { args, .. } => {
            for a in args {
                rewrite_expr(a, rename, denied)?;
            }
        }
        Stmt::Return { value, .. } => {
            if let Some(v) = value {
                rewrite_expr(v, rename, denied)?;
            }
        }
        Stmt::ExprStmt { expr, .. } => rewrite_expr(expr, rename, denied)?,
    }
    Ok(())
}
```

Find:

```rust
fn rewrite_fn_signature_and_body(f: &mut Fn, rename: &HashMap<Symbol, Symbol>) {
    for p in &mut f.params {
        rewrite_type(&mut p.ty, rename);
    }
    if let Some(rt) = &mut f.return_type {
        rewrite_type(rt, rename);
    }
    if let Some(wc) = &mut f.where_clause {
        rewrite_expr(wc, rename);
    }
    for s in &mut f.body {
        rewrite_stmt(s, rename);
    }
}
```

Replace with:

```rust
fn rewrite_fn_signature_and_body(f: &mut Fn, rename: &HashMap<Symbol, Symbol>, denied: &HashMap<Symbol, String>) -> Result<(), KestrelcError> {
    for p in &mut f.params {
        rewrite_type(&mut p.ty, rename);
    }
    if let Some(rt) = &mut f.return_type {
        rewrite_type(rt, rename);
    }
    if let Some(wc) = &mut f.where_clause {
        rewrite_expr(wc, rename, denied)?;
    }
    for s in &mut f.body {
        rewrite_stmt(s, rename, denied)?;
    }
    Ok(())
}
```

`rewrite_type` and `rewrite_struct` are unchanged -- leave them exactly as they are.

- [ ] **Step 4: Change `declared`'s type to carry `pub_`, and check it in both import forms**

Find:

```rust
    // module name -> its own declared fn/struct names, used to
    // validate every from-import actually names something real.
    let declared: HashMap<String, (Vec<String>, Vec<String>)> = discovered
        .iter()
        .map(|(path, (_src, prog))| {
            let name = module_name_for_path(path);
            let fns = prog.fns.iter().map(|f| f.name.resolve().to_string()).collect();
            let structs = prog.structs.iter().map(|s| s.name.resolve().to_string()).collect();
            (name, (fns, structs))
        })
        .collect();
```

Replace with:

```rust
    // module name -> its own declared fn/struct names paired with
    // whether each is `pub` -- used to validate every from-import and
    // bare-use qualified call actually names something real AND
    // visible (see docs/superpowers/specs/2026-07-25-visibility-
    // design.md).
    let declared: HashMap<String, (Vec<(String, bool)>, Vec<(String, bool)>)> = discovered
        .iter()
        .map(|(path, (_src, prog))| {
            let name = module_name_for_path(path);
            let fns = prog.fns.iter().map(|f| (f.name.resolve().to_string(), f.pub_)).collect();
            let structs = prog.structs.iter().map(|s| (s.name.resolve().to_string(), s.pub_)).collect();
            (name, (fns, structs))
        })
        .collect();
```

Find:

```rust
        let mut rename: HashMap<Symbol, Symbol> = HashMap::new();
        for f in &program.fns {
            let is_entry_main = is_entry && f.name == crate::interner::well_known::main();
            if !is_entry_main {
                rename.insert(f.name, intern(&format!("{module_name}${}", f.name.resolve())));
            }
        }
        for s in &program.structs {
            rename.insert(s.name, intern(&format!("{module_name}${}", s.name.resolve())));
        }

        let mut seen_from_names: HashMap<String, String> = HashMap::new();
        for u in &program.uses {
            if let UseDecl::Module(module) = u {
                // Bare `use module;` -- every declared name becomes
                // callable/constructible as `module.name(...)`, which
                // the parser already desugars to a plain `Call`/
                // `StructLit` under the synthesized symbol
                // `"module.name"` (see parser.rs's qualified-call
                // parsing). Map that synthesized symbol straight to
                // the real qualified `module$name` here -- no new
                // rewrite logic needed, this reuses the exact same
                // rename table `rewrite_expr`'s Call/StructLit arms
                // already consult.
                let source_module = module.resolve().to_string();
                let (source_fns, source_structs) = declared.get(&source_module).ok_or_else(|| {
                    KestrelcError::internal(
                        ErrorKind::Resolve,
                        format!("kestrelc: module '{source_module}' not found"),
                    )
                })?;
                for fn_name in source_fns {
                    rename.insert(
                        intern(&format!("{source_module}.{fn_name}")),
                        intern(&format!("{source_module}${fn_name}")),
                    );
                }
                for struct_name in source_structs {
                    rename.insert(
                        intern(&format!("{source_module}.{struct_name}")),
                        intern(&format!("{source_module}${struct_name}")),
                    );
                }
            }
            if let UseDecl::Names { names, module } = u {
                let source_module = module.resolve().to_string();
                let (source_fns, source_structs) = declared.get(&source_module).ok_or_else(|| {
                    KestrelcError::internal(
                        ErrorKind::Resolve,
                        format!("kestrelc: module '{source_module}' not found"),
                    )
                })?;
                for name in names {
                    let name_text = name.resolve().to_string();
                    if !source_fns.contains(&name_text) && !source_structs.contains(&name_text) {
                        return Err(KestrelcError::internal(
                            ErrorKind::Resolve,
                            format!("kestrelc: '{name_text}' not found in module '{source_module}'"),
                        ));
                    }
                    let collides_locally = program.fns.iter().any(|f| f.name.resolve().as_ref() == name_text)
                        || program.structs.iter().any(|s| s.name.resolve().as_ref() == name_text);
                    if collides_locally {
                        return Err(KestrelcError::internal(
                            ErrorKind::Resolve,
                            format!(
                                "kestrelc: '{name_text}' imported from '{source_module}' collides with a local declaration in '{module_name}'"
                            ),
                        ));
                    }
                    if let Some(prior_module) = seen_from_names.get(&name_text) {
                        return Err(KestrelcError::internal(
                            ErrorKind::Resolve,
                            format!(
                                "kestrelc: '{name_text}' imported from both '{prior_module}' and '{source_module}' in '{module_name}'"
                            ),
                        ));
                    }
                    seen_from_names.insert(name_text.clone(), source_module.clone());
                    rename.insert(*name, intern(&format!("{source_module}${name_text}")));
                }
            }
        }

        for mut f in program.fns {
            rewrite_fn_signature_and_body(&mut f, &rename);
            if let Some(&qualified) = rename.get(&f.name) {
                f.name = qualified;
            } // else: the entry file's own `main`, left unqualified.
            merged_fns.push(f);
        }
        for mut s in program.structs {
            rewrite_struct(&mut s, &rename);
            if let Some(&qualified) = rename.get(&s.name) {
                s.name = qualified;
            }
            merged_structs.push(s);
        }
```

Replace with:

```rust
        let mut rename: HashMap<Symbol, Symbol> = HashMap::new();
        let mut denied: HashMap<Symbol, String> = HashMap::new();
        for f in &program.fns {
            let is_entry_main = is_entry && f.name == crate::interner::well_known::main();
            if !is_entry_main {
                rename.insert(f.name, intern(&format!("{module_name}${}", f.name.resolve())));
            }
        }
        for s in &program.structs {
            rename.insert(s.name, intern(&format!("{module_name}${}", s.name.resolve())));
        }

        let mut seen_from_names: HashMap<String, String> = HashMap::new();
        for u in &program.uses {
            if let UseDecl::Module(module) = u {
                // Bare `use module;` -- every declared *public* name
                // becomes callable/constructible as `module.name(...)`,
                // which the parser already desugars to a plain `Call`/
                // `StructLit` under the synthesized symbol
                // `"module.name"` (see parser.rs's qualified-call
                // parsing). Map that synthesized symbol straight to
                // the real qualified `module$name` here -- no new
                // rewrite logic needed for the public case, this reuses
                // the exact same rename table `rewrite_expr`'s Call/
                // StructLit arms already consult. A private name gets a
                // `denied` entry instead, so an actual attempt to call
                // it (not just the `use module;` line itself) is what
                // raises the "is private" error, right at the call
                // site -- there's no way to know at this point whether
                // the importing module will ever try to call any
                // particular one of the source module's names.
                let source_module = module.resolve().to_string();
                let (source_fns, source_structs) = declared.get(&source_module).ok_or_else(|| {
                    KestrelcError::internal(
                        ErrorKind::Resolve,
                        format!("kestrelc: module '{source_module}' not found"),
                    )
                })?;
                for (fn_name, is_pub) in source_fns {
                    let qualified_call_symbol = intern(&format!("{source_module}.{fn_name}"));
                    if *is_pub {
                        rename.insert(qualified_call_symbol, intern(&format!("{source_module}${fn_name}")));
                    } else {
                        denied.insert(
                            qualified_call_symbol,
                            format!("kestrelc: '{fn_name}' is private to module '{source_module}'"),
                        );
                    }
                }
                for (struct_name, is_pub) in source_structs {
                    let qualified_call_symbol = intern(&format!("{source_module}.{struct_name}"));
                    if *is_pub {
                        rename.insert(qualified_call_symbol, intern(&format!("{source_module}${struct_name}")));
                    } else {
                        denied.insert(
                            qualified_call_symbol,
                            format!("kestrelc: '{struct_name}' is private to module '{source_module}'"),
                        );
                    }
                }
            }
            if let UseDecl::Names { names, module } = u {
                let source_module = module.resolve().to_string();
                let (source_fns, source_structs) = declared.get(&source_module).ok_or_else(|| {
                    KestrelcError::internal(
                        ErrorKind::Resolve,
                        format!("kestrelc: module '{source_module}' not found"),
                    )
                })?;
                for name in names {
                    let name_text = name.resolve().to_string();
                    let is_pub = source_fns
                        .iter()
                        .find(|(n, _)| n == &name_text)
                        .or_else(|| source_structs.iter().find(|(n, _)| n == &name_text))
                        .map(|(_, p)| *p);
                    match is_pub {
                        None => {
                            return Err(KestrelcError::internal(
                                ErrorKind::Resolve,
                                format!("kestrelc: '{name_text}' not found in module '{source_module}'"),
                            ));
                        }
                        Some(false) => {
                            return Err(KestrelcError::internal(
                                ErrorKind::Resolve,
                                format!("kestrelc: '{name_text}' is private to module '{source_module}'"),
                            ));
                        }
                        Some(true) => {}
                    }
                    let collides_locally = program.fns.iter().any(|f| f.name.resolve().as_ref() == name_text)
                        || program.structs.iter().any(|s| s.name.resolve().as_ref() == name_text);
                    if collides_locally {
                        return Err(KestrelcError::internal(
                            ErrorKind::Resolve,
                            format!(
                                "kestrelc: '{name_text}' imported from '{source_module}' collides with a local declaration in '{module_name}'"
                            ),
                        ));
                    }
                    if let Some(prior_module) = seen_from_names.get(&name_text) {
                        return Err(KestrelcError::internal(
                            ErrorKind::Resolve,
                            format!(
                                "kestrelc: '{name_text}' imported from both '{prior_module}' and '{source_module}' in '{module_name}'"
                            ),
                        ));
                    }
                    seen_from_names.insert(name_text.clone(), source_module.clone());
                    rename.insert(*name, intern(&format!("{source_module}${name_text}")));
                }
            }
        }

        for mut f in program.fns {
            rewrite_fn_signature_and_body(&mut f, &rename, &denied)?;
            if let Some(&qualified) = rename.get(&f.name) {
                f.name = qualified;
            } // else: the entry file's own `main`, left unqualified.
            merged_fns.push(f);
        }
        for mut s in program.structs {
            rewrite_struct(&mut s, &rename);
            if let Some(&qualified) = rename.get(&s.name) {
                s.name = qualified;
            }
            merged_structs.push(s);
        }
```

- [ ] **Step 5: Build to catch any remaining call-site fallout**

Run: `cd kestrelc && cargo build --lib 2>&1 | tail -100`
Expected: builds cleanly. If there's a type error about `?` in a function not returning `Result`, or a call to `rewrite_expr`/`rewrite_stmt`/`rewrite_fn_signature_and_body` missing the new `denied` argument, it means a call site outside the ones shown above still needs updating -- search `kestrelc/src/modules.rs` for every call to those three functions and make sure each passes `denied` and either propagates via `?` (inside another function already returning `Result<_, KestrelcError>`) or is itself inside `merge_modules`, which already returns `Result<Program, KestrelcError>`.

- [ ] **Step 6: Run the new tests to verify they pass**

Run: `cd kestrelc && cargo test --lib modules:: 2>&1 | tail -60`
Expected: the 6 new tests from Step 1 now PASS. Some *other*, pre-existing `modules::` tests will now FAIL -- that's expected and fixed in the next step, not a sign this step did something wrong.

- [ ] **Step 7: Fix pre-existing `modules.rs` test fixtures that now need `pub`**

These fixtures call/construct a cross-module fn/struct that was written before `pub` existed, so under the new private-by-default rule they now fail with the new private-access error. Fix each by adding `pub` to the source module's declaration:

In `fn a_from_import_resolves_to_the_source_modules_qualified_name`, find:
```rust
        fs::write(dir.join("geometry.kes"), "pure fn sqrt(x: i64) -> i64 { return x; }").unwrap();
```
Replace with:
```rust
        fs::write(dir.join("geometry.kes"), "pub pure fn sqrt(x: i64) -> i64 { return x; }").unwrap();
```

In `fn a_from_import_colliding_with_a_local_declaration_is_a_compile_error`, find:
```rust
        fs::write(dir.join("geometry.kes"), "fn helper() {}").unwrap();
```
Replace with:
```rust
        fs::write(dir.join("geometry.kes"), "pub fn helper() {}").unwrap();
```
(This test is specifically about the *collision* error, not privacy -- `helper` must be `pub` so the collision check is actually reached instead of failing earlier on privacy.)

In `fn two_from_imports_of_the_same_name_from_different_modules_is_a_compile_error`, find:
```rust
        fs::write(dir.join("a.kes"), "fn helper() {}").unwrap();
        fs::write(dir.join("b.kes"), "fn helper() {}").unwrap();
```
Replace with:
```rust
        fs::write(dir.join("a.kes"), "pub fn helper() {}").unwrap();
        fs::write(dir.join("b.kes"), "pub fn helper() {}").unwrap();
```
(Same reasoning -- this test is about the "imported from both" error, which needs both to be reachably `pub` first.)

In `fn a_struct_from_import_is_qualified_and_struct_lit_follows`, find:
```rust
        fs::write(dir.join("shapes.kes"), "struct Point { x: i64, y: i64 }").unwrap();
```
Replace with:
```rust
        fs::write(dir.join("shapes.kes"), "pub struct Point { x: i64, y: i64 }").unwrap();
```

In `fn a_bare_use_qualified_call_resolves_to_the_source_modules_qualified_name`, find:
```rust
        fs::write(dir.join("geometry.kes"), "pure fn square(x: i64) -> i64 { return x * x; }").unwrap();
```
Replace with:
```rust
        fs::write(dir.join("geometry.kes"), "pub pure fn square(x: i64) -> i64 { return x * x; }").unwrap();
```

`fn same_named_functions_in_unrelated_modules_dont_collide_after_qualification` needs NO change: it only ever does a bare `use a; use b;` with no call to either module's `helper`, so privacy never comes into play there.

- [ ] **Step 8: Run the full unit test suite to confirm everything passes**

Run: `cd kestrelc && cargo test --lib 2>&1 | tail -20`
Expected: PASS, 0 failures.

- [ ] **Step 9: Commit**

```bash
git add kestrelc/src/modules.rs
git commit -m "Enforce pub/private visibility in merge_modules"
```

---

### Task 3: Integration tests + fix pre-existing integration fixtures

**Files:**
- Modify: `kestrelc/tests/integration.rs`

**Interfaces:**
- Consumes: the private-access compile error (containing the substring `"private"`) from Task 2, verified end-to-end via the real compiled binary.

- [ ] **Step 1: Fix pre-existing integration fixtures that now need `pub`**

In `fn a_from_import_compiles_and_runs_across_two_files_with_correct_output`, find:
```rust
    fs::write(
        scratch.join("geometry.kes"),
        r#"
        pure fn square(x: i64) -> i64 {
            return x * x;
        }
        "#,
    )
    .unwrap();
```
Replace with:
```rust
    fs::write(
        scratch.join("geometry.kes"),
        r#"
        pub pure fn square(x: i64) -> i64 {
            return x * x;
        }
        "#,
    )
    .unwrap();
```

In `fn a_bare_use_qualified_call_compiles_and_runs_across_two_files`, find:
```rust
    fs::write(
        scratch.join("geometry.kes"),
        "pure fn square(x: i64) -> i64 { return x * x; }\n",
    )
    .unwrap();
```
Replace with:
```rust
    fs::write(
        scratch.join("geometry.kes"),
        "pub pure fn square(x: i64) -> i64 { return x * x; }\n",
    )
    .unwrap();
```

In `fn editing_an_imported_module_invalidates_the_compile_cache`, find:
```rust
    fs::write(&geometry_path, "pure fn square(x: i64) -> i64 { return x * x; }\n").unwrap();
```
(the first of the two `fs::write(&geometry_path, ...)` calls in this test) and replace with:
```rust
    fs::write(&geometry_path, "pub pure fn square(x: i64) -> i64 { return x * x; }\n").unwrap();
```
Then find the second one, later in the same test:
```rust
    fs::write(&geometry_path, "pure fn square(x: i64) -> i64 { return x * x * x; }\n").unwrap();
```
Replace with:
```rust
    fs::write(&geometry_path, "pub pure fn square(x: i64) -> i64 { return x * x * x; }\n").unwrap();
```

`fn a_from_import_naming_a_nonexistent_function_is_a_compile_error` and `fn same_named_functions_in_two_unrelated_modules_compile_and_run_correctly` need NO change: the first imports a name (`nope`) that doesn't exist regardless of any other declaration's visibility, and the second never calls either module's `helper` at all.

- [ ] **Step 2: Run those fixed tests to confirm they still pass**

Run: `cd kestrelc && cargo test --test integration a_from_import_compiles_and_runs_across_two_files_with_correct_output a_bare_use_qualified_call_compiles_and_runs_across_two_files editing_an_imported_module_invalidates_the_compile_cache 2>&1 | tail -20`
Expected: PASS, all 3.

- [ ] **Step 3: Write new end-to-end integration tests for privacy**

Append to `kestrelc/tests/integration.rs`:

```rust
#[test]
fn a_from_import_of_a_private_function_is_a_distinct_compile_error() {
    let scratch = scratch_dir("visibility_from_private");
    fs::write(
        scratch.join("geometry.kes"),
        "fn square(x: i64) -> i64 { return x * x; }\n",
    )
    .unwrap();
    let entry = scratch.join("main.kes");
    fs::write(&entry, "use square from geometry;\nfn main() { print(square(7)); }\n").unwrap();

    let out = Command::new(kestrelc_bin())
        .arg(&entry)
        .current_dir(&scratch)
        .output()
        .expect("failed to run kestrelc");
    assert!(!out.status.success(), "kestrelc should have rejected importing a private function");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("private"), "got: {stderr}");
    assert!(!stderr.contains("not found"), "private access should not read as \"not found\", got: {stderr}");
}

#[test]
fn a_bare_use_qualified_call_to_a_private_function_is_a_distinct_compile_error() {
    let scratch = scratch_dir("visibility_qualified_private");
    fs::write(
        scratch.join("geometry.kes"),
        "fn square(x: i64) -> i64 { return x * x; }\n",
    )
    .unwrap();
    let entry = scratch.join("main.kes");
    fs::write(&entry, "use geometry;\nfn main() { print(geometry.square(7)); }\n").unwrap();

    let out = Command::new(kestrelc_bin())
        .arg(&entry)
        .current_dir(&scratch)
        .output()
        .expect("failed to run kestrelc");
    assert!(!out.status.success(), "kestrelc should have rejected a qualified call to a private function");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("private"), "got: {stderr}");
}

#[test]
fn a_pub_function_compiles_and_runs_across_two_files_both_import_forms() {
    let scratch = scratch_dir("visibility_pub_both_forms");
    fs::write(
        scratch.join("geometry.kes"),
        "pub pure fn square(x: i64) -> i64 { return x * x; }\n",
    )
    .unwrap();

    // Form 1: from-import.
    let entry1 = scratch.join("main_from.kes");
    fs::write(&entry1, "use square from geometry;\nfn main() { print(square(6)); }\n").unwrap();
    let out1 = Command::new(kestrelc_bin())
        .arg(&entry1)
        .current_dir(&scratch)
        .output()
        .expect("failed to run kestrelc");
    assert!(out1.status.success(), "compile failed:\n{}", String::from_utf8_lossy(&out1.stderr));
    let run1 = Command::new(scratch.join("main_from")).output().expect("failed to run compiled binary");
    assert_eq!(native_stdout(&run1), "36\n");

    // Form 2: bare use + qualified call.
    let entry2 = scratch.join("main_qualified.kes");
    fs::write(&entry2, "use geometry;\nfn main() { print(geometry.square(7)); }\n").unwrap();
    let out2 = Command::new(kestrelc_bin())
        .arg(&entry2)
        .current_dir(&scratch)
        .output()
        .expect("failed to run kestrelc");
    assert!(out2.status.success(), "compile failed:\n{}", String::from_utf8_lossy(&out2.stderr));
    let run2 = Command::new(scratch.join("main_qualified")).output().expect("failed to run compiled binary");
    assert_eq!(native_stdout(&run2), "49\n");
}

#[test]
fn a_private_function_still_works_when_called_only_within_its_own_module() {
    let scratch = scratch_dir("visibility_private_same_module_ok");
    fs::write(scratch.join("dummy.kes"), "fn noop() {}\n").unwrap();
    let entry = scratch.join("main.kes");
    fs::write(
        &entry,
        "use dummy;\nfn helper() { print(1); }\nfn main() { helper(); }\n",
    )
    .unwrap();

    let out = Command::new(kestrelc_bin())
        .arg(&entry)
        .current_dir(&scratch)
        .output()
        .expect("failed to run kestrelc");
    assert!(out.status.success(), "compile failed:\n{}", String::from_utf8_lossy(&out.stderr));
    let bin = scratch.join("main");
    let run = Command::new(&bin).output().expect("failed to run compiled binary");
    assert!(run.status.success(), "compiled binary exited with failure");
    assert_eq!(native_stdout(&run), "1\n");
}
```

- [ ] **Step 4: Run the new tests to verify they pass**

Run: `cd kestrelc && cargo test --test integration a_from_import_of_a_private_function_is_a_distinct_compile_error a_bare_use_qualified_call_to_a_private_function_is_a_distinct_compile_error a_pub_function_compiles_and_runs_across_two_files_both_import_forms a_private_function_still_works_when_called_only_within_its_own_module 2>&1 | tail -20`
Expected: PASS, all 4.

- [ ] **Step 5: Run the entire test suite (unit + integration) to confirm no regressions anywhere**

Run: `cd kestrelc && cargo test 2>&1 | tail -15`
Expected: PASS, 0 failures.

- [ ] **Step 6: Commit**

```bash
git add kestrelc/tests/integration.rs
git commit -m "Add integration tests for pub/private visibility"
```

---

## Self-Review Notes

- **Spec coverage:** private-by-default and `pub` opt-in (Task 1's parser changes + tests); both `from`-import and bare qualified-call paths gated uniformly (Task 2's `declared`/`denied` changes cover both `UseDecl::Names` and `UseDecl::Module` branches identically); declaration-level only, no per-field visibility (Task 1 only adds `pub_` to `Fn`/`StructDecl`, never to `Param`/struct fields); same-module access untouched (Task 2's `Step 7`/Task 3's new test `a_private_function_still_works_when_called_only_within_its_own_module` both directly verify this); `pub` before `pure` only (Task 1's `pure_pub_fn_wrong_keyword_order_is_a_parse_error` test); distinct private-vs-not-found error message (Task 2's tests assert `.contains("private")` and NOT `.contains("not found")`).
- **Placeholder scan:** no TBD/TODO; every step shows the actual before/after code, taken directly from the real current file contents (not reconstructed from memory).
- **Type consistency:** `Fn.pub_`/`StructDecl.pub_` (Task 1) match the types `declared`'s new `Vec<(String, bool)>` shape reads from (Task 2, Step 4) and what Task 2's tests assert on (`program.fns[0].pub_`/`find_fn(&merged, "geometry$square")`, matching Task 1's field name and Task 2's qualification scheme exactly). `rewrite_expr`/`rewrite_stmt`/`rewrite_fn_signature_and_body`'s new `-> Result<(), KestrelcError>` signatures and `denied: &HashMap<Symbol, String>` parameter are consistent across all three (each threads the same two parameters through in the same order: `rename` then `denied`).
