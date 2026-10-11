//! The GPU objects (textures, meshes) that Kestrel values own through a `GpuHandle`.
//!
//! The handle is an id. The objects themselves live here, in a registry on the
//! window's thread, because raylib may only be called there. Dropping a handle
//! can happen on any thread (a `par for` pool thread too), so a drop only
//! *queues* the id (`queue_drop`); the window's thread frees the queue at the
//! start of each frame (`drain`), and everything when the window closes
//! (`close_all`).
//!
//! The queue is shared by the whole process, and so is the id counter, but each
//! thread's registry holds only the objects that thread made. `drain` takes only
//! the queued ids its own registry holds and leaves the others queued, so a
//! thread that never made an object (or whose window has closed) never touches
//! someone else's. An id that no registry holds (a second drop of the same
//! handle, or a drop after the window closed) is ignored, and just stays queued.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::raylib as rl;

/// What a handle stands for. Headless objects keep only what natives check.
pub enum Object {
    Texture { texture: Option<rl::Texture>, width: i32, height: i32 },
    Mesh { parts: Vec<rl::Mesh>, has_uvs: bool, vertex_count: i32, triangle_count: i32 },
}

/// The ids not handed out yet: the next one. 0 means no object, so they start at 1.
static NEXT_ID: AtomicI64 = AtomicI64::new(1);

/// The ids whose handles were dropped and whose objects are not freed yet.
static QUEUE: Mutex<Vec<i64>> = Mutex::new(Vec::new());

thread_local! {
    /// The objects made on this thread, which is the window's thread.
    static REGISTRY: RefCell<HashMap<i64, Object>> = RefCell::new(HashMap::new());
}

/// The queue, even if a thread panicked while holding it (a queue of ids can't be left half-changed).
fn queue() -> MutexGuard<'static, Vec<i64>> {
    QUEUE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Adds an object made on the window's thread; gives its new id (1, 2, 3, ... for the whole process).
pub fn add(object: Object) -> i64 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    REGISTRY.with_borrow_mut(|registry| registry.insert(id, object));
    id
}

/// Runs `f` on the object `id` stands for, or gives `None` (id 0, or an object already freed).
/// `f` must not add or free objects.
pub fn with<T>(id: i64, f: impl FnOnce(&Object) -> T) -> Option<T> {
    REGISTRY.with_borrow(|registry| registry.get(&id).map(f))
}

/// The runtime's drop hook: queues `id`. Safe on any thread; never touches the GPU.
pub fn queue_drop(id: i64) {
    if id != 0 {
        queue().push(id);
    }
}

/// On the window's thread: frees every queued object (unloading it from raylib when `real`).
/// Queued ids this thread's registry doesn't hold stay queued for the thread that does.
pub fn drain(real: bool) {
    let mut freed = Vec::new();
    REGISTRY.with_borrow_mut(|registry| {
        if registry.is_empty() {
            return;
        }
        queue().retain(|id| match registry.remove(id) {
            Some(object) => {
                freed.push(object);
                false
            }
            None => true,
        });
    });
    // The lock is released: unloading can take a while.
    for object in freed {
        free(object, real);
    }
}

/// When the window closes: frees every object (raylib unloads only when `real`, before
/// `CloseWindow`), then forgets them. A real window also frees its own GPU objects, which no
/// Kestrel value holds: the `draw.rects` mesh and the default material.
pub fn close_all(real: bool) {
    let all = REGISTRY.take();
    // Their queued drops (if any) have nothing left to free.
    queue().retain(|id| !all.contains_key(id));
    for object in all.into_values() {
        free(object, real);
    }
    if real {
        // SAFETY: the window's thread, with its window still open (`close_all` runs before `CloseWindow`).
        unsafe { crate::bulk::close() };
        crate::material::close();
    }
}

/// How many objects are alive on this thread (tests).
pub fn live() -> usize {
    REGISTRY.with_borrow(HashMap::len)
}

/// Releases `object`'s GPU memory. Only a real window has any: a headless object holds nothing.
fn free(object: Object, real: bool) {
    if !real {
        return;
    }
    match object {
        Object::Texture { texture, .. } => {
            if let Some(texture) = texture {
                // SAFETY: the window's thread, with its window still open (`close_all` runs before `CloseWindow`).
                // The texture was made by raylib and is unloaded once, because `object` is gone after this.
                unsafe { rl::UnloadTexture(texture) };
            }
        }
        Object::Mesh { parts, .. } => {
            for part in parts {
                // SAFETY: as for the texture. `UnloadMesh` only `free`s what `without_cpu_pointers` leaves:
                // the buffer ids raylib made when the mesh was uploaded.
                unsafe { rl::UnloadMesh(without_cpu_pointers(part)) };
            }
        }
    }
}

