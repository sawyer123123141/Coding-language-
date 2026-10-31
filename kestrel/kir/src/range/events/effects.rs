//! What breaks `@prove(deterministic)`: a native (each in the words of what it reads or writes), the
//! program's shared random generator, a call through a function value whose type isn't `pure`, and
//! a call through an interface whose method is neither `pure` nor promises `deterministic`. The
//! other contracts never read these events.

use super::{Walk, What};
use crate::range::summary::{Part, Reason};
use crate::ir::FuncIdx;

/// Something that makes a function's result depend on more than its arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// A native that reads the clock (`window.delta`, `time.now`).
    Clock,
    /// `time.sleep`, which waits on the clock.
    Sleep,
    /// A native of the `args` module, which reads the command line, or `env.get`, which reads the
    /// environment (`environment`).
    Process { environment: bool },
    /// A native that reads the keyboard (`input.held`).
    Keyboard,
    /// A native that reads the mouse (`input.mouse`).
    Mouse,
    /// A native of the `file` module: one that writes (`file.write`) or one that reads
    /// (`file.read`, and `file.exists`, `file.list_dir` and `file.error`, which read what is there).
    File { writes: bool },
    /// Any other native: it talks to the window.
    Window,
    /// `randf`, `randf_range`, `randi_range` or `rand_seed`.
    SharedRandom,
    /// A call through a function value whose type isn't `pure`.
    ImpureFnValue,
    /// A call through an interface value whose method (the interface's own, `Shape.area`) is
    /// neither `pure` nor promises `deterministic`.
    ImpureMethod(FuncIdx),
}

impl Effect {
    /// The effect of calling the native called `name` (as its module names it: `window.delta`).
    pub fn of_native(name: &str) -> Effect {
        match name {
            "window.delta" | "window.time" | "window.ticks_usec" | "window.fps" | "time.now" | "time.unix" => Effect::Clock,
            "time.sleep" => Effect::Sleep,
            "args.all" | "args.program" => Effect::Process { environment: false },
            "env.get" => Effect::Process { environment: true },
            "input.held" | "input.pressed" | "input.released" => Effect::Keyboard,
            "input.mouse" | "input.wheel" | "input.mouse_delta" => Effect::Mouse,
            "file.write" | "file.write_text" | "file.make_dir" | "file.remove" => Effect::File { writes: true },
            "file.read" | "file.read_text" | "file.exists" | "file.list_dir" | "file.error" => Effect::File { writes: false },
            _ => Effect::Window,
        }
    }

    /// What a chain says after the call: ", which reads the clock".
    pub fn tail(self) -> &'static str {
        match self {
            Effect::Clock => ", which reads the clock",
            Effect::Sleep => ", which waits on the clock",
            Effect::Process { environment: false } => ", which reads the command line",
            Effect::Process { environment: true } => ", which reads the environment",
            Effect::Keyboard => ", which reads the keyboard",
            Effect::Mouse => ", which reads the mouse",
            Effect::File { writes: false } => ", which reads a file",
            Effect::File { writes: true } => ", which writes a file",
            Effect::Window => ", which talks to the window, which a deterministic function can't depend on",
            Effect::SharedRandom => ", which uses the program's shared random generator",
            Effect::ImpureFnValue => ", which isn't `pure`",
            Effect::ImpureMethod(_) => ", which isn't `pure` and doesn't promise `deterministic`",
        }
    }
}

impl Walk<'_> {
    /// An effect at `line`: `via` is how the call is named ("calls `randf`").
    pub(super) fn effect(&mut self, line: u32, effect: Effect, via: String) {
        let r = Reason { direct: None, via, tail: effect.tail().to_string(), line: self.here(line), part: Part::Whole };
        self.push(line, What::Effect(effect, r));
    }
}

#[cfg(test)]
mod tests {
    use super::Effect;

    /// Every native of the modules that need no window, with its effect. A native added to one of
    /// them, or renamed, fails `every_windowless_native_has_its_own_effect` until it is pinned here,
    /// so none can fall through to `Effect::Window`.
    const PINNED: &[(&str, Effect)] = &[
        ("file.read", Effect::File { writes: false }),
        ("file.read_text", Effect::File { writes: false }),
        ("file.write", Effect::File { writes: true }),
        ("file.write_text", Effect::File { writes: true }),
        ("file.exists", Effect::File { writes: false }),
        ("file.list_dir", Effect::File { writes: false }),
        ("file.make_dir", Effect::File { writes: true }),
        ("file.remove", Effect::File { writes: true }),
        ("file.error", Effect::File { writes: false }),
        ("time.now", Effect::Clock),
        ("time.unix", Effect::Clock),
        ("time.sleep", Effect::Sleep),
        ("args.all", Effect::Process { environment: false }),
        ("args.program", Effect::Process { environment: false }),
        ("env.get", Effect::Process { environment: true }),
    ];

    /// The natives `src` (a module's `.kes` file) declares, as `module.name`.
    fn natives(module: &str, src: &str) -> Vec<String> {
        src.lines()
            .filter_map(|l| l.trim().strip_prefix("pub native "))
            .map(|rest| format!("{module}.{}", rest.split('(').next().unwrap_or_default().trim()))
            .collect()
    }

    #[test]
    fn every_windowless_native_has_its_own_effect() {
        let declared: Vec<String> = [
            natives("file", include_str!("../../../../driver/std/file.kes")),
            natives("time", include_str!("../../../../driver/std/time.kes")),
            natives("args", include_str!("../../../../driver/std/args.kes")),
            natives("env", include_str!("../../../../driver/std/env.kes")),
        ]
        .concat();
        let pinned: Vec<&str> = PINNED.iter().map(|(name, _)| *name).collect();
        assert_eq!(declared, pinned, "pin each native of `file`, `time`, `args` and `env` to its effect");
        for (name, effect) in PINNED {
            assert_eq!(Effect::of_native(name), *effect, "{name}");
        }
    }
}
