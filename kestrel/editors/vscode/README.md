# Kestrel for VS Code

Highlighting, brackets and comments for `.kes` files, and a client for `kestrel-lsp`, Kestrel's language server:

- **Errors as you type**, from the real parser and checker, in every file of the program (a module's error shows in the module's file). Unsaved text counts, in modules too.
- **Hover**: the type of a name or expression, a function's signature, a struct's fields, an enum's variants.
- **Go to definition**: functions, methods, structs, enums and their variants, fields, locals, constants, and items of other module files and of the built-in modules (`window`, `draw`, `draw3d`, `input`, opened from a copy in the temp folder).
- **The cost lens**: what `kestrel check --report` says about each line (every check range analysis left in, every allocation, every other way to fail and every call that isn't inlined), with the reason, and the change that removes it when there is a simple one: `@assume(k >= 0 and k < xs.len)` for a bounds check on a named index, `@assume(b != 0)` for a division by a name, `+%` (`-%`, `*%`) for an integer overflow that may be meant to wrap. It runs when a file is opened or saved (it compiles the program), shows as a hint at the end of the line (or as an Information diagnostic), and the line's hover lists it all. `@prove` contract errors and the `@assume`-removes-nothing warnings come at the same time.

## Install

From `kestrel/`:

```bash
cargo build --release -p kestrel-lsp
cd editors/vscode
npm install
npx @vscode/vsce package --skip-license --allow-missing-repository
code --install-extension kestrel-0.1.0.vsix
```

Then set `kestrel.server.path` to the server (`kestrel/target/release/kestrel-lsp.exe` on Windows), or put its folder on `PATH`, and reload the window (`Developer: Reload Window`). To try the extension without installing it, start VS Code with `code --extensionDevelopmentPath=<this folder>`: run from the repository, it finds the server in `kestrel/target/release` (or `target/debug`) by itself.

After rebuilding the server, run `Developer: Reload Window`.

## Settings

| Setting | Default | |
|---|---|---|
| `kestrel.server.path` | empty | The server. Empty: `kestrel-lsp` on `PATH`, else the repository's `target/release`, then `target/debug`. |
| `kestrel.costLens.enabled` | `true` | Run the cost lens on open and save. |
| `kestrel.costLens.build` | `checked` | `checked` shows the checks of `kestrel run` (overflow checks too); `release` those of `--release`. |
| `kestrel.costLens.show` | `hints` | `hints` (end of line), `diagnostics` (Information, also in Problems) or `both`. |

## How files are checked

Each open file is checked as the entry file of a program, with the modules it uses (`use geom` reads `geom.kes` next to it, from the open editor when it is open). A file with `pub` items and no `main` is checked as a module, so it doesn't need a `main`. As with `kestrel check`, a syntax error stops checking at that stage; while the file doesn't parse, hover and go to definition answer from the last version that did.

## Not built yet

Completion, rename, find references, signature help, formatting, semantic highlighting, and code actions that insert the lens's `@assume` for you. The lens is from the last save: until the next one its items move with the lines around an edit, and an edited line has none.
