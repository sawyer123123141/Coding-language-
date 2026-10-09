# Core Rebuild 2a: Vertical Slice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A new `kestrel` compiler that takes programs using functions, scalar types and control flow in the new syntax, runs them via JIT (`kestrel run`) and builds them into a native exe (`kestrel build`), with identical output.

**Architecture:** Cargo workspace at `kestrel/` with six crates (syntax → check → kir → codegen, plus runtime and cli). The parser builds an AST with `ExprId`/`BindId`; the checker produces side tables; KIR is a typed, structured, desugared IR; one Cranelift lowering generic over `cranelift_module::Module` serves both JIT and object output. The runtime is a Rust crate linked as an rlib for JIT and a staticlib for exes.

**Tech Stack:** Rust 2021 edition (toolchain 1.97, host `x86_64-pc-windows-msvc`), Cranelift 0.116 (same version `kestrelc` already uses successfully on this machine), `find-msvc-tools` 0.1.9 for locating Microsoft `link.exe`.

**Spec:** `docs/superpowers/specs/2026-10-09-kestrel-core-rebuild-design.md` (architecture) and `docs/superpowers/specs/2026-10-09-kestrel-syntax-design.md` (syntax). This plan is the first of several (2a-2e) implementing the core rebuild; it covers the "thin vertical slice" from the spec's Risks section.

## Global Constraints

