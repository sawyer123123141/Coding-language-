use std::process::ExitCode;

fn main() -> ExitCode {
    let run = || match kestrel_lsp::serve_stdio() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kestrel-lsp: {e}");
            ExitCode::from(1)
        }
    };
    match std::thread::Builder::new().stack_size(kestrel_lsp::STACK_SIZE).spawn(run) {
        Ok(worker) => worker.join().unwrap_or(ExitCode::from(3)),
        // Couldn't reserve the stack; the default one still handles ordinary programs.
        Err(_) => run(),
    }
}
