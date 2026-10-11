//! The window's one default raylib material: raylib's default shader and its 1 by 1 white
//! texture. raylib draws a mesh (`DrawMesh`) with a material; `draw.rects` and the mesh natives
//! share this one and set its colour (and texture) for each draw.
//!
//! It is made on first use, so a headless window, or a real one that never draws a mesh, never
//! makes it. It is freed when the window closes (`gpu::close_all`).

use std::cell::Cell;

use crate::raylib as rl;

thread_local! {
    /// The material, once the window's thread has made it.
    static MATERIAL: Cell<Option<rl::Material>> = const { Cell::new(None) };
}

/// The material with its diffuse colour set to `color`, which a draw multiplies its vertex
/// colours and texture by. It is made now if this is its first use. The colour stays set until
/// the next call; the diffuse texture is raylib's white default.
///
/// # Safety
/// A real window must be open, on this thread.
pub(crate) unsafe fn tinted(color: rl::Color) -> rl::Material {
    let material = MATERIAL.get().unwrap_or_else(|| {
        // SAFETY: the window is open, on this thread (this function's contract).
        let made = unsafe { rl::LoadMaterialDefault() };
        MATERIAL.set(Some(made));
        made
    });
    // SAFETY: `LoadMaterialDefault` allocated `MAX_MATERIAL_MAPS` maps, so the diffuse one (the
    // first) is inside them, and they live until `close`. Nothing else holds a reference to them.
    unsafe { (*material.maps.add(rl::MATERIAL_MAP_DIFFUSE)).color = color };
    material
}

/// Runs `draw` with the material tinted `color` (see `tinted`) and with `texture` as its diffuse
/// texture, then puts raylib's white default texture back: the material is shared, and a program's
/// texture is only borrowed for the draw (its `Texture` frees it, maybe before the next frame).
///
/// # Safety
/// A real window must be open, on this thread, and `texture` must be a live texture of it.
pub(crate) unsafe fn with_texture(color: rl::Color, texture: rl::Texture, draw: impl FnOnce(rl::Material)) {
    // SAFETY: the window is open, on this thread (this function's contract).
    let material = unsafe { tinted(color) };
    // SAFETY: as in `tinted`, the diffuse map is inside the maps, which live until `close`, and nothing
    // else holds a reference to them; `draw` only reads them, through `DrawMesh`.
    unsafe {
        let diffuse = material.maps.add(rl::MATERIAL_MAP_DIFFUSE);
        let default = (*diffuse).texture;
        (*diffuse).texture = texture;
        draw(material);
        (*diffuse).texture = default;
    }
}

/// Frees the material, if this thread made one, and forgets it.
///
/// Only the array of maps is freed. The shader and the texture are raylib's defaults, which raylib
/// frees itself when it closes. (`UnloadMaterial` would also unload any other texture left on a
/// map, and a texture set for a draw belongs to the program's `Texture`, which frees it.)
pub(crate) fn close() {
    if let Some(material) = MATERIAL.take() {
        // SAFETY: `maps` was allocated by raylib (`LoadMaterialDefault`), with the allocator
        // `MemFree` releases, and it is freed once: the material is no longer stored.
        unsafe { rl::MemFree(material.maps.cast()) };
    }
}

/// Whether this thread has made the material (tests).
#[cfg(test)]
pub(crate) fn made() -> bool {
    MATERIAL.get().is_some()
}
