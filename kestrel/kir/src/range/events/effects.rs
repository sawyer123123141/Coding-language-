//! What breaks `@prove(deterministic)`: a native (each in the words of what it reads), the
//! program's shared random generator, a call through a function value whose type isn't `pure`, and
//! a call through an interface whose method is neither `pure` nor promises `deterministic`. The
//! other contracts never read these events.

use super::{Walk, What};
use crate::range::summary::{Part, Reason};
use crate::ir::FuncIdx;

/// Something that makes a function's result depend on more than its arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// A native that reads the clock (`window.delta`).
    Clock,
    /// A native that reads the keyboard (`input.held`).
    Keyboard,
    /// A native that reads the mouse (`input.mouse`).
    Mouse,
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
            "window.delta" | "window.time" | "window.ticks_usec" | "window.fps" => Effect::Clock,
            "input.held" | "input.pressed" | "input.released" => Effect::Keyboard,
            "input.mouse" | "input.wheel" | "input.mouse_delta" => Effect::Mouse,
            _ => Effect::Window,
        }
    }

    /// What a chain says after the call: ", which reads the clock".
    pub fn tail(self) -> &'static str {
        match self {
            Effect::Clock => ", which reads the clock",
            Effect::Keyboard => ", which reads the keyboard",
            Effect::Mouse => ", which reads the mouse",
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
