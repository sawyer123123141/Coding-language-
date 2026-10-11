//! The Kestrel particles demo (`kestrel/demos/particles/particles.kes`) ported to Rust, on the same
//! vendored raylib and the same bulk-drawing path, to compare the two honestly. See `../results.md`.
//!
//! Windows only (it calls a few Win32 functions for the pinning and CPU switches below).
//!
//! Build and run, from this folder (the window is ALWAYS hidden: there is no visible mode):
//!
//! ```text
//! cargo build --release
//! target/release/particles-rust --count 200000 --draw direct --update rayon --seconds 11
//! ```
//!
//! It prints one line a second like the demo's, with the same times (`update`, `draw` and `frame`
//! in ms per frame, `fps`), then extra columns: `copy`, `fill`, `upload` and `drawmesh`, the
//! parts of the draw time (see `bulk.rs`), and `moves`, how often a frame's draw changed the
//! logical CPU the thread ran on. Drop the first line (it holds start-up) and take the
//! median of the rest.
//!
//! Switches (all `--name value`):
//!
//! - `--count N`: particles, default 200000.
//! - `--draw direct | copy | dots`: `direct` fills the mesh's vertices straight from the
//!   particles; `copy` first copies the positions into a `Vec` like the Kestrel demo (the copy is
//!   draw time); `dots` draws one `DrawRectangleV` per particle (the demo's `BULK = false`).
//! - `--update rayon | rayon:N | pool:N | scope:N | single | none`: how the step runs. `rayon` is
//!   `par_iter_mut` on rayon's global pool; `pool:N` is a pool built like Kestrel's `par for`;
//!   `scope:N` starts N threads every frame with `std::thread::scope`.
//! - `--seconds S`: run time, default 11.
//! - `--pow particle | hoisted`: `particle` calls `pow` for every particle, as the demo does
//!   (default); `hoisted` calls it once a frame.
//! - `--gap sleep:US | spin:US`: waits that long on the main thread after the update and before the
//!   draw (not counted in either), to see whether the draw is slow right after the update.
//! - `--upload gl | gl-nodraw | gl-drawonly | memcpy | skip`: what happens to the vertices after the
//!   fill: the real upload and draw (default), only the upload, only the draw (of the first
//!   frame's vertices), a plain `memcpy` into another `Vec`, or nothing. Takes the GPU out of the draw.
//! - `--finish yes`: calls `glFinish` at the end of every frame (after the draw's timer stops), so
//!   the GPU is idle when the next frame starts. A way to see what the draw costs without the
//!   last frame's work still running.
//! - `--repeat N`: runs the update N times each frame and reports the time of one. Keeps the cores busy,
//!   so the update's own cost can be read even when the frame is held to 60 fps.
//! - `--pin-main HEX | --pin-process HEX`: an affinity mask for the main thread or the process,
//!   set before the window opens. `--pin-late HEX` sets the main thread's after the first second.
//!
//! The simulation is the demo's with `target` fixed at the window centre (640, 360), as the measured
//! copies of the demo have it, and with no Space key. The start state comes from a fixed seed.

mod bulk;
mod sim;

use std::ffi::{c_uint, CString};
use std::time::{Duration, Instant};

use kestrel_window::raylib as rl;

use bulk::{Bulk, Upload};
use sim::{Frame, Particle, Update, HEIGHT, V2, WIDTH};

const FLAGS: c_uint = rl::FLAG_WINDOW_RESIZABLE | rl::FLAG_VSYNC_HINT | rl::FLAG_MSAA_4X_HINT | rl::FLAG_WINDOW_HIDDEN;
const TARGET: V2 = V2 {
    x: WIDTH / 2.0,
    y: HEIGHT / 2.0,
};
const DOT_SIZE: V2 = V2 { x: 2.0, y: 2.0 };
/// Seconds between the printed lines.
const REPORT: f64 = 1.0;

