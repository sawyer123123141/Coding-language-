//! The server driven over standard input and output, as an editor drives it.

mod common;

use common::{project, url, Client};
use serde_json::{json, Value};

/// On disk, `main.kes` is empty and `geom.kes` has an error: the editor's unsaved text is what counts.
const MAIN: &str = "use geom\nmain() {\n    print(\"é🦅\", geom.area(2, true))\n}\n";
const GEOM_ON_DISK: &str = "pub area(w: int, h: int) -> int {\n    w * nope\n}\n";
const GEOM_FIXED: &str = "pub area(w: int, h: int) -> int {\n    w * h\n}\n";


fn messages(diags: &[Value]) -> Vec<String> {
    diags.iter().map(|d| d["message"].as_str().unwrap().to_string()).collect()
}

#[test]
fn diagnostics_hover_and_definition_over_stdio() {
    let dir = project("stdio", &[("main.kes", ""), ("geom.kes", GEOM_ON_DISK)]);
    let (main, geom) = (url(&dir.join("main.kes")), url(&dir.join("geom.kes")));
    let mut c = Client::start(json!({}));

    // The program's error is at its column in UTF-16 units (`é` is one unit and `🦅` two), and
    // the module's error (read from the disk) is shown in the module's file.
    c.open(&main, MAIN);
    let in_main = c.diagnostics(&main, |d| !d.is_empty());
    assert_eq!(messages(&in_main), ["expected int, found bool"]);
    assert_eq!(in_main[0]["range"]["start"], json!({"line": 2, "character": 30}), "{in_main:?}");
    assert_eq!(in_main[0]["range"]["end"], json!({"line": 2, "character": 34}), "{in_main:?}");
    assert_eq!(in_main[0]["severity"], json!(1));
    let in_geom = c.diagnostics(&geom, |d| !d.is_empty());
    assert_eq!(messages(&in_geom), ["unknown name `nope`"]);
    assert_eq!(in_geom[0]["range"]["start"], json!({"line": 1, "character": 8}));

    // The module opened with its error fixed (unsaved): the program reads the buffer, not the
    // disk, so the module's error goes.
    c.open(&geom, GEOM_FIXED);
    c.diagnostics(&geom, |d| d.is_empty());
    // The program's own error stays.
    c.diagnostics(&main, |d| d.len() == 1);

    // Fixed in the buffer: the error goes.
    let fixed = MAIN.replace("true", "3");
    c.change(&main, 2, &fixed);
    c.diagnostics(&main, |d| d.is_empty());

    // Hover on `area` (after the non-ASCII text on its line): the signature.
    let pos = json!({"line": 2, "character": 23});
    let h = c.request("textDocument/hover", json!({"textDocument": {"uri": main}, "position": pos}));
    let text = h["contents"]["value"].as_str().unwrap();
    assert!(text.contains("geom.area(w: int, h: int) -> int"), "{text}");

    // Go to definition: `area` in the module file.
    let d = c.request("textDocument/definition", json!({"textDocument": {"uri": main}, "position": pos}));
    assert!(d["uri"].as_str().unwrap().to_lowercase().ends_with("geom.kes"), "{d}");
    assert_eq!(d["range"], json!({"start": {"line": 0, "character": 4}, "end": {"line": 0, "character": 8}}));

    // While the text doesn't parse, hover answers from the last analysis that did.
    let broken = fixed.replace("main() {", "main() {\n    let = ");
    c.change(&main, 3, &broken);
    c.diagnostics(&main, |d| !d.is_empty());
    let pos = json!({"line": 3, "character": 23});
    let h = c.request("textDocument/hover", json!({"textDocument": {"uri": main}, "position": pos}));
    assert!(h["contents"]["value"].as_str().unwrap().contains("geom.area"), "{h}");
    c.stop();
}

const LENS: &str = "pub get(xs: list[int], k: int) -> int {
    xs[k]
}

main() {
    var xs = list[int]()
    xs.add(1)
    print(get(xs, 0))
}
";