- All new code lives under `kestrel/` at the repo root. Do not modify `kestrelc/`, `kestrel.js`, `test/` or `kestrelc-devtool/` in this plan.
- Cranelift crates pinned to `0.116`. Calling convention always from `isa.default_call_conv()`. `enable_probestack = true`, `probestack_strategy = inline`, `opt_level = speed`, `is_pic = true`.
- Never invoke a bare `link.exe` from PATH (on this machine it is Git's coreutils `link`). Use `find_msvc_tools::find("x86_64-pc-windows-msvc", "link.exe")`.
- No source file over ~400 lines. Split by responsibility when a file approaches that.
- Comments: short. One line explaining *why* where needed; no history narration.
- Every compile error is a `Diagnostic` with a `Span`; the compiler collects all errors in a stage rather than stopping at the first.
- `int` = 32-bit signed, `float` = 32-bit float, `i64`, `u8`, `f64`, `bool`.
- Integer overflow and division by zero call the runtime panic (exit code 101) when checked arithmetic is on (`kestrel run`, `kestrel build`); `kestrel build --release` wraps on overflow (division by zero still panics).
- Printing uses Rust `Display` formatting for every type; `print(a, b)` joins arguments with one space and ends with a newline.
- Commit after every task, message style: imperative, short subject, `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` trailer.

## Language subset for 2a

Everything below parses, checks, and runs. Anything else gets a clear "not supported yet" diagnostic.

- Functions: `[pub] [pure] name(params) [-> T] { ... }`, params `name: T` or `var name: T`, recursion, labelled arguments `f(a, b: 2)`, call-site `var` (`f(var x)`).
- Top-level `let NAME = <constant expression>` (literals, operators, other earlier constants).
- Types: `int i64 u8 float f64 bool`. String literals only as direct `print` arguments.
- Statements: `let`/`var` (optional `: T`), assignment `=`, `+= -= *= /= %=`, `if`/`else if`/`else`, `while`, `for i from a to b`, `break`, `continue`, `return`.
- Expressions: literals (`42`, `1_000`, `0xFF`, `3.14`, `true`), names, unary `-`/`not`, binary `* / % + - == != < > <= >= and or`, calls, `if` as an expression, conversions `int(x) i64(x) u8(x) float(x) f64(x)`, `print(...)`.
- Checks: no shadowing, `let` immutability, types with implicit int→float and narrow→wide int, `pure` (no `print`, no `var` params, only calls `pure` functions), call-site `var` must match the parameter, exclusivity, no-effect statements, `main()` required.

## File Map

```
kestrel/
  Cargo.toml                    workspace
  .gitignore
  syntax/src/lib.rs             parse() entry, re-exports
  syntax/src/span.rs            Span, LineIndex
  syntax/src/diag.rs            Diagnostic, render, render_short
  syntax/src/token.rs           Tok, Token, keyword table
  syntax/src/lexer.rs           lex()
  syntax/src/newlines.rs        newline filtering (line continuation)
  syntax/src/ast.rs             AST types, to_sexpr()
  syntax/src/parser/mod.rs      Parser core helpers + recovery
  syntax/src/parser/expr.rs     expressions
  syntax/src/parser/stmt.rs     blocks + statements
  syntax/src/parser/item.rs     items + types
  syntax/tests/lexer.rs
  syntax/tests/parser.rs
  check/src/lib.rs              check() entry
  check/src/ty.rs               Ty
  check/src/tables.rs           Tables, LocalInfo, FnInfo, Res, ids
  check/src/collect.rs          signatures + constants
  check/src/body/mod.rs         BodyChecker core, scopes, blocks, statements
  check/src/body/expr.rs        expression typing
  check/src/body/call.rs        calls, builtins, var/exclusivity, purity
  check/tests/check.rs
  kir/src/lib.rs                lower() entry, re-exports
  kir/src/ir.rs                 KIR data types
  kir/src/lower/mod.rs          functions, statements
  kir/src/lower/expr.rs         expressions, calls, coercions
  kir/tests/lower.rs
  runtime/src/lib.rs            module list
  runtime/src/print.rs          buffered stdout + print_* exports
  runtime/src/panic.rs          kestrel_rt_panic, fmod
  runtime/src/start.rs          kestrel_rt_start
  codegen/src/lib.rs            Options, define_module()
  codegen/src/isa.rs            target ISA construction
  codegen/src/rt.rs             runtime import declarations
  codegen/src/func/mod.rs       FuncLowerer core + statements
  codegen/src/func/expr.rs      expressions
  codegen/src/func/arith.rs     checked arithmetic, casts, comparisons
  codegen/src/jit.rs            run_jit()
  codegen/src/object.rs         emit_object()
  cli/build.rs                  builds runtime staticlib, exports its path
  cli/src/main.rs               argument parsing, commands
  cli/src/driver.rs             source → KIR with diagnostics
  cli/src/link.rs               object + runtime → exe
  cli/tests/programs.rs         JIT + exe program harness
  cli/tests/errors.rs           diagnostic harness
  tests/programs/*.kes
  tests/errors/*.kes
```

---

### Task 1: Workspace, spans and diagnostics

**Files:**
- Create: `kestrel/Cargo.toml`, `kestrel/.gitignore`, `kestrel/syntax/Cargo.toml`, `kestrel/syntax/src/lib.rs`, `kestrel/syntax/src/span.rs`, `kestrel/syntax/src/diag.rs`
- Test: `kestrel/syntax/tests/diag.rs`

**Interfaces:**
- Produces: `Span { start: u32, end: u32 }`, `Span::new(usize, usize)`, `Span::to(Span) -> Span`; `LineIndex::new(&str)`, `LineIndex::line(u32) -> u32`, `LineIndex::line_col(&str, u32) -> (u32, u32)`, `LineIndex::line_text(&str, u32) -> &str`; `Severity { Error, Warning }`; `Diagnostic { severity, message, span, help }` with `Diagnostic::error(Span, impl Into<String>)`, `Diagnostic::warning(...)`, `.with_help(impl Into<String>)`, `.is_error()`; `render(src, path, &Diagnostic) -> String`; `render_short(src, &Diagnostic) -> String` producing `LINE:COL: error: MESSAGE`.

- [ ] **Step 1: Create the workspace**

`kestrel/Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["syntax"]

[workspace.package]
version = "0.1.0"
edition = "2021"

[workspace.dependencies]
kestrel-syntax = { path = "syntax" }
kestrel-check = { path = "check" }
kestrel-kir = { path = "kir" }
kestrel-runtime = { path = "runtime" }
kestrel-codegen = { path = "codegen" }
cranelift-codegen = "0.116"
cranelift-frontend = "0.116"
cranelift-module = "0.116"
cranelift-jit = "0.116"
cranelift-object = "0.116"
cranelift-native = "0.116"
find-msvc-tools = "0.1.9"
```

`kestrel/.gitignore`:
```
target/
*.exe
*.obj
*.pdb
```

`kestrel/syntax/Cargo.toml`:
```toml
[package]
name = "kestrel-syntax"
version.workspace = true
edition.workspace = true

[dependencies]
```

`kestrel/syntax/src/lib.rs`:
```rust
pub mod diag;
pub mod span;

pub use diag::{render, render_short, Diagnostic, Severity};
pub use span::{LineIndex, Span};
```

- [ ] **Step 2: Write the failing tests**

`kestrel/syntax/tests/diag.rs`:
```rust
use kestrel_syntax::{render, render_short, Diagnostic, LineIndex, Span};

const SRC: &str = "main() {\n    let x = 5 +\n}\n";

#[test]
fn line_col_is_one_based() {
    let idx = LineIndex::new(SRC);
    assert_eq!(idx.line_col(SRC, 0), (1, 1));
    assert_eq!(idx.line_col(SRC, 13), (2, 5));
    assert_eq!(idx.line(13), 2);
    assert_eq!(idx.line_text(SRC, 2), "    let x = 5 +");
}

#[test]
fn short_render() {
    let d = Diagnostic::error(Span::new(13, 16), "bad let");
    assert_eq!(render_short(SRC, &d), "2:5: error: bad let");
}

#[test]
fn full_render_with_caret_and_help() {
    let d = Diagnostic::error(Span::new(13, 16), "bad let").with_help("try harder");
    let out = render(SRC, "t.kes", &d);
    let expected = "error: bad let\n  --> t.kes:2:5\n   |\n 2 |     let x = 5 +\n   |     ^^^\n   = help: try harder\n";
    assert_eq!(out, expected);
}

#[test]
fn span_to_covers_both() {
    assert_eq!(Span::new(5, 7).to(Span::new(2, 3)), Span::new(2, 7));
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd kestrel && cargo test -p kestrel-syntax`
Expected: compile errors (`span`/`diag` modules missing).

- [ ] **Step 4: Implement `span.rs`**

```rust
/// A byte range in one source file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start: start as u32, end: end as u32 }
    }

    /// Smallest span covering both.
    pub fn to(self, other: Span) -> Span {
        Span { start: self.start.min(other.start), end: self.end.max(other.end) }
    }
}

/// Maps byte offsets to 1-based line and column numbers.
pub struct LineIndex {
    line_starts: Vec<u32>,
}

impl LineIndex {
    pub fn new(src: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i as u32 + 1);
            }
        }
        LineIndex { line_starts }
    }

    /// 1-based line containing `offset`.
    pub fn line(&self, offset: u32) -> u32 {
        match self.line_starts.binary_search(&offset) {
            Ok(i) => i as u32 + 1,
            Err(i) => i as u32,
        }
    }

    /// 1-based (line, column); column counts characters, not bytes.
    pub fn line_col(&self, src: &str, offset: u32) -> (u32, u32) {
        let line = self.line(offset);
        let start = self.line_starts[(line - 1) as usize] as usize;
        let col = src[start..offset as usize].chars().count() as u32 + 1;
        (line, col)
    }

    /// Text of a 1-based line without its line ending.
    pub fn line_text<'a>(&self, src: &'a str, line: u32) -> &'a str {
        let start = self.line_starts[(line - 1) as usize] as usize;
        let end = self.line_starts.get(line as usize).map(|&e| e as usize).unwrap_or(src.len());
        src[start..end].trim_end_matches(['\n', '\r'])
    }
}
```

- [ ] **Step 5: Implement `diag.rs`**

```rust
use crate::span::{LineIndex, Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Span,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Diagnostic { severity: Severity::Error, message: message.into(), span, help: None }
    }

    pub fn warning(span: Span, message: impl Into<String>) -> Self {
        Diagnostic { severity: Severity::Warning, message: message.into(), span, help: None }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

fn label(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

/// One line, stable format used by tests: `LINE:COL: error: MESSAGE`.
pub fn render_short(src: &str, d: &Diagnostic) -> String {
    let (line, col) = LineIndex::new(src).line_col(src, d.span.start);
    format!("{line}:{col}: {}: {}", label(d.severity), d.message)
}

/// Full human-readable form with source line and caret underline.
pub fn render(src: &str, path: &str, d: &Diagnostic) -> String {
    let idx = LineIndex::new(src);
    let (line, col) = idx.line_col(src, d.span.start);
    let text = idx.line_text(src, line);
    let gutter = line.to_string().len() + 1;
    let pad = " ".repeat(gutter);
    // Underline to the span end, clipped to this line, at least one caret.
    let span_chars = src[d.span.start as usize..(d.span.end as usize).min(src.len())]
        .chars()
        .take_while(|&c| c != '\n')
        .count();
    let carets = "^".repeat(span_chars.max(1));
    let mut out = format!(
        "{}: {}\n{pad}--> {path}:{line}:{col}\n{pad} |\n{:>w$} | {text}\n{pad} | {}{carets}\n",
        label(d.severity),
        d.message,
        line,
        " ".repeat(col as usize - 1),
        w = gutter,
    );
    if let Some(help) = &d.help {
        out.push_str(&format!("{pad} = help: {help}\n"));
    }
    out
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cd kestrel && cargo test -p kestrel-syntax`
Expected: 4 passed.

If `full_render_with_caret_and_help` fails, compare character by character. The expected layout is: gutter width = digits of the line number + 1 (here 2), so `  -->` has two spaces, `   |` has three, and the source line is right-aligned as ` 2 | `.

- [ ] **Step 7: Commit**

```bash
git add kestrel/
git commit -m "Add kestrel workspace with spans and diagnostics

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Tokens, lexer and line continuation

**Files:**
- Create: `kestrel/syntax/src/token.rs`, `kestrel/syntax/src/lexer.rs`, `kestrel/syntax/src/newlines.rs`
- Modify: `kestrel/syntax/src/lib.rs`
- Test: `kestrel/syntax/tests/lexer.rs`

**Interfaces:**
- Consumes: `Span`, `Diagnostic` (Task 1).
- Produces: `Tok` enum (variants below), `Token { tok: Tok, span: Span }`, `Tok::describe(&self) -> String`, `Tok::continues_line(&self) -> bool`, `lex(src: &str) -> (Vec<Token>, Vec<Diagnostic>)`. The token list always ends with exactly one `Tok::Eof`. Newlines that continue a statement are already removed (see rules below).

**Line-continuation rules** (from the syntax spec). A newline token is dropped when:
1. the innermost open bracket is `(` or `[` (braces `{` reset this, so blocks inside call arguments still have statement newlines),
2. the previous kept token ends a line that must continue: a binary operator, `=`, a compound assignment, `,`, `->`, `.`, or `{`,
3. the next non-newline token is `else`, `.`, `}` or end of file,
4. it is at the start of the file or directly follows another newline.

- [ ] **Step 1: Write the failing tests**

`kestrel/syntax/tests/lexer.rs`:
```rust
use kestrel_syntax::lexer::lex;
use kestrel_syntax::token::Tok;

fn toks(src: &str) -> Vec<Tok> {
    let (t, d) = lex(src);
    assert!(d.is_empty(), "unexpected diagnostics: {d:?}");
    t.into_iter().map(|t| t.tok).collect()
}

fn errors(src: &str) -> Vec<String> {
    lex(src).1.into_iter().map(|d| d.message).collect()
}

fn id(s: &str) -> Tok {
    Tok::Ident(s.into())
}

#[test]
fn keywords_and_identifiers() {
    assert_eq!(
        toks("pure let var par square self"),
        vec![Tok::Pure, Tok::Let, Tok::Var, Tok::Par, id("square"), Tok::SelfKw, Tok::Eof]
    );
}

#[test]
fn numbers() {
    assert_eq!(
        toks("42 1_000 0xFF 3.14 5"),
        vec![Tok::Int(42), Tok::Int(1000), Tok::Int(255), Tok::Float(3.14), Tok::Int(5), Tok::Eof]
    );
}

#[test]
fn strings_with_escapes() {
    assert_eq!(toks(r#""a\n\"b\" \{x\}""#), vec![Tok::Str("a\n\"b\" {x}".into()), Tok::Eof]);
}

#[test]
fn operators() {
    assert_eq!(
        toks("-> == != <= >= += -= *= /= %= = < > ? ;"),
        vec![
            Tok::Arrow, Tok::EqEq, Tok::NotEq, Tok::Le, Tok::Ge, Tok::PlusEq, Tok::MinusEq,
            Tok::StarEq, Tok::SlashEq, Tok::PercentEq, Tok::Assign, Tok::Lt, Tok::Gt,
            Tok::Question, Tok::Semi, Tok::Eof,
        ]
    );
}

#[test]
fn comments_are_skipped() {
    assert_eq!(toks("a // hello\nb"), vec![id("a"), Tok::Newline, id("b"), Tok::Eof]);
}

#[test]
fn newline_after_operator_continues() {
    assert_eq!(
        toks("x = a +\n  b\ny"),
        vec![id("x"), Tok::Assign, id("a"), Tok::Plus, id("b"), Tok::Newline, id("y"), Tok::Eof]
    );
}

#[test]
fn operator_at_line_start_does_not_continue() {
    // This is the trap the no-effect check later catches.
    assert_eq!(
        toks("x = a\n  + b"),
        vec![id("x"), Tok::Assign, id("a"), Tok::Newline, Tok::Plus, id("b"), Tok::Eof]
    );
}

#[test]
fn else_on_next_line_continues() {
    assert_eq!(toks("}\nelse {"), vec![Tok::RBrace, Tok::Else, Tok::LBrace, Tok::Eof]);
}

#[test]
fn newlines_inside_parens_are_dropped_but_not_inside_braces_within_them() {
    assert_eq!(
        toks("f(a,\n b)\n"),
        vec![id("f"), Tok::LParen, id("a"), Tok::Comma, id("b"), Tok::RParen, Tok::Eof]
    );
    assert_eq!(
        toks("f(if c {\nx\ny\n})"),
        vec![
            id("f"), Tok::LParen, Tok::If, id("c"), Tok::LBrace, id("x"), Tok::Newline,
            id("y"), Tok::RBrace, Tok::RParen, Tok::Eof,
        ]
    );
}

#[test]
fn blank_lines_collapse() {
    assert_eq!(toks("\n\na\n\n\nb\n\n"), vec![id("a"), Tok::Newline, id("b"), Tok::Eof]);
}

#[test]
fn helpful_errors() {
    assert_eq!(errors("a && b"), vec!["Kestrel uses `and`, not `&&`"]);
    assert_eq!(errors("a || b"), vec!["Kestrel uses `or`, not `||`"]);
    assert_eq!(errors("!a"), vec!["Kestrel uses `not`, not `!`"]);
    assert_eq!(
        errors("\"hp {x}\""),
        vec!["string interpolation isn't supported yet; escape the brace as `\\{`"]
    );
    assert_eq!(errors("\"abc"), vec!["unterminated string"]);
    assert_eq!(errors("a @ b"), vec!["unexpected character `@`"]);
    assert_eq!(errors("99999999999999999999"), vec!["number is too large"]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd kestrel && cargo test -p kestrel-syntax --test lexer`
Expected: compile errors (`lexer`, `token` modules missing).

- [ ] **Step 3: Implement `token.rs`**

```rust
use crate::span::Span;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(u64),
    Float(f64),
    Str(String),
    // keywords
    Pure, Let, Var, If, Else, While, For, From, To, In, Par, Match, Struct, Enum, Arena,
    Return, Break, Continue, And, Or, Not, True, False, Use, Pub, SelfKw,
    // punctuation
    LParen, RParen, LBrace, RBrace, LBracket, RBracket, Comma, Colon, Dot, Arrow, Question,
    Semi, Newline,
    // operators
    Plus, Minus, Star, Slash, Percent, Assign, EqEq, NotEq, Lt, Gt, Le, Ge,
    PlusEq, MinusEq, StarEq, SlashEq, PercentEq,
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

pub fn keyword(s: &str) -> Option<Tok> {
    Some(match s {
        "pure" => Tok::Pure,
        "let" => Tok::Let,
        "var" => Tok::Var,
        "if" => Tok::If,
        "else" => Tok::Else,
        "while" => Tok::While,
        "for" => Tok::For,
        "from" => Tok::From,
        "to" => Tok::To,
        "in" => Tok::In,
        "par" => Tok::Par,
        "match" => Tok::Match,
        "struct" => Tok::Struct,
        "enum" => Tok::Enum,
        "arena" => Tok::Arena,
        "return" => Tok::Return,
        "break" => Tok::Break,
        "continue" => Tok::Continue,
        "and" => Tok::And,
        "or" => Tok::Or,
        "not" => Tok::Not,
        "true" => Tok::True,
        "false" => Tok::False,
        "use" => Tok::Use,
        "pub" => Tok::Pub,
        "self" => Tok::SelfKw,
        _ => return None,
    })
}

impl Tok {
    /// A line ending in this token continues onto the next line.
    pub fn continues_line(&self) -> bool {
        use Tok::*;
        matches!(
            self,
            Plus | Minus | Star | Slash | Percent | Assign | EqEq | NotEq | Lt | Gt | Le | Ge
                | PlusEq | MinusEq | StarEq | SlashEq | PercentEq | And | Or | Comma | Arrow
                | Dot | LBrace
        )
    }

    /// How the token reads in an error message.
    pub fn describe(&self) -> String {
        use Tok::*;
        match self {
            Ident(s) => format!("name `{s}`"),
            Int(n) => format!("number `{n}`"),
            Float(f) => format!("number `{f}`"),
            Str(_) => "a string".into(),
            Newline => "end of line".into(),
            Eof => "end of file".into(),
            other => format!("`{}`", other.symbol()),
        }
    }

    fn symbol(&self) -> &'static str {
        use Tok::*;
        match self {
            Pure => "pure", Let => "let", Var => "var", If => "if", Else => "else",
            While => "while", For => "for", From => "from", To => "to", In => "in",
            Par => "par", Match => "match", Struct => "struct", Enum => "enum",
            Arena => "arena", Return => "return", Break => "break", Continue => "continue",
            And => "and", Or => "or", Not => "not", True => "true", False => "false",
            Use => "use", Pub => "pub", SelfKw => "self",
            LParen => "(", RParen => ")", LBrace => "{", RBrace => "}", LBracket => "[",
            RBracket => "]", Comma => ",", Colon => ":", Dot => ".", Arrow => "->",
            Question => "?", Semi => ";", Plus => "+", Minus => "-", Star => "*",
            Slash => "/", Percent => "%", Assign => "=", EqEq => "==", NotEq => "!=",
            Lt => "<", Gt => ">", Le => "<=", Ge => ">=", PlusEq => "+=", MinusEq => "-=",
            StarEq => "*=", SlashEq => "/=", PercentEq => "%=",
            Ident(_) | Int(_) | Float(_) | Str(_) | Newline | Eof => "",
        }
    }
}
```

- [ ] **Step 4: Implement `lexer.rs`**

```rust
use crate::diag::Diagnostic;
use crate::newlines;
use crate::span::Span;
use crate::token::{keyword, Tok, Token};

/// Source text to tokens. Always ends with one `Eof`; statement-continuing
/// newlines are already removed.
pub fn lex(src: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut lx = Lexer { src, bytes: src.as_bytes(), pos: 0, toks: Vec::new(), diags: Vec::new() };
    lx.run();
    let end = Span::new(src.len(), src.len());
    lx.toks.push(Token { tok: Tok::Eof, span: end });
    (newlines::filter(lx.toks), lx.diags)
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    toks: Vec<Token>,
    diags: Vec<Diagnostic>,
}

impl<'a> Lexer<'a> {
    fn peek(&self, ahead: usize) -> u8 {
        *self.bytes.get(self.pos + ahead).unwrap_or(&0)
    }

    fn push(&mut self, tok: Tok, start: usize) {
        self.toks.push(Token { tok, span: Span::new(start, self.pos) });
    }

    fn error(&mut self, start: usize, msg: impl Into<String>) {
        self.diags.push(Diagnostic::error(Span::new(start, self.pos.max(start + 1)), msg));
    }

    fn run(&mut self) {
        while self.pos < self.bytes.len() {
            let start = self.pos;
            match self.peek(0) {
                b' ' | b'\t' | b'\r' => self.pos += 1,
                b'\n' => {
                    self.pos += 1;
                    self.push(Tok::Newline, start);
                }
                b'/' if self.peek(1) == b'/' => {
                    while self.pos < self.bytes.len() && self.peek(0) != b'\n' {
                        self.pos += 1;
                    }
                }
                b'"' => self.string(),
                b'0'..=b'9' => self.number(),
                c if c == b'_' || c.is_ascii_alphabetic() => self.word(),
                _ => self.punct(),
            }
        }
    }

    fn word(&mut self) {
        let start = self.pos;
        while self.peek(0) == b'_' || self.peek(0).is_ascii_alphanumeric() {
            self.pos += 1;
        }
        let text = &self.src[start..self.pos];
        let tok = keyword(text).unwrap_or_else(|| Tok::Ident(text.to_string()));
        self.push(tok, start);
    }

    fn number(&mut self) {
        let start = self.pos;
        if self.peek(0) == b'0' && (self.peek(1) == b'x' || self.peek(1) == b'X') {
            self.pos += 2;
            while self.peek(0).is_ascii_hexdigit() || self.peek(0) == b'_' {
                self.pos += 1;
            }
            let digits: String = self.src[start + 2..self.pos].chars().filter(|&c| c != '_').collect();
            match u64::from_str_radix(&digits, 16) {
                Ok(n) => self.push(Tok::Int(n), start),
                Err(_) => self.error(start, "number is too large"),
            }
            return;
        }
        while self.peek(0).is_ascii_digit() || self.peek(0) == b'_' {
            self.pos += 1;
        }
        let is_float = self.peek(0) == b'.' && self.peek(1).is_ascii_digit();
        if is_float {
            self.pos += 1;
            while self.peek(0).is_ascii_digit() || self.peek(0) == b'_' {
                self.pos += 1;
            }
        }
        let text: String = self.src[start..self.pos].chars().filter(|&c| c != '_').collect();
        if is_float {
            self.push(Tok::Float(text.parse().unwrap()), start);
        } else {
            match text.parse::<u64>() {
                Ok(n) => self.push(Tok::Int(n), start),
                Err(_) => self.error(start, "number is too large"),
            }
        }
    }

    fn string(&mut self) {
        let start = self.pos;
        self.pos += 1;
        let mut value = String::new();
        loop {
            if self.pos >= self.bytes.len() || self.peek(0) == b'\n' {
                self.error(start, "unterminated string");
                return;
            }
            match self.peek(0) {
                b'"' => {
                    self.pos += 1;
                    self.push(Tok::Str(value), start);
                    return;
                }
                b'\\' => {
                    let ch = match self.peek(1) {
                        b'n' => '\n',
                        b't' => '\t',
                        b'"' => '"',
                        b'\\' => '\\',
                        b'{' => '{',
                        b'}' => '}',
                        _ => {
                            let at = self.pos;
                            self.pos += 2;
                            self.error(at, "unknown escape sequence");
                            continue;
                        }
                    };
                    value.push(ch);
                    self.pos += 2;
                }
                b'{' => {
                    let at = self.pos;
                    self.pos += 1;
                    self.error(at, "string interpolation isn't supported yet; escape the brace as `\\{`");
                }
                _ => {
                    let ch = self.src[self.pos..].chars().next().unwrap();
                    value.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn punct(&mut self) {
        let start = self.pos;
        let two = [self.peek(0), self.peek(1)];
        let pair = match &two {
            b"->" => Some(Tok::Arrow),
            b"==" => Some(Tok::EqEq),
            b"!=" => Some(Tok::NotEq),
            b"<=" => Some(Tok::Le),
            b">=" => Some(Tok::Ge),
            b"+=" => Some(Tok::PlusEq),
            b"-=" => Some(Tok::MinusEq),
            b"*=" => Some(Tok::StarEq),
            b"/=" => Some(Tok::SlashEq),
            b"%=" => Some(Tok::PercentEq),
            _ => None,
        };
        if let Some(tok) = pair {
            self.pos += 2;
            self.push(tok, start);
            return;
        }
        if &two == b"&&" || &two == b"||" {
            self.pos += 2;
            let msg = if two[0] == b'&' { "Kestrel uses `and`, not `&&`" } else { "Kestrel uses `or`, not `||`" };
            self.error(start, msg);
            return;
        }
        let ch = self.src[self.pos..].chars().next().unwrap();
        self.pos += ch.len_utf8();
        let tok = match ch {
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            '{' => Tok::LBrace,
            '}' => Tok::RBrace,
            '[' => Tok::LBracket,
            ']' => Tok::RBracket,
            ',' => Tok::Comma,
            ':' => Tok::Colon,
            '.' => Tok::Dot,
            '?' => Tok::Question,
            ';' => Tok::Semi,
            '+' => Tok::Plus,
            '-' => Tok::Minus,
            '*' => Tok::Star,
            '/' => Tok::Slash,
            '%' => Tok::Percent,
            '=' => Tok::Assign,
            '<' => Tok::Lt,
            '>' => Tok::Gt,
            '!' => return self.error(start, "Kestrel uses `not`, not `!`"),
            other => return self.error(start, format!("unexpected character `{other}`")),
        };
        self.push(tok, start);
    }
}
```

- [ ] **Step 5: Implement `newlines.rs`**

```rust
use crate::token::{Tok, Token};

/// Drops newlines that don't end a statement (see the continuation rules above).
pub fn filter(raw: Vec<Token>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(raw.len());
    let mut brackets: Vec<Tok> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let t = &raw[i];
        if t.tok == Tok::Newline {
            let mut j = i + 1;
            while j < raw.len() && raw[j].tok == Tok::Newline {
                j += 1;
            }
            let in_parens = matches!(brackets.last(), Some(Tok::LParen) | Some(Tok::LBracket));
            let prev_continues = out.last().map_or(true, |p| p.tok.continues_line());
            let next_continues = matches!(
                raw.get(j).map(|t| &t.tok),
                None | Some(Tok::Else) | Some(Tok::Dot) | Some(Tok::RBrace) | Some(Tok::Eof)
            );
            if !(in_parens || prev_continues || next_continues) {
                out.push(t.clone());
            }
            i = j;
            continue;
        }
        match t.tok {
            Tok::LParen | Tok::LBracket | Tok::LBrace => brackets.push(t.tok.clone()),
            Tok::RParen | Tok::RBracket | Tok::RBrace => {
                brackets.pop();
            }
            _ => {}
        }
        out.push(t.clone());
        i += 1;
    }
    out
}
```

- [ ] **Step 6: Register the modules**

`kestrel/syntax/src/lib.rs`:
```rust
pub mod diag;
pub mod lexer;
mod newlines;
pub mod span;
pub mod token;

pub use diag::{render, render_short, Diagnostic, Severity};
pub use span::{LineIndex, Span};
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cd kestrel && cargo test -p kestrel-syntax`
Expected: all lexer tests and the 4 diag tests pass.

- [ ] **Step 8: Commit**

```bash
git add kestrel/syntax
git commit -m "Add kestrel lexer with line-continuation rules

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: AST and parser

**Files:**
- Create: `kestrel/syntax/src/ast.rs`, `kestrel/syntax/src/parser/mod.rs`, `kestrel/syntax/src/parser/expr.rs`, `kestrel/syntax/src/parser/stmt.rs`, `kestrel/syntax/src/parser/item.rs`
- Modify: `kestrel/syntax/src/lib.rs`
- Test: `kestrel/syntax/tests/parser.rs`

**Interfaces:**
- Consumes: `lex`, `Tok`, `Token`, `Span`, `Diagnostic` (Tasks 1-2).
- Produces: `kestrel_syntax::parse(src: &str) -> (ast::Program, Vec<Diagnostic>)` and every type in `ast.rs` below, exactly as written (later tasks match on these). `ExprId`/`BindId` are dense, starting at 0; `Program.expr_count`/`bind_count` are their totals. `Expr::to_sexpr`, `Block::to_sexpr`, `Stmt::to_sexpr` for tests.

- [ ] **Step 1: Write the AST (`ast.rs`)**

```rust
use crate::span::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExprId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BindId(pub u32);

#[derive(Clone, Debug)]
pub struct Program {
    pub items: Vec<Item>,
    /// Total `ExprId`s handed out; side tables size by this.
    pub expr_count: u32,
    pub bind_count: u32,
}

#[derive(Clone, Debug)]
pub enum Item {
    Fn(FnDecl),
    Const(ConstDecl),
}

#[derive(Clone, Debug)]
pub struct FnDecl {
    pub name: String,
    pub name_span: Span,
    pub is_pure: bool,
    pub is_pub: bool,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub bind: Binding,
    pub is_var: bool,
    pub ty: TypeExpr,
}

#[derive(Clone, Debug)]
pub struct ConstDecl {
    pub is_pub: bool,
    pub bind: Binding,
    pub ty: Option<TypeExpr>,
    pub value: Expr,
    pub span: Span,
}

/// A name being introduced: `let` target, parameter, loop variable.
#[derive(Clone, Debug)]
pub struct Binding {
    pub id: BindId,
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct TypeExpr {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TypeKind {
    Named { name: String, args: Vec<TypeExpr> },
    Array(Box<TypeExpr>),
    Optional(Box<TypeExpr>),
}

#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let { is_var: bool, bind: Binding, ty: Option<TypeExpr>, value: Expr, span: Span },
    Assign { target: Expr, op: AssignOp, value: Expr, span: Span },
    While { cond: Expr, body: Block, span: Span },
    ForRange { var: Binding, start: Expr, end: Expr, body: Block, span: Span },
    Break(Span),
    Continue(Span),
    Return { value: Option<Expr>, span: Span },
    Expr(Expr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOp {
    Set,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub id: ExprId,
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(u64),
    Float(f64),
    Bool(bool),
    Str(String),
    Ident(String),
    Unary { op: UnaryOp, expr: Box<Expr> },
    Binary { op: BinaryOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Call { name: String, name_span: Span, args: Vec<Arg> },
    If { cond: Box<Expr>, then_block: Block, else_block: Option<Block> },
}

#[derive(Clone, Debug)]
pub struct Arg {
    pub label: Option<String>,
    pub is_var: bool,
    pub value: Expr,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}

impl BinaryOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Rem => "%",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Gt => ">",
            BinaryOp::Le => "<=",
            BinaryOp::Ge => ">=",
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
        }
    }

    pub fn is_comparison(self) -> bool {
        matches!(self, BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Le | BinaryOp::Ge)
    }
}

impl AssignOp {
    pub fn symbol(self) -> &'static str {
        match self {
            AssignOp::Set => "=",
            AssignOp::Add => "+=",
            AssignOp::Sub => "-=",
            AssignOp::Mul => "*=",
            AssignOp::Div => "/=",
            AssignOp::Rem => "%=",
        }
    }

    /// The arithmetic a compound assignment performs, if any.
    pub fn binary(self) -> Option<BinaryOp> {
        match self {
            AssignOp::Set => None,
            AssignOp::Add => Some(BinaryOp::Add),
            AssignOp::Sub => Some(BinaryOp::Sub),
            AssignOp::Mul => Some(BinaryOp::Mul),
            AssignOp::Div => Some(BinaryOp::Div),
            AssignOp::Rem => Some(BinaryOp::Rem),
        }
    }
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Let { span, .. }
            | Stmt::Assign { span, .. }
            | Stmt::While { span, .. }
            | Stmt::ForRange { span, .. }
            | Stmt::Return { span, .. } => *span,
            Stmt::Break(span) | Stmt::Continue(span) => *span,
            Stmt::Expr(e) => e.span,
        }
    }

    pub fn to_sexpr(&self) -> String {
        match self {
            Stmt::Let { is_var, bind, value, .. } => {
                format!("({} {} {})", if *is_var { "var" } else { "let" }, bind.name, value.to_sexpr())
            }
            Stmt::Assign { target, op, value, .. } => {
                format!("({} {} {})", op.symbol(), target.to_sexpr(), value.to_sexpr())
            }
            Stmt::While { cond, body, .. } => format!("(while {} {})", cond.to_sexpr(), body.to_sexpr()),
            Stmt::ForRange { var, start, end, body, .. } => format!(
                "(for {} {} {} {})",
                var.name,
                start.to_sexpr(),
                end.to_sexpr(),
                body.to_sexpr()
            ),
            Stmt::Break(_) => "break".into(),
            Stmt::Continue(_) => "continue".into(),
            Stmt::Return { value: None, .. } => "return".into(),
            Stmt::Return { value: Some(v), .. } => format!("(return {})", v.to_sexpr()),
            Stmt::Expr(e) => e.to_sexpr(),
        }
    }
}

impl Block {
    pub fn to_sexpr(&self) -> String {
        let inner: Vec<String> = self.stmts.iter().map(Stmt::to_sexpr).collect();
        format!("{{{}}}", inner.join("; "))
    }
}

impl Expr {
    /// Compact prefix form for tests: `1 + 2 * 3` is `(+ 1 (* 2 3))`.
    pub fn to_sexpr(&self) -> String {
        match &self.kind {
            ExprKind::Int(n) => n.to_string(),
            ExprKind::Float(f) => f.to_string(),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Str(s) => format!("{s:?}"),
            ExprKind::Ident(name) => name.clone(),
            ExprKind::Unary { op, expr } => {
                let sym = match op {
                    UnaryOp::Neg => "-",
                    UnaryOp::Not => "not",
                };
                format!("({sym} {})", expr.to_sexpr())
            }
            ExprKind::Binary { op, lhs, rhs } => format!("({} {} {})", op.symbol(), lhs.to_sexpr(), rhs.to_sexpr()),
            ExprKind::Call { name, args, .. } => {
                let mut s = format!("(call {name}");
                for a in args {
                    s.push(' ');
                    if let Some(l) = &a.label {
                        s.push_str(&format!("{l}: "));
                    }
                    if a.is_var {
                        s.push_str("var ");
                    }
                    s.push_str(&a.value.to_sexpr());
                }
                s.push(')');
                s
            }
            ExprKind::If { cond, then_block, else_block } => match else_block {
                Some(e) => format!("(if {} {} else {})", cond.to_sexpr(), then_block.to_sexpr(), e.to_sexpr()),
                None => format!("(if {} {})", cond.to_sexpr(), then_block.to_sexpr()),
            },
        }
    }
}
```

- [ ] **Step 2: Write the failing parser tests**

`kestrel/syntax/tests/parser.rs`:
```rust
use kestrel_syntax::ast::*;
use kestrel_syntax::parse;

fn ok(src: &str) -> Program {
    let (p, d) = parse(src);
    let msgs: Vec<&String> = d.iter().map(|d| &d.message).collect();
    assert!(d.is_empty(), "unexpected diagnostics: {msgs:?}");
    p
}

fn errs(src: &str) -> Vec<String> {
    parse(src).1.into_iter().map(|d| d.message).collect()
}

fn fn_decl(p: &Program, i: usize) -> &FnDecl {
    match &p.items[i] {
        Item::Fn(f) => f,
        other => panic!("expected fn, got {other:?}"),
    }
}

/// Parses `src` as the body of `main` and returns the body as an s-expression.
fn body(src: &str) -> String {
    let p = ok(&format!("main() {{\n{src}\n}}"));
    fn_decl(&p, 0).body.to_sexpr()
}

#[test]
fn parses_fib() {
    let p = ok("pure fib(n: int) -> int {\n    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }\n}\n\nmain() {\n    print(fib(10))\n}\n");
    assert_eq!(p.items.len(), 2);
    let fib = fn_decl(&p, 0);
    assert!(fib.is_pure);
    assert_eq!(fib.params.len(), 1);
    assert!(fib.ret.is_some());
    assert_eq!(
        fib.body.to_sexpr(),
        "{(if (< n 2) {n} else {(+ (call fib (- n 1)) (call fib (- n 2)))})}"
    );
}

#[test]
fn precedence() {
    assert_eq!(body("1 + 2 * 3 - 4"), "{(- (+ 1 (* 2 3)) 4)}");
    assert_eq!(body("not a and b or c"), "{(or (and (not a) b) c)}");
    assert_eq!(body("-x * 2 == y"), "{(== (* (- x) 2) y)}");
    assert_eq!(body("(1 + 2) * 3"), "{(* (+ 1 2) 3)}");
}

#[test]
fn calls_with_labels_and_var() {
    assert_eq!(body("f(a, b: 2, var c)"), "{(call f a b: 2 var c)}");
    assert_eq!(body("f()"), "{(call f)}");
}

#[test]
fn else_if_chains_nest() {
    assert_eq!(
        body("if a { 1 } else if b { 2 } else { 3 }"),
        "{(if a {1} else {(if b {2} else {3})})}"
    );
}

#[test]
fn statements() {
    assert_eq!(
        body("let x = 1\nvar y: int = 2\ny += x\nwhile y < 10 { y = y * 2 }\nfor i from 0 to 3 { break }\nreturn"),
        "{(let x 1); (var y 2); (+= y x); (while (< y 10) {(= y (* y 2))}); (for i 0 3 {break}); return}"
    );
    assert_eq!(body("let a = 1; let b = 2"), "{(let a 1); (let b 2)}");
}

#[test]
fn continuation_and_else_on_next_line() {
    assert_eq!(body("let x = 1 +\n    2"), "{(let x (+ 1 2))}");
    assert_eq!(body("if a {\n 1\n}\nelse {\n 2\n}"), "{(if a {1} else {2})}");
}

#[test]
fn types() {
    let p = ok("f(a: [int], b: list[vec3], c: map[string, int]?, var d: float) {}\n");
    let f = fn_decl(&p, 0);
    assert!(matches!(f.params[0].ty.kind, TypeKind::Array(_)));
    match &f.params[1].ty.kind {
        TypeKind::Named { name, args } => {
            assert_eq!(name, "list");
            assert_eq!(args.len(), 1);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(f.params[2].ty.kind, TypeKind::Optional(_)));
    assert!(f.params[3].is_var);
}

#[test]
fn top_level_constant() {
    let p = ok("let SIZE = 32\nmain() {}\n");
    assert!(matches!(&p.items[0], Item::Const(c) if c.bind.name == "SIZE"));
}

#[test]
fn ids_are_dense() {
    let p = ok("main() { let a = 1 + 2 }\n");
    assert_eq!(p.expr_count, 3);
    assert_eq!(p.bind_count, 1);
}

#[test]
fn helpful_errors_and_recovery() {
    assert_eq!(errs("fn main() {}"), vec!["Kestrel functions don't use `fn`: write `name(params) { ... }`"]);
    assert_eq!(
        errs("main() {\n let = 5\n let y =\n}"),
        vec!["expected a variable name, found `=`", "expected an expression, found `}`"]
    );
    assert_eq!(errs("main() { a < b < c }"), vec!["comparisons can't be chained; combine them with `and`"]);
    assert_eq!(errs("main() {\n for x in xs { }\n}"), vec!["for-in loops aren't supported yet"]);
    assert_eq!(errs("struct A {}\nmain() {}"), vec!["`struct` and `enum` aren't supported yet"]);
    assert_eq!(errs("main() { a b }"), vec!["expected end of line, found name `b`"]);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd kestrel && cargo test -p kestrel-syntax --test parser`
Expected: compile errors (`parse`, `ast` missing).

- [ ] **Step 4: Implement the parser core (`parser/mod.rs`)**

```rust
mod expr;
mod item;
mod stmt;

use crate::ast::{BindId, Binding, Expr, ExprId, ExprKind, Program};
use crate::diag::Diagnostic;
use crate::span::Span;
use crate::token::{Tok, Token};

pub(crate) type PResult<T> = Result<T, ()>;

pub struct Parser<'t> {
    toks: &'t [Token],
    pos: usize,
    diags: Vec<Diagnostic>,
    next_expr: u32,
    next_bind: u32,
}

impl<'t> Parser<'t> {
    /// `toks` must end with `Tok::Eof` (as `lex` guarantees).
    pub fn new(toks: &'t [Token]) -> Self {
        Parser { toks, pos: 0, diags: Vec::new(), next_expr: 0, next_bind: 0 }
    }

    pub fn parse_program(mut self) -> (Program, Vec<Diagnostic>) {
        let items = self.items();
        let program = Program { items, expr_count: self.next_expr, bind_count: self.next_bind };
        (program, self.diags)
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }

    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    /// Consumes the current token. Never moves past `Eof`.
    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn at(&self, t: &Tok) -> bool {
        self.peek() == t
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.at(t) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: &Tok, what: &str) -> PResult<Span> {
        if self.at(t) {
            Ok(self.bump().span)
        } else {
            let found = self.peek().describe();
            self.error_here(format!("expected {what}, found {found}"));
            Err(())
        }
    }

    fn expect_ident(&mut self, what: &str) -> PResult<(String, Span)> {
        if let Tok::Ident(name) = self.peek().clone() {
            let span = self.bump().span;
            Ok((name, span))
        } else {
            let found = self.peek().describe();
            self.error_here(format!("expected {what}, found {found}"));
            Err(())
        }
    }

    fn skip_newlines(&mut self) {
        while self.at(&Tok::Newline) || self.at(&Tok::Semi) {
            self.bump();
        }
    }

    fn error(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(Diagnostic::error(span, msg));
    }

    fn error_here(&mut self, msg: impl Into<String>) {
        let span = self.span();
        self.error(span, msg);
    }

    fn expr_node(&mut self, kind: ExprKind, span: Span) -> Expr {
        let id = ExprId(self.next_expr);
        self.next_expr += 1;
        Expr { id, kind, span }
    }

    fn binding(&mut self, name: String, span: Span) -> Binding {
        let id = BindId(self.next_bind);
        self.next_bind += 1;
        Binding { id, name, span }
    }

    /// Skips to the end of the current statement: a newline or `;` at this
    /// nesting level, or the `}` that closes the enclosing block.
    fn recover_stmt(&mut self) {
        let mut depth = 0i32;
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::LBrace | Tok::LParen | Tok::LBracket => depth += 1,
                Tok::RBrace if depth == 0 => return,
                Tok::RBrace | Tok::RParen | Tok::RBracket => depth = (depth - 1).max(0),
                Tok::Newline | Tok::Semi if depth == 0 => return,
                _ => {}
            }
            self.bump();
        }
    }

    /// Skips to the next top-level item: a newline outside any braces.
    fn recover_item(&mut self) {
        let mut depth = 0i32;
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::LBrace => depth += 1,
                Tok::RBrace => depth -= 1,
                Tok::Newline if depth <= 0 => return,
                _ => {}
            }
            self.bump();
        }
    }
}
```

- [ ] **Step 5: Implement expressions (`parser/expr.rs`)**

```rust
use super::{PResult, Parser};
use crate::ast::{Arg, BinaryOp, Block, Expr, ExprKind, Stmt, UnaryOp};
use crate::span::Span;
use crate::token::Tok;

fn cmp_op(t: &Tok) -> Option<BinaryOp> {
    Some(match t {
        Tok::EqEq => BinaryOp::Eq,
        Tok::NotEq => BinaryOp::Ne,
        Tok::Lt => BinaryOp::Lt,
        Tok::Gt => BinaryOp::Gt,
        Tok::Le => BinaryOp::Le,
        Tok::Ge => BinaryOp::Ge,
        _ => return None,
    })
}

impl<'t> Parser<'t> {
    pub(crate) fn expr(&mut self) -> PResult<Expr> {
        self.or_expr()
    }

    fn binary(&mut self, op: BinaryOp, lhs: Expr, rhs: Expr) -> Expr {
        let span = lhs.span.to(rhs.span);
        self.expr_node(ExprKind::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) }, span)
    }

    fn or_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.and_expr()?;
        while self.eat(&Tok::Or) {
            let rhs = self.and_expr()?;
            lhs = self.binary(BinaryOp::Or, lhs, rhs);
        }
        Ok(lhs)
    }

    fn and_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.cmp_expr()?;
        while self.eat(&Tok::And) {
            let rhs = self.cmp_expr()?;
            lhs = self.binary(BinaryOp::And, lhs, rhs);
        }
        Ok(lhs)
    }

    fn cmp_expr(&mut self) -> PResult<Expr> {
        let lhs = self.add_expr()?;
        let Some(op) = cmp_op(self.peek()) else { return Ok(lhs) };
        self.bump();
        let rhs = self.add_expr()?;
        let e = self.binary(op, lhs, rhs);
        if cmp_op(self.peek()).is_some() {
            self.error_here("comparisons can't be chained; combine them with `and`");
            return Err(());
        }
        Ok(e)
    }

    fn add_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.mul_expr()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinaryOp::Add,
                Tok::Minus => BinaryOp::Sub,
                _ => return Ok(lhs),
            };
            self.bump();
            let rhs = self.mul_expr()?;
            lhs = self.binary(op, lhs, rhs);
        }
    }

    fn mul_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinaryOp::Mul,
                Tok::Slash => BinaryOp::Div,
                Tok::Percent => BinaryOp::Rem,
                _ => return Ok(lhs),
            };
            self.bump();
            let rhs = self.unary()?;
            lhs = self.binary(op, lhs, rhs);
        }
    }

    fn unary(&mut self) -> PResult<Expr> {
        let start = self.span();
        let op = match self.peek() {
            Tok::Minus => UnaryOp::Neg,
            Tok::Not => UnaryOp::Not,
            _ => return self.primary(),
        };
        self.bump();
        let inner = self.unary()?;
        let span = start.to(inner.span);
        Ok(self.expr_node(ExprKind::Unary { op, expr: Box::new(inner) }, span))
    }

    fn primary(&mut self) -> PResult<Expr> {
        let span = self.span();
        let kind = match self.peek().clone() {
            Tok::Int(n) => ExprKind::Int(n),
            Tok::Float(f) => ExprKind::Float(f),
            Tok::Str(s) => ExprKind::Str(s),
            Tok::True => ExprKind::Bool(true),
            Tok::False => ExprKind::Bool(false),
            Tok::Ident(name) => {
                self.bump();
                if self.at(&Tok::LParen) {
                    return self.call(name, span);
                }
                return Ok(self.expr_node(ExprKind::Ident(name), span));
            }
            Tok::LParen => {
                self.bump();
                let inner = self.expr()?;
                self.expect(&Tok::RParen, "`)`")?;
                return Ok(inner);
            }
            Tok::If => return self.if_expr(),
            other => {
                self.error_here(format!("expected an expression, found {}", other.describe()));
                return Err(());
            }
        };
        self.bump();
        Ok(self.expr_node(kind, span))
    }

    fn call(&mut self, name: String, name_span: Span) -> PResult<Expr> {
        self.expect(&Tok::LParen, "`(`")?;
        let mut args = Vec::new();
        while !self.at(&Tok::RParen) {
            args.push(self.arg()?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        let end = self.expect(&Tok::RParen, "`)` or `,`")?;
        Ok(self.expr_node(ExprKind::Call { name, name_span, args }, name_span.to(end)))
    }

    fn arg(&mut self) -> PResult<Arg> {
        let start = self.span();
        let mut label = None;
        if self.peek_at(1) == &Tok::Colon {
            if let Tok::Ident(name) = self.peek().clone() {
                self.bump();
                self.bump();
                label = Some(name);
            }
        }
        let is_var = self.eat(&Tok::Var);
        let value = self.expr()?;
        Ok(Arg { label, is_var, span: start.to(value.span), value })
    }

    pub(crate) fn if_expr(&mut self) -> PResult<Expr> {
        let start = self.expect(&Tok::If, "`if`")?;
        let cond = self.expr()?;
        let then_block = self.block()?;
        let else_block = if self.eat(&Tok::Else) {
            if self.at(&Tok::If) {
                let nested = self.if_expr()?;
                let span = nested.span;
                Some(Block { stmts: vec![Stmt::Expr(nested)], span })
            } else {
                Some(self.block()?)
            }
        } else {
            None
        };
        let end = else_block.as_ref().map_or(then_block.span, |b| b.span);
        let kind = ExprKind::If { cond: Box::new(cond), then_block, else_block };
        Ok(self.expr_node(kind, start.to(end)))
    }
}
```

- [ ] **Step 6: Implement blocks and statements (`parser/stmt.rs`)**

```rust
use super::{PResult, Parser};
use crate::ast::{AssignOp, Block, Stmt};
use crate::token::Tok;

impl<'t> Parser<'t> {
    pub(crate) fn block(&mut self) -> PResult<Block> {
        let start = self.expect(&Tok::LBrace, "`{`")?;
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !self.at(&Tok::RBrace) && !self.at(&Tok::Eof) {
            match self.stmt() {
                Ok(s) => {
                    stmts.push(s);
                    let ended = self.at(&Tok::RBrace) || self.eat(&Tok::Newline) || self.eat(&Tok::Semi);
                    if !ended {
                        let found = self.peek().describe();
                        self.error_here(format!("expected end of line, found {found}"));
                        self.recover_stmt();
                    }
                }
                Err(()) => self.recover_stmt(),
            }
            self.skip_newlines();
        }
        let end = self.expect(&Tok::RBrace, "`}`")?;
        Ok(Block { stmts, span: start.to(end) })
    }

    fn stmt(&mut self) -> PResult<Stmt> {
        let start = self.span();
        match self.peek() {
            Tok::Let | Tok::Var => self.let_stmt(),
            Tok::For => self.for_stmt(),
            Tok::While => {
                self.bump();
                let cond = self.expr()?;
                let body = self.block()?;
                Ok(Stmt::While { span: start.to(body.span), cond, body })
            }
            Tok::Par => {
                self.error_here("`par for` isn't supported yet");
                Err(())
            }
            Tok::Break => {
                self.bump();
                Ok(Stmt::Break(start))
            }
            Tok::Continue => {
                self.bump();
                Ok(Stmt::Continue(start))
            }
            Tok::Return => {
                self.bump();
                let ends = matches!(self.peek(), Tok::Newline | Tok::Semi | Tok::RBrace | Tok::Eof);
                let value = if ends { None } else { Some(self.expr()?) };
                let span = value.as_ref().map_or(start, |v| start.to(v.span));
                Ok(Stmt::Return { value, span })
            }
            _ => self.expr_or_assign(),
        }
    }

    fn let_stmt(&mut self) -> PResult<Stmt> {
        let start = self.span();
        let is_var = self.at(&Tok::Var);
        self.bump();
        let (name, name_span) = self.expect_ident("a variable name")?;
        let bind = self.binding(name, name_span);
        let ty = if self.eat(&Tok::Colon) { Some(self.type_expr()?) } else { None };
        self.expect(&Tok::Assign, "`=`")?;
        let value = self.expr()?;
        Ok(Stmt::Let { is_var, bind, ty, span: start.to(value.span), value })
    }

    fn for_stmt(&mut self) -> PResult<Stmt> {
        let start = self.span();
        self.bump();
        let (name, name_span) = self.expect_ident("a loop variable name")?;
        if self.at(&Tok::In) || self.at(&Tok::Comma) {
            self.error_here("for-in loops aren't supported yet");
            return Err(());
        }
        let var = self.binding(name, name_span);
        self.expect(&Tok::From, "`from`")?;
        let lo = self.expr()?;
        self.expect(&Tok::To, "`to`")?;
        let hi = self.expr()?;
        let body = self.block()?;
        Ok(Stmt::ForRange { var, start: lo, end: hi, span: start.to(body.span), body })
    }

    fn expr_or_assign(&mut self) -> PResult<Stmt> {
        let target = self.expr()?;
        let op = match self.peek() {
            Tok::Assign => AssignOp::Set,
            Tok::PlusEq => AssignOp::Add,
            Tok::MinusEq => AssignOp::Sub,
            Tok::StarEq => AssignOp::Mul,
            Tok::SlashEq => AssignOp::Div,
            Tok::PercentEq => AssignOp::Rem,
            _ => return Ok(Stmt::Expr(target)),
        };
        self.bump();
        let value = self.expr()?;
        Ok(Stmt::Assign { span: target.span.to(value.span), target, op, value })
    }
}
```

- [ ] **Step 7: Implement items and types (`parser/item.rs`)**

```rust
use super::{PResult, Parser};
use crate::ast::{ConstDecl, FnDecl, Item, Param, TypeExpr, TypeKind};
use crate::span::Span;
use crate::token::Tok;

impl<'t> Parser<'t> {
    pub(crate) fn items(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        self.skip_newlines();
        while !self.at(&Tok::Eof) {
            match self.item() {
                Ok(item) => {
                    items.push(item);
                    let ended = self.at(&Tok::Eof) || self.eat(&Tok::Newline) || self.eat(&Tok::Semi);
                    if !ended {
                        let found = self.peek().describe();
                        self.error_here(format!("expected end of line, found {found}"));
                        self.recover_item();
                    }
                }
                Err(()) => self.recover_item(),
            }
            self.skip_newlines();
        }
        items
    }

    fn item(&mut self) -> PResult<Item> {
        let start = self.span();
        let is_pub = self.eat(&Tok::Pub);
        match self.peek().clone() {
            Tok::Let => self.const_decl(is_pub, start),
            Tok::Pure => {
                self.bump();
                self.fn_decl(is_pub, true, start)
            }
            Tok::Ident(name) if name == "fn" => {
                self.error_here("Kestrel functions don't use `fn`: write `name(params) { ... }`");
                Err(())
            }
            Tok::Ident(_) => self.fn_decl(is_pub, false, start),
            Tok::Var => {
                self.error_here("global variables aren't supported yet");
                Err(())
            }
            Tok::Struct | Tok::Enum => {
                self.error_here("`struct` and `enum` aren't supported yet");
                Err(())
            }
            Tok::Use => {
                self.error_here("`use` isn't supported yet");
                Err(())
            }
            other => {
                self.error_here(format!("expected a function or `let`, found {}", other.describe()));
                Err(())
            }
        }
    }

    fn fn_decl(&mut self, is_pub: bool, is_pure: bool, start: Span) -> PResult<Item> {
        let (name, name_span) = self.expect_ident("a function name")?;
        self.expect(&Tok::LParen, "`(`")?;
        let mut params = Vec::new();
        while !self.at(&Tok::RParen) {
            params.push(self.param()?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RParen, "`)` or `,`")?;
        let ret = if self.eat(&Tok::Arrow) { Some(self.type_expr()?) } else { None };
        let body = self.block()?;
        let span = start.to(body.span);
        Ok(Item::Fn(FnDecl { name, name_span, is_pure, is_pub, params, ret, body, span }))
    }

    fn param(&mut self) -> PResult<Param> {
        let is_var = self.eat(&Tok::Var);
        if self.at(&Tok::SelfKw) {
            self.error_here("`self` parameters aren't supported yet");
            return Err(());
        }
        let (name, span) = self.expect_ident("a parameter name")?;
        let bind = self.binding(name, span);
        self.expect(&Tok::Colon, "`:` and a type")?;
        let ty = self.type_expr()?;
        Ok(Param { bind, is_var, ty })
    }

    fn const_decl(&mut self, is_pub: bool, start: Span) -> PResult<Item> {
        self.bump();
        let (name, name_span) = self.expect_ident("a constant name")?;
        let bind = self.binding(name, name_span);
        let ty = if self.eat(&Tok::Colon) { Some(self.type_expr()?) } else { None };
        self.expect(&Tok::Assign, "`=`")?;
        let value = self.expr()?;
        let span = start.to(value.span);
        Ok(Item::Const(ConstDecl { is_pub, bind, ty, value, span }))
    }

    pub(crate) fn type_expr(&mut self) -> PResult<TypeExpr> {
        let start = self.span();
        let mut ty = if self.eat(&Tok::LBracket) {
            let inner = self.type_expr()?;
            let end = self.expect(&Tok::RBracket, "`]`")?;
            TypeExpr { kind: TypeKind::Array(Box::new(inner)), span: start.to(end) }
        } else {
            let (name, span) = self.expect_ident("a type")?;
            let mut args = Vec::new();
            let mut end = span;
            if self.eat(&Tok::LBracket) {
                loop {
                    args.push(self.type_expr()?);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                end = self.expect(&Tok::RBracket, "`]`")?;
            }
            TypeExpr { kind: TypeKind::Named { name, args }, span: start.to(end) }
        };
        if self.at(&Tok::Question) {
            let end = self.bump().span;
            ty = TypeExpr { span: ty.span.to(end), kind: TypeKind::Optional(Box::new(ty)) };
        }
        Ok(ty)
    }
}
```

- [ ] **Step 8: Expose `parse` (`lib.rs`)**

```rust
pub mod ast;
pub mod diag;
pub mod lexer;
mod newlines;
pub mod parser;
pub mod span;
pub mod token;

pub use diag::{render, render_short, Diagnostic, Severity};
pub use span::{LineIndex, Span};

/// Lex and parse one source file. Diagnostics from both stages, in order.
pub fn parse(src: &str) -> (ast::Program, Vec<Diagnostic>) {
    let (toks, mut diags) = lexer::lex(src);
    let (program, parse_diags) = parser::Parser::new(&toks).parse_program();
    diags.extend(parse_diags);
    (program, diags)
}
```

- [ ] **Step 9: Run tests to verify they pass**

Run: `cd kestrel && cargo test -p kestrel-syntax`
Expected: all syntax tests pass (diag, lexer, parser). No warnings about unused code other than none expected; fix any `unused` warning by deleting the unused item.

- [ ] **Step 10: Commit**

```bash
git add kestrel/syntax
git commit -m "Add kestrel parser for functions, statements and expressions

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Checker core: types, signatures, scopes, statements, expressions

**Files:**
- Create: `kestrel/check/Cargo.toml`, `kestrel/check/src/lib.rs`, `kestrel/check/src/ty.rs`, `kestrel/check/src/tables.rs`, `kestrel/check/src/collect.rs`, `kestrel/check/src/body/mod.rs`, `kestrel/check/src/body/expr.rs`, `kestrel/check/src/body/call.rs`
- Modify: `kestrel/Cargo.toml` (add `"check"` to `members`)
- Test: `kestrel/check/tests/check.rs`

**Interfaces:**
- Consumes: `kestrel_syntax::{parse, ast::*, Diagnostic, Span}` (Task 3).
- Produces: `kestrel_check::check(&Program) -> (Option<Tables>, Vec<Diagnostic>)` (`Some` only when there are no errors); `Ty` (`Unit Never Bool Int I64 U8 Float F64 Str Error`) with `from_name`, `name`, `is_int`, `is_float`, `is_numeric`, `coerces_to`, `Ty::common`; `Tables` with fields `expr_ty: Vec<Ty>` (by `ExprId`), `coerce: HashMap<ExprId, Ty>` (implicit conversion target), `res: HashMap<ExprId, Res>`, `bind_local: HashMap<BindId, LocalId>`, `call_args: HashMap<ExprId, Vec<usize>>`, `locals: Vec<LocalInfo>`, `fns: Vec<FnInfo>`, `consts: Vec<ConstInfo>`, `fn_by_name`, `const_by_name`, `main: Option<FnId>`; `Res { Local(LocalId), Const(ConstId), Fn(FnId), Print, Convert(Ty) }`; `LocalInfo { name, ty, mutable, is_param, is_var_param, addressed, owner: FnId, span }`; `FnInfo { name, item, is_pure, params: Vec<ParamSig>, ret, span, locals: Vec<LocalId>, io, mutates_params }` where `locals` lists params first in order; `ParamSig { name, ty, is_var }`; `ConstInfo { name, ty, item }`.
- Calls are **not** handled in this task: any call reports `function calls aren't supported yet`. Task 5 replaces `body/call.rs`.

**Rules implemented here** (spec section 3 steps 1-2):
- Every name resolves to one local, constant or function; unknown names are errors.
- No shadowing: declaring a name already visible (local in any enclosing scope, a constant, or a function) is an error.
- Parameter and return types are required; locals are inferred. Integer literals take the expected type when it's numeric (otherwise `int`); float literals are `float` unless `f64` is expected. Literals out of range are errors.
- Implicit conversions: int→float, narrower→wider int, `float`→`f64`. Each is recorded in `Tables::coerce`.
- `let` locals and non-`var` parameters can't be assigned. Assigning to a `var` parameter marks the function `mutates_params`.
- Blocks are expressions: in a value context the last expression statement is the block's value. A block whose last statement is `return`/`break`/`continue` has type `Never`.
- An expression statement that is a literal, name, unary or binary expression is the error `this line has no effect; did you mean to continue the line above?`.
- Constants may use only literals, operators and constants declared earlier.
- String literals are only allowed as direct `print` arguments (handled in Task 5); anywhere else: `string values aren't supported yet (only as `print` arguments)`.

- [ ] **Step 1: Crate setup**

Add `"check"` to `members` in `kestrel/Cargo.toml`:
```toml
members = ["syntax", "check"]
```

`kestrel/check/Cargo.toml`:
```toml
[package]
name = "kestrel-check"
version.workspace = true
edition.workspace = true

[dependencies]
kestrel-syntax.workspace = true
```

- [ ] **Step 2: Write the failing tests**

`kestrel/check/tests/check.rs`:
```rust
use kestrel_check::{check, Tables, Ty};
use kestrel_syntax::parse;

fn run(src: &str) -> (Option<Tables>, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().map(|d| d.message).collect())
}

fn ok(src: &str) -> Tables {
    let (t, errs) = run(src);
    assert!(errs.is_empty(), "unexpected errors: {errs:?}");
    t.expect("tables")
}

fn errors(src: &str) -> Vec<String> {
    run(src).1
}

fn local_ty(t: &Tables, name: &str) -> Ty {
    t.locals.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("no local {name}")).ty
}

#[test]
fn statements_and_inference() {
    let t = ok("main() {\n var x = 1\n x += 2\n let y: float = x\n while x < 10 { x = x * 2 }\n for i from 0 to 3 { x -= i }\n let z = if x > 3 { 1 } else { 2 }\n}\n");
    assert_eq!(local_ty(&t, "x"), Ty::Int);
    assert_eq!(local_ty(&t, "y"), Ty::Float);
    assert_eq!(local_ty(&t, "z"), Ty::Int);
    assert_eq!(local_ty(&t, "i"), Ty::Int);
    assert!(t.coerce.values().any(|&ty| ty == Ty::Float), "x should be coerced to float");
}

#[test]
fn literals_take_the_expected_type() {
    let t = ok("main() {\n let a: i64 = 5\n let b: u8 = 255\n let c: f64 = 1.5\n let d: float = 3\n}\n");
    assert_eq!(local_ty(&t, "a"), Ty::I64);
    assert_eq!(local_ty(&t, "b"), Ty::U8);
    assert_eq!(local_ty(&t, "c"), Ty::F64);
    assert_eq!(local_ty(&t, "d"), Ty::Float);
}

#[test]
fn constants() {
    let t = ok("let SIZE = 32\nlet AREA = SIZE * SIZE\nmain() {\n let x = AREA\n}\n");
    assert_eq!(t.consts[1].ty, Ty::Int);
    assert_eq!(local_ty(&t, "x"), Ty::Int);
}

#[test]
fn diverging_branches_satisfy_return_type() {
    ok("f(x: int) -> int {\n if x > 0 { return 1 } else { return 2 }\n}\nmain() {}\n");
    ok("f(x: int) -> int {\n if x > 0 { 1 } else { 2 }\n}\nmain() {}\n");
}

#[test]
fn errors_are_reported() {
    let cases: &[(&str, &str)] = &[
        ("f() {}\n", "program needs a `main()` function"),
        ("main() {\n let x = 1\n if true { let x = 2 }\n}\n", "`x` is already defined; Kestrel doesn't allow shadowing"),
        ("main() {\n let x = 1\n x = 2\n}\n", "can't change `x`: it was declared with `let`"),
        ("main() {\n let x: int = true\n}\n", "expected int, found bool"),
        ("main() {\n let x: int = 1.5\n}\n", "expected int, found float"),
        ("main() {\n let x = 1 + true\n}\n", "`+` needs numbers, found int and bool"),
        ("main() {\n while 1 { }\n}\n", "expected bool, found int"),
        ("main() {\n let x = y\n}\n", "unknown name `y`"),
        ("main() {\n let x = 3000000000\n}\n", "`3000000000` doesn't fit in int"),
        ("main() {\n let b: u8 = 256\n}\n", "`256` doesn't fit in u8"),
        ("main() {\n break\n}\n", "`break` outside a loop"),
        ("main() {\n var total = 1\n total = total\n - 2\n}\n", "this line has no effect; did you mean to continue the line above?"),
        ("main() {\n let x = if true { 1 }\n}\n", "this `if` needs an `else` to produce a value"),
        ("f() -> int { }\nmain() {}\n", "`f` must return int, but its body can finish without a value"),
        ("let A = B\nlet B = 1\nmain() {}\n", "unknown name `B`"),
        ("f(x: vec9) {}\nmain() {}\n", "unknown type `vec9`"),
        ("main() {\n let s = \"hi\"\n}\n", "string values aren't supported yet (only as `print` arguments)"),
        ("pure f(var x: int) {}\nmain() {}\n", "pure functions can't take `var` parameters"),
        ("f(x: int) {\n x = 1\n}\nmain() {}\n", "can't change parameter `x`"),
        ("main() {\n let u: u8 = 1\n let n = -u\n}\n", "can't negate a u8"),
        ("main() {}\nmain() {}\n", "`main` is defined twice"),
        ("print() {}\nmain() {}\n", "`print` is a built-in name; pick another"),
    ];
    for (src, expected) in cases {
        let errs = errors(src);
        assert_eq!(errs, vec![expected.to_string()], "source:\n{src}");
    }
}

#[test]
fn assigning_var_param_marks_function() {
    let t = ok("bump(var x: int) {\n x += 1\n}\nmain() {}\n");
    assert!(t.fns[0].mutates_params);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd kestrel && cargo test -p kestrel-check`
Expected: compile errors (crate empty).

- [ ] **Step 4: Implement `ty.rs`**

```rust
/// A checked type. `Never` is the type of code that doesn't finish
/// (`return`, `break`); `Error` suppresses follow-on errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    Unit,
    Never,
    Bool,
    Int,
    I64,
    U8,
    Float,
    F64,
    Str,
    Error,
}

impl Ty {
    pub fn from_name(name: &str) -> Option<Ty> {
        Some(match name {
            "int" => Ty::Int,
            "i64" => Ty::I64,
            "u8" => Ty::U8,
            "float" => Ty::Float,
            "f64" => Ty::F64,
            "bool" => Ty::Bool,
            "string" => Ty::Str,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Ty::Unit => "nothing",
            Ty::Never => "never",
            Ty::Bool => "bool",
            Ty::Int => "int",
            Ty::I64 => "i64",
            Ty::U8 => "u8",
            Ty::Float => "float",
            Ty::F64 => "f64",
            Ty::Str => "string",
            Ty::Error => "{error}",
        }
    }

    pub fn is_int(self) -> bool {
        matches!(self, Ty::Int | Ty::I64 | Ty::U8)
    }

    pub fn is_float(self) -> bool {
        matches!(self, Ty::Float | Ty::F64)
    }

    pub fn is_numeric(self) -> bool {
        self.is_int() || self.is_float()
    }

    /// Whether a value of `self` may be used where `to` is expected without
    /// an explicit conversion: int→float, narrower→wider int, float→f64.
    pub fn coerces_to(self, to: Ty) -> bool {
        use Ty::*;
        self == to
            || matches!(self, Never | Error)
            || to == Error
            || matches!(
                (self, to),
                (U8, Int) | (U8, I64) | (Int, I64) | (U8 | Int | I64, Float | F64) | (Float, F64)
            )
    }

    /// The type two operands meet at, if one converts to the other.
    pub fn common(a: Ty, b: Ty) -> Option<Ty> {
        if a.coerces_to(b) {
            Some(b)
        } else if b.coerces_to(a) {
            Some(a)
        } else {
            None
        }
    }
}
```

- [ ] **Step 5: Implement `tables.rs`**

```rust
use std::collections::HashMap;

use kestrel_syntax::ast::{BindId, ExprId};
use kestrel_syntax::Span;

use crate::ty::Ty;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FnId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConstId(pub u32);

/// What a name or call expression refers to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Res {
    Local(LocalId),
    Const(ConstId),
    Fn(FnId),
    Print,
    Convert(Ty),
}

#[derive(Clone, Debug)]
pub struct LocalInfo {
    pub name: String,
    pub ty: Ty,
    pub mutable: bool,
    pub is_param: bool,
    pub is_var_param: bool,
    /// Passed as a `var` argument somewhere, so it needs a memory slot.
    pub addressed: bool,
    pub owner: FnId,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ParamSig {
    pub name: String,
    pub ty: Ty,
    pub is_var: bool,
}

#[derive(Clone, Debug)]
pub struct FnInfo {
    pub name: String,
    /// Index into `Program::items`.
    pub item: usize,
    pub is_pure: bool,
    pub params: Vec<ParamSig>,
    pub ret: Ty,
    pub span: Span,
    /// Every local owned by this function, parameters first in order.
    pub locals: Vec<LocalId>,
    pub io: bool,
    pub mutates_params: bool,
}

#[derive(Clone, Debug)]
pub struct ConstInfo {
    pub name: String,
    pub ty: Ty,
    pub item: usize,
}

/// Everything the checker learned, keyed by AST ids.
#[derive(Debug, Default)]
pub struct Tables {
    pub expr_ty: Vec<Ty>,
    pub coerce: HashMap<ExprId, Ty>,
    pub res: HashMap<ExprId, Res>,
    pub bind_local: HashMap<BindId, LocalId>,
    /// For a call: entry `i` is the index (into the call's args) of the
    /// argument bound to parameter `i`.
    pub call_args: HashMap<ExprId, Vec<usize>>,
    pub locals: Vec<LocalInfo>,
    pub fns: Vec<FnInfo>,
    pub consts: Vec<ConstInfo>,
    pub fn_by_name: HashMap<String, FnId>,
    pub const_by_name: HashMap<String, ConstId>,
    pub main: Option<FnId>,
}

impl Tables {
    pub fn ty(&self, e: ExprId) -> Ty {
        self.expr_ty[e.0 as usize]
    }

    pub fn local(&self, id: LocalId) -> &LocalInfo {
        &self.locals[id.0 as usize]
    }

    pub fn func(&self, id: FnId) -> &FnInfo {
        &self.fns[id.0 as usize]
    }
}
```

- [ ] **Step 6: Implement `collect.rs`**

```rust
use std::collections::HashSet;

use kestrel_syntax::ast::{Item, Program, TypeExpr, TypeKind};
use kestrel_syntax::{Diagnostic, Span};

use crate::tables::{ConstInfo, FnId, FnInfo, ParamSig, Tables};
use crate::ty::Ty;

const BUILTINS: &[&str] = &["print", "int", "i64", "u8", "float", "f64", "bool", "string"];

pub(crate) fn resolve_type(ty: &TypeExpr, diags: &mut Vec<Diagnostic>) -> Ty {
    let msg = match &ty.kind {
        TypeKind::Named { name, args } if args.is_empty() => match Ty::from_name(name) {
            Some(t) => return t,
            None => format!("unknown type `{name}`"),
        },
        TypeKind::Named { name, .. } => format!("`{name}[...]` types aren't supported yet"),
        TypeKind::Array(_) => "array types aren't supported yet".to_string(),
        TypeKind::Optional(_) => "optional types aren't supported yet".to_string(),
    };
    diags.push(Diagnostic::error(ty.span, msg));
    Ty::Error
}

/// Records every function signature and constant name before bodies are checked.
pub(crate) fn collect(program: &Program, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let mut seen: HashSet<String> = HashSet::new();
    for (item_idx, item) in program.items.iter().enumerate() {
        let (name, span) = match item {
            Item::Fn(f) => (&f.name, f.name_span),
            Item::Const(c) => (&c.bind.name, c.bind.span),
        };
        if BUILTINS.contains(&name.as_str()) {
            diags.push(Diagnostic::error(span, format!("`{name}` is a built-in name; pick another")));
        } else if !seen.insert(name.clone()) {
            diags.push(Diagnostic::error(span, format!("`{name}` is defined twice")));
        }
        match item {
            Item::Fn(f) => {
                let mut params = Vec::new();
                for p in &f.params {
                    if f.is_pure && p.is_var {
                        diags.push(Diagnostic::error(p.bind.span, "pure functions can't take `var` parameters"));
                    }
                    let ty = resolve_type(&p.ty, diags);
                    params.push(ParamSig { name: p.bind.name.clone(), ty, is_var: p.is_var });
                }
                let ret = f.ret.as_ref().map_or(Ty::Unit, |r| resolve_type(r, diags));
                let id = FnId(t.fns.len() as u32);
                if f.name == "main" && t.main.is_none() {
                    if !f.params.is_empty() || f.ret.is_some() {
                        diags.push(Diagnostic::error(f.name_span, "`main` takes no parameters and returns nothing"));
                    }
                    t.main = Some(id);
                }
                t.fn_by_name.entry(f.name.clone()).or_insert(id);
                t.fns.push(FnInfo {
                    name: f.name.clone(),
                    item: item_idx,
                    is_pure: f.is_pure,
                    params,
                    ret,
                    span: f.name_span,
                    locals: Vec::new(),
                    io: false,
                    mutates_params: false,
                });
            }
            Item::Const(c) => {
                t.consts.push(ConstInfo { name: c.bind.name.clone(), ty: Ty::Error, item: item_idx });
            }
        }
    }
    if t.main.is_none() {
        diags.push(Diagnostic::error(Span::new(0, 0), "program needs a `main()` function"));
    }
}
```

- [ ] **Step 7: Implement the body checker core (`body/mod.rs`)**

```rust
mod call;
mod expr;

use std::collections::HashMap;

use kestrel_syntax::ast::{AssignOp, Binding, Block, Expr, ExprKind, Item, Program, Stmt};
use kestrel_syntax::{Diagnostic, Span};

use crate::collect::resolve_type;
use crate::tables::{ConstId, FnId, LocalId, LocalInfo, Res, Tables};
use crate::ty::Ty;

/// What the surrounding code does with a block's or expression's value.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Expect {
    Discard,
    Value(Option<Ty>),
}

pub(crate) struct BodyChecker<'a> {
    t: &'a mut Tables,
    diags: &'a mut Vec<Diagnostic>,
    /// `None` while checking a constant initializer.
    fn_id: Option<FnId>,
    scopes: Vec<HashMap<String, LocalId>>,
    loop_depth: u32,
    ret: Ty,
    is_pure: bool,
}

pub(crate) fn check_consts(program: &Program, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let consts = program.items.iter().filter_map(|it| match it {
        Item::Const(c) => Some(c),
        Item::Fn(_) => None,
    });
    for (i, c) in consts.enumerate() {
        let mut bc = BodyChecker::new(t, diags, None, Ty::Unit, false);
        let want = c.ty.as_ref().map(|ty| resolve_type(ty, bc.diags));
        let ty = if is_const_expr(&c.value) {
            bc.check_value(&c.value, want)
        } else {
            bc.error(c.value.span, "constants can only use literals, operators and earlier constants");
            Ty::Error
        };
        t.consts[i].ty = ty;
        t.const_by_name.insert(c.bind.name.clone(), ConstId(i as u32));
    }
}

fn is_const_expr(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Ident(_) => true,
        ExprKind::Unary { expr, .. } => is_const_expr(expr),
        ExprKind::Binary { lhs, rhs, .. } => is_const_expr(lhs) && is_const_expr(rhs),
        ExprKind::Str(_) | ExprKind::Call { .. } | ExprKind::If { .. } => false,
    }
}

pub(crate) fn check_fn(program: &Program, fn_id: FnId, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let info = &t.fns[fn_id.0 as usize];
    let Item::Fn(decl) = &program.items[info.item] else { unreachable!("FnInfo points at a fn item") };
    let (ret, is_pure, params) = (info.ret, info.is_pure, info.params.clone());
    let mut bc = BodyChecker::new(t, diags, Some(fn_id), ret, is_pure);
    bc.scopes.push(HashMap::new());
    for (p, sig) in decl.params.iter().zip(&params) {
        bc.declare(&p.bind, sig.ty, p.is_var, true, p.is_var);
    }
    let expect = if ret == Ty::Unit { Expect::Discard } else { Expect::Value(Some(ret)) };
    let body = bc.check_block(&decl.body, expect);
    if ret != Ty::Unit && body == Ty::Unit {
        let msg = format!("`{}` must return {}, but its body can finish without a value", decl.name, ret.name());
        bc.diags.push(Diagnostic::error(decl.name_span, msg).with_help("end the body with a value or a `return`"));
    }
}

impl<'a> BodyChecker<'a> {
    fn new(t: &'a mut Tables, diags: &'a mut Vec<Diagnostic>, fn_id: Option<FnId>, ret: Ty, is_pure: bool) -> Self {
        BodyChecker { t, diags, fn_id, scopes: Vec::new(), loop_depth: 0, ret, is_pure }
    }

    fn error(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(Diagnostic::error(span, msg));
    }

    fn error_help(&mut self, span: Span, msg: impl Into<String>, help: impl Into<String>) {
        self.diags.push(Diagnostic::error(span, msg).with_help(help));
    }

    fn lookup_local(&self, name: &str) -> Option<LocalId> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn declare(&mut self, bind: &Binding, ty: Ty, mutable: bool, is_param: bool, is_var_param: bool) -> LocalId {
        let name = &bind.name;
        let taken = self.lookup_local(name).is_some()
            || self.t.const_by_name.contains_key(name)
            || self.t.fn_by_name.contains_key(name);
        if taken {
            self.error_help(
                bind.span,
                format!("`{name}` is already defined; Kestrel doesn't allow shadowing"),
                "pick a different name",
            );
        }
        let owner = self.fn_id.expect("locals only exist inside functions");
        let id = LocalId(self.t.locals.len() as u32);
        self.t.locals.push(LocalInfo {
            name: name.clone(),
            ty,
            mutable,
            is_param,
            is_var_param,
            addressed: false,
            owner,
            span: bind.span,
        });
        self.t.fns[owner.0 as usize].locals.push(id);
        self.t.bind_local.insert(bind.id, id);
        self.scopes.last_mut().expect("a scope is open").insert(name.clone(), id);
        id
    }

    /// Checks `e` against an expected type, recording a conversion or an error.
    pub(crate) fn check_value(&mut self, e: &Expr, want: Option<Ty>) -> Ty {
        let found = self.check_expr(e, want);
        match want {
            Some(w) => {
                self.coerce(e, found, w);
                w
            }
            None => found,
        }
    }

    pub(crate) fn coerce(&mut self, e: &Expr, from: Ty, to: Ty) {
        if from == to || matches!(from, Ty::Never | Ty::Error) || to == Ty::Error {
            return;
        }
        if from.coerces_to(to) {
            self.t.coerce.insert(e.id, to);
            return;
        }
        let msg = format!("expected {}, found {}", to.name(), from.name());
        if from.is_numeric() && to.is_numeric() {
            self.error_help(e.span, msg, format!("convert it explicitly: `{}(...)`", to.name()));
        } else {
            self.error(e.span, msg);
        }
    }

    pub(crate) fn check_block(&mut self, block: &Block, expect: Expect) -> Ty {
        self.scopes.push(HashMap::new());
        let mut ty = Ty::Unit;
        let last = block.stmts.len().saturating_sub(1);
        for (i, stmt) in block.stmts.iter().enumerate() {
            ty = match (stmt, expect) {
                (Stmt::Expr(e), Expect::Value(want)) if i == last => self.check_value(e, want),
                _ => self.check_stmt(stmt),
            };
        }
        self.scopes.pop();
        ty
    }

    /// Checks a statement; returns `Never` when control can't continue past it.
    fn check_stmt(&mut self, s: &Stmt) -> Ty {
        match s {
            Stmt::Let { is_var, bind, ty, value, .. } => {
                let ann = ty.as_ref().map(|t| resolve_type(t, self.diags));
                let mut local_ty = self.check_value(value, ann);
                if matches!(local_ty, Ty::Unit | Ty::Never) {
                    self.error(value.span, "this expression has no value to store");
                    local_ty = Ty::Error;
                }
                self.declare(bind, local_ty, *is_var, false, false);
                Ty::Unit
            }
            Stmt::Assign { target, op, value, .. } => {
                self.check_assign(target, *op, value);
                Ty::Unit
            }
            Stmt::While { cond, body, .. } => {
                self.check_value(cond, Some(Ty::Bool));
                self.loop_depth += 1;
                self.check_block(body, Expect::Discard);
                self.loop_depth -= 1;
                Ty::Unit
            }
            Stmt::ForRange { var, start, end, body, .. } => {
                let st = self.check_expr(start, None);
                let et = self.check_expr(end, if st.is_int() { Some(st) } else { None });
                let var_ty = match Ty::common(st, et) {
                    _ if st == Ty::Error || et == Ty::Error => Ty::Error,
                    Some(t) if t.is_int() => {
                        self.coerce(start, st, t);
                        self.coerce(end, et, t);
                        t
                    }
                    _ => {
                        let msg = format!("`for ... from ... to` needs integers, found {} and {}", st.name(), et.name());
                        self.error(start.span.to(end.span), msg);
                        Ty::Error
                    }
                };
                self.scopes.push(HashMap::new());
                self.declare(var, var_ty, false, false, false);
                self.loop_depth += 1;
                self.check_block(body, Expect::Discard);
                self.loop_depth -= 1;
                self.scopes.pop();
                Ty::Unit
            }
            Stmt::Break(span) | Stmt::Continue(span) => {
                if self.loop_depth == 0 {
                    let word = if matches!(s, Stmt::Break(_)) { "break" } else { "continue" };
                    self.error(*span, format!("`{word}` outside a loop"));
                }
                Ty::Never
            }
            Stmt::Return { value, span } => {
                match (value, self.ret) {
                    (Some(v), Ty::Unit) => {
                        self.check_expr(v, None);
                        self.error(v.span, "this function doesn't return a value");
                    }
                    (Some(v), r) => {
                        self.check_value(v, Some(r));
                    }
                    (None, Ty::Unit) => {}
                    (None, r) => self.error(*span, format!("this function must return {}", r.name())),
                }
                Ty::Never
            }
            Stmt::Expr(e) => self.check_discarded(e),
        }
    }

    fn check_assign(&mut self, target: &Expr, op: AssignOp, value: &Expr) {
        let ExprKind::Ident(name) = &target.kind else {
            self.error(target.span, "only variables can be assigned to");
            self.check_expr(value, None);
            return;
        };
        let Some(local) = self.lookup_local(name) else {
            if self.t.const_by_name.contains_key(name) {
                self.error(target.span, format!("can't change constant `{name}`"));
            } else {
                self.error(target.span, format!("unknown name `{name}`"));
            }
            self.check_expr(value, None);
            return;
        };
        let info = self.t.locals[local.0 as usize].clone();
        self.t.res.insert(target.id, Res::Local(local));
        self.t.expr_ty[target.id.0 as usize] = info.ty;
        if !info.mutable {
            if info.is_param {
                self.error_help(
                    target.span,
                    format!("can't change parameter `{name}`"),
                    format!("declare it as `var {name}` to change the caller's value"),
                );
            } else {
                self.error_help(target.span, format!("can't change `{name}`: it was declared with `let`"), "declare it with `var`");
            }
        } else if info.is_var_param {
            let f = self.fn_id.expect("inside a function");
            self.t.fns[f.0 as usize].mutates_params = true;
        }
        if op != AssignOp::Set && !info.ty.is_numeric() && info.ty != Ty::Error {
            self.error(target.span, format!("`{}` needs a number, found {}", op.symbol(), info.ty.name()));
        }
        self.check_value(value, Some(info.ty));
    }

    /// An expression whose value nobody uses.
    fn check_discarded(&mut self, e: &Expr) -> Ty {
        let ty = match &e.kind {
            ExprKind::If { .. } => self.check_if(e, Expect::Discard),
            ExprKind::Call { .. } => self.check_discarded_call(e),
            _ => {
                let ty = self.check_expr(e, None);
                if ty != Ty::Error {
                    self.error(e.span, "this line has no effect; did you mean to continue the line above?");
                }
                ty
            }
        };
        if ty == Ty::Never {
            Ty::Never
        } else {
            Ty::Unit
        }
    }
}
```

- [ ] **Step 8: Implement expressions (`body/expr.rs`)**

```rust
use kestrel_syntax::ast::{BinaryOp, Expr, ExprKind, UnaryOp};
use kestrel_syntax::Diagnostic;

use super::{BodyChecker, Expect};
use crate::tables::Res;
use crate::ty::Ty;

fn is_value_ty(t: Ty) -> bool {
    !matches!(t, Ty::Unit | Ty::Never | Ty::Error)
}

impl<'a> BodyChecker<'a> {
    /// Types `e`, using `want` only as a hint for literals and branches.
    /// Records the result in `Tables::expr_ty`.
    pub(crate) fn check_expr(&mut self, e: &Expr, want: Option<Ty>) -> Ty {
        let ty = self.expr_inner(e, want);
        self.t.expr_ty[e.id.0 as usize] = ty;
        ty
    }

    fn expr_inner(&mut self, e: &Expr, want: Option<Ty>) -> Ty {
        match &e.kind {
            ExprKind::Int(n) => self.int_literal(e, *n, want),
            ExprKind::Float(_) => {
                if want == Some(Ty::F64) {
                    Ty::F64
                } else {
                    Ty::Float
                }
            }
            ExprKind::Bool(_) => Ty::Bool,
            ExprKind::Str(_) => {
                self.error(e.span, "string values aren't supported yet (only as `print` arguments)");
                Ty::Error
            }
            ExprKind::Ident(name) => self.ident(e, name),
            ExprKind::Unary { op, expr } => self.unary(*op, expr, want),
            ExprKind::Binary { op, lhs, rhs } => self.binary(e, *op, lhs, rhs, want),
            ExprKind::Call { .. } => self.check_call(e, want),
            ExprKind::If { .. } => self.check_if(e, Expect::Value(want)),
        }
    }

    fn int_literal(&mut self, e: &Expr, n: u64, want: Option<Ty>) -> Ty {
        let ty = match want {
            Some(t) if t.is_numeric() => t,
            _ => Ty::Int,
        };
        let max = match ty {
            Ty::U8 => u8::MAX as u64,
            Ty::Int => i32::MAX as u64,
            Ty::I64 => i64::MAX as u64,
            _ => return ty,
        };
        if n > max {
            let mut d = Diagnostic::error(e.span, format!("`{n}` doesn't fit in {}", ty.name()));
            if ty == Ty::Int {
                d = d.with_help("annotate it as i64: `let x: i64 = ...`");
            }
            self.diags.push(d);
            return Ty::Error;
        }
        ty
    }

    fn ident(&mut self, e: &Expr, name: &str) -> Ty {
        if let Some(local) = self.lookup_local(name) {
            self.t.res.insert(e.id, Res::Local(local));
            return self.t.locals[local.0 as usize].ty;
        }
        if let Some(&c) = self.t.const_by_name.get(name) {
            self.t.res.insert(e.id, Res::Const(c));
            return self.t.consts[c.0 as usize].ty;
        }
        if self.t.fn_by_name.contains_key(name) {
            self.error(e.span, format!("`{name}` is a function; call it with `{name}(...)`"));
        } else {
            self.error(e.span, format!("unknown name `{name}`"));
        }
        Ty::Error
    }

    fn unary(&mut self, op: UnaryOp, inner: &Expr, want: Option<Ty>) -> Ty {
        match op {
            UnaryOp::Not => {
                self.check_value(inner, Some(Ty::Bool));
                Ty::Bool
            }
            UnaryOp::Neg => {
                let ty = self.check_expr(inner, want.filter(|t| t.is_numeric()));
                if ty == Ty::U8 {
                    self.error(inner.span, "can't negate a u8");
                    return Ty::Error;
                }
                if !ty.is_numeric() && ty != Ty::Error {
                    self.error(inner.span, format!("`-` needs a number, found {}", ty.name()));
                    return Ty::Error;
                }
                ty
            }
        }
    }

    fn binary(&mut self, e: &Expr, op: BinaryOp, lhs: &Expr, rhs: &Expr, want: Option<Ty>) -> Ty {
        match op {
            BinaryOp::And | BinaryOp::Or => {
                self.check_value(lhs, Some(Ty::Bool));
                self.check_value(rhs, Some(Ty::Bool));
                Ty::Bool
            }
            _ if op.is_comparison() => {
                let lt = self.check_expr(lhs, None);
                let rt = self.check_expr(rhs, Some(lt).filter(|t| t.is_numeric()));
                if lt == Ty::Bool && rt == Ty::Bool && matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
                    return Ty::Bool;
                }
                self.operands(e, op, lhs, lt, rhs, rt);
                Ty::Bool
            }
            _ => {
                let hint = want.filter(|t| t.is_numeric());
                let lt = self.check_expr(lhs, hint);
                let rt = self.check_expr(rhs, Some(lt).filter(|t| t.is_numeric()).or(hint));
                self.operands(e, op, lhs, lt, rhs, rt)
            }
        }
    }

    /// Both operands must be numbers that meet at a common type.
    fn operands(&mut self, e: &Expr, op: BinaryOp, lhs: &Expr, lt: Ty, rhs: &Expr, rt: Ty) -> Ty {
        if lt == Ty::Error || rt == Ty::Error {
            return Ty::Error;
        }
        let sym = op.symbol();
        if !lt.is_numeric() || !rt.is_numeric() {
            self.error(e.span, format!("`{sym}` needs numbers, found {} and {}", lt.name(), rt.name()));
            return Ty::Error;
        }
        match Ty::common(lt, rt) {
            Some(c) => {
                self.coerce(lhs, lt, c);
                self.coerce(rhs, rt, c);
                c
            }
            None => {
                let msg = format!("`{sym}` can't mix {} and {}", lt.name(), rt.name());
                self.error_help(e.span, msg, "convert one side explicitly, e.g. `int(x)` or `u8(x)`");
                Ty::Error
            }
        }
    }

    pub(crate) fn check_if(&mut self, e: &Expr, expect: Expect) -> Ty {
        let ExprKind::If { cond, then_block, else_block } = &e.kind else { unreachable!("check_if on non-if") };
        self.check_value(cond, Some(Ty::Bool));
        match expect {
            Expect::Discard => {
                let t1 = self.check_block(then_block, Expect::Discard);
                let t2 = match else_block {
                    Some(b) => self.check_block(b, Expect::Discard),
                    None => Ty::Unit,
                };
                if t1 == Ty::Never && t2 == Ty::Never {
                    Ty::Never
                } else {
                    Ty::Unit
                }
            }
            Expect::Value(want) => {
                let Some(else_block) = else_block else {
                    self.check_block(then_block, Expect::Discard);
                    self.error(e.span, "this `if` needs an `else` to produce a value");
                    return Ty::Error;
                };
                let t1 = self.check_block(then_block, Expect::Value(want));
                let hint = want.or(if is_value_ty(t1) { Some(t1) } else { None });
                let t2 = self.check_block(else_block, Expect::Value(hint));
                if t1 == Ty::Never && t2 == Ty::Never {
                    Ty::Never
                } else if let Some(w) = want {
                    w
                } else if t1 == Ty::Never {
                    t2
                } else {
                    t1
                }
            }
        }
    }
}
```

- [ ] **Step 9: Temporary call handling (`body/call.rs`)**

Task 5 replaces this file completely.

```rust
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
```

- [ ] **Step 10: Wire up `lib.rs`**

```rust
mod body;
mod collect;
pub mod tables;
pub mod ty;

use kestrel_syntax::ast::Program;
use kestrel_syntax::Diagnostic;

pub use tables::{ConstId, ConstInfo, FnId, FnInfo, LocalId, LocalInfo, ParamSig, Res, Tables};
pub use ty::Ty;

/// Resolves names and checks types. Tables are returned only when there
/// are no errors; warnings may still be present.
pub fn check(program: &Program) -> (Option<Tables>, Vec<Diagnostic>) {
    let mut t = Tables { expr_ty: vec![Ty::Error; program.expr_count as usize], ..Tables::default() };
    let mut diags = Vec::new();
    collect::collect(program, &mut t, &mut diags);
    body::check_consts(program, &mut t, &mut diags);
    for f in 0..t.fns.len() {
        body::check_fn(program, FnId(f as u32), &mut t, &mut diags);
    }
    let ok = !diags.iter().any(|d| d.is_error());
    (ok.then_some(t), diags)
}
```

- [ ] **Step 11: Run tests to verify they pass**

Run: `cd kestrel && cargo test -p kestrel-check`
Expected: all 6 tests pass. `is_pure` is unused until Task 5; if the compiler warns about it, leave it (Task 5 uses it).

- [ ] **Step 12: Commit**

```bash
git add kestrel/Cargo.toml kestrel/check
git commit -m "Add kestrel checker: scopes, types, statements, expressions

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Checker calls: arguments, builtins, `var`, exclusivity, purity

**Files:**
- Modify: `kestrel/check/src/body/call.rs` (replace the whole file)
- Test: `kestrel/check/tests/check.rs` (append)

**Interfaces:**
- Consumes: `BodyChecker` internals from Task 4: fields `t`, `fn_id`, `is_pure`; methods `check_expr`, `check_value`, `error`, `error_help`.
- Produces: for every call expression, `Tables::res[call.id]` is `Res::Fn(id)`, `Res::Print` or `Res::Convert(ty)`; for user calls without errors, `Tables::call_args[call.id]` maps parameter index → argument index. A local passed as a `var` argument gets `LocalInfo::addressed = true` (unless it is itself a `var` parameter, which is already a pointer). `FnInfo::io` is set by `print`. Same two `pub(crate)` methods as the temporary file: `check_call(&mut self, e: &Expr, want: Option<Ty>) -> Ty` and `check_discarded_call(&mut self, e: &Expr) -> Ty`.

**Rules:**
- `print(...)`: any number of arguments of any value type; string literals allowed only here. Not allowed in `pure` functions.
- `int(x) i64(x) u8(x) float(x) f64(x)`: exactly one numeric argument.
- User calls: positional arguments first, then labelled ones in any order; every parameter bound exactly once.
- A `var` parameter needs a `var`-marked argument that is a changeable variable of exactly the parameter's type (no conversion through a reference). A non-`var` parameter must not get a `var` argument.
- Exclusivity: a variable passed as `var` may not appear as another argument of the same call.
- A `pure` function may only call `pure` functions.
- Calling a `pure` function and ignoring the result is an error (it does nothing).

- [ ] **Step 1: Write the failing tests**

Append to `kestrel/check/tests/check.rs`:
```rust
use kestrel_check::Res;

#[test]
fn calls_resolve_and_mark_var_arguments() {
    let t = ok("pure square(x: int) -> int { x * x }\nbump(var n: int, by: int) {\n n += by\n}\nmain() {\n var total = 0\n bump(var total, by: square(3))\n bump(var total, 2)\n print(\"total\", total, 1.5, true)\n let f = float(total) / 2.0\n let i = int(f)\n}\n");
    let total = t.locals.iter().find(|l| l.name == "total").unwrap();
    assert!(total.addressed);
    assert!(t.fns[1].mutates_params);
    assert!(t.fns[2].io);
    assert_eq!(t.call_args.len(), 3);
    assert!(t.res.values().any(|r| *r == Res::Print));
    assert!(t.res.values().any(|r| *r == Res::Convert(Ty::Int)));
    assert_eq!(local_ty(&t, "f"), Ty::Float);
    assert_eq!(local_ty(&t, "i"), Ty::Int);
}

#[test]
fn labels_reorder_arguments() {
    let t = ok("f(a: int, b: int) -> int { a - b }\nmain() {\n let x = f(b: 1, a: 2)\n}\n");
    assert_eq!(t.call_args.values().next().unwrap(), &vec![1, 0]);
}

#[test]
fn call_errors() {
    let cases: &[(&str, &str)] = &[
        ("pure f() -> int {\n print(1)\n 1\n}\nmain() {}\n", "pure functions can't call `print`"),
        ("g() {}\npure f() {\n g()\n}\nmain() {}\n", "pure functions can only call pure functions; `g` isn't pure"),
        ("f(var x: int) {}\nmain() {\n var a = 1\n f(a)\n}\n", "parameter `x` of `f` is `var`, so the call must say so"),
        ("f(x: int) {}\nmain() {\n var a = 1\n f(var a)\n}\n", "parameter `x` of `f` isn't `var`"),
        ("f(var x: int) {}\nmain() {\n let a = 1\n f(var a)\n}\n", "can't pass `a` as `var`: it was declared with `let`"),
        ("f(var x: int, y: int) {}\nmain() {\n var a = 1\n f(var a, a)\n}\n", "`a` is passed as `var` and also used in another argument of the same call"),
        ("f(x: int) {}\nmain() {\n f(1, 2)\n}\n", "`f` takes 1 argument(s), got 2"),
        ("f(x: int) {}\nmain() {\n f()\n}\n", "missing argument `x` for `f`"),
        ("f(x: int) {}\nmain() {\n f(y: 1)\n}\n", "`f` has no parameter named `y`"),
        ("f(a: int, b: int) {}\nmain() {\n f(b: 1, 2)\n}\n", "positional arguments must come before labelled ones"),
        ("main() {\n g()\n}\n", "unknown function `g`"),
        ("pure sq(x: int) -> int { x * x }\nmain() {\n sq(2)\n}\n", "the result of pure function `sq` is unused, so this line does nothing"),
        ("f(var x: i64) {}\nmain() {\n var a = 1\n f(var a)\n}\n", "`var` parameter `x` needs exactly i64, found int"),
        ("main() {\n let x = int(true)\n}\n", "can't convert bool to int"),
        ("main() {\n let x = int(1, 2)\n}\n", "`int(...)` takes exactly one value"),
        ("f(x: int) {}\nmain() {\n f(1.5)\n}\n", "expected int, found float"),
    ];
    for (src, expected) in cases {
        assert_eq!(errors(src), vec![expected.to_string()], "source:\n{src}");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd kestrel && cargo test -p kestrel-check`
Expected: the three new tests fail (every call reports `function calls aren't supported yet`); Task 4's tests still pass.

- [ ] **Step 3: Replace `body/call.rs`**

```rust
use kestrel_syntax::ast::{Arg, Expr, ExprKind};
use kestrel_syntax::Span;

use super::BodyChecker;
use crate::tables::{FnId, LocalId, ParamSig, Res};
use crate::ty::Ty;

const CONVERSIONS: &[(&str, Ty)] = &[
    ("int", Ty::Int),
    ("i64", Ty::I64),
    ("u8", Ty::U8),
    ("float", Ty::Float),
    ("f64", Ty::F64),
];

impl<'a> BodyChecker<'a> {
    pub(crate) fn check_call(&mut self, e: &Expr, _want: Option<Ty>) -> Ty {
        let ExprKind::Call { name, name_span, args } = &e.kind else { unreachable!("check_call on non-call") };
        if name == "print" {
            return self.check_print(e, args);
        }
        if let Some(&(_, to)) = CONVERSIONS.iter().find(|(n, _)| n == name) {
            return self.check_convert(e, to, args);
        }
        let Some(&fid) = self.t.fn_by_name.get(name) else {
            self.error(*name_span, format!("unknown function `{name}`"));
            for a in args {
                self.check_expr(&a.value, None);
            }
            return Ty::Error;
        };
        self.t.res.insert(e.id, Res::Fn(fid));
        self.check_callee_purity(*name_span, fid);
        self.check_user_call(e, fid, args)
    }

    /// A call statement whose result is thrown away.
    pub(crate) fn check_discarded_call(&mut self, e: &Expr) -> Ty {
        let ty = self.check_call(e, None);
        match self.t.res.get(&e.id).copied() {
            Some(Res::Fn(f)) if self.t.fns[f.0 as usize].is_pure => {
                let name = self.t.fns[f.0 as usize].name.clone();
                self.error(e.span, format!("the result of pure function `{name}` is unused, so this line does nothing"));
            }
            Some(Res::Convert(_)) => {
                self.error(e.span, "this line has no effect; did you mean to continue the line above?");
            }
            _ => {}
        }
        ty
    }

    fn check_print(&mut self, e: &Expr, args: &[Arg]) -> Ty {
        self.t.res.insert(e.id, Res::Print);
        if self.is_pure {
            self.error(e.span, "pure functions can't call `print`");
        }
        if let Some(f) = self.fn_id {
            self.t.fns[f.0 as usize].io = true;
        }
        for a in args {
            if a.label.is_some() || a.is_var {
                self.error(a.span, "`print` arguments can't be labelled or `var`");
            }
            if let ExprKind::Str(_) = a.value.kind {
                self.t.expr_ty[a.value.id.0 as usize] = Ty::Str;
                continue;
            }
            let ty = self.check_expr(&a.value, None);
            if ty == Ty::Unit {
                self.error(a.value.span, "this has no value to print");
            }
        }
        Ty::Unit
    }

    fn check_convert(&mut self, e: &Expr, to: Ty, args: &[Arg]) -> Ty {
        self.t.res.insert(e.id, Res::Convert(to));
        if args.len() != 1 || args[0].label.is_some() || args[0].is_var {
            self.error(e.span, format!("`{}(...)` takes exactly one value", to.name()));
            for a in args {
                self.check_expr(&a.value, None);
            }
            return to;
        }
        let from = self.check_expr(&args[0].value, Some(to));
        if !from.is_numeric() && from != Ty::Error {
            self.error(args[0].value.span, format!("can't convert {} to {}", from.name(), to.name()));
        }
        to
    }

    fn check_callee_purity(&mut self, span: Span, callee: FnId) {
        let info = &self.t.fns[callee.0 as usize];
        if self.is_pure && !info.is_pure {
            let name = info.name.clone();
            self.error_help(
                span,
                format!("pure functions can only call pure functions; `{name}` isn't pure"),
                format!("mark `{name}` as `pure` if it has no effects"),
            );
        }
    }

    fn check_user_call(&mut self, e: &Expr, fid: FnId, args: &[Arg]) -> Ty {
        let info = &self.t.fns[fid.0 as usize];
        let (params, ret, fname) = (info.params.clone(), info.ret, info.name.clone());
        let (slots, mut ok) = self.bind_args(&fname, &params, args);
        if ok {
            for (pi, slot) in slots.iter().enumerate() {
                if slot.is_none() {
                    self.error(e.span, format!("missing argument `{}` for `{fname}`", params[pi].name));
                    ok = false;
                }
            }
        }
        let mut var_args: Vec<(LocalId, usize)> = Vec::new();
        for (pi, slot) in slots.iter().enumerate() {
            let Some(ai) = *slot else { continue };
            let (a, p) = (&args[ai], &params[pi]);
            if p.is_var {
                if let Some(l) = self.check_var_arg(a, p, &fname) {
                    var_args.push((l, ai));
                }
            } else {
                if a.is_var {
                    self.error_help(a.span, format!("parameter `{}` of `{fname}` isn't `var`", p.name), "remove `var`");
                }
                self.check_value(&a.value, Some(p.ty));
            }
        }
        for (ai, a) in args.iter().enumerate() {
            if !slots.contains(&Some(ai)) {
                self.check_expr(&a.value, None);
            }
        }
        self.check_exclusive(args, &var_args);
        if ok {
            let order = slots.into_iter().map(|s| s.expect("all bound")).collect();
            self.t.call_args.insert(e.id, order);
        }
        ret
    }

    /// Matches arguments to parameters: positional first, then labels.
    fn bind_args(&mut self, fname: &str, params: &[ParamSig], args: &[Arg]) -> (Vec<Option<usize>>, bool) {
        let mut slots: Vec<Option<usize>> = vec![None; params.len()];
        let mut ok = true;
        let mut seen_label = false;
        for (ai, a) in args.iter().enumerate() {
            let pi = match &a.label {
                None if seen_label => {
                    self.error(a.span, "positional arguments must come before labelled ones");
                    ok = false;
                    continue;
                }
                None if ai >= params.len() => {
                    self.error(a.span, format!("`{fname}` takes {} argument(s), got {}", params.len(), args.len()));
                    ok = false;
                    break;
                }
                None => ai,
                Some(label) => {
                    seen_label = true;
                    match params.iter().position(|p| &p.name == label) {
                        Some(pi) => pi,
                        None => {
                            self.error(a.span, format!("`{fname}` has no parameter named `{label}`"));
                            ok = false;
                            continue;
                        }
                    }
                }
            };
            if slots[pi].is_some() {
                self.error(a.span, format!("parameter `{}` is given twice", params[pi].name));
                ok = false;
                continue;
            }
            slots[pi] = Some(ai);
        }
        (slots, ok)
    }

    /// Checks an argument for a `var` parameter; returns the variable passed.
    fn check_var_arg(&mut self, a: &Arg, p: &ParamSig, fname: &str) -> Option<LocalId> {
        let ExprKind::Ident(name) = &a.value.kind else {
            self.error(a.value.span, "only a variable can be passed as `var`");
            self.check_expr(&a.value, None);
            return None;
        };
        if !a.is_var {
            self.error_help(
                a.span,
                format!("parameter `{}` of `{fname}` is `var`, so the call must say so", p.name),
                format!("write `var {name}`"),
            );
        }
        let ty = self.check_expr(&a.value, None);
        let Some(Res::Local(l)) = self.t.res.get(&a.value.id).copied() else {
            if ty != Ty::Error {
                self.error(a.value.span, "only a variable can be passed as `var`");
            }
            return None;
        };
        let info = self.t.locals[l.0 as usize].clone();
        if !info.mutable {
            self.error_help(
                a.value.span,
                format!("can't pass `{name}` as `var`: it was declared with `let`"),
                "declare it with `var`",
            );
        }
        if ty != p.ty && ty != Ty::Error && p.ty != Ty::Error {
            let msg = format!("`var` parameter `{}` needs exactly {}, found {}", p.name, p.ty.name(), ty.name());
            self.error(a.value.span, msg);
        }
        if info.is_var_param {
            let f = self.fn_id.expect("inside a function");
            self.t.fns[f.0 as usize].mutates_params = true;
        } else {
            self.t.locals[l.0 as usize].addressed = true;
        }
        Some(l)
    }

    /// A variable passed as `var` can't also be another argument of the same call.
    fn check_exclusive(&mut self, args: &[Arg], var_args: &[(LocalId, usize)]) {
        for &(l, ai) in var_args {
            for (bi, b) in args.iter().enumerate() {
                let reported_from_other_side = var_args.iter().any(|&(_, x)| x == bi) && bi < ai;
                if bi == ai || reported_from_other_side {
                    continue;
                }
                if self.t.res.get(&b.value.id) == Some(&Res::Local(l)) {
                    let name = self.t.locals[l.0 as usize].name.clone();
                    self.error(b.span, format!("`{name}` is passed as `var` and also used in another argument of the same call"));
                }
            }
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd kestrel && cargo test -p kestrel-check`
Expected: all check tests pass, no warnings.

- [ ] **Step 5: Commit**

```bash
git add kestrel/check
git commit -m "Check calls: labels, var arguments, exclusivity, purity, builtins

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: KIR and lowering

**Files:**
- Create: `kestrel/kir/Cargo.toml`, `kestrel/kir/src/lib.rs`, `kestrel/kir/src/ir.rs`, `kestrel/kir/src/lower/mod.rs`, `kestrel/kir/src/lower/expr.rs`
- Modify: `kestrel/Cargo.toml` (add `"kir"` to `members`)
- Test: `kestrel/kir/tests/lower.rs`

**Interfaces:**
- Consumes: `Program`/AST (Task 3); `Tables`, `Res`, `Ty`, `FnId`, `LocalId` (Tasks 4-5); `LineIndex::line` (Task 1).
- Produces: `kestrel_kir::lower(&Program, &Tables, &LineIndex) -> Module` and the IR types in `ir.rs` below, exactly as written (codegen in Task 8 matches on them). Invariants codegen may rely on:
  - `FuncIdx(i)` == `FnId(i)`; `Module::main` is the `main` function.
  - In each `Func`, `params` are `LocalIdx(0..n)` and `locals[..n]` are the parameters.
  - Every `Expr::ty` is the value's runtime type; implicit conversions are explicit `Cast` nodes; an integer literal used as a float is already a `Float` literal.
  - A function with a return type ends its body with `Stmt::Return(Some(_))` on every path that finishes (the checker guarantees it); a value-returning `If` branch whose block diverges has `then_val`/`else_val` = `None`.
  - Comparisons have `ty == Bool`; their operand type is `lhs.ty` (both operands have the same type).
  - Compound assignment `x += v` is `Assign(x, Binary(Add, Local(x), v))`.
  - Arguments of a call are in **parameter order**. Arguments are evaluated in that order (a known 2a limitation when labels reorder side-effecting arguments; tracked for 2b).

- [ ] **Step 1: Crate setup**

`kestrel/Cargo.toml` members:
```toml
members = ["syntax", "check", "kir"]
```

`kestrel/kir/Cargo.toml`:
```toml
[package]
name = "kestrel-kir"
version.workspace = true
edition.workspace = true

[dependencies]
kestrel-syntax.workspace = true
kestrel-check.workspace = true
```

- [ ] **Step 2: Write the IR (`ir.rs`)**

```rust
//! Kestrel's typed, structured middle layer. Loops stay loops; no
//! Cranelift (or any backend) types appear here.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KTy {
    Bool,
    U8,
    I32,
    I64,
    F32,
    F64,
}

impl KTy {
    pub fn is_float(self) -> bool {
        matches!(self, KTy::F32 | KTy::F64)
    }

    pub fn is_int(self) -> bool {
        matches!(self, KTy::U8 | KTy::I32 | KTy::I64)
    }

    pub fn is_signed(self) -> bool {
        matches!(self, KTy::I32 | KTy::I64)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FuncIdx(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalIdx(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    pub funcs: Vec<Func>,
    pub main: FuncIdx,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Func {
    pub name: String,
    pub params: Vec<LocalIdx>,
    pub ret: Option<KTy>,
    pub locals: Vec<Local>,
    pub body: Vec<Stmt>,
    pub is_pure: bool,
    pub io: bool,
    pub mutates_params: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Local {
    pub name: String,
    pub ty: KTy,
    pub kind: LocalKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalKind {
    /// Lives in a register.
    Value,
    /// Passed as a `var` argument somewhere, so it needs a memory slot.
    Addressed,
    /// A `var` parameter: holds a pointer to the caller's value.
    RefParam,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Let(LocalIdx, Expr),
    Assign(LocalIdx, Expr),
    If { cond: Expr, then_body: Vec<Stmt>, else_body: Vec<Stmt> },
    While { cond: Expr, body: Vec<Stmt> },
    /// `for var from start to end`: `end` exclusive, step +1, both
    /// evaluated once before the first iteration.
    ForRange { var: LocalIdx, start: Expr, end: Expr, body: Vec<Stmt> },
    Break,
    Continue,
    Return(Option<Expr>),
    /// A call whose result (if any) is discarded.
    Call(FuncIdx, Vec<Arg>),
    Print(Vec<PrintArg>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum PrintArg {
    Value(Expr),
    Str(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: KTy,
    /// Source line, for runtime error messages.
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    Local(LocalIdx),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// Short-circuiting.
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    /// Convert the operand to this expression's `ty`.
    Cast(Box<Expr>),
    Call(FuncIdx, Vec<Arg>),
    If {
        cond: Box<Expr>,
        then_body: Vec<Stmt>,
        then_val: Option<Box<Expr>>,
        else_body: Vec<Stmt>,
        else_val: Option<Box<Expr>>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Value(Expr),
    /// Pass a pointer to this local (for a `var` parameter).
    Ref(LocalIdx),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

impl BinOp {
    pub fn is_comparison(self) -> bool {
        matches!(self, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge)
    }
}
```

- [ ] **Step 3: Write the failing tests**

`kestrel/kir/tests/lower.rs`:
```rust
use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn lower_src(src: &str) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    lower(&p, &t.unwrap(), &LineIndex::new(src))
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap()
}

#[test]
fn tail_expression_becomes_return() {
    let m = lower_src("pure sq(x: int) -> int { x * x }\nmain() {}\n");
    let sq = func(&m, "sq");
    assert_eq!(sq.ret, Some(KTy::I32));
    assert_eq!(sq.params, vec![LocalIdx(0)]);
    assert_eq!(sq.locals[0].kind, LocalKind::Value);
    assert!(matches!(
        &sq.body[..],
        [Stmt::Return(Some(Expr { kind: ExprKind::Binary(BinOp::Mul, _, _), ty: KTy::I32, .. }))]
    ));
    assert_eq!(m.main, FuncIdx(1));
}

#[test]
fn var_params_and_addressed_locals() {
    let m = lower_src("bump(var n: int) {\n n += 1\n}\nmain() {\n var t = 0\n bump(var t)\n}\n");
    let bump = func(&m, "bump");
    assert_eq!(bump.locals[0].kind, LocalKind::RefParam);
    match &bump.body[0] {
        Stmt::Assign(LocalIdx(0), Expr { kind: ExprKind::Binary(BinOp::Add, l, r), .. }) => {
            assert!(matches!(l.kind, ExprKind::Local(LocalIdx(0))));
            assert!(matches!(r.kind, ExprKind::Int(1)));
        }
        other => panic!("{other:?}"),
    }
    let main = func(&m, "main");
    assert_eq!(main.locals[0].kind, LocalKind::Addressed);
    assert!(matches!(&main.body[1], Stmt::Call(FuncIdx(0), args) if args == &vec![Arg::Ref(LocalIdx(0))]));
}

#[test]
fn coercions_become_casts_and_literals_adapt() {
    let m = lower_src("main() {\n let a = 1\n let b: f64 = a\n let c: float = 3\n}\n");
    let main = func(&m, "main");
    match &main.body[1] {
        Stmt::Let(LocalIdx(1), Expr { kind: ExprKind::Cast(inner), ty: KTy::F64, .. }) => {
            assert!(matches!(inner.kind, ExprKind::Local(LocalIdx(0))));
            assert_eq!(inner.ty, KTy::I32);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(&main.body[2], Stmt::Let(_, Expr { kind: ExprKind::Float(f), ty: KTy::F32, .. }) if *f == 3.0));
}

#[test]
fn constants_are_inlined() {
    let m = lower_src("let N = 4\nmain() {\n let x = N * 2\n}\n");
    match &func(&m, "main").body[0] {
        Stmt::Let(_, Expr { kind: ExprKind::Binary(BinOp::Mul, l, r), .. }) => {
            assert!(matches!(l.kind, ExprKind::Int(4)));
            assert!(matches!(r.kind, ExprKind::Int(2)));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn if_expression_and_print() {
    let m = lower_src("main() {\n let x = if true { 1 } else { 2 }\n print(\"x is\", x)\n}\n");
    let main = func(&m, "main");
    assert!(matches!(&main.body[0], Stmt::Let(_, Expr { kind: ExprKind::If { then_val: Some(_), else_val: Some(_), .. }, .. })));
    match &main.body[1] {
        Stmt::Print(args) => {
            assert_eq!(args[0], PrintArg::Str("x is".into()));
            assert!(matches!(&args[1], PrintArg::Value(Expr { kind: ExprKind::Local(LocalIdx(0)), .. })));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn labelled_arguments_are_in_parameter_order() {
    let m = lower_src("f(a: int, b: int) -> int { a - b }\nmain() {\n let x = f(b: 1, a: 2)\n}\n");
    match &func(&m, "main").body[0] {
        Stmt::Let(_, Expr { kind: ExprKind::Call(FuncIdx(0), args), .. }) => {
            assert!(matches!(&args[0], Arg::Value(Expr { kind: ExprKind::Int(2), .. })));
            assert!(matches!(&args[1], Arg::Value(Expr { kind: ExprKind::Int(1), .. })));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn diverging_tail_adds_no_return() {
    let m = lower_src("f(x: int) -> int {\n if x > 0 { return 1 } else { return 2 }\n}\nmain() {}\n");
    assert!(matches!(&func(&m, "f").body[..], [Stmt::If { .. }]));
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cd kestrel && cargo test -p kestrel-kir`
Expected: compile error (`lower` missing).

- [ ] **Step 5: Implement `lower/mod.rs`**

```rust
mod expr;

use std::collections::HashMap;

use kestrel_check::{FnId, LocalId, Res, Tables, Ty};
use kestrel_syntax::ast::{self, Program};
use kestrel_syntax::{LineIndex, Span};

use crate::ir::*;

/// Lowers a checked program. `t` must come from a `check` with no errors.
pub fn lower(program: &Program, t: &Tables, lines: &LineIndex) -> Module {
    let funcs = (0..t.fns.len())
        .map(|i| FnLowerer { program, t, lines, fn_id: FnId(i as u32), local_map: HashMap::new() }.lower())
        .collect();
    Module { funcs, main: FuncIdx(t.main.expect("checked program has main").0) }
}

pub(crate) fn kty(ty: Ty) -> KTy {
    match ty {
        Ty::Bool => KTy::Bool,
        Ty::U8 => KTy::U8,
        Ty::Int => KTy::I32,
        Ty::I64 => KTy::I64,
        Ty::Float => KTy::F32,
        Ty::F64 => KTy::F64,
        other => panic!("type {other:?} has no runtime representation"),
    }
}

pub(crate) struct FnLowerer<'a> {
    program: &'a Program,
    t: &'a Tables,
    lines: &'a LineIndex,
    fn_id: FnId,
    local_map: HashMap<LocalId, LocalIdx>,
}

impl<'a> FnLowerer<'a> {
    fn lower(mut self) -> Func {
        // Copy the shared references out so borrows of them don't hold `self`.
        let (t, program) = (self.t, self.program);
        let info = t.func(self.fn_id);
        let ast::Item::Fn(decl) = &program.items[info.item] else { unreachable!("FnInfo points at a fn") };
        let mut locals = Vec::new();
        for (i, &lid) in info.locals.iter().enumerate() {
            let l = t.local(lid);
            let kind = if l.is_var_param {
                LocalKind::RefParam
            } else if l.addressed {
                LocalKind::Addressed
            } else {
                LocalKind::Value
            };
            self.local_map.insert(lid, LocalIdx(i as u32));
            locals.push(Local { name: l.name.clone(), ty: kty(l.ty), kind });
        }
        let params = (0..info.params.len()).map(|i| LocalIdx(i as u32)).collect();
        let ret = if info.ret == Ty::Unit { None } else { Some(kty(info.ret)) };
        let body = if ret.is_some() {
            let (mut stmts, tail) = self.block_value(&decl.body);
            if let Some(v) = tail {
                stmts.push(Stmt::Return(Some(v)));
            }
            stmts
        } else {
            self.block_stmts(&decl.body)
        };
        Func {
            name: info.name.clone(),
            params,
            ret,
            locals,
            body,
            is_pure: info.is_pure,
            io: info.io,
            mutates_params: info.mutates_params,
        }
    }

    pub(crate) fn line(&self, span: Span) -> u32 {
        self.lines.line(span.start)
    }

    fn bind_local(&self, bind: &ast::Binding) -> LocalIdx {
        self.local_map[&self.t.bind_local[&bind.id]]
    }

    pub(crate) fn block_stmts(&mut self, block: &ast::Block) -> Vec<Stmt> {
        block.stmts.iter().map(|s| self.stmt(s)).collect()
    }

    /// A block used as a value: its statements and its tail value, which is
    /// `None` when the block can't finish normally.
    pub(crate) fn block_value(&mut self, block: &ast::Block) -> (Vec<Stmt>, Option<Expr>) {
        let Some((last, init)) = block.stmts.split_last() else { return (Vec::new(), None) };
        let mut stmts: Vec<Stmt> = init.iter().map(|s| self.stmt(s)).collect();
        match last {
            ast::Stmt::Expr(e) if self.t.ty(e.id) != Ty::Never => {
                let v = self.expr(e);
                (stmts, Some(v))
            }
            other => {
                stmts.push(self.stmt(other));
                (stmts, None)
            }
        }
    }

    fn stmt(&mut self, s: &ast::Stmt) -> Stmt {
        match s {
            ast::Stmt::Let { bind, value, .. } => {
                let idx = self.bind_local(bind);
                Stmt::Let(idx, self.expr(value))
            }
            ast::Stmt::Assign { target, op, value, .. } => {
                let Some(Res::Local(lid)) = self.t.res.get(&target.id).copied() else {
                    unreachable!("checker resolved the assignment target")
                };
                let idx = self.local_map[&lid];
                let rhs = self.expr(value);
                let value = match op.binary() {
                    None => rhs,
                    Some(bop) => {
                        let (ty, line) = (rhs.ty, rhs.line);
                        let cur = Expr { kind: ExprKind::Local(idx), ty, line };
                        Expr { kind: ExprKind::Binary(binop(bop), Box::new(cur), Box::new(rhs)), ty, line }
                    }
                };
                Stmt::Assign(idx, value)
            }
            ast::Stmt::While { cond, body, .. } => Stmt::While { cond: self.expr(cond), body: self.block_stmts(body) },
            ast::Stmt::ForRange { var, start, end, body, .. } => Stmt::ForRange {
                var: self.bind_local(var),
                start: self.expr(start),
                end: self.expr(end),
                body: self.block_stmts(body),
            },
            ast::Stmt::Break(_) => Stmt::Break,
            ast::Stmt::Continue(_) => Stmt::Continue,
            ast::Stmt::Return { value, .. } => Stmt::Return(value.as_ref().map(|v| self.expr(v))),
            ast::Stmt::Expr(e) => self.expr_stmt(e),
        }
    }

    /// An expression whose value is discarded.
    fn expr_stmt(&mut self, e: &ast::Expr) -> Stmt {
        match &e.kind {
            ast::ExprKind::If { cond, then_block, else_block } => {
                let cond = self.expr(cond);
                let then_body = self.block_stmts(then_block);
                let else_body = match else_block {
                    Some(b) => self.block_stmts(b),
                    None => Vec::new(),
                };
                Stmt::If { cond, then_body, else_body }
            }
            ast::ExprKind::Call { args, .. } => match self.t.res[&e.id].clone() {
                Res::Print => Stmt::Print(args.iter().map(|a| self.print_arg(&a.value)).collect()),
                Res::Fn(f) => {
                    let (fi, args) = self.call_parts(e, f, args);
                    Stmt::Call(fi, args)
                }
                other => unreachable!("checker rejects a discarded {other:?}"),
            },
            _ => unreachable!("checker rejects statements with no effect"),
        }
    }

    fn print_arg(&mut self, e: &ast::Expr) -> PrintArg {
        match &e.kind {
            ast::ExprKind::Str(s) => PrintArg::Str(s.clone()),
            _ => PrintArg::Value(self.expr(e)),
        }
    }
}

pub(crate) fn binop(op: ast::BinaryOp) -> BinOp {
    match op {
        ast::BinaryOp::Add => BinOp::Add,
        ast::BinaryOp::Sub => BinOp::Sub,
        ast::BinaryOp::Mul => BinOp::Mul,
        ast::BinaryOp::Div => BinOp::Div,
        ast::BinaryOp::Rem => BinOp::Rem,
        ast::BinaryOp::Eq => BinOp::Eq,
        ast::BinaryOp::Ne => BinOp::Ne,
        ast::BinaryOp::Lt => BinOp::Lt,
        ast::BinaryOp::Gt => BinOp::Gt,
        ast::BinaryOp::Le => BinOp::Le,
        ast::BinaryOp::Ge => BinOp::Ge,
        ast::BinaryOp::And | ast::BinaryOp::Or => unreachable!("lowered to And/Or nodes"),
    }
}
```

- [ ] **Step 6: Implement `lower/expr.rs`**

```rust
use kestrel_check::{ConstId, FnId, Res};
use kestrel_syntax::ast;

use super::{binop, kty, FnLowerer};
use crate::ir::*;

impl<'a> FnLowerer<'a> {
    /// Lowers `e` and applies any implicit conversion the checker recorded.
    pub(crate) fn expr(&mut self, e: &ast::Expr) -> Expr {
        let raw = self.expr_raw(e);
        match self.t.coerce.get(&e.id) {
            Some(&to) => Expr { ty: kty(to), line: raw.line, kind: ExprKind::Cast(Box::new(raw)) },
            None => raw,
        }
    }

    fn expr_raw(&mut self, e: &ast::Expr) -> Expr {
        let ty = kty(self.t.ty(e.id));
        let line = self.line(e.span);
        let kind = match &e.kind {
            ast::ExprKind::Int(n) => {
                if ty.is_float() {
                    ExprKind::Float(*n as f64)
                } else {
                    ExprKind::Int(*n as i64)
                }
            }
            ast::ExprKind::Float(f) => ExprKind::Float(*f),
            ast::ExprKind::Bool(b) => ExprKind::Bool(*b),
            ast::ExprKind::Str(_) => unreachable!("strings only appear as print arguments"),
            ast::ExprKind::Ident(_) => match self.t.res[&e.id].clone() {
                Res::Local(l) => ExprKind::Local(self.local_map[&l]),
                Res::Const(c) => return self.const_value(c),
                other => unreachable!("name resolved to {other:?}"),
            },
            ast::ExprKind::Unary { op, expr } => {
                let op = match op {
                    ast::UnaryOp::Neg => UnOp::Neg,
                    ast::UnaryOp::Not => UnOp::Not,
                };
                ExprKind::Unary(op, Box::new(self.expr(expr)))
            }
            ast::ExprKind::Binary { op, lhs, rhs } => {
                let l = Box::new(self.expr(lhs));
                let r = Box::new(self.expr(rhs));
                match op {
                    ast::BinaryOp::And => ExprKind::And(l, r),
                    ast::BinaryOp::Or => ExprKind::Or(l, r),
                    other => ExprKind::Binary(binop(*other), l, r),
                }
            }
            ast::ExprKind::Call { args, .. } => match self.t.res[&e.id].clone() {
                Res::Convert(_) => ExprKind::Cast(Box::new(self.expr(&args[0].value))),
                Res::Fn(f) => {
                    let (fi, a) = self.call_parts(e, f, args);
                    ExprKind::Call(fi, a)
                }
                other => unreachable!("{other:?} used as a value"),
            },
            ast::ExprKind::If { cond, then_block, else_block } => {
                let else_block = else_block.as_ref().expect("checker requires `else` for a value `if`");
                let cond = Box::new(self.expr(cond));
                let (then_body, then_val) = self.block_value(then_block);
                let (else_body, else_val) = self.block_value(else_block);
                ExprKind::If {
                    cond,
                    then_body,
                    then_val: then_val.map(Box::new),
                    else_body,
                    else_val: else_val.map(Box::new),
                }
            }
        };
        Expr { kind, ty, line }
    }

    /// A constant's initializer, re-lowered at each use.
    fn const_value(&mut self, c: ConstId) -> Expr {
        let (t, program) = (self.t, self.program);
        let item = t.consts[c.0 as usize].item;
        let ast::Item::Const(decl) = &program.items[item] else { unreachable!("ConstInfo points at a const") };
        self.expr(&decl.value)
    }

    /// Arguments in parameter order; `var` parameters get a reference.
    pub(crate) fn call_parts(&mut self, e: &ast::Expr, f: FnId, args: &[ast::Arg]) -> (FuncIdx, Vec<Arg>) {
        let t = self.t;
        let order = &t.call_args[&e.id];
        let params = &t.func(f).params;
        let lowered = order
            .iter()
            .zip(params)
            .map(|(&ai, p)| {
                let v = &args[ai].value;
                if p.is_var {
                    let Some(Res::Local(l)) = t.res.get(&v.id).copied() else {
                        unreachable!("checker requires a variable for a `var` argument")
                    };
                    Arg::Ref(self.local_map[&l])
                } else {
                    Arg::Value(self.expr(v))
                }
            })
            .collect();
        (FuncIdx(f.0), lowered)
    }
}
```

- [ ] **Step 7: Wire up `lib.rs`**

```rust
pub mod ir;
mod lower;

pub use ir::*;
pub use lower::lower;
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cd kestrel && cargo test -p kestrel-kir`
Expected: 7 passed.

- [ ] **Step 9: Commit**

```bash
git add kestrel/Cargo.toml kestrel/kir
git commit -m "Add KIR and lowering from the checked AST

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Runtime crate

**Files:**
- Create: `kestrel/runtime/Cargo.toml`, `kestrel/runtime/src/lib.rs`, `kestrel/runtime/src/print.rs`, `kestrel/runtime/src/panic.rs`, `kestrel/runtime/src/start.rs`
- Modify: `kestrel/Cargo.toml` (add `"runtime"` to `members`)
- Test: unit tests inside `kestrel/runtime/src/start.rs` and `panic.rs`

**Interfaces:**
- Produces these `extern "C"` symbols (codegen in Task 8 imports them by exactly these names and signatures; Rust types shown):
  - `kestrel_rt_print_i64(v: i64)`, `kestrel_rt_print_f32(v: f32)`, `kestrel_rt_print_f64(v: f64)`, `kestrel_rt_print_bool(v: i8)`, `kestrel_rt_print_str(ptr: *const u8, len: i64)`, `kestrel_rt_print_space()`, `kestrel_rt_print_newline()`
  - `kestrel_rt_panic(kind: i32, line: i32) -> !` with `kind` 1 = integer overflow, 2 = division by zero. Flushes stdout, prints `kestrel: <what> on line <line>` to stderr, exits with code 101.
  - `kestrel_rt_fmod_f32(a: f32, b: f32) -> f32`, `kestrel_rt_fmod_f64(a: f64, b: f64) -> f64`
  - `kestrel_rt_start(entry: extern "C" fn()) -> i32`: calls `entry`, flushes stdout, returns 0.
- Also exported as Rust items: `PANIC_OVERFLOW: i32 = 1`, `PANIC_DIV_ZERO: i32 = 2`, `print::flush()`.
- Built as both `rlib` (linked into the `kestrel` binary for JIT) and `staticlib` (linked into exes).

- [ ] **Step 1: Crate setup**

`kestrel/Cargo.toml` members:
```toml
members = ["syntax", "check", "kir", "runtime"]
```

`kestrel/runtime/Cargo.toml`:
```toml
[package]
name = "kestrel-runtime"
version.workspace = true
edition.workspace = true

[lib]
crate-type = ["rlib", "staticlib"]

[dependencies]
```

- [ ] **Step 2: Write the failing tests**

`kestrel/runtime/src/start.rs` (test module at the bottom; the implementation comes in Step 4):
```rust
#[cfg(test)]
mod tests {
    use super::kestrel_rt_start;
    use std::sync::atomic::{AtomicBool, Ordering};

    static RAN: AtomicBool = AtomicBool::new(false);

    extern "C" fn entry() {
        RAN.store(true, Ordering::SeqCst);
    }

    #[test]
    fn start_runs_entry_and_returns_zero() {
        assert_eq!(kestrel_rt_start(entry), 0);
        assert!(RAN.load(Ordering::SeqCst));
    }
}
```

`kestrel/runtime/src/panic.rs` test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmod_matches_rust_remainder() {
        assert_eq!(kestrel_rt_fmod_f32(7.5, 2.0), 1.5);
        assert_eq!(kestrel_rt_fmod_f64(-7.5, 2.0), -1.5);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd kestrel && cargo test -p kestrel-runtime`
Expected: compile errors (functions missing).

- [ ] **Step 4: Implement**

`kestrel/runtime/src/lib.rs`:
```rust
//! Functions compiled Kestrel code calls into. Every export is
//! `extern "C"` with a fixed name; codegen declares matching imports.

pub mod panic;
pub mod print;
pub mod start;

pub use panic::{PANIC_DIV_ZERO, PANIC_OVERFLOW};
```

`kestrel/runtime/src/print.rs`:
```rust
use std::cell::RefCell;
use std::fmt::Arguments;
use std::io::{self, BufWriter, Stdout, Write};

thread_local! {
    static OUT: RefCell<BufWriter<Stdout>> = RefCell::new(BufWriter::new(io::stdout()));
}

fn write_fmt(args: Arguments) {
    OUT.with(|o| {
        let _ = o.borrow_mut().write_fmt(args);
    });
}

fn write_bytes(bytes: &[u8]) {
    OUT.with(|o| {
        let _ = o.borrow_mut().write_all(bytes);
    });
}

pub fn flush() {
    OUT.with(|o| {
        let _ = o.borrow_mut().flush();
    });
}

#[no_mangle]
pub extern "C" fn kestrel_rt_print_i64(v: i64) {
    write_fmt(format_args!("{v}"));
}

#[no_mangle]
pub extern "C" fn kestrel_rt_print_f32(v: f32) {
    write_fmt(format_args!("{v}"));
}

#[no_mangle]
pub extern "C" fn kestrel_rt_print_f64(v: f64) {
    write_fmt(format_args!("{v}"));
}

#[no_mangle]
pub extern "C" fn kestrel_rt_print_bool(v: i8) {
    write_bytes(if v != 0 { b"true" } else { b"false" });
}

/// # Safety
/// `ptr` must point to `len` readable bytes (compiled code passes string data).
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_print_str(ptr: *const u8, len: i64) {
    write_bytes(std::slice::from_raw_parts(ptr, len as usize));
}

#[no_mangle]
pub extern "C" fn kestrel_rt_print_space() {
    write_bytes(b" ");
}

#[no_mangle]
pub extern "C" fn kestrel_rt_print_newline() {
    write_bytes(b"\n");
}
```

`kestrel/runtime/src/panic.rs` (above its test module):
```rust
pub const PANIC_OVERFLOW: i32 = 1;
pub const PANIC_DIV_ZERO: i32 = 2;

#[no_mangle]
pub extern "C" fn kestrel_rt_panic(kind: i32, line: i32) -> ! {
    crate::print::flush();
    let what = match kind {
        PANIC_OVERFLOW => "integer overflow",
        PANIC_DIV_ZERO => "division by zero",
        _ => "runtime error",
    };
    eprintln!("kestrel: {what} on line {line}");
    std::process::exit(101);
}

#[no_mangle]
pub extern "C" fn kestrel_rt_fmod_f32(a: f32, b: f32) -> f32 {
    a % b
}

#[no_mangle]
pub extern "C" fn kestrel_rt_fmod_f64(a: f64, b: f64) -> f64 {
    a % b
}
```

`kestrel/runtime/src/start.rs` (above its test module):
```rust
/// Runs a compiled Kestrel `main`, then flushes output. Returns the exit code.
#[no_mangle]
pub extern "C" fn kestrel_rt_start(entry: extern "C" fn()) -> i32 {
    entry();
    crate::print::flush();
    0
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd kestrel && cargo test -p kestrel-runtime`
Expected: 2 passed.

- [ ] **Step 6: Verify the staticlib is produced**

Run: `cd kestrel && cargo build -p kestrel-runtime && ls target/debug/kestrel_runtime.lib`
Expected: the file exists (on Linux/macOS it is `target/debug/libkestrel_runtime.a`).

- [ ] **Step 7: Commit**

```bash
git add kestrel/Cargo.toml kestrel/runtime
git commit -m "Add kestrel runtime: buffered print, panics, program start

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Cranelift codegen and JIT

**Files:**
- Create: `kestrel/codegen/Cargo.toml`, `kestrel/codegen/src/lib.rs`, `kestrel/codegen/src/isa.rs`, `kestrel/codegen/src/rt.rs`, `kestrel/codegen/src/func/mod.rs`, `kestrel/codegen/src/func/expr.rs`, `kestrel/codegen/src/func/arith.rs`, `kestrel/codegen/src/jit.rs`
- Modify: `kestrel/Cargo.toml` (add `"codegen"` to `members`)
- Test: `kestrel/codegen/tests/jit.rs`

**Interfaces:**
- Consumes: KIR (`kestrel_kir::*`, Task 6) and runtime symbols (Task 7).
- Produces: `kestrel_codegen::Options { checked_arith: bool }` (`Default` = checked); `kestrel_codegen::JitProgram::compile(&kir::Module, Options) -> Result<JitProgram, String>`, `JitProgram::run_main(&self) -> i32`, `JitProgram::fn_ptr(&self, name: &str) -> Option<*const u8>`; crate-internal `define_module<M: Module>(&mut M, &kir::Module, Options) -> Result<Vec<FuncId>, String>` (used by Task 10's object output) and `symbol(name) -> String` (`k_<name>`).

**How values are represented:** `bool`/`u8` → `I8`, `int` → `I32`, `i64` → `I64`, `float` → `F32`, `f64` → `F64`. `Value` locals are Cranelift variables, `Addressed` locals are 1/4/8-byte stack slots, `RefParam` locals hold a pointer. Statements after a `return`/`break`/`continue` in the same list are unreachable and are not emitted.

**Arithmetic:** with `checked_arith`, `int`/`i64` `+ - *` use `sadd_overflow`/`ssub_overflow`/`smul_overflow` and call `kestrel_rt_panic(1, line)` on overflow; `u8` arithmetic is done in 32 bits and panics outside 0..=255. Without it, they wrap. Division and remainder by zero always panic (`kind` 2); `MIN / -1` panics when checked and wraps (to `MIN`, remainder 0) when not. Integer `/` and `%` truncate toward zero (`%` takes the dividend's sign). Float `%` calls `kestrel_rt_fmod_*`. `int(x)` from a float saturates; NaN becomes 0.

- [ ] **Step 1: Crate setup**

`kestrel/Cargo.toml` members:
```toml
members = ["syntax", "check", "kir", "runtime", "codegen"]
```

`kestrel/codegen/Cargo.toml`:
```toml
[package]
name = "kestrel-codegen"
version.workspace = true
edition.workspace = true

[dependencies]
kestrel-kir.workspace = true
kestrel-runtime.workspace = true
cranelift-codegen.workspace = true
cranelift-frontend.workspace = true
cranelift-module.workspace = true
cranelift-jit.workspace = true
cranelift-object.workspace = true
cranelift-native.workspace = true

[dev-dependencies]
kestrel-syntax.workspace = true
kestrel-check.workspace = true
```

- [ ] **Step 2: Write the failing tests**

`kestrel/codegen/tests/jit.rs`:
```rust
use kestrel_codegen::{JitProgram, Options};
use kestrel_syntax::{parse, LineIndex};

fn jit_with(src: &str, opts: Options) -> JitProgram {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let m = kestrel_kir::lower(&p, &t.unwrap(), &LineIndex::new(src));
    JitProgram::compile(&m, opts).expect("codegen")
}

fn jit(src: &str) -> JitProgram {
    jit_with(&format!("{src}\nmain() {{}}\n"), Options::default())
}

fn call0<R>(p: &JitProgram, name: &str) -> R {
    let f: extern "C" fn() -> R = unsafe { std::mem::transmute(p.fn_ptr(name).expect(name)) };
    f()
}

fn call1(p: &JitProgram, name: &str, x: i32) -> i32 {
    let f: extern "C" fn(i32) -> i32 = unsafe { std::mem::transmute(p.fn_ptr(name).expect(name)) };
    f(x)
}

#[test]
fn recursion() {
    let p = jit("pure fib(n: int) -> int {\n if n < 2 { n } else { fib(n - 1) + fib(n - 2) }\n}");
    assert_eq!(call1(&p, "fib", 20), 6765);
}

#[test]
fn loops_break_continue() {
    let p = jit("sum_to(n: int) -> int {\n var t = 0\n for i from 0 to n { t += i }\n t\n}\nodd_sum() -> int {\n var i = 0\n var s = 0\n while true {\n  i += 1\n  if i > 10 { break }\n  if i % 2 == 0 { continue }\n  s += i\n }\n s\n}");
    assert_eq!(call1(&p, "sum_to", 100), 4950);
    assert_eq!(call0::<i32>(&p, "odd_sum"), 25);
}

#[test]
fn var_parameters() {
    let p = jit("bump(var x: int, by: int) {\n x += by\n}\nf() -> int {\n var a = 5\n bump(var a, by: 3)\n bump(var a, 2)\n a\n}");
    assert_eq!(call0::<i32>(&p, "f"), 10);
}

#[test]
fn floats_and_conversions() {
    let p = jit("f() -> int {\n let x: float = 7\n let y = x / 2.0\n int(y * 10.0)\n}\ng() -> i64 {\n let a = 3\n let b: i64 = a\n b * 1000000000\n}\nh() -> int {\n int(7.5 % 2.0 * 10.0)\n}\nsat() -> int {\n int(f64(2147483647) * 2.0)\n}\nu() -> int {\n let a: u8 = 200\n let b: u8 = 55\n int(a + b)\n}\nd() -> int {\n -7 / 2\n}\nr() -> int {\n -7 % 2\n}");
    assert_eq!(call0::<i32>(&p, "f"), 35);
    assert_eq!(call0::<i64>(&p, "g"), 3_000_000_000);
    assert_eq!(call0::<i32>(&p, "h"), 15);
    assert_eq!(call0::<i32>(&p, "sat"), i32::MAX);
    assert_eq!(call0::<i32>(&p, "u"), 255);
    assert_eq!(call0::<i32>(&p, "d"), -3);
    assert_eq!(call0::<i32>(&p, "r"), -1);
}

#[test]
fn short_circuit() {
    let p = jit("side(var n: int) -> bool {\n n += 1\n true\n}\nsc() -> int {\n var n = 0\n let a = false and side(var n)\n let b = true or side(var n)\n let c = true and side(var n)\n n\n}");
    assert_eq!(call0::<i32>(&p, "sc"), 1);
}

#[test]
fn if_expression_with_diverging_branch() {
    let p = jit("pick(x: int) -> int {\n let y = if x > 0 { x * 2 } else { return -1 }\n y + 1\n}");
    assert_eq!(call1(&p, "pick", 5), 11);
    assert_eq!(call1(&p, "pick", -3), -1);
}

#[test]
fn unchecked_arithmetic_wraps() {
    let src = "w() -> int {\n let a = 2147483647\n a + 1\n}\nmain() {}\n";
    let p = jit_with(src, Options { checked_arith: false });
    assert_eq!(call0::<i32>(&p, "w"), i32::MIN);
}

#[test]
fn main_runs_through_runtime() {
    let p = jit_with("main() {\n print(1, 2.5, true, \"hi\")\n}\n", Options::default());
    assert_eq!(p.run_main(), 0);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd kestrel && cargo test -p kestrel-codegen`
Expected: compile errors (crate empty).

- [ ] **Step 4: Implement `isa.rs`**

```rust
use cranelift_codegen::isa::OwnedTargetIsa;
use cranelift_codegen::settings::{self, Configurable};

/// The host machine, with the flags every Kestrel build uses.
pub(crate) fn host_isa() -> Result<OwnedTargetIsa, String> {
    let mut flags = settings::builder();
    // Probestack: Windows grows the stack one guard page at a time, so large
    // frames must touch each page in order.
    for (key, value) in [
        ("opt_level", "speed"),
        ("is_pic", "true"),
        ("enable_probestack", "true"),
        ("probestack_strategy", "inline"),
    ] {
        flags.set(key, value).map_err(|e| e.to_string())?;
    }
    cranelift_native::builder()
        .map_err(|e| e.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())
}
```

- [ ] **Step 5: Implement `rt.rs`**

```rust
use cranelift_codegen::ir::{types, AbiParam, Signature, Type};
use cranelift_module::{FuncId, Linkage, Module};

/// Runtime functions, declared as imports in a module.
pub(crate) struct RtFuncs {
    pub print_i64: FuncId,
    pub print_f32: FuncId,
    pub print_f64: FuncId,
    pub print_bool: FuncId,
    pub print_str: FuncId,
    pub print_space: FuncId,
    pub print_newline: FuncId,
    pub panic: FuncId,
    pub fmod_f32: FuncId,
    pub fmod_f64: FuncId,
}

impl RtFuncs {
    pub fn declare<M: Module>(module: &mut M) -> Result<Self, String> {
        let cc = module.isa().default_call_conv();
        let ptr = module.target_config().pointer_type();
        let mut decl = |name: &str, params: &[Type], ret: Option<Type>| -> Result<FuncId, String> {
            let mut sig = Signature::new(cc);
            sig.params.extend(params.iter().map(|&p| AbiParam::new(p)));
            sig.returns.extend(ret.map(AbiParam::new));
            module.declare_function(name, Linkage::Import, &sig).map_err(|e| e.to_string())
        };
        use types::{F32, F64, I32, I64, I8};
        Ok(RtFuncs {
            print_i64: decl("kestrel_rt_print_i64", &[I64], None)?,
            print_f32: decl("kestrel_rt_print_f32", &[F32], None)?,
            print_f64: decl("kestrel_rt_print_f64", &[F64], None)?,
            print_bool: decl("kestrel_rt_print_bool", &[I8], None)?,
            print_str: decl("kestrel_rt_print_str", &[ptr, I64], None)?,
            print_space: decl("kestrel_rt_print_space", &[], None)?,
            print_newline: decl("kestrel_rt_print_newline", &[], None)?,
            panic: decl("kestrel_rt_panic", &[I32, I32], None)?,
            fmod_f32: decl("kestrel_rt_fmod_f32", &[F32, F32], Some(F32))?,
            fmod_f64: decl("kestrel_rt_fmod_f64", &[F64, F64], Some(F64))?,
        })
    }
}

/// Addresses of every runtime export, for registering with the JIT.
pub(crate) fn jit_symbols() -> Vec<(&'static str, *const u8)> {
    use kestrel_runtime::{panic, print, start};
    vec![
        ("kestrel_rt_print_i64", print::kestrel_rt_print_i64 as *const u8),
        ("kestrel_rt_print_f32", print::kestrel_rt_print_f32 as *const u8),
        ("kestrel_rt_print_f64", print::kestrel_rt_print_f64 as *const u8),
        ("kestrel_rt_print_bool", print::kestrel_rt_print_bool as *const u8),
        ("kestrel_rt_print_str", print::kestrel_rt_print_str as *const u8),
        ("kestrel_rt_print_space", print::kestrel_rt_print_space as *const u8),
        ("kestrel_rt_print_newline", print::kestrel_rt_print_newline as *const u8),
        ("kestrel_rt_panic", panic::kestrel_rt_panic as *const u8),
        ("kestrel_rt_fmod_f32", panic::kestrel_rt_fmod_f32 as *const u8),
        ("kestrel_rt_fmod_f64", panic::kestrel_rt_fmod_f64 as *const u8),
        ("kestrel_rt_start", start::kestrel_rt_start as *const u8),
    ]
}
```

- [ ] **Step 6: Implement the function lowerer core (`func/mod.rs`)**

```rust
mod arith;
mod expr;

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{
    types, AbiParam, Block, Function, InstBuilder, MemFlags, Signature, StackSlot, StackSlotData,
    StackSlotKind, TrapCode, Type, Value,
};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, DataId, FuncId, Module};
use kestrel_kir::{self as kir, KTy, LocalIdx, LocalKind};

use crate::rt::RtFuncs;
use crate::Options;

pub(crate) fn cl_type(t: KTy) -> Type {
    match t {
        KTy::Bool | KTy::U8 => types::I8,
        KTy::I32 => types::I32,
        KTy::I64 => types::I64,
        KTy::F32 => types::F32,
        KTy::F64 => types::F64,
    }
}

pub(crate) fn signature(f: &kir::Func, cc: CallConv, ptr: Type) -> Signature {
    let mut sig = Signature::new(cc);
    for p in &f.params {
        let local = &f.locals[p.0 as usize];
        let ty = if local.kind == LocalKind::RefParam { ptr } else { cl_type(local.ty) };
        sig.params.push(AbiParam::new(ty));
    }
    if let Some(r) = f.ret {
        sig.returns.push(AbiParam::new(cl_type(r)));
    }
    sig
}

/// String literal data, shared across all functions of a module.
#[derive(Default)]
pub(crate) struct Strings {
    ids: HashMap<String, DataId>,
}

/// Where a local's value lives.
#[derive(Clone, Copy)]
enum Slot {
    Var(Variable),
    Stack(StackSlot),
    /// Variable holding a pointer to the value.
    Ptr(Variable),
}

pub(crate) struct Shared<'a> {
    pub funcs: &'a [FuncId],
    pub rt: &'a RtFuncs,
    pub opts: Options,
}

pub(crate) struct FuncLowerer<'a, 'b, M: Module> {
    module: &'a mut M,
    strings: &'a mut Strings,
    shared: &'a Shared<'a>,
    b: FunctionBuilder<'b>,
    f: &'a kir::Func,
    ptr: Type,
    slots: Vec<Slot>,
    /// (continue target, break target) of each enclosing loop.
    loops: Vec<(Block, Block)>,
}

pub(crate) fn lower_func<M: Module>(
    module: &mut M,
    strings: &mut Strings,
    shared: &Shared,
    func: &mut Function,
    fb_ctx: &mut FunctionBuilderContext,
    f: &kir::Func,
) -> Result<(), String> {
    let ptr = module.target_config().pointer_type();
    let mut b = FunctionBuilder::new(func, fb_ctx);
    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    b.seal_block(entry);
    let params: Vec<Value> = b.block_params(entry).to_vec();

    let mut slots = Vec::with_capacity(f.locals.len());
    for (i, local) in f.locals.iter().enumerate() {
        let ty = cl_type(local.ty);
        let slot = match local.kind {
            LocalKind::Value => {
                let v = Variable::from_u32(i as u32);
                b.declare_var(v, ty);
                Slot::Var(v)
            }
            LocalKind::RefParam => {
                let v = Variable::from_u32(i as u32);
                b.declare_var(v, ptr);
                Slot::Ptr(v)
            }
            LocalKind::Addressed => {
                let size = ty.bytes();
                let data = StackSlotData::new(StackSlotKind::ExplicitSlot, size, size.trailing_zeros() as u8);
                Slot::Stack(b.create_sized_stack_slot(data))
            }
        };
        slots.push(slot);
    }
    for (p, &val) in f.params.iter().zip(&params) {
        match slots[p.0 as usize] {
            Slot::Var(v) | Slot::Ptr(v) => b.def_var(v, val),
            Slot::Stack(ss) => {
                b.ins().stack_store(val, ss, 0);
            }
        }
    }

    let mut fl = FuncLowerer { module, strings, shared, b, f, ptr, slots, loops: Vec::new() };
    if fl.stmts(&f.body)? {
        if f.ret.is_some() {
            // The checker guarantees value-returning bodies don't fall off the end.
            fl.b.ins().trap(TrapCode::unwrap_user(1));
        } else {
            fl.b.ins().return_(&[]);
        }
    }
    fl.b.seal_all_blocks();
    fl.b.finalize();
    Ok(())
}

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    fn local_ty(&self, l: LocalIdx) -> KTy {
        self.f.locals[l.0 as usize].ty
    }

    pub(crate) fn load_local(&mut self, l: LocalIdx) -> Value {
        let ty = cl_type(self.local_ty(l));
        match self.slots[l.0 as usize] {
            Slot::Var(v) => self.b.use_var(v),
            Slot::Stack(ss) => self.b.ins().stack_load(ty, ss, 0),
            Slot::Ptr(v) => {
                let p = self.b.use_var(v);
                self.b.ins().load(ty, MemFlags::trusted(), p, 0)
            }
        }
    }

    fn store_local(&mut self, l: LocalIdx, val: Value) {
        match self.slots[l.0 as usize] {
            Slot::Var(v) => self.b.def_var(v, val),
            Slot::Stack(ss) => {
                self.b.ins().stack_store(val, ss, 0);
            }
            Slot::Ptr(v) => {
                let p = self.b.use_var(v);
                self.b.ins().store(MemFlags::trusted(), val, p, 0);
            }
        }
    }

    /// Address of a local passed to a `var` parameter.
    pub(crate) fn local_addr(&mut self, l: LocalIdx) -> Value {
        match self.slots[l.0 as usize] {
            Slot::Stack(ss) => self.b.ins().stack_addr(self.ptr, ss, 0),
            Slot::Ptr(v) => self.b.use_var(v),
            Slot::Var(_) => unreachable!("KIR marks every `var` argument Addressed"),
        }
    }

    /// Lowers statements; returns whether control can reach the end.
    fn stmts(&mut self, list: &[kir::Stmt]) -> Result<bool, String> {
        for s in list {
            if !self.stmt(s)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn stmt(&mut self, s: &kir::Stmt) -> Result<bool, String> {
        match s {
            kir::Stmt::Let(l, e) | kir::Stmt::Assign(l, e) => {
                let v = self.expr(e)?;
                self.store_local(*l, v);
            }
            kir::Stmt::If { cond, then_body, else_body } => return self.if_stmt(cond, then_body, else_body),
            kir::Stmt::While { cond, body } => self.while_loop(cond, body)?,
            kir::Stmt::ForRange { var, start, end, body } => self.for_range(*var, start, end, body)?,
            kir::Stmt::Break => {
                let (_, brk) = *self.loops.last().expect("break inside a loop");
                self.b.ins().jump(brk, &[]);
                return Ok(false);
            }
            kir::Stmt::Continue => {
                let (cont, _) = *self.loops.last().expect("continue inside a loop");
                self.b.ins().jump(cont, &[]);
                return Ok(false);
            }
            kir::Stmt::Return(value) => {
                match value {
                    Some(e) => {
                        let v = self.expr(e)?;
                        self.b.ins().return_(&[v]);
                    }
                    None => {
                        self.b.ins().return_(&[]);
                    }
                }
                return Ok(false);
            }
            kir::Stmt::Call(fi, args) => {
                self.call(*fi, args)?;
            }
            kir::Stmt::Print(args) => self.print(args)?,
        }
        Ok(true)
    }

    fn if_stmt(&mut self, cond: &kir::Expr, then_body: &[kir::Stmt], else_body: &[kir::Stmt]) -> Result<bool, String> {
        let c = self.expr(cond)?;
        let (then_b, else_b, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().brif(c, then_b, &[], else_b, &[]);
        self.b.switch_to_block(then_b);
        let then_falls = self.stmts(then_body)?;
        if then_falls {
            self.b.ins().jump(merge, &[]);
        }
        self.b.switch_to_block(else_b);
        let else_falls = self.stmts(else_body)?;
        if else_falls {
            self.b.ins().jump(merge, &[]);
        }
        self.b.switch_to_block(merge);
        if !then_falls && !else_falls {
            self.b.ins().trap(TrapCode::unwrap_user(1));
            return Ok(false);
        }
        Ok(true)
    }

    fn while_loop(&mut self, cond: &kir::Expr, body: &[kir::Stmt]) -> Result<(), String> {
        let (header, body_b, exit) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().jump(header, &[]);
        self.b.switch_to_block(header);
        let c = self.expr(cond)?;
        self.b.ins().brif(c, body_b, &[], exit, &[]);
        self.b.switch_to_block(body_b);
        self.loops.push((header, exit));
        let falls = self.stmts(body)?;
        self.loops.pop();
        if falls {
            self.b.ins().jump(header, &[]);
        }
        self.b.switch_to_block(exit);
        Ok(())
    }

    fn for_range(&mut self, var: LocalIdx, start: &kir::Expr, end: &kir::Expr, body: &[kir::Stmt]) -> Result<(), String> {
        let start_v = self.expr(start)?;
        let end_v = self.expr(end)?;
        self.store_local(var, start_v);
        let ty = self.local_ty(var);
        let (header, body_b, step, exit) =
            (self.b.create_block(), self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().jump(header, &[]);

        self.b.switch_to_block(header);
        let i = self.load_local(var);
        let cc = if ty.is_signed() { IntCC::SignedLessThan } else { IntCC::UnsignedLessThan };
        let more = self.b.ins().icmp(cc, i, end_v);
        self.b.ins().brif(more, body_b, &[], exit, &[]);

        self.b.switch_to_block(body_b);
        self.loops.push((step, exit));
        let falls = self.stmts(body)?;
        self.loops.pop();
        if falls {
            self.b.ins().jump(step, &[]);
        }

        // `i < end` held, so `i + 1` can't overflow.
        self.b.switch_to_block(step);
        let i = self.load_local(var);
        let next = self.b.ins().iadd_imm(i, 1);
        self.store_local(var, next);
        self.b.ins().jump(header, &[]);

        self.b.switch_to_block(exit);
        Ok(())
    }

    pub(crate) fn call(&mut self, fi: kir::FuncIdx, args: &[kir::Arg]) -> Result<Option<Value>, String> {
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(match a {
                kir::Arg::Value(e) => self.expr(e)?,
                kir::Arg::Ref(l) => self.local_addr(*l),
            });
        }
        let id = self.shared.funcs[fi.0 as usize];
        Ok(self.call_id(id, &vals))
    }

    pub(crate) fn call_id(&mut self, id: FuncId, args: &[Value]) -> Option<Value> {
        let fref = self.module.declare_func_in_func(id, self.b.func);
        let inst = self.b.ins().call(fref, args);
        self.b.inst_results(inst).first().copied()
    }

    fn print(&mut self, args: &[kir::PrintArg]) -> Result<(), String> {
        let rt = self.shared.rt;
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                self.call_id(rt.print_space, &[]);
            }
            match a {
                kir::PrintArg::Str(s) => {
                    let (p, n) = self.string(s)?;
                    self.call_id(rt.print_str, &[p, n]);
                }
                kir::PrintArg::Value(e) => {
                    let v = self.expr(e)?;
                    match e.ty {
                        KTy::Bool => self.call_id(rt.print_bool, &[v]),
                        KTy::U8 => {
                            let w = self.b.ins().uextend(types::I64, v);
                            self.call_id(rt.print_i64, &[w])
                        }
                        KTy::I32 => {
                            let w = self.b.ins().sextend(types::I64, v);
                            self.call_id(rt.print_i64, &[w])
                        }
                        KTy::I64 => self.call_id(rt.print_i64, &[v]),
                        KTy::F32 => self.call_id(rt.print_f32, &[v]),
                        KTy::F64 => self.call_id(rt.print_f64, &[v]),
                    };
                }
            }
        }
        self.call_id(rt.print_newline, &[]);
        Ok(())
    }

    /// Pointer and length of a string literal's bytes.
    fn string(&mut self, s: &str) -> Result<(Value, Value), String> {
        let id = match self.strings.ids.get(s) {
            Some(&id) => id,
            None => {
                let id = self.module.declare_anonymous_data(false, false).map_err(|e| e.to_string())?;
                let mut desc = DataDescription::new();
                // Zero-sized data objects aren't allowed; the length stays 0.
                let bytes = if s.is_empty() { vec![0] } else { s.as_bytes().to_vec() };
                desc.define(bytes.into_boxed_slice());
                self.module.define_data(id, &desc).map_err(|e| e.to_string())?;
                self.strings.ids.insert(s.to_string(), id);
                id
            }
        };
        let gv = self.module.declare_data_in_func(id, self.b.func);
        let p = self.b.ins().symbol_value(self.ptr, gv);
        let n = self.b.ins().iconst(types::I64, s.len() as i64);
        Ok((p, n))
    }
}

```

- [ ] **Step 7: Implement expressions (`func/expr.rs`)**

```rust
use cranelift_codegen::ir::{types, InstBuilder, TrapCode, Value};
use cranelift_module::Module;
use kestrel_kir::{self as kir, ExprKind, KTy, UnOp};

use super::{cl_type, FuncLowerer};

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    pub(crate) fn expr(&mut self, e: &kir::Expr) -> Result<Value, String> {
        let ty = cl_type(e.ty);
        Ok(match &e.kind {
            ExprKind::Int(n) => self.iconst(e.ty, *n),
            ExprKind::Float(f) => {
                if e.ty == KTy::F32 {
                    self.b.ins().f32const(*f as f32)
                } else {
                    self.b.ins().f64const(*f)
                }
            }
            ExprKind::Bool(v) => self.b.ins().iconst(types::I8, *v as i64),
            ExprKind::Local(l) => self.load_local(*l),
            ExprKind::Unary(UnOp::Not, inner) => {
                let v = self.expr(inner)?;
                self.b.ins().bxor_imm(v, 1)
            }
            ExprKind::Unary(UnOp::Neg, inner) => {
                let v = self.expr(inner)?;
                if e.ty.is_float() {
                    self.b.ins().fneg(v)
                } else {
                    let zero = self.iconst(e.ty, 0);
                    self.int_arith(kir::BinOp::Sub, zero, v, e.ty, e.line)
                }
            }
            ExprKind::Binary(op, l, r) => {
                let a = self.expr(l)?;
                let b = self.expr(r)?;
                self.binary(*op, a, b, l.ty, e.line)
            }
            ExprKind::And(l, r) => self.short_circuit(l, r, true)?,
            ExprKind::Or(l, r) => self.short_circuit(l, r, false)?,
            ExprKind::Cast(inner) => {
                let v = self.expr(inner)?;
                self.cast(v, inner.ty, e.ty)
            }
            ExprKind::Call(fi, args) => self.call(*fi, args)?.expect("value-returning call"),
            ExprKind::If { cond, then_body, then_val, else_body, else_val } => {
                let c = self.expr(cond)?;
                let (then_b, else_b, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                let result = self.b.append_block_param(merge, ty);
                self.b.ins().brif(c, then_b, &[], else_b, &[]);
                for (block, body, val) in [(then_b, then_body, then_val), (else_b, else_body, else_val)] {
                    self.b.switch_to_block(block);
                    if self.stmts(body)? {
                        match val {
                            Some(v) => {
                                let x = self.expr(v)?;
                                self.b.ins().jump(merge, &[x]);
                            }
                            None => {
                                self.b.ins().trap(TrapCode::unwrap_user(1));
                            }
                        }
                    }
                }
                self.b.switch_to_block(merge);
                result
            }
        })
    }

    /// An integer constant, masked to the type's width as Cranelift expects.
    pub(crate) fn iconst(&mut self, ty: KTy, n: i64) -> Value {
        let masked = match ty {
            KTy::Bool | KTy::U8 => n & 0xFF,
            KTy::I32 => n & 0xFFFF_FFFF,
            _ => n,
        };
        self.b.ins().iconst(cl_type(ty), masked)
    }

    fn short_circuit(&mut self, l: &kir::Expr, r: &kir::Expr, is_and: bool) -> Result<Value, String> {
        let a = self.expr(l)?;
        let (rhs_b, merge) = (self.b.create_block(), self.b.create_block());
        let result = self.b.append_block_param(merge, types::I8);
        if is_and {
            self.b.ins().brif(a, rhs_b, &[], merge, &[a]);
        } else {
            self.b.ins().brif(a, merge, &[a], rhs_b, &[]);
        }
        self.b.switch_to_block(rhs_b);
        let b = self.expr(r)?;
        self.b.ins().jump(merge, &[b]);
        self.b.switch_to_block(merge);
        Ok(result)
    }
}
```

- [ ] **Step 8: Implement arithmetic, comparisons and casts (`func/arith.rs`)**

```rust
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, InstBuilder, TrapCode, Value};
use cranelift_module::Module;
use kestrel_kir::{BinOp, KTy};
use kestrel_runtime::{PANIC_DIV_ZERO, PANIC_OVERFLOW};

use super::{cl_type, FuncLowerer};

fn bits(t: KTy) -> u32 {
    match t {
        KTy::Bool | KTy::U8 => 8,
        KTy::I32 | KTy::F32 => 32,
        KTy::I64 | KTy::F64 => 64,
    }
}

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// `a op b` where both operands have type `t`.
    pub(crate) fn binary(&mut self, op: BinOp, a: Value, b: Value, t: KTy, line: u32) -> Value {
        if op.is_comparison() {
            return self.compare(op, a, b, t);
        }
        if t.is_float() {
            return match op {
                BinOp::Add => self.b.ins().fadd(a, b),
                BinOp::Sub => self.b.ins().fsub(a, b),
                BinOp::Mul => self.b.ins().fmul(a, b),
                BinOp::Div => self.b.ins().fdiv(a, b),
                BinOp::Rem => {
                    let f = if t == KTy::F32 { self.shared.rt.fmod_f32 } else { self.shared.rt.fmod_f64 };
                    self.call_id(f, &[a, b]).expect("fmod returns a value")
                }
                _ => unreachable!("comparisons handled above"),
            };
        }
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul => self.int_arith(op, a, b, t, line),
            BinOp::Div | BinOp::Rem => self.int_div(op, a, b, t, line),
            _ => unreachable!("comparisons handled above"),
        }
    }

    /// Integer `+ - *`, overflow-checked when `checked_arith` is on.
    pub(crate) fn int_arith(&mut self, op: BinOp, a: Value, b: Value, t: KTy, line: u32) -> Value {
        let plain = |fl: &mut Self, x: Value, y: Value| match op {
            BinOp::Add => fl.b.ins().iadd(x, y),
            BinOp::Sub => fl.b.ins().isub(x, y),
            _ => fl.b.ins().imul(x, y),
        };
        if !self.shared.opts.checked_arith {
            return plain(self, a, b);
        }
        if t == KTy::U8 {
            // Compute in 32 bits; anything outside 0..=255 (including
            // negative results, which look huge unsigned) overflowed.
            let x = self.b.ins().uextend(types::I32, a);
            let y = self.b.ins().uextend(types::I32, b);
            let r = plain(self, x, y);
            let bad = self.b.ins().icmp_imm(IntCC::UnsignedGreaterThan, r, 255);
            self.panic_if(bad, PANIC_OVERFLOW, line);
            return self.b.ins().ireduce(types::I8, r);
        }
        let (r, overflowed) = match op {
            BinOp::Add => self.b.ins().sadd_overflow(a, b),
            BinOp::Sub => self.b.ins().ssub_overflow(a, b),
            _ => self.b.ins().smul_overflow(a, b),
        };
        self.panic_if(overflowed, PANIC_OVERFLOW, line);
        r
    }

    fn int_div(&mut self, op: BinOp, a: Value, b: Value, t: KTy, line: u32) -> Value {
        let is_zero = self.b.ins().icmp_imm(IntCC::Equal, b, 0);
        self.panic_if(is_zero, PANIC_DIV_ZERO, line);
        if !t.is_signed() {
            return if op == BinOp::Div { self.b.ins().udiv(a, b) } else { self.b.ins().urem(a, b) };
        }
        // MIN / -1 overflows (and traps in hardware).
        let min = if t == KTy::I32 { i32::MIN as i64 } else { i64::MIN };
        let min_v = self.iconst(t, min);
        let neg_one = self.iconst(t, -1);
        let a_is_min = self.b.ins().icmp(IntCC::Equal, a, min_v);
        let b_is_neg1 = self.b.ins().icmp(IntCC::Equal, b, neg_one);
        let both = self.b.ins().band(a_is_min, b_is_neg1);
        let divisor = if self.shared.opts.checked_arith {
            self.panic_if(both, PANIC_OVERFLOW, line);
            b
        } else {
            // Wrapping: MIN / 1 == MIN and MIN % 1 == 0, matching wrapping semantics.
            let one = self.iconst(t, 1);
            self.b.ins().select(both, one, b)
        };
        if op == BinOp::Div {
            self.b.ins().sdiv(a, divisor)
        } else {
            self.b.ins().srem(a, divisor)
        }
    }

    /// Calls the runtime panic (which doesn't return) when `cond` is true.
    fn panic_if(&mut self, cond: Value, kind: i32, line: u32) {
        let (fail, ok) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(cond, fail, &[], ok, &[]);
        self.b.switch_to_block(fail);
        self.b.set_cold_block(fail);
        let k = self.b.ins().iconst(types::I32, kind as i64);
        let l = self.b.ins().iconst(types::I32, line as i64);
        self.call_id(self.shared.rt.panic, &[k, l]);
        self.b.ins().trap(TrapCode::unwrap_user(2));
        self.b.switch_to_block(ok);
    }

    fn compare(&mut self, op: BinOp, a: Value, b: Value, t: KTy) -> Value {
        if t.is_float() {
            let cc = match op {
                BinOp::Eq => FloatCC::Equal,
                BinOp::Ne => FloatCC::NotEqual,
                BinOp::Lt => FloatCC::LessThan,
                BinOp::Gt => FloatCC::GreaterThan,
                BinOp::Le => FloatCC::LessThanOrEqual,
                _ => FloatCC::GreaterThanOrEqual,
            };
            return self.b.ins().fcmp(cc, a, b);
        }
        let signed = t.is_signed();
        let cc = match op {
            BinOp::Eq => IntCC::Equal,
            BinOp::Ne => IntCC::NotEqual,
            BinOp::Lt if signed => IntCC::SignedLessThan,
            BinOp::Lt => IntCC::UnsignedLessThan,
            BinOp::Gt if signed => IntCC::SignedGreaterThan,
            BinOp::Gt => IntCC::UnsignedGreaterThan,
            BinOp::Le if signed => IntCC::SignedLessThanOrEqual,
            BinOp::Le => IntCC::UnsignedLessThanOrEqual,
            _ if signed => IntCC::SignedGreaterThanOrEqual,
            _ => IntCC::UnsignedGreaterThanOrEqual,
        };
        self.b.ins().icmp(cc, a, b)
    }

    /// Converts `v` from `from` to `to`: int widening/narrowing (narrowing
    /// truncates), int↔float, float→int saturating (NaN → 0).
    pub(crate) fn cast(&mut self, v: Value, from: KTy, to: KTy) -> Value {
        if from == to {
            return v;
        }
        let to_ty = cl_type(to);
        match (from.is_float(), to.is_float()) {
            (false, false) => {
                let (fb, tb) = (bits(from), bits(to));
                if tb > fb {
                    if from.is_signed() {
                        self.b.ins().sextend(to_ty, v)
                    } else {
                        self.b.ins().uextend(to_ty, v)
                    }
                } else if tb < fb {
                    self.b.ins().ireduce(to_ty, v)
                } else {
                    v
                }
            }
            (false, true) => {
                if from.is_signed() {
                    self.b.ins().fcvt_from_sint(to_ty, v)
                } else {
                    let wide = self.b.ins().uextend(types::I32, v);
                    self.b.ins().fcvt_from_sint(to_ty, wide)
                }
            }
            (true, false) => {
                if to == KTy::U8 {
                    let wide = self.b.ins().fcvt_to_sint_sat(types::I32, v);
                    let zero = self.b.ins().iconst(types::I32, 0);
                    let max = self.b.ins().iconst(types::I32, 255);
                    let lo = self.b.ins().smax(wide, zero);
                    let clamped = self.b.ins().smin(lo, max);
                    self.b.ins().ireduce(types::I8, clamped)
                } else {
                    self.b.ins().fcvt_to_sint_sat(to_ty, v)
                }
            }
            (true, true) => {
                if to == KTy::F64 {
                    self.b.ins().fpromote(types::F64, v)
                } else {
                    self.b.ins().fdemote(types::F32, v)
                }
            }
        }
    }
}
```

- [ ] **Step 9: Implement `lib.rs` (module definition shared by JIT and object output)**

```rust
mod func;
mod isa;
pub mod jit;
mod rt;

use cranelift_codegen::ir::Function;
use cranelift_frontend::FunctionBuilderContext;
use cranelift_module::{FuncId, Linkage, Module};
use kestrel_kir as kir;

pub use jit::JitProgram;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Trap on integer overflow (debug-style). Off for `--release`.
    pub checked_arith: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { checked_arith: true }
    }
}

/// Symbol name of a Kestrel function, kept apart from C names like `main`.
pub(crate) fn symbol(name: &str) -> String {
    format!("k_{name}")
}

/// Declares and defines every function of `m` in `module`.
/// Returns their ids, indexed by `FuncIdx`.
pub(crate) fn define_module<M: Module>(module: &mut M, m: &kir::Module, opts: Options) -> Result<Vec<FuncId>, String> {
    let cc = module.isa().default_call_conv();
    let ptr = module.target_config().pointer_type();
    let rt = rt::RtFuncs::declare(module)?;
    let mut ids = Vec::with_capacity(m.funcs.len());
    for f in &m.funcs {
        let sig = func::signature(f, cc, ptr);
        let id = module.declare_function(&symbol(&f.name), Linkage::Local, &sig).map_err(|e| e.to_string())?;
        ids.push(id);
    }
    let shared = func::Shared { funcs: &ids, rt: &rt, opts };
    let mut strings = func::Strings::default();
    let mut ctx = module.make_context();
    let mut fb_ctx = FunctionBuilderContext::new();
    for (f, &id) in m.funcs.iter().zip(&ids) {
        ctx.func = Function::with_name_signature(Default::default(), func::signature(f, cc, ptr));
        func::lower_func(module, &mut strings, &shared, &mut ctx.func, &mut fb_ctx, f)?;
        module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("codegen failed for `{}`: {e:?}", f.name))?;
        module.clear_context(&mut ctx);
    }
    Ok(ids)
}
```

- [ ] **Step 10: Implement `jit.rs`**

```rust
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::FuncId;
use kestrel_kir as kir;

use crate::{define_module, isa, rt, Options};

/// A module compiled into this process's memory.
pub struct JitProgram {
    module: JITModule,
    main: FuncId,
    funcs: Vec<(String, FuncId)>,
}

impl JitProgram {
    pub fn compile(m: &kir::Module, opts: Options) -> Result<Self, String> {
        let mut builder = JITBuilder::with_isa(isa::host_isa()?, cranelift_module::default_libcall_names());
        for (name, addr) in rt::jit_symbols() {
            builder.symbol(name, addr);
        }
        let mut module = JITModule::new(builder);
        let ids = define_module(&mut module, m, opts)?;
        module.finalize_definitions().map_err(|e| e.to_string())?;
        let funcs = m.funcs.iter().zip(&ids).map(|(f, &id)| (f.name.clone(), id)).collect();
        Ok(JitProgram { main: ids[m.main.0 as usize], module, funcs })
    }

    /// Machine code address of a Kestrel function (tests call these directly).
    pub fn fn_ptr(&self, name: &str) -> Option<*const u8> {
        let (_, id) = self.funcs.iter().find(|(n, _)| n == name)?;
        Some(self.module.get_finalized_function(*id))
    }

    /// Runs `main` through the runtime entry point; returns the exit code.
    /// The compiled code stays mapped until the process exits.
    pub fn run_main(&self) -> i32 {
        let code = self.module.get_finalized_function(self.main);
        // SAFETY: the checker requires `main()` to take no parameters and
        // return nothing, and it was compiled with the host's C convention.
        let entry: extern "C" fn() = unsafe { std::mem::transmute(code) };
        kestrel_runtime::start::kestrel_rt_start(entry)
    }
}
```

- [ ] **Step 11: Run tests to verify they pass**

Run: `cd kestrel && cargo test -p kestrel-codegen`
Expected: 8 passed. `main_runs_through_runtime` also prints `1 2.5 true hi` to the test's stdout (visible with `-- --nocapture`).

If a test fails with a Cranelift verifier error, the message names the function and instruction: check that every block is terminated exactly once (a `jump`/`brif`/`return_`/`trap` per block) and that no instruction is emitted after a terminator.

- [ ] **Step 12: Commit**

```bash
git add kestrel/Cargo.toml kestrel/codegen
git commit -m "Add Cranelift codegen and JIT for KIR

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: CLI `run`/`check` and the program test harness

**Files:**
- Create: `kestrel/cli/Cargo.toml`, `kestrel/cli/src/main.rs`, `kestrel/cli/src/driver.rs`, `kestrel/cli/tests/programs.rs`, `kestrel/tests/programs/hello.kes`, `fib.kes`, `loops.kes`, `var_params.kes`, `numbers.kes`, `if_expr.kes`, `overflow.kes`, `div_zero.kes` (all under `kestrel/tests/programs/`)
- Modify: `kestrel/Cargo.toml` (add `"cli"` to `members`)

**Interfaces:**
- Consumes: `parse`, `render`, `render_short`, `LineIndex` (syntax), `check`, `lower`, `JitProgram`, `Options`.
- Produces: binary `kestrel` with `kestrel run <file>` and `kestrel check <file>`, flag `--diagnostics=short` (one `LINE:COL: error: MESSAGE` line per diagnostic, used by Task 11). Exit codes: 0 success, 1 compile errors, 2 usage/IO error, 3 internal compiler error, 101 runtime panic. `driver::compile(path, src, DiagStyle) -> Option<kir::Module>` (Task 10 reuses it). Test harness format, used by every later plan:
  - `// expect: <text>` — one expected stdout line each, in order (stdout must match exactly).
  - `// expect-exit: <n>` — expected exit code (default 0).
  - `// expect-stderr: <text>` — stderr must contain this text.

- [ ] **Step 1: Crate setup**

`kestrel/Cargo.toml` members:
```toml
members = ["syntax", "check", "kir", "runtime", "codegen", "cli"]
```

`kestrel/cli/Cargo.toml`:
```toml
[package]
name = "kestrel-cli"
version.workspace = true
edition.workspace = true

[[bin]]
name = "kestrel"
path = "src/main.rs"

[dependencies]
kestrel-syntax.workspace = true
kestrel-check.workspace = true
kestrel-kir.workspace = true
kestrel-codegen.workspace = true
```

- [ ] **Step 2: Write the test programs**

`kestrel/tests/programs/hello.kes`:
```
// expect: hello world
// expect: 1 2.5 true
main() {
    print("hello world")
    print(1, 2.5, true)
}
```

`kestrel/tests/programs/fib.kes`:
```
// expect: 0 1 1 2 3 5 8 13 21 34
// expect: 6765
pure fib(n: int) -> int {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}

main() {
    print(fib(0), fib(1), fib(2), fib(3), fib(4), fib(5), fib(6), fib(7), fib(8), fib(9))
    print(fib(20))
}
```

`kestrel/tests/programs/loops.kes`:
```
// expect: 4950
// expect: 25
// expect: 36
main() {
    var total = 0
    for i from 0 to 100 { total += i }
    print(total)

    var odd = 0
    var i = 0
    while true {
        i += 1
        if i > 10 { break }
        if i % 2 == 0 { continue }
        odd += i
    }
    print(odd)

    var cells = 0
    for x from 0 to 6 {
        for y from 0 to 6 {
            cells += 1
        }
    }
    print(cells)
}
```

`kestrel/tests/programs/var_params.kes`:
```
// expect: 10
// expect: 2 1
bump(var x: int, by: int) {
    x += by
}

swap(var a: int, var b: int) {
    let t = a
    a = b
    b = t
}

main() {
    var n = 5
    bump(var n, by: 3)
    bump(var n, 2)
    print(n)
    var p = 1
    var q = 2
    swap(var p, var q)
    print(p, q)
}
```

`kestrel/tests/programs/numbers.kes`:
```
// expect: 3.5
// expect: 35
// expect: 3000000000
// expect: 255
// expect: -3 -1
// expect: 1024
// expect: 0.1
let SIZE = 32

area(width: int, height: int) -> int {
    width * height
}

main() {
    let x: float = 7
    print(x / 2.0)
    print(int(x / 2.0 * 10.0))
    let big: i64 = 3
    print(big * 1000000000)
    let a: u8 = 200
    print(a + 55)
    print(-7 / 2, -7 % 2)
    print(area(height: SIZE, width: SIZE))
    print(0.1)
}
```

`kestrel/tests/programs/if_expr.kes`:
```
// expect: big
// expect: 11 -1
classify(n: int) -> int {
    if n > 100 { 2 } else if n > 10 { 1 } else { 0 }
}

pick(x: int) -> int {
    let y = if x > 0 { x * 2 } else { return -1 }
    y + 1
}

main() {
    if classify(500) == 2 { print("big") } else { print("small") }
    print(pick(5), pick(-3))
}
```

`kestrel/tests/programs/overflow.kes` (line numbers matter: `x += 1` is line 7):
```
// expect: before
// expect-exit: 101
// expect-stderr: integer overflow on line 7
main() {
    print("before")
    var x = 2147483647
    x += 1
    print("after")
}
```

`kestrel/tests/programs/div_zero.kes` (the division is on line 5):
```
// expect-exit: 101
// expect-stderr: division by zero on line 5
main() {
    var d = 0
    print(10 / d)
}
```

- [ ] **Step 3: Write the failing harness**

`kestrel/cli/tests/programs.rs`:
```rust
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// What a test program declares about itself in `// expect...` comments.
struct Expect {
    stdout: String,
    exit: i32,
    stderr: Vec<String>,
}

fn expectations(src: &str) -> Expect {
    let mut lines = Vec::new();
    let mut exit = 0;
    let mut stderr = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("// expect-exit:") {
            exit = rest.trim().parse().expect("expect-exit number");
        } else if let Some(rest) = line.strip_prefix("// expect-stderr:") {
            stderr.push(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("// expect:") {
            lines.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        }
    }
    let stdout = lines.iter().map(|l| format!("{l}\n")).collect();
    Expect { stdout, exit, stderr }
}

pub fn programs() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/programs");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "kes"))
        .collect();
    files.sort();
    files
}

fn kestrel(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kestrel")).args(args).output().expect("run kestrel")
}

/// Compares one run against the program's expectations; describes any mismatch.
fn compare(name: &str, mode: &str, out: &Output, exp: &Expect) -> Option<String> {
    let stdout = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
    let stderr = String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n");
    let code = out.status.code().unwrap_or(-1);
    let ok = stdout == exp.stdout && code == exp.exit && exp.stderr.iter().all(|s| stderr.contains(s.as_str()));
    (!ok).then(|| {
        format!(
            "{name} [{mode}]: exit {code} (expected {})\n--- stdout ---\n{stdout}--- expected ---\n{}--- stderr ---\n{stderr}",
            exp.exit, exp.stdout
        )
    })
}

#[test]
fn programs_run_via_jit() {
    let mut failures = Vec::new();
    for path in programs() {
        let src = fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let out = kestrel(&["run", path.to_str().unwrap()]);
        failures.extend(compare(&name, "jit", &out, &expectations(&src)));
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn check_command_reports_success() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/programs/hello.kes");
    let out = kestrel(&["check", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cd kestrel && cargo test -p kestrel-cli`
Expected: compile error (no `src/main.rs`).

- [ ] **Step 5: Implement `driver.rs`**

```rust
use kestrel_syntax::{parse, render, render_short, Diagnostic, LineIndex};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DiagStyle {
    Full,
    Short,
}

/// Parses, checks and lowers `src`, printing every diagnostic to stderr.
/// Returns `None` when there are errors. Checking is skipped if parsing failed.
pub fn compile(path: &str, src: &str, style: DiagStyle) -> Option<kestrel_kir::Module> {
    let (program, mut diags) = parse(src);
    let tables = if diags.iter().any(Diagnostic::is_error) {
        None
    } else {
        let (tables, check_diags) = kestrel_check::check(&program);
        diags.extend(check_diags);
        tables
    };
    report(path, src, &diags, style);
    let tables = tables?;
    Some(kestrel_kir::lower(&program, &tables, &LineIndex::new(src)))
}

fn report(path: &str, src: &str, diags: &[Diagnostic], style: DiagStyle) {
    for d in diags {
        match style {
            DiagStyle::Full => eprint!("{}", render(src, path, d)),
            DiagStyle::Short => eprintln!("{}", render_short(src, d)),
        }
    }
    let errors = diags.iter().filter(|d| d.is_error()).count();
    if errors > 0 && style == DiagStyle::Full {
        eprintln!("{errors} error{}", if errors == 1 { "" } else { "s" });
    }
}
```

- [ ] **Step 6: Implement `main.rs`**

```rust
mod driver;

use std::process::ExitCode;

use driver::DiagStyle;
use kestrel_codegen::{JitProgram, Options};

const USAGE: &str = "usage: kestrel <run|check> <file.kes> [--diagnostics=short]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let style = if args.iter().any(|a| a == "--diagnostics=short") { DiagStyle::Short } else { DiagStyle::Full };
    let positional: Vec<&str> = args.iter().filter(|a| !a.starts_with("--")).map(String::as_str).collect();
    let [cmd, path] = positional[..] else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("kestrel: can't read {path}: {e}");
            return ExitCode::from(2);
        }
    };
    match cmd {
        "check" | "run" => {}
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    }
    let Some(module) = driver::compile(path, &src, style) else { return ExitCode::from(1) };
    if cmd == "check" {
        return ExitCode::SUCCESS;
    }
    match JitProgram::compile(&module, Options::default()) {
        Ok(program) => ExitCode::from(program.run_main() as u8),
        Err(e) => {
            eprintln!("kestrel: internal compiler error: {e}");
            ExitCode::from(3)
        }
    }
}
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cd kestrel && cargo test -p kestrel-cli`
Expected: 2 passed. If a program fails, the assertion message shows its actual vs expected stdout, exit code and stderr.

- [ ] **Step 8: Try it by hand**

Run: `cd kestrel && cargo run -q -p kestrel-cli -- run tests/programs/fib.kes`
Expected:
```
0 1 1 2 3 5 8 13 21 34
6765
```

- [ ] **Step 9: Commit**

```bash
git add kestrel/Cargo.toml kestrel/cli kestrel/tests
git commit -m "Add kestrel CLI with run/check and the program test harness

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Object output, linking and `kestrel build`

**Files:**
- Create: `kestrel/codegen/src/object.rs`, `kestrel/cli/build.rs`, `kestrel/cli/src/link.rs`
- Modify: `kestrel/codegen/src/lib.rs` (export `emit_object`), `kestrel/cli/Cargo.toml` (find-msvc-tools on Windows), `kestrel/cli/src/main.rs` (replace whole file), `kestrel/cli/tests/programs.rs` (append), `kestrel/codegen/tests/jit.rs` (append)

**Interfaces:**
- Consumes: `define_module` (Task 8), `driver::compile` (Task 9).
- Produces: `kestrel_codegen::emit_object(&kir::Module, Options) -> Result<Vec<u8>, String>`: an object file whose exported C `main` calls `kestrel_rt_start(k_main)`. CLI: `kestrel build <file> [-o <exe>] [--release] [--diagnostics=short]`; default output is `<file stem><EXE_SUFFIX>` in the current directory; `--release` turns off overflow checks. Build-time env `KESTREL_RUNTIME_LIB` = absolute path of the runtime staticlib built by `cli/build.rs`.

**Why a build script:** `cargo test` does not produce `staticlib` outputs of workspace members (verified on this machine), so `cli/build.rs` runs a nested `cargo build -p kestrel-runtime --lib` into its own target directory and exports the resulting library path. The nested build uses a separate target dir, so it never waits on the outer build's lock.

- [ ] **Step 1: Write the failing tests**

Append to `kestrel/codegen/tests/jit.rs`:
```rust
#[test]
fn emits_an_object_file() {
    let src = "main() {\n print(1)\n}\n";
    let (p, _) = parse(src);
    let (t, _) = kestrel_check::check(&p);
    let m = kestrel_kir::lower(&p, &t.unwrap(), &LineIndex::new(src));
    let bytes = kestrel_codegen::emit_object(&m, Options::default()).expect("object");
    assert!(bytes.len() > 100);
}
```

Append to `kestrel/cli/tests/programs.rs`:
```rust
fn build(path: &Path, exe: &Path, extra: &[&str]) -> Output {
    let mut args = vec!["build", path.to_str().unwrap(), "-o", exe.to_str().unwrap()];
    args.extend_from_slice(extra);
    kestrel(&args)
}

#[test]
fn programs_run_as_executables() {
    let out_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("programs");
    fs::create_dir_all(&out_dir).unwrap();
    let mut failures = Vec::new();
    for path in programs() {
        let src = fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let exe = out_dir.join(path.file_stem().unwrap()).with_extension(std::env::consts::EXE_EXTENSION);
        let built = build(&path, &exe, &[]);
        if !built.status.success() {
            failures.push(format!("{name} [build]: {}", String::from_utf8_lossy(&built.stderr)));
            continue;
        }
        let out = Command::new(&exe).output().expect("run built exe");
        failures.extend(compare(&name, "exe", &out, &expectations(&src)));
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn release_builds_wrap_on_overflow() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/programs/overflow.kes");
    let exe = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("overflow_release")
        .with_extension(std::env::consts::EXE_EXTENSION);
    let built = build(&path, &exe, &["--release"]);
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let out = Command::new(&exe).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"), "before\nafter\n");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd kestrel && cargo test -p kestrel-codegen emits_an_object_file; cargo test -p kestrel-cli`
Expected: `emit_object` doesn't exist; the CLI rejects `build` (usage error), so the new CLI tests fail.

- [ ] **Step 3: Implement `codegen/src/object.rs`**

```rust
use cranelift_codegen::ir::{types, AbiParam, InstBuilder, Signature};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};
use kestrel_kir as kir;

use crate::{define_module, isa, Options};

/// Compiles `m` to a native object file whose C `main` runs the Kestrel `main`.
pub fn emit_object(m: &kir::Module, opts: Options) -> Result<Vec<u8>, String> {
    let builder = ObjectBuilder::new(isa::host_isa()?, "kestrel", cranelift_module::default_libcall_names())
        .map_err(|e| e.to_string())?;
    let mut module = ObjectModule::new(builder);
    let ids = define_module(&mut module, m, opts)?;
    define_c_main(&mut module, ids[m.main.0 as usize])?;
    module.finish().emit().map_err(|e| e.to_string())
}

/// `int main(int argc, char **argv) { return kestrel_rt_start(k_main); }`
fn define_c_main(module: &mut ObjectModule, kestrel_main: FuncId) -> Result<(), String> {
    let cc = module.isa().default_call_conv();
    let ptr = module.target_config().pointer_type();

    let mut start_sig = Signature::new(cc);
    start_sig.params.push(AbiParam::new(ptr));
    start_sig.returns.push(AbiParam::new(types::I32));
    let start = module
        .declare_function("kestrel_rt_start", Linkage::Import, &start_sig)
        .map_err(|e| e.to_string())?;

    let mut sig = Signature::new(cc);
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(ptr));
    sig.returns.push(AbiParam::new(types::I32));
    let main_id = module.declare_function("main", Linkage::Export, &sig).map_err(|e| e.to_string())?;

    let mut ctx = module.make_context();
    ctx.func.signature = sig;
    let mut fb_ctx = FunctionBuilderContext::new();
    let mut b = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
    let block = b.create_block();
    b.append_block_params_for_function_params(block);
    b.switch_to_block(block);
    b.seal_block(block);
    let main_ref = module.declare_func_in_func(kestrel_main, b.func);
    let entry = b.ins().func_addr(ptr, main_ref);
    let start_ref = module.declare_func_in_func(start, b.func);
    let call = b.ins().call(start_ref, &[entry]);
    let code = b.inst_results(call)[0];
    b.ins().return_(&[code]);
    b.finalize();
    module.define_function(main_id, &mut ctx).map_err(|e| format!("codegen failed for C main: {e:?}"))
}
```

In `kestrel/codegen/src/lib.rs`, add next to `pub mod jit;`:
```rust
pub mod object;
```
and next to `pub use jit::JitProgram;`:
```rust
pub use object::emit_object;
```

- [ ] **Step 4: Build the runtime staticlib from the CLI's build script**

Append to `kestrel/cli/Cargo.toml`:
```toml
[target.'cfg(windows)'.dependencies]
find-msvc-tools.workspace = true
```

`kestrel/cli/build.rs`:
```rust
use std::env;
use std::path::PathBuf;
use std::process::Command;

/// Builds kestrel-runtime as a staticlib (which `cargo test` doesn't do for
/// workspace members) and exports its path as KESTREL_RUNTIME_LIB.
fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let workspace = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("..");
    let target_dir = out.join("runtime-target");
    let release = env::var("PROFILE").unwrap() == "release";
    let windows = env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows";

    let mut cmd = Command::new(env::var("CARGO").unwrap());
    cmd.current_dir(&workspace)
        .args(["build", "-p", "kestrel-runtime", "--lib", "--target-dir"])
        .arg(&target_dir);
    if release {
        cmd.arg("--release");
    }
    // Keep the outer build's settings (profile, features, target dir) out of the nested one.
    for (key, _) in env::vars() {
        if key.starts_with("CARGO_") && key != "CARGO_HOME" {
            cmd.env_remove(&key);
        }
    }
    cmd.env_remove("RUSTFLAGS");
    let status = cmd.status().expect("start cargo for kestrel-runtime");
    assert!(status.success(), "building kestrel-runtime failed");

    let file = if windows { "kestrel_runtime.lib" } else { "libkestrel_runtime.a" };
    let lib = target_dir.join(if release { "release" } else { "debug" }).join(file);
    assert!(lib.exists(), "runtime staticlib missing at {}", lib.display());
    println!("cargo:rustc-env=KESTREL_RUNTIME_LIB={}", lib.display());
    println!("cargo:rerun-if-changed=../runtime/src");
    println!("cargo:rerun-if-changed=../runtime/Cargo.toml");
}
```

- [ ] **Step 5: Implement `cli/src/link.rs`**

```rust
use std::path::Path;
use std::process::Command;

const RUNTIME_LIB: &str = env!("KESTREL_RUNTIME_LIB");

/// Links an object file with the Kestrel runtime into an executable.
pub fn link(obj: &Path, exe: &Path) -> Result<(), String> {
    let mut cmd = linker()?;
    if cfg!(windows) {
        cmd.arg("/NOLOGO")
            .arg(format!("/OUT:{}", exe.display()))
            .arg(obj)
            .arg(RUNTIME_LIB)
            // What Rust's std needs (`rustc --print native-static-libs`).
            .args(["kernel32.lib", "ntdll.lib", "userenv.lib", "ws2_32.lib", "dbghelp.lib", "msvcrt.lib"])
            .args(["/NODEFAULTLIB:libcmt", "/SUBSYSTEM:CONSOLE"]);
    } else {
        cmd.arg(obj).arg(RUNTIME_LIB).arg("-o").arg(exe).args(["-lpthread", "-ldl", "-lm"]);
    }
    let out = cmd.output().map_err(|e| format!("couldn't start the linker: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "linking failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn linker() -> Result<Command, String> {
    // Never a bare `link.exe` from PATH: Git for Windows ships an unrelated `link`.
    find_msvc_tools::find("x86_64-pc-windows-msvc", "link.exe")
        .ok_or_else(|| "couldn't find the Microsoft linker; install Visual Studio Build Tools (C++)".to_string())
}

#[cfg(not(windows))]
fn linker() -> Result<Command, String> {
    Ok(Command::new("cc"))
}
```

- [ ] **Step 6: Replace `cli/src/main.rs`**

```rust
mod driver;
mod link;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use driver::DiagStyle;
use kestrel_codegen::{emit_object, JitProgram, Options};

const USAGE: &str = "usage: kestrel <run|check|build> <file.kes> [-o <exe>] [--release] [--diagnostics=short]";

struct Args {
    cmd: String,
    path: String,
    out: Option<String>,
    release: bool,
    style: DiagStyle,
}

fn parse_args(raw: Vec<String>) -> Option<Args> {
    let mut it = raw.into_iter();
    let (mut positional, mut out, mut release, mut style) = (Vec::new(), None, false, DiagStyle::Full);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" => out = Some(it.next()?),
            "--release" => release = true,
            "--diagnostics=short" => style = DiagStyle::Short,
            s if s.starts_with('-') => return None,
            _ => positional.push(a),
        }
    }
    let [cmd, path]: [String; 2] = positional.try_into().ok()?;
    matches!(cmd.as_str(), "run" | "check" | "build").then_some(Args { cmd, path, out, release, style })
}

fn main() -> ExitCode {
    let Some(args) = parse_args(std::env::args().skip(1).collect()) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let src = match std::fs::read_to_string(&args.path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("kestrel: can't read {}: {e}", args.path);
            return ExitCode::from(2);
        }
    };
    let Some(module) = driver::compile(&args.path, &src, args.style) else { return ExitCode::from(1) };
    let opts = Options { checked_arith: !args.release };
    let result = match args.cmd.as_str() {
        "check" => return ExitCode::SUCCESS,
        "run" => JitProgram::compile(&module, opts).map(|p| p.run_main()),
        _ => build(&module, opts, &args).map(|()| 0),
    };
    match result {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("kestrel: {e}");
            ExitCode::from(3)
        }
    }
}

fn build(module: &kestrel_kir::Module, opts: Options, args: &Args) -> Result<(), String> {
    let exe = match &args.out {
        Some(o) => PathBuf::from(o),
        None => {
            let stem = Path::new(&args.path).file_stem().ok_or("source path has no file name")?;
            PathBuf::from(stem).with_extension(std::env::consts::EXE_EXTENSION)
        }
    };
    let bytes = emit_object(module, opts)?;
    let obj = std::env::temp_dir().join(format!("kestrel-{}.o", std::process::id()));
    std::fs::write(&obj, bytes).map_err(|e| format!("can't write {}: {e}", obj.display()))?;
    let linked = link::link(&obj, &exe);
    let _ = std::fs::remove_file(&obj);
    linked
}
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cd kestrel && cargo test`
Expected: every test in the workspace passes, including `programs_run_via_jit`, `programs_run_as_executables` and `release_builds_wrap_on_overflow`. The first run takes longer while `cli/build.rs` builds the runtime staticlib.

If linking fails with unresolved symbols, compare the Windows library list in `link.rs` with the current output of: `cd kestrel && cargo rustc -p kestrel-runtime --lib --crate-type staticlib -- --print native-static-libs` and update the list to match.

- [ ] **Step 8: Measure the dev loop (record, don't optimise yet)**

Run (Git Bash):
```bash
cd kestrel && cargo build --release -p kestrel-cli
s=$(date +%s%N); ./target/release/kestrel run tests/programs/fib.kes > /dev/null; echo "run: $(( ($(date +%s%N)-s)/1000000 )) ms"
s=$(date +%s%N); ./target/release/kestrel build tests/programs/fib.kes -o fib.exe; echo "build: $(( ($(date +%s%N)-s)/1000000 )) ms"; rm -f fib.exe
```
Expected: `run` well under 50 ms, `build` a few hundred ms at most. Put both numbers in the commit message body.

- [ ] **Step 9: Commit**

```bash
git add kestrel/codegen kestrel/cli
git commit -m "Add object output, runtime linking and kestrel build

run: <N> ms, build: <M> ms for tests/programs/fib.kes (release CLI).

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 11: Error tests and full diagnostics

**Files:**
- Create: `kestrel/cli/tests/errors.rs`, and under `kestrel/tests/errors/`: `shadowing.kes`, `immutable.kes`, `types.kes`, `purity.kes`, `var_args.kes`, `no_effect.kes`, `parse.kes`, `missing_main.kes`, `unknown.kes`

**Interfaces:**
- Consumes: `kestrel check --diagnostics=short` output (`LINE:COL: error: MESSAGE` per line) and full rendering (Tasks 1, 9).
- Produces: error-test format used by every later plan: `// error <line>: <message substring>`, one per expected error. The harness requires every expected error to appear on that line **and** no other errors to appear, and exit code 1.

Line numbers in these files are 1-based and count the `// error` comment lines themselves. Don't reflow them.

- [ ] **Step 1: Write the error programs**

`kestrel/tests/errors/shadowing.kes`:
```
// error 4: `x` is already defined; Kestrel doesn't allow shadowing
main() {
    let x = 1
    if true { let x = 2 }
}
```

`kestrel/tests/errors/immutable.kes`:
```
// error 6: can't change `count`: it was declared with `let`
// error 10: can't change parameter `n`
main() {
    let count = 0
    print(count)
    count = 1
}

f(n: int) {
    n = 2
}
```

`kestrel/tests/errors/types.kes`:
```
// error 5: expected int, found bool
// error 6: `+` needs numbers, found int and bool
// error 7: expected bool, found int
main() {
    let a: int = true
    let b = 1 + true
    while 1 { }
}
```

`kestrel/tests/errors/purity.kes`:
```
// error 8: pure functions can't call `print`
// error 9: pure functions can only call pure functions; `log` isn't pure
log(x: int) {
    print(x)
}

pure noisy(x: int) -> int {
    print(x)
    log(x)
    x
}

main() {}
```

`kestrel/tests/errors/var_args.kes`:
```
// error 11: parameter `x` of `bump` is `var`, so the call must say so
// error 12: `a` is passed as `var` and also used in another argument of the same call
// error 13: parameter `y` of `bump` isn't `var`
// error 13: `a` is passed as `var` and also used in another argument of the same call
bump(var x: int, y: int) {
    x += y
}

main() {
    var a = 1
    bump(a, 1)
    bump(var a, a)
    bump(var a, var a)
}
```

`kestrel/tests/errors/no_effect.kes`:
```
// error 8: this line has no effect; did you mean to continue the line above?
// error 9: the result of pure function `sq` is unused, so this line does nothing
pure sq(x: int) -> int { x * x }

main() {
    var total = 1
    total = total
        - 2
    sq(3)
}
```

`kestrel/tests/errors/parse.kes`:
```
// error 4: expected a variable name, found `=`
// error 5: comparisons can't be chained; combine them with `and`
main() {
    let = 5
    let ok = 1 < 2 < 3
}
```

`kestrel/tests/errors/missing_main.kes`:
```
// error 1: program needs a `main()` function
helper() {}
```

`kestrel/tests/errors/unknown.kes`:
```
// error 4: unknown name `y`
// error 5: unknown function `nope`
main() {
    let x = y
    nope()
}
```

- [ ] **Step 2: Write the harness**

`kestrel/cli/tests/errors.rs`:
```rust
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn errors_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/errors")
}

fn kestrel(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kestrel")).args(args).output().expect("run kestrel")
}

/// `// error <line>: <message>` comments.
fn expected_errors(src: &str) -> Vec<(u32, String)> {
    src.lines()
        .filter_map(|l| l.trim().strip_prefix("// error "))
        .map(|rest| {
            let (line, msg) = rest.split_once(':').expect("`// error <line>: <message>`");
            (line.trim().parse().expect("line number"), msg.trim().to_string())
        })
        .collect()
}

/// `LINE:COL: error: MESSAGE` lines from `--diagnostics=short`.
fn actual_errors(stderr: &str) -> Vec<(u32, String)> {
    stderr
        .lines()
        .filter_map(|l| {
            let (pos, msg) = l.split_once(": error: ")?;
            let line = pos.split(':').next()?.parse().ok()?;
            Some((line, msg.to_string()))
        })
        .collect()
}

#[test]
fn error_programs_report_exactly_the_expected_errors() {
    let mut files: Vec<PathBuf> = fs::read_dir(errors_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "kes"))
        .collect();
    files.sort();
    assert!(!files.is_empty());
    let mut failures = Vec::new();
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let src = fs::read_to_string(&path).unwrap();
        let out = kestrel(&["check", path.to_str().unwrap(), "--diagnostics=short"]);
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        if out.status.code() != Some(1) {
            failures.push(format!("{name}: exit {:?}, expected 1\n{stderr}", out.status.code()));
            continue;
        }
        let mut actual = actual_errors(&stderr);
        for (line, msg) in expected_errors(&src) {
            match actual.iter().position(|(l, m)| *l == line && m.contains(&msg)) {
                Some(i) => {
                    actual.remove(i);
                }
                None => failures.push(format!("{name}: missing error on line {line}: {msg}\n{stderr}")),
            }
        }
        for (line, msg) in actual {
            failures.push(format!("{name}: unexpected error on line {line}: {msg}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn full_diagnostics_show_location_source_and_caret() {
    let path = errors_dir().join("unknown.kes");
    let out = kestrel(&["check", path.to_str().unwrap()]);
    let stderr = String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n");
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr.contains("error: unknown name `y`"), "{stderr}");
    assert!(stderr.contains("unknown.kes:4:13"), "{stderr}");
    assert!(stderr.contains(" 4 |     let x = y\n"), "{stderr}");
    assert!(stderr.contains("^"), "{stderr}");
    assert!(stderr.ends_with("2 errors\n"), "{stderr}");
}
```

- [ ] **Step 3: Run the tests**

Run: `cd kestrel && cargo test -p kestrel-cli --test errors`
Expected: both pass. These exercise checker behaviour already built in Tasks 3-5; if one fails, the message says which file and line differ. Fix the compiler, not the expectation, unless the expectation contradicts the spec.

- [ ] **Step 4: Run the whole suite and check file sizes**

Run: `cd kestrel && cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all tests pass; no clippy warnings (fix any it reports).

Run (Git Bash): `cd kestrel && find . -name '*.rs' -not -path './target/*' | xargs wc -l | sort -n | tail -5`
Expected: no file above ~400 lines. If one is, split it by responsibility before committing.

- [ ] **Step 5: Commit**

```bash
git add kestrel/cli/tests/errors.rs kestrel/tests/errors
git commit -m "Add error-test harness and diagnostics tests

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## After this plan

2a ends with a working vertical slice: functions, scalars and control flow through JIT and exe, with both test harnesses. Next plans, each written once the previous one lands:

- **2b:** structs and enums with methods (`self` / `var self`), `match`, `vec2`/`vec3`, string values and interpolation. Also fix argument evaluation order when labels reorder arguments (evaluate in source order).
- **2c:** `[T]`, `array[T](n)`, `list[T]`, `for v in` / `for i, v in`, arenas (`arena { }`, `in x { }`, `Arena`), value semantics (moves, copies, drops), the arena escape check.
- **2d:** the voxel heightmap milestone and the Kestrel / Rust / optimised-Rust benchmark.
- **2e:** modules, `T?`, `map`, the remaining syntax-spec features, and deleting `kestrelc/`, `kestrel.js` and the old JS tests.
