//! A Godot add-on that runs Kestrel. GDScript creates a `Kestrel`, loads a
//! `.kes` file, and calls its `pub` functions:
//!
//! ```gdscript
//! var k := Kestrel.new()
//! if k.load("res://voxels.kes"):
//!     var heights: PackedInt32Array = k.run("heights", [0, 0, 1337])
//! ```
//!
//! Files are compiled in memory and recompiled when they change on disk.

mod bridge;
mod worker;

use std::path::PathBuf;

use godot::classes::ProjectSettings;
use godot::prelude::*;
use kestrel_codegen::jit::Thunk;
use kestrel_codegen::{ExportInfo, JitProgram, Options};
use kestrel_driver::{DiagStyle, SourceFile};

use worker::{Job, Reply, Worker};

struct KestrelExtension;

#[gdextension]
unsafe impl ExtensionLibrary for KestrelExtension {}

/// A compiled program and what it can be asked to run.
struct Loaded {
    /// Owns the machine code the thunks point into.
    _program: JitProgram,
    exports: Vec<(ExportInfo, Thunk)>,
    files: Vec<SourceFile>,
}

/// Loads a Kestrel program and runs its `pub` functions.
#[derive(GodotClass)]
#[class(base = RefCounted)]
pub struct Kestrel {
    base: Base<RefCounted>,
    path: Option<PathBuf>,
    loaded: Option<Loaded>,
    worker: Option<Worker>,
}

#[godot_api]
impl IRefCounted for Kestrel {
    fn init(base: Base<RefCounted>) -> Self {
        Kestrel { base, path: None, loaded: None, worker: None }
    }
}

#[godot_api]
impl Kestrel {
    /// Compiles `path` (`res://...` or a file path) and the modules it uses.
    /// Errors are printed; returns whether it worked.
    #[func]
    fn load(&mut self, path: GString) -> bool {
        let path = ProjectSettings::singleton().globalize_path(&path).to_string();
        self.path = Some(PathBuf::from(path));
        self.compile()
    }

    /// Calls the `pub` function `name` with `args`. Recompiles first if a
    /// source file changed. Returns its result (`null` for none, or when it
    /// fails; failures are printed as errors).
    #[func]
    fn run(&mut self, name: GString, args: VarArray) -> Variant {
        let _ = self.reload_if_changed();
        let name = name.to_string();
        match self.call(&name, &args) {
            Ok(v) => v,
            Err(msg) => {
                godot_error!("Kestrel: {msg}");
                Variant::nil()
            }
        }
    }

    /// Recompiles if a source file changed; returns whether a new version
    /// is now loaded (so a scene can rebuild what it made with the old one).
    #[func]
    fn refresh(&mut self) -> bool {
        self.reload_if_changed()
    }

    /// The names of the functions `run` can call.
    #[func]
    fn functions(&self) -> PackedStringArray {
        let mut names = PackedStringArray::new();
        for (info, _) in self.loaded.iter().flat_map(|l| &l.exports) {
            if runnable(info).is_ok() {
                names.push(&GString::from(info.name.as_str()));
            }
        }
        names
    }
}

/// Whether Godot can call `info`'s function, or why not.
fn runnable(info: &ExportInfo) -> Result<(), String> {
    for (i, p) in info.params.iter().enumerate() {
        if let Some(t) = bridge::unsupported(p) {
            return Err(format!("`{}` can't be run from Godot: parameter {} is {t}", info.name, i + 1));
        }
    }
    if let Some(t) = info.ret.as_ref().and_then(bridge::unsupported) {
        return Err(format!("`{}` can't be run from Godot: it returns {t}", info.name));
    }
    Ok(())
}

impl Kestrel {
    fn compile(&mut self) -> bool {
        let Some(path) = self.path.clone() else { return false };
        let (compiled, diagnostics) = kestrel_driver::compile_file(&path, DiagStyle::Full, false);
        let Some(compiled) = compiled else {
            godot_error!("{diagnostics}");
            return false;
        };
        if !diagnostics.is_empty() {
            godot_warn!("{diagnostics}");
        }
        match JitProgram::compile(&compiled.module, Options::default()) {
            Ok(program) => {
                let exports = program.exports();
                self.loaded = Some(Loaded { _program: program, exports, files: compiled.files });
                true
            }
            Err(e) => {
                godot_error!("Kestrel: internal compiler error: {e}");
                false
            }
        }
    }

    /// Hot reload: a changed file is recompiled. If that fails, the errors
    /// are printed and the previous version keeps running. Returns whether
    /// a new version was loaded.
    fn reload_if_changed(&mut self) -> bool {
        let changed = self.loaded.as_ref().is_some_and(|l| l.files.iter().any(SourceFile::changed));
        if !changed {
            return false;
        }
        let old = self.loaded.take();
        if self.compile() {
            return true;
        }
        // Keep running the old code, but don't retry until the file changes again.
        self.loaded = old.map(|mut l| {
            l.files = l.files.iter().map(|f| SourceFile { modified: f.current(), ..f.clone() }).collect();
            l
        });
        false
    }

    fn call(&mut self, name: &str, args: &VarArray) -> Result<Variant, String> {
        let loaded = self.loaded.as_ref().ok_or("nothing is loaded; call `load` first")?;
        let (info, thunk) = loaded.exports.iter().find(|(i, _)| i.name == name).ok_or_else(|| {
            format!("there's no `pub` function `{name}` in the loaded file")
        })?;
        runnable(info)?;
        if args.len() != info.params.len() {
            return Err(format!("`{name}` takes {} argument(s), got {}", info.params.len(), args.len()));
        }
        let mut buf = vec![0u8; info.args_size as usize];
        let mut keep = Vec::new();
        for (i, ((ty, &at), v)) in info.params.iter().zip(&info.arg_offsets).zip(args.iter_shared()).enumerate() {
            bridge::write_arg(&v, ty, &mut buf, at as usize, &mut keep)
                .map_err(|e| format!("argument {} of `{name}` {e}", i + 1))?;
        }
        let job = Job { thunk: *thunk, args: buf, ret_size: info.ret_size as usize, heap: bridge::heap_of(info.ret.as_ref()) };
        let ret_ty = info.ret.clone();
        if self.worker.is_none() {
            self.worker = Some(Worker::start()?);
        }
        let reply = self.worker.as_ref().expect("just started").call(job);
        drop(keep);
        match reply {
            Some(Reply::Done { ret, heap, output }) => {
                print_output(&output);
                Ok(ret_ty.map_or(Variant::nil(), |t| bridge::read_ret(&t, &ret, &heap)))
            }
            Some(Reply::Failed { msg, output }) => {
                print_output(&output);
                self.worker = None;
                Err(format!("{msg} (in `{name}`)"))
            }
            None => {
                self.worker = None;
                Err(format!("the Kestrel thread stopped while running `{name}`"))
            }
        }
    }
}

/// Prints Kestrel's `print` output to Godot's output panel.
fn print_output(bytes: &[u8]) {
    for line in String::from_utf8_lossy(bytes).lines() {
        godot_print!("{line}");
    }
}
