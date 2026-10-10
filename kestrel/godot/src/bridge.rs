//! Converting between Godot values and the bytes Kestrel's export thunks
//! take and return (see `kestrel_codegen::ExportInfo`).

use godot::prelude::*;
use kestrel_codegen::ExportTy;

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
            let (bytes, len) = match **elem {
                ExportTy::U8 => {
                    let a = v.try_to::<PackedByteArray>().map_err(|_| want("a PackedByteArray"))?;
                    (a.as_slice().to_vec(), a.len())
                }
                ExportTy::Int => {
                    let a = v.try_to::<PackedInt32Array>().map_err(|_| want("a PackedInt32Array"))?;
                    (a.as_slice().iter().flat_map(|n| n.to_le_bytes()).collect(), a.len())
                }
                ExportTy::I64 => {
                    let a = v.try_to::<PackedInt64Array>().map_err(|_| want("a PackedInt64Array"))?;
                    (a.as_slice().iter().flat_map(|n| n.to_le_bytes()).collect(), a.len())
                }
                ExportTy::Float => {
                    let a = v.try_to::<PackedFloat32Array>().map_err(|_| want("a PackedFloat32Array"))?;
                    (floats(a.as_slice().iter().copied()), a.len())
                }
                ExportTy::F64 => {
                    let a = v.try_to::<PackedFloat64Array>().map_err(|_| want("a PackedFloat64Array"))?;
                    (a.as_slice().iter().flat_map(|n| n.to_le_bytes()).collect(), a.len())
                }
                ExportTy::Vec2 => {
                    let a = v.try_to::<PackedVector2Array>().map_err(|_| want("a PackedVector2Array"))?;
                    (floats(a.as_slice().iter().flat_map(|p| [p.x, p.y])), a.len())
                }
                ExportTy::Vec3 => {
                    let a = v.try_to::<PackedVector3Array>().map_err(|_| want("a PackedVector3Array"))?;
                    (floats(a.as_slice().iter().flat_map(|p| [p.x, p.y, p.z])), a.len())
                }
                _ => unreachable!("checked by `unsupported`"),
            };
            put_seq(buf, at, bytes, len, *list, keep);
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
            ExportTy::U8 => Variant::from(PackedByteArray::from(heap)),
            ExportTy::Int => {
                let v: Vec<i32> = heap.chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().expect("4 bytes"))).collect();
                Variant::from(PackedInt32Array::from(&v[..]))
            }
            ExportTy::I64 => {
                let v: Vec<i64> = heap.chunks_exact(8).map(|c| i64::from_le_bytes(c.try_into().expect("8 bytes"))).collect();
                Variant::from(PackedInt64Array::from(&v[..]))
            }
            ExportTy::Float => {
                let v: Vec<f32> = f32s(heap).collect();
                Variant::from(PackedFloat32Array::from(&v[..]))
            }
            ExportTy::F64 => {
                let v: Vec<f64> = heap.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().expect("8 bytes"))).collect();
                Variant::from(PackedFloat64Array::from(&v[..]))
            }
            ExportTy::Vec2 => {
                let v: Vec<Vector2> = heap
                    .chunks_exact(8)
                    .map(|c| {
                        let f: Vec<f32> = f32s(c).collect();
                        Vector2::new(f[0], f[1])
                    })
                    .collect();
                Variant::from(PackedVector2Array::from(&v[..]))
            }
            ExportTy::Vec3 => {
                let v: Vec<Vector3> = heap
                    .chunks_exact(12)
                    .map(|c| {
                        let f: Vec<f32> = f32s(c).collect();
                        Vector3::new(f[0], f[1], f[2])
                    })
                    .collect();
                Variant::from(PackedVector3Array::from(&v[..]))
            }
            _ => Variant::nil(),
        },
        ExportTy::Other(_) => Variant::nil(),
    }
}
