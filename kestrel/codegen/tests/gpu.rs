//! `GpuHandle`: a struct holding one owns a GPU object. Dropping the struct (end of
//! scope, an overwrite, a list's drop) hands its id to the runtime's drop hook once.
//! This binary has its own hook and statics, so other tests can't disturb the counts;
//! its own tests take turns (`SERIAL`) and each starts from ids 1, 2, ...

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard};

mod common;

use common::{jit_natives_builtin, run_captured};

static DROPPED: Mutex<Vec<i64>> = Mutex::new(Vec::new());
static NEXT: AtomicI64 = AtomicI64::new(1);
static SERIAL: Mutex<()> = Mutex::new(());

fn record(id: i64) {
    DROPPED.lock().unwrap().push(id);
}
#[repr(C)]
struct Tex {
    id: i64,
    w: i32,
}
extern "C" fn k_make(out: *mut Tex, w: i32) {
    let id = NEXT.fetch_add(1, Ordering::SeqCst);
    // SAFETY: the result pointer comes first and has room for a `Tex`.
    unsafe { out.write_unaligned(Tex { id, w }) }
}
extern "C" fn k_dropped() -> i32 {
    DROPPED.lock().unwrap().len() as i32
}

/// A string argument: a pointer to this 16-byte header.
#[repr(C)]
struct KStr {
    ptr: *const u8,
    len: i64,
}
extern "C" fn k_take(t: *const Tex) -> i32 {
    // SAFETY: a struct argument is a pointer to a live `Tex`.
    unsafe { (*t).w }
}
extern "C" fn k_named(out: *mut Tex, name: *const KStr) {
    let id = NEXT.fetch_add(1, Ordering::SeqCst);
    // SAFETY: the result pointer comes first and has room for a `Tex`; the string is live.
    unsafe { out.write_unaligned(Tex { id, w: (*name).len as i32 }) }
}

const SYMBOLS: &[(&str, *const u8)] = &[
    ("kestrel_make", k_make as *const u8),
    ("kestrel_dropped", k_dropped as *const u8),
    ("kestrel_take", k_take as *const u8),
    ("kestrel_named", k_named as *const u8),
];

/// Takes the turn, forgets earlier runs and sets the hook.
fn start() -> MutexGuard<'static, ()> {
    let turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    DROPPED.lock().unwrap().clear();
    NEXT.store(1, Ordering::SeqCst);
    kestrel_runtime::gpu::set_drop_hook(record);
    turn
}

/// What `main` printed, how many objects were made, and the ids dropped, in order.
fn run(src: &str) -> (String, i64, Vec<i64>) {
    let p = jit_natives_builtin(src, SYMBOLS).expect("codegen");
    let out = run_captured(&p);
    (out, NEXT.load(Ordering::SeqCst) - 1, DROPPED.lock().unwrap().clone())
}

const PROGRAM: &str = "struct Tex {
    id: GpuHandle
    w: int
}
struct Holder {
    t: Tex
    n: int
}
native make(w: int) -> Tex
native dropped() -> int
width(t: Tex) -> int {
    t.w
}
build(w: int) -> Tex {
    make(w)
}
main() {
    let a = make(10)
    print(a, width(a))
    var b = make(20)
    b = make(30)
    print(dropped())
    var texs = list[Tex]()
    texs.add(build(40))
    texs.add(make(50))
    let moved = a
    var h = Holder(t: make(60), n: 1)
    h.t = make(70)
    print(dropped())
    let o: Tex? = make(80)
    print(moved.w, texs.len, h.n)
}
";

#[test]
fn gpu_objects_are_dropped_exactly_once() {
    let _turn = start();
    let (out, _, dropped) = run(PROGRAM);
    assert_eq!(out, "Tex(id: 1, w: 10) 10\n1\n2\n10 2 1\n");
    let mut dropped = dropped;
    dropped.sort_unstable();
    assert_eq!(dropped, (1..=8).collect::<Vec<i64>>(), "every object is dropped once");
}

/// Declarations the cases below share.
const HEAD: &str = "struct Tex {
    id: GpuHandle
    w: int
}
struct Pair {
    a: Tex
    b: Tex
}
enum Slot {
    Empty
    Full { t: Tex }
}
native make(w: int) -> Tex
native dropped() -> int
";

