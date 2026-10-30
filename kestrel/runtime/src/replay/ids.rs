//! The ids of the logged natives and the window modes a recording names.
//!
//! The native log's ids (`NativeId`), in module order, with the bytes of each result:
//!
//! | id | native | result |
//! |---|---|---|
//! | 1 | `window.frame` | `bool`, 1 |
//! | 2 | `window.delta` | `float`, 4 |
//! | 3 | `window.time` | `float`, 4 |
//! | 4 | `window.ticks_usec` | `i64`, 8 |
//! | 5 | `window.size` | `vec2`, 8 |
//! | 6 | `window.fps` | `int`, 4 |
//! | 7 | `draw.text_width` | `float`, 4 |
//! | 8 | `draw.load_texture` | the `Texture`'s width and height, 8 |
//! | 9 | `draw.make_texture` | the `Texture`'s width and height, 8 |
//! | 10 | `draw3d.make_mesh` | the `Mesh`'s vertex and triangle counts, 8 |
//! | 11 | `input.held` | `bool`, 1 |
//! | 12 | `input.pressed` | `bool`, 1 |
//! | 13 | `input.released` | `bool`, 1 |
//! | 14 | `input.mouse` | `vec2`, 8 |
//! | 15 | `input.wheel` | `float`, 4 |
//! | 16 | `input.mouse_delta` | `vec2`, 8 |
//! | 17 | `time.now` | `f64`, 8 |
//! | 18 | `time.unix` | `i64`, 8 |

/// A logged native: its byte in the native log.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeId {
    WindowFrame = 1,
    WindowDelta,
    WindowTime,
    WindowTicksUsec,
    WindowSize,
    WindowFps,
    DrawTextWidth,
    DrawLoadTexture,
    DrawMakeTexture,
    Draw3dMakeMesh,
    InputHeld,
    InputPressed,
    InputReleased,
    InputMouse,
    InputWheel,
    InputMouseDelta,
    TimeNow,
    TimeUnix,
}

impl NativeId {
    /// Every id, in order: `ALL[i]` has the byte `i + 1`.
    pub const ALL: [NativeId; 18] = [
        NativeId::WindowFrame,
        NativeId::WindowDelta,
        NativeId::WindowTime,
        NativeId::WindowTicksUsec,
        NativeId::WindowSize,
        NativeId::WindowFps,
        NativeId::DrawTextWidth,
        NativeId::DrawLoadTexture,
        NativeId::DrawMakeTexture,
        NativeId::Draw3dMakeMesh,
        NativeId::InputHeld,
        NativeId::InputPressed,
        NativeId::InputReleased,
        NativeId::InputMouse,
        NativeId::InputWheel,
        NativeId::InputMouseDelta,
        NativeId::TimeNow,
        NativeId::TimeUnix,
    ];

    /// The id whose byte is `byte`.
    pub fn from_u8(byte: u8) -> Option<NativeId> {
        NativeId::ALL.get(usize::from(byte).wrapping_sub(1)).copied()
    }

    /// The native's Kestrel name, for messages.
    pub fn name(self) -> &'static str {
        match self {
            NativeId::WindowFrame => "window.frame",
            NativeId::WindowDelta => "window.delta",
            NativeId::WindowTime => "window.time",
            NativeId::WindowTicksUsec => "window.ticks_usec",
            NativeId::WindowSize => "window.size",
            NativeId::WindowFps => "window.fps",
            NativeId::DrawTextWidth => "draw.text_width",
            NativeId::DrawLoadTexture => "draw.load_texture",
            NativeId::DrawMakeTexture => "draw.make_texture",
            NativeId::Draw3dMakeMesh => "draw3d.make_mesh",
            NativeId::InputHeld => "input.held",
            NativeId::InputPressed => "input.pressed",
            NativeId::InputReleased => "input.released",
            NativeId::InputMouse => "input.mouse",
            NativeId::InputWheel => "input.wheel",
            NativeId::InputMouseDelta => "input.mouse_delta",
            NativeId::TimeNow => "time.now",
            NativeId::TimeUnix => "time.unix",
        }
    }

    /// How many bytes its result takes in the log, after the id byte.
    pub fn width(self) -> usize {
        match self {
            NativeId::WindowFrame | NativeId::InputHeld | NativeId::InputPressed | NativeId::InputReleased => 1,
            NativeId::WindowDelta | NativeId::WindowTime | NativeId::WindowFps | NativeId::DrawTextWidth | NativeId::InputWheel => 4,
            _ => 8,
        }
    }
}

/// The window a recording was made with. A replay draws on the headless path whatever it was,
/// and prints the headless lines only for `Headless`, since only that recording has them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowMode {
    Real,
    Hidden,
    Headless,
    HeadlessQuiet,
}

impl WindowMode {
    pub fn as_str(self) -> &'static str {
        match self {
            WindowMode::Real => "real",
            WindowMode::Hidden => "hidden",
            WindowMode::Headless => "headless",
            WindowMode::HeadlessQuiet => "headless-quiet",
        }
    }

    pub fn parse(text: &str) -> Option<WindowMode> {
        [WindowMode::Real, WindowMode::Hidden, WindowMode::Headless, WindowMode::HeadlessQuiet].into_iter().find(|m| m.as_str() == text)
    }
}
