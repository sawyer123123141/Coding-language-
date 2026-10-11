//! The window's state, and the switch between the two backends.
//!
//! A program has one window, used from the thread that runs `main`, so the
//! state is a `thread_local`. It is *not opened*, *open* or *closed*.
//!
//! - The **real** backend drives raylib.
//! - The **headless** backend (`KESTREL_HEADLESS=<n>`) makes no raylib call:
//!   `frame()` gives `true` `n` times, the clock is 60 frames a second, and
//!   the natives print what they would have drawn (see `headless.rs`), unless
//!   `KESTREL_HEADLESS_QUIET=1` turns the lines off.
//!
//! Both follow the same rules, so a program behaves alike on either:
//! `close()` only asks, and the next `frame()` closes the window and gives
//! `false`. After that `frame()` keeps giving `false`, drawing and
//! `window.screenshot` are errors, and the queries (`delta`, `time`, `ticks_usec`,
//! `size`, `fps`) give the values they had at the end, because raylib has nothing
//! left to ask. Input reads as nothing.
//!
//! The GPU objects (`gpu.rs`) are freed at the start of each frame, and all of
//! them when the window closes, in the real case before raylib closes.
//!
//! The `check_*` functions give an error message instead of failing, so tests
//! can see the message. The same functions without the prefix call
//! `kestrel_runtime::panic::fail_at_call` with it, which adds the line of the program's call.

use std::cell::RefCell;
use std::env;

use kestrel_runtime::panic::fail_at_call;

use crate::abi::V2;
use crate::gpu;
use crate::headless;
use crate::raylib as rl;
use crate::real;

/// Which backend a window uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Real,
    Headless,
}

/// The values of the window queries when raylib isn't asked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fixed {
    pub size: V2,
    pub time: f32,
    /// `window.ticks_usec()`: microseconds, exact however long the program has run.
    pub ticks: i64,
    pub delta: f32,
    pub fps: i32,
}

/// Where a query reads from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View {
    /// Ask raylib. `time_base` is raylib's `GetTime()` when the window opened:
    /// `window.time()` counts from there, not from when GLFW started.
    Real { time_base: f64 },
    /// These values: a headless window, or one that has closed.
    Fixed(Fixed),
}

/// What the environment says about the window, read at `window.open`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// `KESTREL_HEADLESS=<n>`: no window, and `n` frames.
    pub headless: Option<u64>,
    /// `KESTREL_HIDDEN=1`: a real window that isn't shown.
    pub hidden: bool,
    /// `KESTREL_HEADLESS_QUIET=1`: a headless window prints none of its lines.
    pub quiet: bool,
}

impl Settings {
    pub fn from_env() -> Result<Settings, String> {
        let var = |name| env::var(name).ok();
        Settings::parse(var("KESTREL_HEADLESS").as_deref(), var("KESTREL_HIDDEN").as_deref(), var("KESTREL_HEADLESS_QUIET").as_deref())
    }

    pub fn parse(headless: Option<&str>, hidden: Option<&str>, quiet: Option<&str>) -> Result<Settings, String> {
        let headless = match headless.map(str::trim) {
            None | Some("") => None,
            Some(n) => Some(
                n.parse::<u64>()
                    .map_err(|_| format!("KESTREL_HEADLESS must be a whole number of frames, like KESTREL_HEADLESS=3, not {n:?}"))?,
            ),
        };
        Ok(Settings { headless, hidden: hidden == Some("1"), quiet: quiet == Some("1") })
    }
}

enum Kind {
    Real { time_base: f64 },
    Headless { frames_to_run: u64 },
}

struct Window {
    kind: Kind,
    /// The size it was opened with: what a headless window reports.
    size: V2,
    /// How many frames have started: the `true`s `frame()` has given.
    frames: u64,
    close_requested: bool,
}

enum State {
    NotOpened,
    Open(Window),
    Closed(Fixed),
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State::NotOpened) };
}

fn before_open(name: &str) -> String {
    format!("{name} was called before window.open")
}

fn after_close(name: &str) -> String {
    format!("{name} was called after the window closed")
}

impl Window {
    fn mode(&self) -> Mode {
        match self.kind {
            Kind::Real { .. } => Mode::Real,
            Kind::Headless { .. } => Mode::Headless,
        }
    }

    /// A headless window's query values.
    fn headless_values(&self) -> Fixed {
        let fps = headless::FPS;
        let ticks = (self.frames.saturating_mul(1_000_000) / fps as u64) as i64;
        Fixed { size: self.size, time: self.frames as f32 / fps as f32, ticks, delta: 1.0 / fps as f32, fps }
    }