#[test]
fn the_cost_lens_runs_on_open_and_save() {
    let dir = project("lens", &[("lens.kes", LENS)]);
    let file = url(&dir.join("lens.kes"));
    let mut c = Client::start(json!({"costLens": {"enabled": true, "show": "both"}}));
    c.open(&file, LENS);
    // Shown as diagnostics too (`show: both`): Information, on the line, with the fix.
    let diags = c.diagnostics(&file, |d| !d.is_empty());
    let bounds: Vec<&Value> = diags.iter().filter(|d| d["message"].as_str().unwrap().starts_with("bounds check")).collect();
    assert_eq!(bounds.len(), 1, "{diags:?}");
    assert_eq!(bounds[0]["severity"], json!(3));
    assert_eq!(bounds[0]["range"]["start"], json!({"line": 1, "character": 4}));
    assert!(bounds[0]["message"].as_str().unwrap().contains("@assume(k >= 0 and k < xs.len)"), "{diags:?}");

    let range = json!({"start": {"line": 0, "character": 0}, "end": {"line": 9, "character": 0}});
    let hints = c.request("textDocument/inlayHint", json!({"textDocument": {"uri": file}, "range": range}));
    let hints = hints.as_array().unwrap();
    let on_line_1 = hints.iter().find(|h| h["position"]["line"] == json!(1)).unwrap_or_else(|| panic!("{hints:?}"));
    assert_eq!(on_line_1["position"]["character"], json!(9));
    assert!(on_line_1["label"].as_str().unwrap().starts_with("bounds check: `xs[k]` may be outside `xs`"), "{on_line_1}");
    // `add` allocates.
    assert!(hints.iter().any(|h| h["position"]["line"] == json!(6) && h["label"].as_str().unwrap().contains("allocation")), "{hints:?}");

    // Hover lists the line's costs.
    let h = c.request("textDocument/hover", json!({"textDocument": {"uri": file}, "position": {"line": 1, "character": 5}}));
    assert!(h["contents"]["value"].as_str().unwrap().contains("bounds check"), "{h}");

    // Saved with the check removed: the lens follows (and asks for the hints again).
    let assumed = LENS.replace("    xs[k]", "    @assume(k >= 0 and k < xs.len)\n    xs[k]");
    std::fs::write(dir.join("lens.kes"), &assumed).unwrap();
    c.change(&file, 2, &assumed);
    c.notify("textDocument/didSave", json!({"textDocument": {"uri": file}}));
    let no_bounds = |d: &[Value]| !messages(d).iter().any(|m| m.starts_with("bounds check"));
    c.diagnostics(&file, no_bounds);

    // Switched off: no hints, and no lens diagnostics.
    c.notify("workspace/didChangeConfiguration", json!({"settings": {"kestrel": {"costLens": {"enabled": false}}}}));
    c.diagnostics(&file, |d| d.iter().all(|d| d["source"] != json!("kestrel cost")));
    let hints = c.request("textDocument/inlayHint", json!({"textDocument": {"uri": file}, "range": range}));
    assert_eq!(hints, json!([]));
    c.stop();
}

#[test]
fn definition_opens_a_built_in_module() {
    let src = "use draw\nmain() {\n    draw.clear(draw.BLACK)\n}\n";
    let dir = project("builtin", &[("pic.kes", src)]);
    let file = url(&dir.join("pic.kes"));
    let mut c = Client::start(json!({"costLens": {"enabled": false}}));
    c.open(&file, src);
    c.diagnostics(&file, |d| d.is_empty());
    let pos = json!({"line": 2, "character": 10});
    let d = c.request("textDocument/definition", json!({"textDocument": {"uri": file}, "position": pos}));
    let target = lsp_types::Url::parse(d["uri"].as_str().unwrap()).unwrap().to_file_path().unwrap();
    assert!(target.ends_with("draw.kes"), "{d}");
    let text = std::fs::read_to_string(&target).unwrap();
    let line = d["range"]["start"]["line"].as_u64().unwrap() as usize;
    assert!(text.lines().nth(line).unwrap().contains("clear("), "{d}");
    c.stop();
}
