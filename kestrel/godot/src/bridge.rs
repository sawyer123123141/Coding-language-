//! Converting between Godot values and the bytes Kestrel's export thunks
//! take and return (see `kestrel_codegen::ExportInfo`).

use godot::meta::PackedElement;
use godot::prelude::*;
use kestrel_codegen::ExportTy;
use kestrel_runtime::arena::MAX_ITEMS;

/// The bytes of `items`. Every element type used here (`u8`, `i32`, `i64`,
/// `f32`, `f64`, `Vector2`, `Vector3`) has the layout Kestrel uses for it:
/// plain little-endian numbers, vectors as packed `f32`s.
fn raw_bytes<T: Copy>(items: &[T]) -> Vec<u8> {
    let len = std::mem::size_of_val(items);
    // SAFETY: `T` is a plain-old-data type (see above), so its bytes are all initialized.
    unsafe { std::slice::from_raw_parts(items.as_ptr() as *const u8, len) }.to_vec()
}

/// A packed array holding the elements whose bytes are `heap`: one copy.
fn packed<T: PackedElement + Copy>(heap: &[u8]) -> PackedArray<T> {
    let n = heap.len() / std::mem::size_of::<T>();
    let mut out = PackedArray::<T>::new();
    out.resize(n);
    // SAFETY: `out` has room for `n` elements, and `T` matches Kestrel's
    // layout for the element type (see `raw_bytes`).
    unsafe { std::ptr::copy_nonoverlapping(heap.as_ptr(), out.as_mut_slice().as_mut_ptr() as *mut u8, n * std::mem::size_of::<T>()) };
    out
}

/// Why a type can't cross between Godot and Kestrel, or `None` if it can.
pub fn unsupported(ty: &ExportTy) -> Option<String> {
    match ty {
        ExportTy::Bool | ExportTy::U8 | ExportTy::Int | ExportTy::I64 | ExportTy::Float | ExportTy::F64 => None,
        ExportTy::Str | ExportTy::Vec2 | ExportTy::Vec3 => None,
        ExportTy::Seq { elem, .. } => match **elem {
            ExportTy::U8 | ExportTy::Int | ExportTy::I64 | ExportTy::Float | ExportTy::F64 | ExportTy::Vec2 | ExportTy::Vec3 => None,
            _ => Some(format!("a list of {}", name(elem))),
        },
        ExportTy::Other(n) => Some(n.clone()),
    }
}

fn name(ty: &ExportTy) -> String {
    match ty {
        ExportTy::Bool => "bool".into(),
        ExportTy::U8 => "u8".into(),
        ExportTy::Int => "int".into(),
        ExportTy::I64 => "i64".into(),
        ExportTy::Float => "float".into(),
        ExportTy::F64 => "f64".into(),
        ExportTy::Str => "string".into(),
        ExportTy::Vec2 => "vec2".into(),
        ExportTy::Vec3 => "vec3".into(),
        ExportTy::Seq { elem, .. } => format!("[{}]", name(elem)),
        ExportTy::Other(n) => n.clone(),
    }
}

/// What the worker must copy out of the call's arena before freeing it.
#[derive(Clone, Copy)]
pub enum Heap {
    None,
    /// A string or array result: `elem_size` bytes per element.
    Seq { elem_size: usize },
}

pub fn heap_of(ty: Option<&ExportTy>) -> Heap {
    match ty {
        Some(ExportTy::Str) => Heap::Seq { elem_size: 1 },
        Some(ExportTy::Seq { elem_size, .. }) => Heap::Seq { elem_size: *elem_size as usize },
        _ => Heap::None,
    }
}

fn put(buf: &mut [u8], at: usize, bytes: &[u8]) {
    buf[at..at + bytes.len()].copy_from_slice(bytes);
}