    /// Ends the previous frame and starts the next, or closes the window.
    /// `None` means a new frame has started; `Some` means the window has
    /// closed, with the values its queries keep giving.
    fn next_frame(&mut self) -> Option<Fixed> {
        match self.kind {
            Kind::Headless { frames_to_run } => {
                if self.close_requested || self.frames >= frames_to_run {
                    gpu::close_all(false);
                    return Some(self.headless_values());
                }
                gpu::drain(false);
            }
            Kind::Real { time_base } => {
                // SAFETY: raylib calls on the thread that opened the window, which is still open.
                unsafe {
                    if self.frames > 0 {
                        rl::EndDrawing();
                    }
                    if self.close_requested || rl::WindowShouldClose() {
                        let last = Fixed {
                            size: real::size(),
                            time: real::time(time_base),
                            ticks: real::ticks(time_base),
                            delta: rl::GetFrameTime(),
                            fps: rl::GetFPS(),
                        };
                        gpu::close_all(true);
                        rl::CloseWindow();
                        return Some(last);
                    }
                    gpu::drain(true);
                    rl::BeginDrawing();
                }
            }
        }
        self.frames += 1;
        None
    }
}

/// `window.open`: opens the window, or says why it can't.
pub fn open(settings: &Settings, width: i32, height: i32, title: &[u8]) -> Result<Mode, String> {
    STATE.with_borrow_mut(|state| {
        match state {
            State::Open(_) => return Err("the window is already open".to_string()),
            State::Closed(_) => return Err(after_close("window.open")),
            State::NotOpened => {}
        }
        if width <= 0 || height <= 0 {
            return Err(format!("window.open needs a width and height above 0, not {width} by {height}"));
        }
        let kind = match settings.headless {
            Some(frames_to_run) => {
                headless::set_quiet(settings.quiet);
                Kind::Headless { frames_to_run }
            }
            None => Kind::Real { time_base: real::init(settings, width, height, title)? },
        };
        let size = V2 { x: width as f32, y: height as f32 };
        let window = Window { kind, size, frames: 0, close_requested: false };
        let mode = window.mode();
        *state = State::Open(window);
        Ok(mode)
    })
}

/// `window.frame`: finishes the previous frame and starts the next. `false`
/// once the window has closed.
pub fn frame() -> Result<bool, String> {
    STATE.with_borrow_mut(|state| match state {
        State::NotOpened => Err(before_open("window.frame")),
        State::Closed(_) => Ok(false),
        State::Open(window) => match window.next_frame() {
            None => Ok(true),
            Some(last) => {
                *state = State::Closed(last);
                Ok(false)
            }
        },
    })
}

/// `window.close`: asks for the window to close at the next `frame()`.
/// Gives the mode when it asked, and `None` when the window had closed already.
pub fn close() -> Result<Option<Mode>, String> {
    STATE.with_borrow_mut(|state| match state {
        State::NotOpened => Err(before_open("window.close")),
        State::Closed(_) => Ok(None),
        State::Open(window) => {
            window.close_requested = true;
            Ok(Some(window.mode()))
        }
    })
}

/// For a native that draws (or saves a screenshot): which backend, if the window is open.
pub fn check_drawing(name: &str) -> Result<Mode, String> {
    STATE.with_borrow(|state| match state {
        State::NotOpened => Err(before_open(name)),
        State::Closed(_) => Err(after_close(name)),
        State::Open(window) => Ok(window.mode()),
    })
}

/// For a native that reads the window or the input: where to read from.
pub fn check_view(name: &str) -> Result<View, String> {
    STATE.with_borrow(|state| match state {
        State::NotOpened => Err(before_open(name)),
        State::Closed(last) => Ok(View::Fixed(*last)),
        State::Open(window) => match window.kind {
            Kind::Real { time_base } => Ok(View::Real { time_base }),
            Kind::Headless { .. } => Ok(View::Fixed(window.headless_values())),
        },
    })
}

/// `check_drawing`, failing the program with the message.
pub fn drawing(name: &str) -> Mode {
    check_drawing(name).unwrap_or_else(|msg| fail_at_call(&msg))
}