#[test]
fn every_way_out_of_a_scope_and_every_container_drops_each_object_once() {
    // (program after the declarations, what it prints). `dropped()` counts what was dropped
    // so far; the rest are dropped when `main` ends. Each case also checks that nothing is
    // dropped twice or left alive.
    let cases: &[(&str, &str)] = &[
        // `return` drops the other locals and moves the returned one out.
        (
            "first_wide(n: int) -> Tex {\n let tmp = make(0)\n var i = 0\n while i < n {\n  let t = make(i + 1)\n  if t.w > 2 {\n   return t\n  }\n  i += 1\n }\n make(99)\n}\n\
             main() {\n let t = first_wide(5)\n print(t.w, dropped())\n}\n",
            "3 3\n",
        ),
        // `continue` and `break` drop the loop body's locals.
        (
            "loop_drops() -> int {\n var kept = 0\n for i from 0 to 6 {\n  let t = make(i)\n  if i == 1 { continue }\n  if i == 4 { break }\n  kept += t.w\n }\n kept\n}\n\
             main() {\n print(loop_drops(), dropped())\n}\n",
            "5 5\n",
        ),
        // Structs, enum payloads and `if` values that hold one.
        (
            "main() {\n let p = Pair(a: make(1), b: make(2))\n let s = Slot.Full(make(3))\n let e = Slot.Empty\n let c = dropped() == 0\n let t = if c { make(4) } else { make(5) }\n print(p.a.w, t.w, dropped())\n}\n",
            "1 4 0\n",
        ),
        // Overwriting an optional, a list, a field of a field, and a `var` parameter drops the old value.
        ("main() {\n var o: Tex? = make(1)\n o = make(2)\n o = none\n print(dropped())\n}\n", "2\n"),
        ("main() {\n var xs = list[Tex]()\n xs.add(make(1))\n xs.add(make(2))\n xs = list[Tex]()\n print(dropped(), xs.len)\n}\n", "2 0\n"),
        (
            "struct Wrap {\n p: Pair\n}\nmain() {\n var w = Wrap(p: Pair(a: make(1), b: make(2)))\n w.p = Pair(a: make(3), b: make(4))\n print(dropped())\n}\n",
            "2\n",
        ),
        ("replace(var t: Tex) {\n t = make(5)\n}\nmain() {\n var t = make(1)\n replace(var t)\n print(t.w, dropped())\n}\n", "5 1\n"),
        (
            "struct Rig {\n t: Tex\n n: int\n\n swap(var self) {\n  self.t = make(self.n)\n }\n}\nmain() {\n var r = Rig(t: make(1), n: 9)\n r.swap()\n r.swap()\n print(r.t.w, dropped())\n}\n",
            "9 2\n",
        ),
        // Arrays, lists and loops over them own their elements; a handle prints as its number.
        ("main() {\n let xs = [make(1), make(2)]\n print(xs.len, dropped())\n}\n", "2 0\n"),
        (
            "main() {\n var xs = list[Tex]()\n xs.add(make(1))\n xs.add(make(2))\n var total = 0\n for t in xs { total += t.w }\n print(total, dropped())\n}\n",
            "3 0\n",
        ),
        ("main() {\n let t = make(5)\n print(t.id, t)\n}\n", "1 Tex(id: 1, w: 5)\n"),
        // A struct with heap memory beside the handle moves and drops like any other.
        (
            "struct Sprite {\n tex: Tex\n frames: list[int]\n name: string\n}\n\
             make_sprite(n: int) -> Sprite {\n var frames = list[int]()\n frames.add(n)\n let s = Sprite(tex: make(n), frames: frames, name: \"s{n}\")\n s\n}\n\
             pick(n: int) -> Sprite {\n var frames = list[int]()\n frames.add(n)\n let s = Sprite(tex: make(n), frames: frames, name: \"p{n}\")\n if n > 100 { return make_sprite(0) }\n return s\n}\n\
             main() {\n let s = make_sprite(1)\n print(s.frames.len, s.name, dropped())\n var sprites = list[Sprite]()\n sprites.add(make_sprite(2))\n sprites.add(pick(3))\n sprites.add(pick(300))\n let moved = s\n sprites.add(moved)\n arena {\n  var tmp = make_sprite(4)\n  tmp = make_sprite(5)\n  print(tmp.name, dropped())\n }\n print(sprites.len, dropped())\n}\n",
            "1 s1 0\ns5 2\n4 3\n",
        ),
    ];
    let _turn = start();
    for (body, expected) in cases {
        DROPPED.lock().unwrap().clear();
        NEXT.store(1, Ordering::SeqCst);
        let (out, made, mut dropped) = run(&format!("{HEAD}{body}"));
        assert_eq!(&out, expected, "output of:\n{body}");
        dropped.sort_unstable();
        assert_eq!(dropped, (1..=made).collect::<Vec<i64>>(), "each of the {made} objects is dropped once, in:\n{body}");
    }
}

