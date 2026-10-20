//! The report as text: grouped by file, then function, then loop, one line per item. See the
//! range-analysis design spec, "The report".

use super::report::{display_name, FuncReport, Item, ItemKind, Owners, Report};
use super::show::number;
use super::sites::CheckKind;
use crate::ir::*;

/// Writes `r` (built from `m`) as the text `kestrel check --report` prints. Functions in files
/// where `user_file` (given a file index of `Module::files`) is false, the built-in modules, are
/// left out, and so are their counts.
pub fn render(r: &Report, m: &Module, user_file: &dyn Fn(u32) -> bool) -> String {
    let owners = Owners::new(m);
    let mut out = String::new();
    let mut file = None;
    let (mut left, mut removed, mut allocs, mut fails) = (0u32, 0u32, 0u32, 0u32);
    for f in r.funcs.iter().filter(|f| user_file(unpack_line(f.lines.0).1)) {
        let home = unpack_line(f.lines.0).1;
        if file != Some(home) {
            file = Some(home);
            out.push_str(m.files.get(home as usize).map_or("(no file name)", |s| s.as_str()));
            out.push('\n');
        }
        let host = Host { m, owners: &owners, file: home, func: f.func };
        out.push_str(&format!("  function `{}` (line {}): {}\n", f.name, unpack_line(f.lines.0).0, summary_text(f)));
        if f.gave_up && !f.never_called {
            out.push_str(&format!("    range analysis stopped early in `{}` ({}), so every check in it stays\n", f.name, f.stopped));
        }
        if !f.never_called {
            host.write_tree(f, 4, &mut out);
        }
        left += f.checks_left();
        removed += f.checks_removed();
        allocs += f.allocations();
        fails += f.fails();
    }
    out.push_str(&format!(
        "total: {} {}, {} removed, {} left; {}; {}\n",
        number(left as i128 + removed as i128),
        plural(left + removed, "check", "checks"),
        number(removed as i128),
        number(left as i128),
        count_text(allocs, "allocation", "allocations"),
        count_text(fails, "other way to fail", "other ways to fail"),
    ));
    out
}

fn plural(n: u32, one: &str, many: &str) -> String {
    if n == 1 { one } else { many }.to_string()
}

/// "no allocations", "1 allocation", "12 allocations".
fn count_text(n: u32, one: &str, many: &str) -> String {
    match n {
        0 => format!("no {many}"),
        1 => format!("1 {one}"),
        n => format!("{} {many}", number(n as i128)),
    }
}

/// What a function's header line says after its name.
fn summary_text(f: &FuncReport) -> String {
    if f.never_called {
        return "its own copy is never called; its checks are listed where it is inlined".to_string();
    }
    let (left, removed) = (f.checks_left(), f.checks_removed());
    let checks = match (left, removed) {
        (0, 0) => "no checks".to_string(),
        (0, r) => format!("no checks left ({} removed)", number(r as i128)),
        (1, r) => format!("1 check left ({} removed)", number(r as i128)),
        (n, r) => format!("{} checks left ({} removed)", number(n as i128), number(r as i128)),
    };
    let fails = match f.fails() {
        0 => String::new(),
        n => format!(", {}", count_text(n, "other way to fail", "other ways to fail")),
    };
    format!("{checks}, {}{fails}", count_text(f.allocations(), "allocation", "allocations"))
}

/// The label of an item's kind.
fn label(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Check(CheckKind::Overflow) => "overflow check",
        ItemKind::Check(CheckKind::Shift) => "shift check",
        ItemKind::Check(CheckKind::Division) => "division check",
        ItemKind::Check(CheckKind::Narrowing) => "narrowing check",
        ItemKind::Check(CheckKind::Bounds) => "bounds check",
        ItemKind::Allocation => "allocation",
        ItemKind::CanFail => "can fail",
        ItemKind::Call => "call",
    }
}

/// What is printed under a function: items, the loops they are in, and `par for` bodies.
enum Node<'a> {
    Item(&'a Item),
    Loop(u32, Vec<Node<'a>>),
    Par(u32, &'a FuncReport),
}

impl Node<'_> {
    /// The packed line to put it in order by.
    fn line(&self) -> u32 {
        match self {
            Node::Item(i) => i.line,
            Node::Loop(l, _) | Node::Par(l, _) => *l,
        }
    }
}

/// Where the lines are printed: the module, who owns which lines, and the function (in a file)
/// the lines are printed under.
struct Host<'a> {
    m: &'a Module,
    owners: &'a Owners,
    file: u32,
    func: FuncIdx,
}

impl Host<'_> {
    /// "line 41", "line 41 (in `density`)" or "geom.kes:41 (in `helper`)".
    fn place(&self, line: u32) -> String {
        let (n, file) = unpack_line(line);
        let base = match self.m.files.get(file as usize) {
            Some(name) if file != self.file && file != NO_FILE => format!("{name}:{n}"),
            _ => format!("line {n}"),
        };
        match self.owners.of(line) {
            Some(o) if o != self.func => format!("{base} (in `{}`)", display_name(self.m, o)),
            _ => base,
        }
    }

    fn write_tree(&self, f: &FuncReport, indent: usize, out: &mut String) {
        let mut root: Vec<Node> = Vec::new();
        for item in &f.items {
            let mut at = &mut root;
            for &l in &item.loops {
                at = open_loop(at, l);
            }
            at.push(Node::Item(item));
        }
        for (line, body) in &f.par_bodies {
            let mut at = &mut root;
            for &l in &body.loops {
                at = open_loop(at, l);
            }
            let i = at.iter().position(|n| n.line() > *line).unwrap_or(at.len());
            at.insert(i, Node::Par(*line, body));
        }
        self.write_nodes(&root, indent, out);
    }

    fn write_nodes(&self, nodes: &[Node], indent: usize, out: &mut String) {
        let pad = " ".repeat(indent);
        for n in nodes {
            match n {
                Node::Item(i) => out.push_str(&format!("{pad}{}: {}: {}\n", self.place(i.line), label(i.kind), i.text)),
                Node::Loop(l, inner) => {
                    out.push_str(&format!("{pad}loop at {}\n", self.place(*l)));
                    self.write_nodes(inner, indent + 2, out);
                }
                Node::Par(l, body) => {
                    out.push_str(&format!("{pad}par for at {}\n", self.place(*l)));
                    if body.gave_up {
                        out.push_str(&format!("{pad}  range analysis stopped early in the `par for` body ({}), so every check in it stays\n", body.stopped));
                    }
                    self.write_tree(body, indent + 2, out);
                }
            }
        }
    }
}

/// The children of the loop at line `l` in `nodes`, which is made if it isn't there.
fn open_loop<'a, 'b>(nodes: &'b mut Vec<Node<'a>>, l: u32) -> &'b mut Vec<Node<'a>> {
    let found = nodes.iter().position(|n| matches!(n, Node::Loop(line, _) if *line == l));
    let i = found.unwrap_or_else(|| {
        nodes.push(Node::Loop(l, Vec::new()));
        nodes.len() - 1
    });
    match &mut nodes[i] {
        Node::Loop(_, inner) => inner,
        _ => unreachable!("just found a loop"),
    }
}
