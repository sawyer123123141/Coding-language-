//! Kestrel's language server: diagnostics from the real parser and checker as you type, hover
//! with types and signatures, go to definition across module files, and the cost lens (what
//! `kestrel check --report` says about each line, on save).

mod config;
mod diagnostics;
mod docs;
pub mod find;
mod hints;
pub mod lens;
mod lower;
mod nav;
pub mod pos;
pub mod resolve;
mod server;
mod shift;
mod work;

pub use server::run;

/// The compiler recurses over the syntax tree, so the server runs on a stack far bigger than
/// the platform default (as `kestrel` does).
pub const STACK_SIZE: usize = 256 * 1024 * 1024;

/// Serves the editor over standard input and output until it shuts the server down.
pub fn serve_stdio() -> Result<(), String> {
    let (conn, io) = lsp_server::Connection::stdio();
    run(conn)?;
    io.join().map_err(|e| e.to_string())
}