/// Why a host array of `len` items can't be passed in, or `None` if it can: a list or an array
/// holds at most `MAX_ITEMS` items (strings aren't limited). `list` says which Kestrel type it is.
pub fn too_long(len: usize, list: bool) -> Option<String> {
    if i64::try_from(len).is_ok_and(|n| n <= MAX_ITEMS) {
        return None;
    }
    let what = if list { "a list" } else { "an array" };
    Some(format!("{what} can't hold more than 2,147,483,647 items"))
}

/// `{ptr, len}` of a string or array whose bytes live in `keep` until the call ends.
fn put_seq(buf: &mut [u8], at: usize, bytes: Vec<u8>, len: usize, list: bool, keep: &mut Vec<Vec<u8>>) {
    put(buf, at, &(bytes.as_ptr() as u64).to_le_bytes());
    put(buf, at + 8, &(len as i64).to_le_bytes());
    if list {
        // A read-only list never grows: capacity = length, no arena.
        put(buf, at + 16, &(len as i64).to_le_bytes());
        put(buf, at + 24, &0u64.to_le_bytes());
    }
    keep.push(bytes);
}

fn floats(vs: impl IntoIterator<Item = f32>) -> Vec<u8> {
    vs.into_iter().flat_map(f32::to_le_bytes).collect()
}

/// Writes `v` into `buf` at `at` as a Kestrel value of type `ty`.
pub fn write_arg(v: &Variant, ty: &ExportTy, buf: &mut [u8], at: usize, keep: &mut Vec<Vec<u8>>) -> Result<(), String> {
    let want = |what: &str| format!("needs {what}, got {:?}", v.get_type());
    match ty {
        ExportTy::Bool => put(buf, at, &[v.try_to::<bool>().map_err(|_| want("a bool"))? as u8]),
        ExportTy::U8 => {
            let n = v.try_to::<i64>().map_err(|_| want("an int"))?;
            let n = u8::try_from(n).map_err(|_| format!("{n} doesn't fit in u8"))?;
            put(buf, at, &[n]);
        }
        ExportTy::Int => {
            let n = v.try_to::<i64>().map_err(|_| want("an int"))?;
            let n = i32::try_from(n).map_err(|_| format!("{n} doesn't fit in int"))?;
            put(buf, at, &n.to_le_bytes());
        }
        ExportTy::I64 => put(buf, at, &v.try_to::<i64>().map_err(|_| want("an int"))?.to_le_bytes()),
        ExportTy::Float => put(buf, at, &(v.try_to::<f64>().map_err(|_| want("a float"))? as f32).to_le_bytes()),
        ExportTy::F64 => put(buf, at, &v.try_to::<f64>().map_err(|_| want("a float"))?.to_le_bytes()),
        ExportTy::Vec2 => {
            let p = v.try_to::<Vector2>().map_err(|_| want("a Vector2"))?;
            put(buf, at, &floats([p.x, p.y]));
        }
        ExportTy::Vec3 => {
            let p = v.try_to::<Vector3>().map_err(|_| want("a Vector3"))?;
            put(buf, at, &floats([p.x, p.y, p.z]));
        }
        ExportTy::Str => {
            let s = v.try_to::<GString>().map_err(|_| want("a String"))?.to_string().into_bytes();
            let len = s.len();
            put_seq(buf, at, s, len, false, keep);
        }
        ExportTy::Seq { list, elem, .. } => {
            // The length is checked before the elements are copied.
            let list = *list;
            fn take<T: PackedElement + Copy>(a: PackedArray<T>, list: bool) -> Result<(Vec<u8>, usize), String> {
                match too_long(a.len(), list) {
                    Some(why) => Err(format!("is too long: {why}")),
                    None => Ok((raw_bytes(a.as_slice()), a.len())),
                }
            }
            let (bytes, len) = match **elem {
                ExportTy::U8 => take(v.try_to::<PackedByteArray>().map_err(|_| want("a PackedByteArray"))?, list)?,
                ExportTy::Int => take(v.try_to::<PackedInt32Array>().map_err(|_| want("a PackedInt32Array"))?, list)?,
                ExportTy::I64 => take(v.try_to::<PackedInt64Array>().map_err(|_| want("a PackedInt64Array"))?, list)?,
                ExportTy::Float => take(v.try_to::<PackedFloat32Array>().map_err(|_| want("a PackedFloat32Array"))?, list)?,
                ExportTy::F64 => take(v.try_to::<PackedFloat64Array>().map_err(|_| want("a PackedFloat64Array"))?, list)?,
                ExportTy::Vec2 => take(v.try_to::<PackedVector2Array>().map_err(|_| want("a PackedVector2Array"))?, list)?,
                ExportTy::Vec3 => take(v.try_to::<PackedVector3Array>().map_err(|_| want("a PackedVector3Array"))?, list)?,
                _ => unreachable!("checked by `unsupported`"),
            };
            put_seq(buf, at, bytes, len, list, keep);
        }
        ExportTy::Other(_) => unreachable!("checked by `unsupported`"),
    }
    Ok(())
}