/// `mesh` without the CPU arrays: the ones `UnloadMesh` would `free` that raylib didn't allocate.
pub(crate) fn without_cpu_pointers(mesh: rl::Mesh) -> rl::Mesh {
    rl::Mesh {
        vertices: std::ptr::null_mut(),
        texcoords: std::ptr::null_mut(),
        texcoords2: std::ptr::null_mut(),
        normals: std::ptr::null_mut(),
        tangents: std::ptr::null_mut(),
        colors: std::ptr::null_mut(),
        indices: std::ptr::null_mut(),
        bone_indices: std::ptr::null_mut(),
        bone_weights: std::ptr::null_mut(),
        anim_vertices: std::ptr::null_mut(),
        anim_normals: std::ptr::null_mut(),
        ..mesh
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;

    use super::*;

    /// Runs `body` on a fresh thread, so it has a registry of its own.
    fn on_a_thread<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> T {
        thread::spawn(body).join().unwrap()
    }

    fn texture(width: i32, height: i32) -> Object {
        Object::Texture { texture: None, width, height }
    }

    fn mesh() -> Object {
        Object::Mesh { parts: Vec::new(), has_uvs: true, vertex_count: 4, triangle_count: 2 }
    }

    /// Whether `id` is waiting in the queue.
    fn queued(id: i64) -> bool {
        QUEUE.lock().unwrap_or_else(PoisonError::into_inner).contains(&id)
    }

    fn size_of_texture(id: i64) -> Option<(i32, i32)> {
        with(id, |object| match object {
            Object::Texture { width, height, .. } => (*width, *height),
            Object::Mesh { .. } => (-1, -1),
        })
    }

    #[test]
    fn ids_count_up_and_find_their_objects() {
        on_a_thread(|| {
            let ids: Vec<i64> = (1..=3).map(|n| add(texture(n, n * 10))).collect();
            // Other tests add objects at the same time, so the ids need not be neighbours.
            assert!(ids[0] >= 1, "0 means no object");
            assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "{ids:?}");
            assert_eq!(live(), 3);
            assert_eq!(size_of_texture(ids[1]), Some((2, 20)));
            assert_eq!(size_of_texture(0), None, "0 is no object");
            assert_eq!(size_of_texture(ids[2] + 1_000_000), None, "an id nobody made");
            let mesh_id = add(mesh());
            assert_eq!(with(mesh_id, |o| matches!(o, Object::Mesh { vertex_count: 4, .. })), Some(true));
        });
    }

    #[test]
    fn a_drop_from_another_thread_waits_for_the_windows_drain() {
        on_a_thread(|| {
            let (keep, gone) = (add(texture(1, 1)), add(mesh()));
            thread::spawn(move || queue_drop(gone)).join().unwrap();
            assert_eq!(live(), 2, "queuing frees nothing");
            assert!(queued(gone));
            drain(false);
            assert_eq!(live(), 1);
            assert!(!queued(gone));
            assert!(with(gone, |_| ()).is_none());
            assert!(with(keep, |_| ()).is_some());
        });
    }

    #[test]
    fn zero_and_a_second_drop_are_ignored() {
        on_a_thread(|| {
            let (a, b) = (add(texture(1, 1)), add(texture(2, 2)));
            queue_drop(0);
            assert!(!queued(0), "0 is never queued");
            drain(false);
            assert_eq!(live(), 2);

            queue_drop(a);
            drain(false);
            assert_eq!(live(), 1);
            queue_drop(a);
            drain(false);
            assert_eq!(live(), 1, "a second drop of the same id does nothing");
            assert_eq!(size_of_texture(b), Some((2, 2)));

            queue_drop(b);
            queue_drop(b);
            drain(false);
            assert_eq!(live(), 0, "and a double drop before the drain frees it once");
        });
    }

    #[test]
    fn close_all_frees_everything_and_later_drops_do_nothing() {
        on_a_thread(|| {
            let (a, b) = (add(texture(1, 1)), add(mesh()));
            queue_drop(b);
            close_all(false);
            assert_eq!(live(), 0);
            assert!(with(a, |_| ()).is_none());
            assert!(!queued(b), "a queued id of a freed object is forgotten");

            // A drop after the close, and an object made later, which keeps its own id.
            let c = add(texture(3, 3));
            queue_drop(a);
            drain(false);
            assert_eq!(live(), 1);
            assert_eq!(size_of_texture(c), Some((3, 3)));
        });
    }

    #[test]
    fn one_object_a_frame_stays_at_one_live_object() {
        on_a_thread(|| {
            let mut most = 0;
            for _ in 0..1000 {
                // A frame starts, the program makes a texture, and its handle is dropped at the end of the frame.
                drain(false);
                let id = add(texture(8, 8));
                most = most.max(live());
                queue_drop(id);
            }
            assert_eq!(most, 1);
            drain(false);
            assert_eq!(live(), 0);
        });
    }

    /// A thread with a registry of one object. It reports the object's id, waits for a
    /// message, drains, and reports how many objects it still has.
    struct Worker {
        id: i64,
        go: mpsc::Sender<()>,
        live_after: mpsc::Receiver<usize>,
        thread: thread::JoinHandle<()>,
    }

    fn worker() -> Worker {
        let (id_tx, id_rx) = mpsc::channel();
        let (go, go_rx) = mpsc::channel();
        let (live_tx, live_after) = mpsc::channel();
        let thread = thread::spawn(move || {
            id_tx.send(add(texture(1, 1))).unwrap();
            go_rx.recv().unwrap();
            drain(false);
            live_tx.send(live()).unwrap();
        });
        Worker { id: id_rx.recv().unwrap(), go, live_after, thread }
    }

    #[test]
    fn a_drain_leaves_another_threads_queued_ids_for_that_thread() {
        let (a, b) = (worker(), worker());
        queue_drop(a.id);
        queue_drop(b.id);

        b.go.send(()).unwrap();
        assert_eq!(b.live_after.recv().unwrap(), 0, "B's drain freed B's object");
        assert!(queued(a.id), "and left A's id for A's thread");
        assert!(!queued(b.id));

        a.go.send(()).unwrap();
        assert_eq!(a.live_after.recv().unwrap(), 0, "A's drain freed A's object");
        assert!(!queued(a.id));
        a.thread.join().unwrap();
        b.thread.join().unwrap();
    }

    #[test]
    fn a_thread_with_no_objects_drains_nothing() {
        let a = worker();
        queue_drop(a.id);
        on_a_thread(|| drain(false));
        assert!(queued(a.id), "a thread that made nothing takes nothing");
        a.go.send(()).unwrap();
        assert_eq!(a.live_after.recv().unwrap(), 0);
        a.thread.join().unwrap();
    }

    #[test]
    fn a_mesh_is_unloaded_without_freeing_arrays_raylib_did_not_allocate() {
        let mut word = 0u32;
        let p = std::ptr::addr_of_mut!(word);
        let mesh = rl::Mesh {
            vertex_count: 3,
            triangle_count: 1,
            vertices: p.cast(),
            texcoords: p.cast(),
            texcoords2: p.cast(),
            normals: p.cast(),
            tangents: p.cast(),
            colors: p.cast(),
            indices: p.cast(),
            bone_count: 2,
            bone_indices: p.cast(),
            bone_weights: p.cast(),
            anim_vertices: p.cast(),
            anim_normals: p.cast(),
            vao_id: 5,
            vbo_id: p,
        };
        let freed = without_cpu_pointers(mesh);
        let cpu = [
            freed.vertices.cast::<u32>(),
            freed.texcoords.cast(),
            freed.texcoords2.cast(),
            freed.normals.cast(),
            freed.tangents.cast(),
            freed.colors.cast(),
            freed.indices.cast(),
            freed.bone_indices.cast(),
            freed.bone_weights.cast(),
            freed.anim_vertices.cast(),
            freed.anim_normals.cast(),
        ];
        assert!(cpu.iter().all(|pointer| pointer.is_null()));
        // What raylib allocated itself when it uploaded the mesh stays, so `UnloadMesh` releases it.
        assert_eq!((freed.vao_id, freed.vbo_id), (5, p));
        assert_eq!((freed.vertex_count, freed.triangle_count, freed.bone_count), (3, 1, 2));
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn the_mesh_layout_is_raylibs() {
        use std::mem::{offset_of, size_of};
        // `typedef struct Mesh` in raylib.h on a 64-bit target.
        assert_eq!(size_of::<rl::Mesh>(), 120);
        assert_eq!(offset_of!(rl::Mesh, vertices), 8);
        assert_eq!(offset_of!(rl::Mesh, bone_count), 64);
        assert_eq!(offset_of!(rl::Mesh, vao_id), 104);
        assert_eq!(offset_of!(rl::Mesh, vbo_id), 112);
    }
}