/// Declarations for the temporaries below.
const TEMPS: &str = "struct Tex {
    id: GpuHandle
    w: int

    width(self) -> int {
        self.w
    }
}
struct Holder {
    t: Tex
    n: int
}
struct Pair {
    a: Tex
    b: Tex
}
enum Slot {
    Empty
    Full { t: Tex }
}
native make(w: int) -> Tex
native named(s: string) -> Tex
native take(t: Tex) -> int
native dropped() -> int
width(t: Tex) -> int {
    t.w
}
mk_pair() -> Pair {
    Pair(a: make(1), b: make(2))
}
slot(n: int) -> Slot {
    if n > 0 {
        Slot.Full(make(n))
    } else {
        Slot.Empty
    }
}
many() -> list[Tex] {
    var xs = list[Tex]()
    xs.add(make(1))
    xs.add(make(2))
    xs
}
";

#[test]
fn owning_values_that_are_never_stored_are_dropped_when_their_block_ends() {
    // A value made and not stored behaves like a hidden `let` of the block it is made in: still
    // alive inside the block (`dropped()` is 0), dropped when the block ends. Every object is
    // also dropped exactly once.
    let cases: &[(&str, &str, &str)] = &[
        ("a discarded call", "main() {\n if dropped() == 0 {\n  make(7)\n  print(dropped())\n }\n print(dropped())\n}\n", "0\n1\n"),
        ("a Kestrel function's argument", "main() {\n if dropped() == 0 {\n  print(width(make(5)), dropped())\n }\n print(dropped())\n}\n", "5 0\n1\n"),
        ("a native's argument", "main() {\n if dropped() == 0 {\n  print(take(make(5)), dropped())\n }\n print(dropped())\n}\n", "5 0\n1\n"),
        ("a field of a call", "main() {\n if dropped() == 0 {\n  print(make(5).w, dropped())\n }\n print(dropped())\n}\n", "5 0\n1\n"),
        ("a method's receiver", "main() {\n if dropped() == 0 {\n  print(make(5).width(), dropped())\n }\n print(dropped())\n}\n", "5 0\n1\n"),
        ("a printed value", "main() {\n if dropped() == 0 {\n  print(make(5))\n  print(dropped())\n }\n print(dropped())\n}\n", "Tex(id: 1, w: 5)\n0\n1\n"),
        ("a field of a construction", "main() {\n if dropped() == 0 {\n  print(Holder(t: make(1), n: 4).n, dropped())\n }\n print(dropped())\n}\n", "4 0\n1\n"),
        (
            "a matched value",
            "main() {\n if dropped() == 0 {\n  let n = match slot(1) {\n   Full s { s.t.w }\n   else { 0 }\n  }\n  print(n, dropped())\n }\n print(dropped())\n}\n",
            "1 0\n1\n",
        ),
        ("a matched value with nothing in it", "main() {\n let n = match slot(0) {\n  Full s { s.t.w }\n  else { 0 }\n }\n print(n, dropped())\n}\n", "0 0\n"),
        ("a discarded `if`", "main() {\n let c = dropped() == 0\n if c { make(1) } else { make(2) }\n print(dropped())\n}\n", "1\n"),
        (
            "a list looped over",
            "main() {\n var total = 0\n if dropped() == 0 {\n  for t in many() { total += t.w }\n  print(total, dropped())\n }\n print(dropped())\n}\n",
            "3 0\n2\n",
        ),
        (
            "an element and the length of a list",
            "main() {\n if dropped() == 0 {\n  print(many()[0].w, many().len, dropped())\n }\n print(dropped())\n}\n",
            "1 2 0\n4\n",
        ),
        (
            "a value made in each iteration of a loop",
            "main() {\n for i from 0 to 3 {\n  print(dropped())\n  print(width(make(i)))\n }\n print(dropped())\n}\n",
            "0\n0\n1\n1\n2\n2\n3\n",
        ),
        (
            "the right side of `and`",
            "main() {\n let b = dropped() == 0 and width(make(5)) == 5\n let skip = dropped() == 99 and width(make(6)) == 6\n print(b, skip, dropped())\n}\n",
            "true false 1\n",
        ),
        (
            "a native with text built in a scratch arena",
            "main() {\n let n = 3\n if dropped() == 0 {\n  print(width(named(\"t{n}\")))\n  named(\"u{n}\")\n  let kept = named(\"v{n}\")\n  print(kept.w, dropped())\n }\n print(dropped())\n}\n",
            "2\n2 0\n3\n",
        ),
        (
            "an owning field or element of a temporary, only read",
            "main() {\n if dropped() == 0 {\n  print(width(mk_pair().b), take(many()[1]), dropped())\n }\n print(dropped())\n}\n",
            "2 2 0\n4\n",
        ),
        (
            "a function's tail and `return`",
            "tail() -> int {\n width(make(3))\n}\nearly(c: bool) -> int {\n if c {\n  return width(make(4))\n }\n width(make(5))\n}\nmain() {\n print(tail(), early(true), early(false), dropped())\n}\n",
            "3 4 5 3\n",
        ),
    ];
    let _turn = start();
    let mut failures = Vec::new();
    for (what, body, expected) in cases {
        DROPPED.lock().unwrap().clear();
        NEXT.store(1, Ordering::SeqCst);
        let (out, made, mut dropped) = run(&format!("{TEMPS}{body}"));
        dropped.sort_unstable();
        if &out != expected {
            failures.push(format!("{what}: printed {out:?}, expected {expected:?}"));
        } else if dropped != (1..=made).collect::<Vec<i64>>() {
            failures.push(format!("{what}: made {made} objects, dropped {dropped:?}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Declarations for the `par for` cases. `make` is a `pure native`, so loop bodies may call it,
/// and the hook is thread-safe (a `Mutex`), as the runtime asks of any hook.
const PAR: &str = "struct Tex {
    id: GpuHandle
    w: int
}
pure native make(w: int) -> Tex
pure width(t: Tex) -> int {
    t.w
}
";

/// Runs `body` after the `par for` declarations on several threads (unless the shell chose a
/// number: the pool reads it once). Checks that each object made was dropped exactly once, and
/// gives what `main` printed and how many were made.
fn par_run(body: &str) -> (String, i64) {
    if std::env::var_os("KESTREL_THREADS").is_none() {
        std::env::set_var("KESTREL_THREADS", "4");
    }
    DROPPED.lock().unwrap().clear();
    NEXT.store(1, Ordering::SeqCst);
    let (out, made, mut dropped) = run(&format!("{PAR}{body}"));
    dropped.sort_unstable();
    assert_eq!(dropped, (1..=made).collect::<Vec<i64>>(), "each of the {made} objects is dropped once, in:\n{body}");
    (out, made)
}

#[test]
fn objects_made_and_dropped_in_par_for_iterations_are_each_dropped_once() {
    let _turn = start();
    // Each iteration makes two objects through a `pure` method: a local, and a temporary.
    let body = "struct Item {\n n: int\n seen: int = 0\n\n pure probe(var self) {\n  let t = make(self.n)\n  self.seen = t.w + width(make(self.n + 1000))\n }\n}\n\
                main() {\n var items = list[Item]()\n for i from 0 to 2000 {\n  items.add(Item(n: i))\n }\n par for it in items {\n  it.probe()\n }\n var total = 0\n for it in items {\n  total += it.seen\n }\n print(items.len, total)\n}\n";
    let total: i64 = (0..2000).map(|n| 2 * n + 1000).sum();
    assert_eq!(par_run(body), (format!("2000 {total}\n"), 4000));
}

#[test]
fn held_temporaries_in_a_par_for_body_and_a_continue_through_them_drop_once() {
    let _turn = start();
    // The body holds a temporary in its own block, and one in an `if` block that `continue`s;
    // `probe` makes a local and a temporary in a loop whose `continue` skips the temporary.
    let body = "struct Item {\n n: int\n seen: int = 0\n\n pure probe(var self) {\n  for i from 0 to 4 {\n   let t = make(i)\n   if i == 1 { continue }\n   self.seen += width(make(i + 10)) + t.w\n  }\n }\n}\n\
                main() {\n var items = list[Item]()\n for i from 0 to 300 {\n  items.add(Item(n: i))\n }\n par for it in items {\n  let w = width(make(it.n))\n  if it.n % 3 == 0 {\n   it.seen = width(make(it.n + 1000)) + w\n   continue\n  }\n  it.seen = w + width(make(it.n + 2000))\n  it.probe()\n }\n var total = 0\n for it in items {\n  total += it.seen\n }\n print(items.len, total)\n}\n";
    // `probe` adds (10 + 0) + (12 + 2) + (13 + 3) = 40.
    let total: i64 = (0..300).map(|n| if n % 3 == 0 { 2 * n + 1000 } else { 2 * n + 2040 }).sum();
    // Items divisible by 3 make 2 objects, the others 1 + 1 + 4 + 3 = 9.
    assert_eq!(par_run(body), (format!("300 {total}\n"), 100 * 2 + 200 * 9));
}
