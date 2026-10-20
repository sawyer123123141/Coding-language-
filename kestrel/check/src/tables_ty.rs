//! Type queries on `Tables`: heap and ownership checks and type names for messages.


use crate::tables::Tables;
use crate::ty::Ty;

impl Tables {
    /// Whether a value of `ty` points at arena memory: strings, arrays,
    /// lists and maps, directly or in fields. (An `Arena` itself doesn't.)
    /// Strings may live in an arena (interpolation builds them there), so
    /// copies copy their bytes, like arrays.
    pub fn has_heap(&self, ty: Ty) -> bool {
        self.has_heap_below(ty, &mut Vec::new())
    }

    /// `has_heap`, skipping the types in `seen`: a struct that contains itself is an
    /// error reported elsewhere, and looking into it again would never end.
    fn has_heap_below(&self, ty: Ty, seen: &mut Vec<Ty>) -> bool {
        match ty {
            // A closure's environment is heap memory (a function value with none holds nothing).
            Ty::Str | Ty::Array(_) | Ty::List(_) | Ty::Map(..) | Ty::Fn(_) | Ty::Iface(_) => true,
            Ty::Optional(e) => self.has_heap_below(self.elem(e), seen),
            Ty::Struct(_) | Ty::Enum(_) if seen.contains(&ty) => false,
            Ty::Struct(s) => {
                seen.push(ty);
                self.strukt(s).fields.iter().any(|f| self.has_heap_below(f.ty, seen))
            }
            Ty::Enum(e) => {
                seen.push(ty);
                self.enm(e).variants.iter().flat_map(|v| &v.fields).any(|f| self.has_heap_below(f.ty, seen))
            }
            _ => false,
        }
    }

    /// Whether `ty` is a function type, or holds one in a field, a payload, an element or an
    /// optional. Such values can't be compared. (Rule 7b of the closures spec asks this; it
    /// doesn't look inside interface values.)
    pub fn holds_fn(&self, ty: Ty) -> bool {
        self.holds_where(ty, &mut Vec::new(), &|t| matches!(t, Ty::Fn(_)))
    }

    /// Whether `ty` is an interface type, or holds one in a field, a payload, an element, a map
    /// value or an optional.
    pub fn holds_iface(&self, ty: Ty) -> bool {
        self.holds_where(ty, &mut Vec::new(), &|t| matches!(t, Ty::Iface(_)))
    }

    /// Whether values of `ty` can't be compared with `==`: it is, or holds, a function type or an
    /// interface type.
    pub fn holds_uncomparable(&self, ty: Ty) -> bool {
        self.holds_where(ty, &mut Vec::new(), &|t| matches!(t, Ty::Fn(_) | Ty::Iface(_)))
    }

    /// Whether `ty` holds an interface value that a zero-filled array would leave empty: one
    /// directly, in a field, a payload or an array element (an optional starts as `none`, and a
    /// list or map starts empty).
    pub fn zero_fill_breaks(&self, ty: Ty) -> bool {
        self.zero_fill_below(ty, &mut Vec::new())
    }

    fn zero_fill_below(&self, ty: Ty, seen: &mut Vec<Ty>) -> bool {
        match ty {
            Ty::Iface(_) => true,
            Ty::Array(e) => self.zero_fill_below(self.elem(e), seen),
            Ty::Struct(_) | Ty::Enum(_) if seen.contains(&ty) => false,
            Ty::Struct(s) => {
                seen.push(ty);
                self.strukt(s).fields.iter().any(|f| self.zero_fill_below(f.ty, seen))
            }
            Ty::Enum(e) => {
                seen.push(ty);
                self.enm(e).variants.iter().flat_map(|v| &v.fields).any(|f| self.zero_fill_below(f.ty, seen))
            }
            _ => false,
        }
    }

    /// Whether `ty` is, or holds in a field, a payload, an element or an optional, a type for which
    /// `leaf` is true. `seen` stops a walk through a type that contains itself (an error reported
    /// elsewhere). The walk does not go inside function or interface types.
    fn holds_where(&self, ty: Ty, seen: &mut Vec<Ty>, leaf: &dyn Fn(Ty) -> bool) -> bool {
        if leaf(ty) {
            return true;
        }
        match ty {
            Ty::Array(e) | Ty::List(e) | Ty::Optional(e) => self.holds_where(self.elem(e), seen, leaf),
            Ty::Map(k, v) => self.holds_where(self.elem(k), seen, leaf) || self.holds_where(self.elem(v), seen, leaf),
            Ty::Struct(_) | Ty::Enum(_) if seen.contains(&ty) => false,
            Ty::Struct(s) => {
                seen.push(ty);
                self.strukt(s).fields.iter().any(|f| self.holds_where(f.ty, seen, leaf))
            }
            Ty::Enum(e) => {
                seen.push(ty);
                self.enm(e).variants.iter().flat_map(|v| &v.fields).any(|f| self.holds_where(f.ty, seen, leaf))
            }
            _ => false,
        }
    }