/// `check_view`, failing the program with the message.
pub fn view(name: &str) -> View {
    check_view(name).unwrap_or_else(|msg| fail_at_call(&msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `body` on a fresh thread, so it starts with no window.
    fn on_a_thread<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::spawn(body).join().unwrap()
    }

    fn headless(frames: u64) -> Settings {
        Settings { headless: Some(frames), hidden: false, quiet: false }
    }

    #[test]
    fn settings_come_from_the_environment_values() {
        assert_eq!(Settings::parse(None, None, None), Ok(Settings { headless: None, hidden: false, quiet: false }));
        assert_eq!(Settings::parse(Some("3"), None, None).unwrap().headless, Some(3));
        assert_eq!(Settings::parse(Some(" 100000 "), None, None).unwrap().headless, Some(100_000));
        assert_eq!(Settings::parse(Some("0"), None, None).unwrap().headless, Some(0));
        assert_eq!(Settings::parse(Some(""), None, None).unwrap().headless, None, "empty is unset");
        assert!(Settings::parse(None, Some("1"), None).unwrap().hidden);
        assert!(!Settings::parse(None, Some("0"), None).unwrap().hidden);
        assert!(!Settings::parse(None, Some("yes"), None).unwrap().hidden);
        assert!(Settings::parse(None, None, Some("1")).unwrap().quiet);
        assert!(!Settings::parse(None, None, Some("0")).unwrap().quiet);
        assert!(!Settings::parse(None, None, Some("true")).unwrap().quiet);
        let err = Settings::parse(Some("lots"), None, None).unwrap_err();
        assert!(err.starts_with("KESTREL_HEADLESS must be a whole number of frames"), "{err}");
        assert!(Settings::parse(Some("-1"), None, None).is_err());
    }

    #[test]
    fn calls_before_open_name_the_function() {
        on_a_thread(|| {
            assert_eq!(check_drawing("draw.circle"), Err("draw.circle was called before window.open".to_string()));
            assert_eq!(check_view("input.held"), Err("input.held was called before window.open".to_string()));
            assert_eq!(frame(), Err("window.frame was called before window.open".to_string()));
            assert_eq!(close(), Err("window.close was called before window.open".to_string()));
        });
    }

    #[test]
    fn a_second_open_is_refused() {
        on_a_thread(|| {
            assert_eq!(open(&headless(1), 10, 10, b"a"), Ok(Mode::Headless));
            assert_eq!(open(&headless(1), 10, 10, b"a"), Err("the window is already open".to_string()));
        });
    }

    #[test]
    fn a_window_needs_a_size() {
        on_a_thread(|| {
            let err = open(&headless(1), 0, 10, b"a").unwrap_err();
            assert_eq!(err, "window.open needs a width and height above 0, not 0 by 10");
            assert!(open(&headless(1), 10, -5, b"a").is_err());
            // Refusing a size left the window unopened.
            assert_eq!(open(&headless(1), 10, 10, b"a"), Ok(Mode::Headless));
        });
    }

    #[test]
    fn a_headless_window_runs_its_frames_then_closes() {
        on_a_thread(|| {
            open(&headless(2), 320, 200, b"hi").unwrap();
            assert_eq!(check_drawing("draw.clear"), Ok(Mode::Headless));
            let at = |frames: i64| {
                let (size, time, ticks) = (V2 { x: 320.0, y: 200.0 }, frames as f32 / 60.0, frames * 1_000_000 / 60);
                View::Fixed(Fixed { size, time, ticks, delta: 1.0 / 60.0, fps: 60 })
            };
            assert_eq!(check_view("window.time"), Ok(at(0)));
            assert_eq!(frame(), Ok(true));
            assert_eq!(check_view("window.time"), Ok(at(1)));
            assert_eq!(frame(), Ok(true));
            assert_eq!(check_view("window.time"), Ok(at(2)));
            assert_eq!(frame(), Ok(false));
            assert_eq!(frame(), Ok(false), "and it stays closed");
            // Closed: drawing is an error; the queries keep their last values.
            assert_eq!(check_drawing("draw.circle"), Err("draw.circle was called after the window closed".to_string()));
            assert_eq!(check_view("window.size"), Ok(at(2)), "and the clock stops at 33333 microseconds");
            assert_eq!(close(), Ok(None), "closing a closed window does nothing");
            assert_eq!(open(&headless(1), 10, 10, b"a"), Err("window.open was called after the window closed".to_string()));
        });
    }

    #[test]
    fn zero_frames_never_start() {
        on_a_thread(|| {
            open(&headless(0), 10, 10, b"a").unwrap();
            assert_eq!(frame(), Ok(false));
        });
    }

    #[test]
    fn close_takes_effect_at_the_next_frame() {
        on_a_thread(|| {
            open(&headless(100), 10, 10, b"a").unwrap();
            assert_eq!(frame(), Ok(true));
            assert_eq!(close(), Ok(Some(Mode::Headless)));
            assert_eq!(check_drawing("draw.circle"), Ok(Mode::Headless), "still open until the frame ends");
            assert_eq!(frame(), Ok(false));
            assert_eq!(frame(), Ok(false));
        });
    }

    #[test]
    fn closing_before_the_first_frame_runs_no_frame() {
        on_a_thread(|| {
            open(&headless(100), 10, 10, b"a").unwrap();
            close().unwrap();
            assert_eq!(frame(), Ok(false));
        });
    }
}