/// A colour the way `draw.rect` makes one: floats 0 to 1, rounded to 8 bits.
fn color(r: f32, g: f32, b: f32) -> rl::Color {
    let channel = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    rl::Color {
        r: channel(r),
        g: channel(g),
        b: channel(b),
        a: 255,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Draw {
    Direct,
    Copy,
    Dots,
}

struct Options {
    count: usize,
    draw: Draw,
    update: Update,
    seconds: f64,
    hoist_pow: bool,
    upload: Upload,
    finish: bool,
    repeat: u32,
    pin_late: Option<usize>,
    /// Wait after the update: sleep (true) or spin, for this long.
    gap: Option<(bool, Duration)>,
}

fn parse_args() -> Result<Options, String> {
    let mut o = Options {
        count: 200_000,
        draw: Draw::Direct,
        update: Update::Rayon,
        seconds: 11.0,
        hoist_pow: false,
        upload: Upload::Gl,
        finish: false,
        repeat: 1,
        pin_late: None,
        gap: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(name) = args.next() {
        let value = args.next().ok_or(format!("{name} needs a value"))?;
        let bad = |what: &str| format!("{name}: {what}, not {value}");
        match name.as_str() {
            "--count" => o.count = value.parse().map_err(|_| bad("a number"))?,
            "--draw" => {
                o.draw = match value.as_str() {
                    "direct" => Draw::Direct,
                    "copy" => Draw::Copy,
                    "dots" => Draw::Dots,
                    _ => return Err(bad("direct, copy or dots")),
                }
            }
            "--update" => o.update = Update::parse(&value)?,
            "--seconds" => o.seconds = value.parse().map_err(|_| bad("a number"))?,
            "--pow" => {
                o.hoist_pow = match value.as_str() {
                    "particle" => false,
                    "hoisted" => true,
                    _ => return Err(bad("particle or hoisted")),
                }
            }
            "--upload" => {
                o.upload = match value.as_str() {
                    "gl" => Upload::Gl,
                    "gl-nodraw" => Upload::GlNoDraw,
                    "gl-drawonly" => Upload::GlDrawOnly,
                    "memcpy" => Upload::Memcpy,
                    "skip" => Upload::Skip,
                    _ => return Err(bad("gl, gl-nodraw, gl-drawonly, memcpy or skip")),
                }
            }
            "--pin-late" => o.pin_late = Some(usize::from_str_radix(&value, 16).map_err(|_| bad("a hex mask"))?),
            "--repeat" => o.repeat = value.parse().map_err(|_| bad("a number"))?,
            "--finish" => o.finish = value == "yes",
            "--gap" => {
                let (kind, us) = value.split_once(':').ok_or(bad("sleep:US or spin:US"))?;
                let us: u64 = us.parse().map_err(|_| bad("sleep:US or spin:US"))?;
                o.gap = Some((kind == "sleep", Duration::from_micros(us)));
            }
            "--pin-main" => pin(false, usize::from_str_radix(&value, 16).map_err(|_| bad("a hex mask"))?),
            "--pin-process" => pin(true, usize::from_str_radix(&value, 16).map_err(|_| bad("a hex mask"))?),
            _ => return Err(format!("unknown switch {name}")),
        }
    }
    Ok(o)
}

#[link(name = "opengl32")]
extern "system" {
    fn glFinish();
}

fn pin(whole_process: bool, mask: usize) {
    extern "system" {
        fn GetCurrentThread() -> isize;
        fn GetCurrentProcess() -> isize;
        fn SetThreadAffinityMask(thread: isize, mask: usize) -> usize;
        fn SetProcessAffinityMask(process: isize, mask: usize) -> i32;
    }
    // SAFETY: plain Win32 calls on this thread and process.
    unsafe {
        if whole_process {
            SetProcessAffinityMask(GetCurrentProcess(), mask);
        } else {
            SetThreadAffinityMask(GetCurrentThread(), mask);
        }
    }
}

/// Waits `gap` on this thread: asleep, or spinning.
fn wait(sleep: bool, gap: Duration) {
    if sleep {
        std::thread::sleep(gap);
    } else {
        let end = Instant::now() + gap;
        while Instant::now() < end {
            std::hint::spin_loop();
        }
    }
}

/// `draw.text` with raylib's default font, as the demo's natives draw it.
fn text(s: &str, x: f32, y: f32) {
    let c = CString::new(s).expect("no NUL in the overlay text");
    // SAFETY: the window is open, on this thread; `c` lives through the call.
    unsafe {
        rl::DrawTextEx(
            rl::GetFontDefault(),
            c.as_ptr(),
            rl::Vector2 { x, y },
            20.0,
            2.0,
            color(0.9, 0.9, 0.95),
        )
    };
}

/// Rounds to two decimals for printing.
fn two(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn main() {
    let o = match parse_args() {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("{msg}\n(see the top of src/main.rs for the switches)");
            std::process::exit(2);
        }
    };
    let title = CString::new("Rust particles").unwrap();
    // SAFETY: nothing else has used raylib yet; `title` lives through the call.
    unsafe {
        rl::SetTraceLogLevel(rl::LOG_ERROR);
        rl::SetConfigFlags(FLAGS);
        rl::InitWindow(WIDTH as i32, HEIGHT as i32, title.as_ptr());
        assert!(rl::IsWindowReady(), "could not open a window");
        rl::SetExitKey(rl::KEY_NULL);
    }
    let (background, dot) = (color(0.05, 0.05, 0.09), color(1.0, 0.65, 0.2));
    let mut parts: Vec<Particle> = sim::spawn(o.count);
    // The positions the `copy` draw copies into, kept across frames.
    let mut positions = vec![V2 { x: 0.0, y: 0.0 }; if o.draw == Draw::Copy { parts.len() } else { 0 }];
    let mut bulk = Bulk::new(o.upload);

    // Sums over the current report interval, and the averages the last one printed.
    let mut frames = 0u32;
    let (mut update_sum, mut draw_sum, mut copy_sum) = (0.0f64, 0.0f64, 0.0f64);
    let (mut update_ms, mut draw_ms, mut frame_ms, mut fps) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let start = Instant::now();
    let mut interval_start = 0.0f64;
    let mut first = true;
    let mut frames_total = 0u32;

    loop {
        // SAFETY (for every block below): the window is open, on this thread.
        // `window.frame()`: finish the previous frame, start the next.
        unsafe {
            if first {
                rl::rlDrawRenderBatchActive();
                first = false;
            } else {
                rl::EndDrawing();
            }
            if rl::WindowShouldClose() || start.elapsed().as_secs_f64() >= o.seconds {
                break;
            }
            rl::BeginDrawing();
        }
        if let Some(mask) = o.pin_late.filter(|_| frames_total == 1000) {
            pin(false, mask);
        }
        frames_total += 1;
        let dt = unsafe { rl::GetFrameTime() };
        let keep = o.hoist_pow.then(|| 0.5f64.powf(dt as f64) as f32);

        let update_start = Instant::now();
        for _ in 0..o.repeat {
            o.update.run(&mut parts, Frame { dt, target: TARGET, keep });
        }
        let update_end = Instant::now();
        if let Some((sleep, gap)) = o.gap {
            wait(sleep, gap);
        }
        let draw_start = Instant::now();

        unsafe {
            rl::rlDrawRenderBatchActive();
            rl::ClearBackground(background);
            match o.draw {
                Draw::Direct => bulk.draw(&parts, |p| p.pos, DOT_SIZE, dot),
                Draw::Copy => {
                    let copy_start = Instant::now();
                    for (dst, p) in positions.iter_mut().zip(&parts) {
                        *dst = p.pos;
                    }
                    copy_sum += copy_start.elapsed().as_secs_f64();
                    bulk.draw(&positions, |p| *p, DOT_SIZE, dot);
                }
                Draw::Dots => {
                    for p in &parts {
                        rl::DrawRectangleV(
                            rl::Vector2 { x: p.pos.x, y: p.pos.y },
                            rl::Vector2 {
                                x: DOT_SIZE.x,
                                y: DOT_SIZE.y,
                            },
                            dot,
                        );
                    }
                }
            }
        }
        let draw_end = Instant::now();

        text(&format!("{} fps   {} particles", two(fps), parts.len()), 10.0, 10.0);
        text(
            &format!(
                "update {} ms   draw {} ms   frame {} ms",
                two(update_ms),
                two(draw_ms),
                two(frame_ms)
            ),
            10.0,
            36.0,
        );
        text("move the mouse to pull   space adds 1000", 10.0, HEIGHT - 28.0);

        if o.finish {
            // SAFETY: the window's GL context is current on this thread.
            unsafe { glFinish() };
        }

        frames += 1;
        update_sum += (update_end - update_start).as_secs_f64() / o.repeat as f64;
        draw_sum += (draw_end - draw_start).as_secs_f64();
        let elapsed = start.elapsed().as_secs_f64() - interval_start;
        if elapsed >= REPORT {
            let n = frames as f64;
            update_ms = update_sum / n * 1000.0;
            draw_ms = draw_sum / n * 1000.0;
            frame_ms = elapsed / n * 1000.0;
            fps = n / elapsed;
            let spent = bulk.take();
            let per_frame_ms = |d: Duration| two(d.as_secs_f64() / n * 1000.0);
            println!(
                "particles {} update {} ms draw {} ms frame {} ms fps {} copy {} ms fill {} ms upload {} ms drawmesh {} ms moves {}",
                parts.len(),
                two(update_ms),
                two(draw_ms),
                two(frame_ms),
                two(fps),
                two(copy_sum / n * 1000.0),
                per_frame_ms(spent.fill),
                per_frame_ms(spent.upload),
                per_frame_ms(spent.draw_mesh),
                two(spent.moves as f64 / n),
            );
            frames = 0;
            (update_sum, draw_sum, copy_sum) = (0.0, 0.0, 0.0);
            interval_start += elapsed;
        }
    }
    // SAFETY: the window is open, on this thread.
    unsafe { rl::CloseWindow() };
}