    /// Whether a constant of type `ty` is built once, before `main`, and read in place
    /// (`kir::lower::consts::hoisted`): it holds an array, list, map or interface value, so
    /// building it again at each use would allocate each time, and it isn't owning (an
    /// `Arena()` constant makes a new arena at each use). The checker's `is_kir_place` asks this
    /// too, so the two agree on which constants are places.
    pub fn const_read_in_place(&self, ty: Ty) -> bool {
        !self.is_owning(ty) && self.holds_where(ty, &mut Vec::new(), &|t| matches!(t, Ty::Array(_) | Ty::List(_) | Ty::Map(..) | Ty::Iface(_)))
    }

    /// Whether `ty` holds an `Arena` or a `GpuHandle` (in fields, payloads or elements):
    /// values of it move, never copy, and are dropped.
    pub fn is_owning(&self, ty: Ty) -> bool {
        match ty {
            Ty::Arena | Ty::Gpu => true,
            Ty::Struct(s) => self.owning_structs.get(s.0 as usize).copied().unwrap_or(false),
            Ty::Enum(e) => self.owning_enums.get(e.0 as usize).copied().unwrap_or(false),
            Ty::Array(e) | Ty::List(e) | Ty::Optional(e) => self.is_owning(self.elem(e)),
            // Maps can't hold owning values (see `check_map_values`), and an interface value never
            // owns anything: an owning implementor can only be lent to one, not stored as one.
            Ty::Iface(_) => false,
            _ => false,
        }
    }

    /// Whether `ty` holds an `Arena` anywhere inside: `is_owning` without the `GpuHandle` case.
    pub fn holds_arena(&self, ty: Ty) -> bool {
        self.holds_arena_below(ty, &mut Vec::new())
    }

    /// `holds_arena`, skipping the types in `seen`: a type that contains itself is an error
    /// reported elsewhere, and looking into it again would never end.
    fn holds_arena_below(&self, ty: Ty, seen: &mut Vec<Ty>) -> bool {
        match ty {
            Ty::Arena => true,
            Ty::Array(e) | Ty::List(e) | Ty::Optional(e) => self.holds_arena_below(self.elem(e), seen),
            Ty::Struct(_) | Ty::Enum(_) if seen.contains(&ty) => false,
            Ty::Struct(s) => {
                seen.push(ty);
                self.strukt(s).fields.iter().any(|f| self.holds_arena_below(f.ty, seen))
            }
            Ty::Enum(e) => {
                seen.push(ty);
                self.enm(e).variants.iter().flat_map(|v| &v.fields).any(|f| self.holds_arena_below(f.ty, seen))
            }
            _ => false,
        }
    }

    /// What makes `ty` owning, for messages: "an `Arena`" when it holds one, else a GPU object.
    pub fn owns_what(&self, ty: Ty) -> &'static str {
        if self.holds_arena(ty) {
            "an `Arena`"
        } else {
            "a GPU object"
        }
    }

    /// Fills `owning_structs` / `owning_enums`, growing from `false` until
    /// nothing changes, so types that contain themselves terminate.
    pub(crate) fn compute_owning(&mut self) {
        self.owning_structs = vec![false; self.structs.len()];
        self.owning_enums = vec![false; self.enums.len()];
        loop {
            let mut changed = false;
            for s in 0..self.structs.len() {
                if !self.owning_structs[s] && self.structs[s].fields.iter().any(|f| self.is_owning(f.ty)) {
                    self.owning_structs[s] = true;
                    changed = true;
                }
            }
            for e in 0..self.enums.len() {
                if !self.owning_enums[e] && self.enums[e].variants.iter().flat_map(|v| &v.fields).any(|f| self.is_owning(f.ty)) {
                    self.owning_enums[e] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// How a type reads in messages: struct types by name.
    pub fn ty_name(&self, ty: Ty) -> String {
        match ty {
            Ty::Struct(id) => self.strukt(id).name.clone(),
            Ty::Enum(id) => self.enm(id).name.clone(),
            Ty::Iface(id) => self.iface(id).name.clone(),
            Ty::Variant(id, v) => format!("{}.{}", self.enm(id).name, self.enm(id).variants[v as usize].name),
            Ty::Array(e) => format!("[{}]", self.ty_name(self.elem(e))),
            Ty::List(e) => format!("list[{}]", self.ty_name(self.elem(e))),
            Ty::Map(k, v) => format!("map[{}, {}]", self.ty_name(self.elem(k)), self.ty_name(self.elem(v))),
            Ty::Optional(e) => format!("{}?", self.ty_name(self.elem(e))),
            Ty::Fn(s) => self.sig_name(s),
            other => other.name().to_string(),
        }
    }
}
