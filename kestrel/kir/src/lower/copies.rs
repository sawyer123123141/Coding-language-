//! What the module says about copies of generics (`Module::copies`, `Module::uncopied`): the
//! notes contracts and the report use to name a copy, and the generics nothing was copied from.

use std::collections::{HashMap, HashSet};

use kestrel_check::{FnId, GenericOwner, Tables};
use kestrel_syntax::ast::Program;
use kestrel_syntax::{LineIndex, Span};

use crate::ir::{pack_line, CopyNote, FuncIdx};

/// The note of every copy.
pub(super) fn notes(t: &Tables, lines: &LineIndex) -> HashMap<FuncIdx, CopyNote> {
    let mut copies = HashMap::new();
    for (i, info) in t.fns.iter().enumerate() {
        let Some(copy) = info.copy else { continue };
        let generic = &t.func(copy.generic).name;
        let made_by = match info.generic {
            Some(GenericOwner::Struct(def)) => instance_name(t.generic.struct_instances.get(&(def, copy.args)).map(|&s| &t.strukt(s).name), &info.name),
            Some(GenericOwner::Enum(def)) => instance_name(t.generic.enum_instances.get(&(def, copy.args)).map(|&e| &t.enm(e).name), &info.name),
            _ => info.name.clone(),
        };
        let made_at = pack_line(lines.file(copy.made_at.start), lines.line(copy.made_at.start));
        copies.insert(FuncIdx(i as u32), CopyNote { generic: generic.clone(), made_by, made_at });
    }
    copies
}

/// Every generic function or method with no copy, by name and packed lines, in declaration order.
pub(super) fn uncopied(program: &Program, t: &Tables, lines: &LineIndex) -> Vec<(String, (u32, u32))> {
    let copied: HashSet<FnId> = t.fns.iter().filter_map(|f| f.copy.map(|c| c.generic)).collect();
    let generics = t.fns.iter().enumerate().filter(|(i, f)| f.generic.is_some() && f.copy.is_none() && !copied.contains(&FnId(*i as u32)));
    generics.map(|(_, info)| (info.name.clone(), packed_lines(lines, info.decl(program).span))).collect()
}

/// An instance's name; the lookup only misses on a compiler bug (the copy was made for it).
fn instance_name(found: Option<&String>, copy: &str) -> String {
    match found {
        Some(name) => name.clone(),
        None => panic!("the instance a copy `{copy}` was made for is missing; this is a compiler bug"),
    }
}

/// The packed first and last line of `span`.
fn packed_lines(lines: &LineIndex, span: Span) -> (u32, u32) {
    let at = |offset: u32| pack_line(lines.file(offset), lines.line(offset));
    (at(span.start), at(span.end.saturating_sub(1).max(span.start)))
}