fn f32s(bytes: &[u8]) -> impl Iterator<Item = f32> + '_ {
    bytes.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().expect("4 bytes")))
}

/// The Godot value of a Kestrel result: `ret` holds the value itself and
/// `heap` the elements of a string or array result, copied out.
pub fn read_ret(ty: &ExportTy, ret: &[u8], heap: &[u8]) -> Variant {
    let n4 = || i32::from_le_bytes(ret[..4].try_into().expect("4 bytes"));
    let n8 = || i64::from_le_bytes(ret[..8].try_into().expect("8 bytes"));
    match ty {
        ExportTy::Bool => Variant::from(ret[0] != 0),
        ExportTy::U8 => Variant::from(ret[0] as i64),
        ExportTy::Int => Variant::from(n4() as i64),
        ExportTy::I64 => Variant::from(n8()),
        ExportTy::Float => Variant::from(f32::from_le_bytes(ret[..4].try_into().expect("4 bytes")) as f64),
        ExportTy::F64 => Variant::from(f64::from_le_bytes(ret[..8].try_into().expect("8 bytes"))),
        ExportTy::Vec2 => {
            let v: Vec<f32> = f32s(&ret[..8]).collect();
            Variant::from(Vector2::new(v[0], v[1]))
        }
        ExportTy::Vec3 => {
            let v: Vec<f32> = f32s(&ret[..12]).collect();
            Variant::from(Vector3::new(v[0], v[1], v[2]))
        }
        ExportTy::Str => Variant::from(GString::from(String::from_utf8_lossy(heap).as_ref())),
        ExportTy::Seq { elem, .. } => match **elem {
            ExportTy::U8 => Variant::from(packed::<u8>(heap)),
            ExportTy::Int => Variant::from(packed::<i32>(heap)),
            ExportTy::I64 => Variant::from(packed::<i64>(heap)),
            ExportTy::Float => Variant::from(packed::<f32>(heap)),
            ExportTy::F64 => Variant::from(packed::<f64>(heap)),
            ExportTy::Vec2 => Variant::from(packed::<Vector2>(heap)),
            ExportTy::Vec3 => Variant::from(packed::<Vector3>(heap)),
            _ => Variant::nil(),
        },
        ExportTy::Other(_) => Variant::nil(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_array_longer_than_the_item_limit_is_refused() {
        let limit = MAX_ITEMS as usize;
        assert_eq!(too_long(0, true), None);
        assert_eq!(too_long(limit, true), None);
        assert_eq!(too_long(limit, false), None);
        assert_eq!(too_long(limit + 1, true).as_deref(), Some("a list can't hold more than 2,147,483,647 items"));
        assert_eq!(too_long(limit + 1, false).as_deref(), Some("an array can't hold more than 2,147,483,647 items"));
        assert_eq!(too_long(usize::MAX, false).as_deref(), Some("an array can't hold more than 2,147,483,647 items"));
    }
}
