# Memoization Cap + Heap-Allocated Large Array Literals Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix two real bugs found while building the Kestrel-vs-C benchmark suite: (1) `pure fn` memoization has no cap, so a function called with ever-distinct arguments grows a hash table unboundedly (measured: 3GB+ RAM, didn't finish in minutes for a 200M-call case); (2) array literals are unconditionally stack-allocated, so a large one (500,000 elements = 4MB) reliably crashes with `STATUS_STACK_OVERFLOW` on Windows.

**Architecture:** Fix 1 is entirely in the C runtime shim (`kestrelc_runtime.c`) — cap each memo slot's table growth at a fixed size, same "always an optimization, never a correctness dependency" posture the file already uses everywhere. Fix 2 is in `codegen.rs`: array literals ≤4KB stay stack-allocated (unchanged, zero overhead, covers the overwhelming majority of real programs); above 4KB, heap-allocate via `malloc` (declared as an external import, same pattern `printf` already uses); above 100MB, reject at `resolve.rs` with a clear compile error instead of ever reaching codegen. Both fixes are native-only — `wasm_codegen.rs` already bump-allocates arrays into wasm linear memory and has no memoization at all, confirmed unaffected by either bug.

**Tech Stack:** Rust (Cranelift), C (runtime shim).

## Global Constraints

- Native-only. No changes to `wasm_codegen.rs`.
- Fix 1: no eviction policy (LRU or similar) — a hard cap, extras silently miss. Matches every other cache in this codebase.
- Fix 2: no explicit free for heap-allocated arrays — matches the language's existing no-lifetime-tracking model (arrays are immutable, constructed once, no scope-based cleanup exists anywhere in the language today).
- Fix 2 thresholds: 4KB stack/heap cutoff, 100MB hard compile-time rejection cap.

---

### Task 1: Cap memoization table growth per slot

**Files:**
- Modify: `kestrelc/runtime/kestrelc_runtime.c`
- Test: `kestrelc/tests/integration.rs`

**Interfaces:**
- Produces: `KESTRELC_MEMO_MAX_SLOT_CAP` constant; `kestrelc_memo_store` stops growing (and stops inserting) once a slot's table capacity reaches this cap.

- [ ] **Step 1: Write the failing integration test**

Add to `kestrelc/tests/integration.rs`, anywhere near the other memoization tests:

```rust
#[test]
fn a_pure_fn_called_with_many_distinct_arguments_stays_bounded_and_fast() {
    // Regression test for the memoization pathology found while
    // building benchmarks/: a pure fn called with a different
    // argument every time never gets a cache hit, but was still
    // growing its memo table without bound -- 200,000,000 such calls
    // used 3GB+ RAM and didn't finish within several minutes before
    // this fix. This test uses a much smaller count (must complete
    // quickly in CI) but the count is well past KESTRELC_MEMO_MAX_SLOT_CAP
    // (65536), so it still exercises the capped-growth path, not just
    // the normal small-cache case.
    let scratch = scratch_dir("memo_many_distinct");
    let src_path = scratch.join("prog.kes");
    fs::write(
        &src_path,
        "pure fn f(x: i64) -> i64 { return (x * 2) % 1000000007; }\n\
         fn main() {\n\
         \x20   let i = 0;\n\
         \x20   let total = 0;\n\
         \x20   while (i < 500000) {\n\
         \x20       total = (total + f(i)) % 1000000007;\n\
         \x20       i = i + 1;\n\
         \x20   }\n\
         \x20   print(total);\n\
         }\n",
    )
    .unwrap();

    let out = Command::new(kestrelc_bin())
        .arg(&src_path)
        .current_dir(&scratch)
        .output()
        .expect("failed to run kestrelc");
    assert!(out.status.success(), "compile failed:\n{}", String::from_utf8_lossy(&out.stderr));

    let bin = scratch.join("prog");
    let start = std::time::Instant::now();
    let run = Command::new(&bin).output().expect("failed to run compiled binary");
    let elapsed = start.elapsed();
    assert!(run.status.success(), "compiled binary exited with failure");
    // 500,000 calls, even all-miss, should complete in well under 5
    // seconds once capped -- the pre-fix pathology made even 20,000,000
    // calls take multiple seconds and grow into gigabytes of RAM, so
    // this is a generous bound, not a tight performance assertion.
    assert!(elapsed.as_secs() < 5, "took {elapsed:?} -- memo table growth may be unbounded again");
}
```

