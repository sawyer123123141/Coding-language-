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

const SYMBOLS: &[(&str, *const u8)] = &[("kestrel_make", k_make as *const u8), ("kestrel_dropped", k_dropped as *const u8)];

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