- [ ] **Step 2: Run it to verify it currently would be slow/risky (informational — don't actually block on this in CI-scale terms, just confirm the test compiles and run it once manually with a time budget)**

```bash
cd kestrelc
export PATH="/c/Users/sawye/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cargo test a_pure_fn_called_with_many_distinct_arguments 2>&1 | tail -30
```

Expected: the test may pass already at 500,000 calls (the pathology gets dramatically worse at higher counts — 500,000 was chosen to keep this test fast in CI even before the fix, not to reliably fail pre-fix). The real verification is Step 6's manual re-run of the actual pathological scale from `results.md`.

- [ ] **Step 3: Add the cap constant and enforce it in `kestrelc_memo_store`**

In `kestrelc/runtime/kestrelc_runtime.c`, find:

```c
#define KESTRELC_MEMO_MAX_ARGS 16
#define KESTRELC_MEMO_INITIAL_CAP 16 // must be a power of 2
#define KESTRELC_MEMO_INITIAL_SLOT_CAPACITY 16
```

Replace with:

```c
#define KESTRELC_MEMO_MAX_ARGS 16
#define KESTRELC_MEMO_INITIAL_CAP 16 // must be a power of 2
#define KESTRELC_MEMO_INITIAL_SLOT_CAPACITY 16
// Hard cap on a single memoized function's own table capacity (not the
// outer per-function slot table -- see kestrelc_memo_ensure_slot_capacity
// for that one). Without this, a function called with a different
// argument on every call (e.g. applied over a loop counter) never gets
// a cache hit but still pays to grow this table forever -- measured:
// 200,000,000 such calls used 3GB+ RAM and didn't finish within several
// minutes. Once a slot's table hits this cap, kestrelc_memo_store stops
// growing it and stops inserting entirely -- new calls simply always
// miss and recompute, same "always an optimization, never a
// correctness dependency" posture every other cache in this file
// already has. 65536 entries * sizeof(kestrelc_memo_entry) (~144 bytes)
// is under 10MB per memoized function in the worst case -- a real,
// bounded ceiling instead of unbounded growth. A function whose calls
// genuinely repeat (the case memoization exists for) is essentially
// never affected: 65536 distinct arguments is already a very unusual
// amount of real cache diversity for a small pure function.
#define KESTRELC_MEMO_MAX_SLOT_CAP 65536
```

Find `kestrelc_memo_store`:

```c
void kestrelc_memo_store(int slot, const long long* args, int nargs, long long result) {
    if (slot < 0 || nargs < 0 || nargs > KESTRELC_MEMO_MAX_ARGS) {
        return;
    }
    if (!kestrelc_memo_ensure_slot_capacity(slot)) {
        return; // allocation failure growing the outer table; skip caching this entry
    }
    // Grow before inserting whenever occupied would reach half of
    // capacity — keeps probe chains short (load factor <= 0.5) no
    // matter how many distinct argument lists a function accumulates.
    if (kestrelc_memo_caps[slot] == 0 || kestrelc_memo_counts[slot] * 2 >= kestrelc_memo_caps[slot]) {
        kestrelc_memo_grow(slot);
        if (kestrelc_memo_caps[slot] == 0) {
            return; // allocation failed in kestrelc_memo_grow; skip caching this entry
        }
    }
    kestrelc_memo_entry e;
    for (int i = 0; i < nargs; i++) {
        e.args[i] = args[i];
    }
    e.nargs = nargs;
    e.result = result;
    e.occupied = 1;
    kestrelc_memo_raw_insert(kestrelc_memo_tables[slot], kestrelc_memo_caps[slot], &e);
    kestrelc_memo_counts[slot]++;
}
```

Replace with (adds one early-return check right after the existing arg-count guard):

```c
void kestrelc_memo_store(int slot, const long long* args, int nargs, long long result) {
    if (slot < 0 || nargs < 0 || nargs > KESTRELC_MEMO_MAX_ARGS) {
        return;
    }
    if (!kestrelc_memo_ensure_slot_capacity(slot)) {
        return; // allocation failure growing the outer table; skip caching this entry
    }
    // Already at the hard cap: stop growing and stop inserting new
    // entries entirely, rather than letting load factor climb toward
    // 1.0 (which would degrade every future lookup's probe chain even
    // for the already-cached entries). This function's cache is simply
    // "full" from here on -- new calls always miss and recompute.
    if (kestrelc_memo_caps[slot] >= KESTRELC_MEMO_MAX_SLOT_CAP) {
        return;
    }
    // Grow before inserting whenever occupied would reach half of
    // capacity — keeps probe chains short (load factor <= 0.5) no
    // matter how many distinct argument lists a function accumulates.
    if (kestrelc_memo_caps[slot] == 0 || kestrelc_memo_counts[slot] * 2 >= kestrelc_memo_caps[slot]) {
        kestrelc_memo_grow(slot);
        if (kestrelc_memo_caps[slot] == 0) {
            return; // allocation failed in kestrelc_memo_grow; skip caching this entry
        }
    }
    kestrelc_memo_entry e;
    for (int i = 0; i < nargs; i++) {
        e.args[i] = args[i];
    }
    e.nargs = nargs;
    e.result = result;
    e.occupied = 1;
    kestrelc_memo_raw_insert(kestrelc_memo_tables[slot], kestrelc_memo_caps[slot], &e);
    kestrelc_memo_counts[slot]++;
}
```

- [ ] **Step 4: Run the new test**

```bash
cd kestrelc
export PATH="/c/Users/sawye/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cargo test a_pure_fn_called_with_many_distinct_arguments 2>&1 | tail -30
```

Expected: PASS, well under the 5-second bound.

- [ ] **Step 5: Run the full test suite**

```bash
cargo test 2>&1 | tail -20
```

Expected: everything green, including existing memoization tests (a function whose calls genuinely repeat, well under 65536 distinct arguments, must be completely unaffected).

- [ ] **Step 6: Manual verification against this session's actual pathological scale**

```bash
cd /tmp
cat > memo_stress.kes << 'EOF'
pure fn square(x: i64) -> i64 {
    return x * x;
}
fn main() {
    let i = 0;
    let total = 0;
    while (i < 200000000) {
        total = (total + square(i)) % 1000000007;
        i = i + 1;
    }
    print(total);
}
EOF
export PATH="/c/Users/sawye/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
"C:\Users\sawye\OneDrive\Coding-language-\kestrelc\target\release\kestrelc.exe" memo_stress.kes
time ./memo_stress
```

Confirm: this now completes in a reasonable time (should be on the order of a few seconds, similar to the equivalent inlined-loop version measured at 0.66s-0.8s in `benchmarks/results.md` — the memoization overhead is now bounded, not eliminated, so expect somewhat slower than the pure inlined version, but nowhere near "3GB RAM, doesn't finish in minutes"). Report the actual measured time and peak memory (Task Manager or similar) in your report.

- [ ] **Step 7: Commit**

```bash
git add kestrelc/runtime/kestrelc_runtime.c kestrelc/tests/integration.rs
git commit -m "Cap memoization table growth per slot

A pure fn called with a different argument every time never gets a
cache hit but was still growing its memo table without bound --
measured 3GB+ RAM and several minutes for a 200,000,000-call case.
KESTRELC_MEMO_MAX_SLOT_CAP bounds each slot's table to 65536 entries
(under 10MB worst case); once hit, new entries are silently dropped
instead of growing further -- same 'always an optimization, never a
correctness dependency' posture every other cache in this file
already has."
```

---

### Task 2: Reject array literals too large to safely compile

**Files:**
- Modify: `kestrelc/src/resolve.rs`
- Test: `kestrelc/tests/integration.rs`

**Interfaces:**
- Produces: a new resolve.rs error for an `ExprKind::ArrayLit` with more than 12,500,000 elements (100MB / 8 bytes per `i64`).

- [ ] **Step 1: Write the failing integration test**

Add to `kestrelc/tests/integration.rs`:

```rust
#[test]
fn rejects_an_array_literal_too_large_to_safely_compile() {
    // Generates the literal mechanically -- 12,500,001 elements is
    // one past the 100MB (12,500,000 * 8 bytes) cap.
    let scratch = scratch_dir("huge_array_literal");
    let src_path = scratch.join("prog.kes");
    let mut src = String::from("fn main() {\n    let arr = [");
    for i in 0..12_500_001u32 {
        if i > 0 {
            src.push_str(", ");
        }
        src.push('0');
    }
    src.push_str("];\n    print(arr[0]);\n}\n");
    fs::write(&src_path, src).unwrap();

    let out = Command::new(kestrelc_bin())
        .arg(&src_path)
        .current_dir(&scratch)
        .output()
        .expect("failed to run kestrelc");
    assert!(!out.status.success(), "expected a compile error for an oversized array literal");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("too large to compile"), "got: {stderr}");
}
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cd kestrelc
export PATH="/c/Users/sawye/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cargo test rejects_an_array_literal_too_large_to_safely_compile 2>&1 | tail -30
```

Expected: FAIL — either the compile currently succeeds (no size check exists yet) or crashes instead of producing a clean error.

- [ ] **Step 3: Add the size check to `resolve.rs`'s `ArrayLit` handling**

Find in `kestrelc/src/resolve.rs`:

```rust
        ExprKind::ArrayLit(elems) => {
            for el in elems {
                resolve_expr(el, locals, struct_locals, fns, structs, span, errors);
            }
        }
```

Replace with:

```rust
        ExprKind::ArrayLit(elems) => {
            // 100MB / 8 bytes per i64 element. A safety net against a
            // literal so large it would itself cause compile-time or
            // runtime memory problems regardless of allocation
            // strategy -- not a meaningful limit for any real program
            // (see codegen.rs's heap-allocation threshold at 4KB for
            // where the *normal* large-array path kicks in well below
            // this cap).
            const MAX_ARRAY_LITERAL_ELEMENTS: usize = 12_500_000;
            if elems.len() > MAX_ARRAY_LITERAL_ELEMENTS {
                errors.push(KestrelcError::new(
                    ErrorKind::Resolve,
                    format!(
                        "array literal with {} elements is too large to compile (over 100MB) — this is almost certainly a mistake",
                        elems.len()
                    ),
                    e.span,
                ));
            }
            for el in elems {
                resolve_expr(el, locals, struct_locals, fns, structs, span, errors);
            }
        }
```

- [ ] **Step 4: Run the new test**

```bash
cd kestrelc
export PATH="/c/Users/sawye/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cargo test rejects_an_array_literal_too_large_to_safely_compile 2>&1 | tail -30
```

Expected: PASS. (This test writes and parses a ~35MB source file — expect it to take several seconds just for lexing/parsing; that's fine, it only runs once.)

- [ ] **Step 5: Run the full test suite**

```bash
cargo test 2>&1 | tail -20
```

Expected: everything green.

- [ ] **Step 6: Commit**

```bash
git add kestrelc/src/resolve.rs kestrelc/tests/integration.rs
git commit -m "Reject array literals over 100MB at compile time

A safety net against a literal large enough to cause compile-time or
runtime memory problems regardless of allocation strategy -- not a
real-program limit (codegen.rs's heap-allocation path, added next,
handles everything below this cap without a crash)."
```

---

### Task 3: Heap-allocate array literals above 4KB

**Files:**
- Modify: `kestrelc/src/codegen.rs`
- Test: `kestrelc/tests/integration.rs`

**Interfaces:**
- Consumes: nothing new from Tasks 1-2.
- Produces: `Codegen` gains a `malloc_id: FuncId`, threaded into `FnCodegen` the same way `printf_id` already is. A new `FnCodegen::alloc_array_buffer(&mut self, elem_count: usize) -> Value` helper returns a base pointer, stack-allocated at or below 4KB, heap-allocated (via `malloc`, never freed) above it. Both existing array-construction call sites (`gen_binding`'s `ArrayLit` arm, and `parallel_map`'s output buffer) are updated to use it instead of always creating a stack slot.

- [ ] **Step 1: Write the failing integration tests**

Add to `kestrelc/tests/integration.rs`:

```rust
#[test]
fn a_small_array_literal_at_the_stack_heap_boundary_still_works() {
    // 512 elements * 8 bytes = exactly 4096 (4KB) -- the boundary
    // itself, still expected to take the stack path unchanged.
    let scratch = scratch_dir("array_at_boundary");
    let src_path = scratch.join("prog.kes");
    let mut src = String::from("fn main() {\n    let arr = [");
    for i in 0..512u32 {
        if i > 0 {
            src.push_str(", ");
        }
        src.push_str(&i.to_string());
    }
    src.push_str("];\n    print(arr[0], arr[511]);\n}\n");
    fs::write(&src_path, src).unwrap();

    let out = Command::new(kestrelc_bin())
        .arg(&src_path)
        .current_dir(&scratch)
        .output()
        .expect("failed to run kestrelc");
    assert!(out.status.success(), "compile failed:\n{}", String::from_utf8_lossy(&out.stderr));

    let bin = scratch.join("prog");
    let run = Command::new(&bin).output().expect("failed to run compiled binary");
    assert!(run.status.success(), "compiled binary exited with failure");
    assert_eq!(native_stdout(&run), "0 511\n");
}

#[test]
fn a_large_array_literal_above_the_threshold_heap_allocates_instead_of_crashing() {
    // This is the direct regression test for the crash found in
    // benchmarks/: a 500,000-element i64 array literal (4MB) reliably
    // crashed with STATUS_STACK_OVERFLOW before this fix. 20,000
    // elements (160KB) is comfortably above the 4KB threshold, small
    // enough to keep this test fast.
    let scratch = scratch_dir("large_array_literal");
    let src_path = scratch.join("prog.kes");
    let mut src = String::from("fn main() {\n    let arr = [");
    for i in 0..20_000u32 {
        if i > 0 {
            src.push_str(", ");
        }
        src.push_str(&i.to_string());
    }
    src.push_str("];\n    let total = 0;\n    let i = 0;\n    while (i < 20000) {\n        total = total + arr[i];\n        i = i + 1;\n    }\n    print(total);\n}\n");
    fs::write(&src_path, src).unwrap();

    let out = Command::new(kestrelc_bin())
        .arg(&src_path)
        .current_dir(&scratch)
        .output()
        .expect("failed to run kestrelc");
    assert!(out.status.success(), "compile failed:\n{}", String::from_utf8_lossy(&out.stderr));

    let bin = scratch.join("prog");
    let run = Command::new(&bin).output().expect("failed to run compiled binary");
    assert!(run.status.success(), "compiled binary exited with failure (this is exactly the stack-overflow crash this task fixes)");
    // sum of 0..20000 = 20000*19999/2 = 199990000
    assert_eq!(native_stdout(&run), "199990000\n");
}
```

- [ ] **Step 2: Run them to verify the large-array one fails (crashes)**

```bash
cd kestrelc
export PATH="/c/Users/sawye/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cargo test a_large_array_literal_above_the_threshold 2>&1 | tail -30
cargo test a_small_array_literal_at_the_stack_heap_boundary 2>&1 | tail -30
```

Expected: the small/boundary test PASSES already (no change to that path yet). The large-array test FAILS — the compiled binary crashes (non-zero/abnormal exit status), confirming the bug this task fixes.

- [ ] **Step 3: Declare `malloc` as an external import in `Codegen::new`**

Find in `kestrelc/src/codegen.rs`:

```rust
        let mut printf_sig = Signature::new(call_conv);
        printf_sig.params.push(AbiParam::new(types::I64)); // format string pointer
        printf_sig.params.push(AbiParam::new(types::I64)); // one argument (0 used if unused)
        printf_sig.returns.push(AbiParam::new(types::I32));
        let printf_id = module
            .declare_function("printf", Linkage::Import, &printf_sig)
            .map_err(|e| KestrelcError::internal(ErrorKind::Codegen, e.to_string()))?;
```

Add right after it:

```rust
        // malloc(size: i64) -> i64 ptr — a real libc function, already
        // linked (kestrelc_runtime.c includes <stdlib.h> and the
        // runtime it's part of is always linked in). Used by
        // FnCodegen::alloc_array_buffer for array literals too large
        // to safely stack-allocate (see that method's own doc comment
        // for the threshold and why). Declared exactly like printf
        // above — a real ABI function, fixed non-variadic signature.
        let mut malloc_sig = Signature::new(call_conv);
        malloc_sig.params.push(AbiParam::new(types::I64)); // size
        malloc_sig.returns.push(AbiParam::new(types::I64)); // ptr
        let malloc_id = module
            .declare_function("malloc", Linkage::Import, &malloc_sig)
            .map_err(|e| KestrelcError::internal(ErrorKind::Codegen, e.to_string()))?;
```

- [ ] **Step 4: Thread `malloc_id` through `Codegen` and `FnCodegen`**

Find in `kestrelc/src/codegen.rs`:

```rust
pub struct Codegen {
    module: ObjectModule,
    fn_ids: HashMap<Symbol, FuncId>,
    printf_id: FuncId,
    pmap_id: FuncId,
    bounds_fail_id: FuncId,
```

Replace with:

```rust
pub struct Codegen {
    module: ObjectModule,
    fn_ids: HashMap<Symbol, FuncId>,
    printf_id: FuncId,
    pmap_id: FuncId,
    bounds_fail_id: FuncId,
    malloc_id: FuncId,
```

Find:

```rust
        Ok(Codegen {
            module,
            fn_ids: HashMap::new(),
            printf_id,
            pmap_id,
            bounds_fail_id,
```

Replace with:

```rust
        Ok(Codegen {
            module,
            fn_ids: HashMap::new(),
            printf_id,
            pmap_id,
            bounds_fail_id,
            malloc_id,
```

Find:

```rust
struct FnCodegen<'a> {
    builder: FunctionBuilder<'a>,
    vars: HashMap<Symbol, Slot>,
    fn_ids: &'a HashMap<Symbol, FuncId>,
    printf_id: FuncId,
    pmap_id: FuncId,
    bounds_fail_id: FuncId,
```

Replace with:

```rust
struct FnCodegen<'a> {
    builder: FunctionBuilder<'a>,
    vars: HashMap<Symbol, Slot>,
    fn_ids: &'a HashMap<Symbol, FuncId>,
    printf_id: FuncId,
    pmap_id: FuncId,
    bounds_fail_id: FuncId,
    malloc_id: FuncId,
```

Find:

```rust
            let mut fc = FnCodegen {
                builder,
                vars,
                fn_ids: &self.fn_ids,
                printf_id: self.printf_id,
                pmap_id: self.pmap_id,
                bounds_fail_id: self.bounds_fail_id,
```

Replace with:

```rust
            let mut fc = FnCodegen {
                builder,
                vars,
                fn_ids: &self.fn_ids,
                printf_id: self.printf_id,
                pmap_id: self.pmap_id,
                bounds_fail_id: self.bounds_fail_id,
                malloc_id: self.malloc_id,
```

- [ ] **Step 5: Add the `alloc_array_buffer` helper**

Add this method anywhere in `FnCodegen`'s `impl` block (e.g. right before `gen_binding`):

```rust
    /// Returns a base pointer for an N-element (i64) array buffer --
    /// stack-allocated at or below 4KB (the overwhelming majority of
    /// real array literals: the existing examples all use 4-6
    /// elements), heap-allocated via `malloc` above it. Never freed --
    /// matches the language's existing no-lifetime-tracking memory
    /// model (arrays are immutable, constructed once, no concept of
    /// scope-based cleanup exists anywhere in the language today), the
    /// same way a stack-allocated array's real lifetime today is
    /// already just "until the process exits" for anything reachable
    /// from `main`.
    ///
    /// Fixes a real crash: a 500,000-element (4MB) array literal
    /// reliably crashed with STATUS_STACK_OVERFLOW against Windows'
    /// default 1MB thread stack before this existed (see
    /// benchmarks/results.md). resolve.rs separately rejects anything
    /// over 100MB at compile time, so `elem_count` here is always
    /// already known to be sane.
    fn alloc_array_buffer(&mut self, elem_count: usize) -> Value {
        let size_bytes = elem_count * 8;
        const STACK_HEAP_THRESHOLD_BYTES: usize = 4096;
        if size_bytes <= STACK_HEAP_THRESHOLD_BYTES {
            let ss = self.builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                size_bytes as u32,
                3, // 8-byte (2^3) alignment for i64 elements
            ));
            self.builder.ins().stack_addr(types::I64, ss, 0)
        } else {
            let local_malloc = self.module.declare_func_in_func(self.malloc_id, self.builder.func);
            let size_val = self.builder.ins().iconst(types::I64, size_bytes as i64);
            let call = self.builder.ins().call(local_malloc, &[size_val]);
            self.builder.inst_results(call)[0]
        }
    }
```

- [ ] **Step 6: Use the helper at both array-construction call sites**

Find in `gen_binding`:

```rust
                let size_bytes = (elems.len() * 8) as u32;
                let ss = self.builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    size_bytes,
                    3, // 8-byte (2^3) alignment for i64 elements
                ));
                let base = self.builder.ins().stack_addr(types::I64, ss, 0);
                for (i, el) in elems.iter().enumerate() {
```

Replace with:

```rust
                let base = self.alloc_array_buffer(elems.len());
                for (i, el) in elems.iter().enumerate() {
```

Find (in the `parallel_map` arm):

```rust
                let size_bytes = (elem_count * 8) as u32;
                let out_slot = self.builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    size_bytes,
                    3,
                ));
                let out_base = self.builder.ins().stack_addr(types::I64, out_slot, 0);
```

Replace with:

```rust
                let out_base = self.alloc_array_buffer(elem_count);
```

- [ ] **Step 7: Build and fix any remaining errors**

```bash
cd kestrelc
export PATH="/c/Users/sawye/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cargo build --features native 2>&1 | tail -60
```

Fix any remaining errors (e.g. an unused `StackSlotData`/`StackSlotKind` import warning is fine to leave if those types are still used elsewhere in the file — check before removing anything).

- [ ] **Step 8: Run the new tests**

```bash
cargo test a_small_array_literal_at_the_stack_heap_boundary 2>&1 | tail -30
cargo test a_large_array_literal_above_the_threshold 2>&1 | tail -30
```

Expected: both PASS now.

- [ ] **Step 9: Run the full test suite**

```bash
cargo test 2>&1 | tail -20
```

Expected: everything green, including every existing array/parallel_map test (both call sites changed).

- [ ] **Step 10: Manual verification against this session's actual crashing case**

```bash
cd /tmp
awk 'BEGIN{srand(45); n=500000; s=""; for(i=0;i<n;i++){ if(i>0) s=s", "; s=s int(rand()*1000000); } print s}' > arr500k.txt
ARR=$(cat arr500k.txt)
echo "fn main() { let arr = [$ARR]; print(arr[0], arr[499999]); }" > stack_crash_repro.kes
export PATH="/c/Users/sawye/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
"C:\Users\sawye\OneDrive\Coding-language-\kestrelc\target\release\kestrelc.exe" stack_crash_repro.kes
./stack_crash_repro
echo "exit: $?"
```

Confirm: compiles and runs successfully now (exit 0, prints the two values), instead of crashing with `STATUS_STACK_OVERFLOW` as it did before this fix. Report the actual output in your report.

- [ ] **Step 11: Commit**

```bash
git add kestrelc/src/codegen.rs kestrelc/tests/integration.rs
git commit -m "Heap-allocate array literals above 4KB instead of crashing

Array literals were unconditionally stack-allocated. A 500,000-element
i64 literal (4MB) reliably crashed with STATUS_STACK_OVERFLOW against
Windows' default 1MB thread stack (see benchmarks/results.md).

Literals at or below 4KB (the overwhelming majority of real programs)
are unchanged -- still stack-allocated, zero overhead. Above that,
FnCodegen::alloc_array_buffer heap-allocates via malloc instead
(declared as an external import, same pattern printf already uses).
Never freed -- matches the language's existing no-lifetime-tracking
memory model. resolve.rs (previous commit) separately rejects
anything over 100MB at compile time as a safety net.

Both existing stack-allocation call sites (array-literal let-bindings,
parallel_map's output buffer) now go through the same helper."
```

---

## After this plan

Ship the way this session has been shipping everything: feature branch off `main`, merged back once tests are green.
