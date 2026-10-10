extends RefCounted
## The same 3D voxel world as bench/voxels3d/voxels3d.kes, in typed GDScript.
## (GDScript floats are 64-bit, so a block right at the surface can come out
## differently now and then.)

const SIZE := 32
const PAD := SIZE + 2
const LAYERS := 4
const HEIGHT := LAYERS * SIZE


class ChunkMesh:
	var vertices := PackedVector3Array()
	var normals := PackedVector3Array()
	var indices := PackedInt32Array()

	func quad(n: Vector3, a: Vector3, b: Vector3, c: Vector3, d: Vector3) -> void:
		var base := vertices.size()
		vertices.append(a); vertices.append(b); vertices.append(c); vertices.append(d)
		for k in 4:
			normals.append(n)
		indices.append(base); indices.append(base + 1); indices.append(base + 2)
		indices.append(base); indices.append(base + 2); indices.append(base + 3)


func hash_at(x: int, y: int, z: int, seed: int) -> int:
	var h: int = (x * 73856093) ^ (y * 19349663) ^ (z * 83492791) ^ (seed * 1610612741)
	h = h & 0x7FFFFFFF
	h = ((h ^ (h >> 13)) * 0x5BD1E995) & 0x7FFFFFFF
	return h ^ (h >> 15)


func lattice(x: int, y: int, z: int, seed: int) -> float:
	return float(hash_at(x, y, z, seed) & 0xFFFF) / 65535.0


func smooth(t: float) -> float:
	return t * t * (3.0 - 2.0 * t)


func mix(a: float, b: float, t: float) -> float:
	return a + (b - a) * t


func octave(x: int, y: int, z: int, shift: int, seed: int) -> float:
	var mask: int = (1 << shift) - 1
	var scale := float(1 << shift)
	var cx: int = x >> shift
	var cy: int = y >> shift
	var cz: int = z >> shift
	var fx := smooth(float(x & mask) / scale)
	var fy := smooth(float(y & mask) / scale)
	var fz := smooth(float(z & mask) / scale)
	var x00 := mix(lattice(cx, cy, cz, seed), lattice(cx + 1, cy, cz, seed), fx)
	var x10 := mix(lattice(cx, cy + 1, cz, seed), lattice(cx + 1, cy + 1, cz, seed), fx)
	var x01 := mix(lattice(cx, cy, cz + 1, seed), lattice(cx + 1, cy, cz + 1, seed), fx)
	var x11 := mix(lattice(cx, cy + 1, cz + 1, seed), lattice(cx + 1, cy + 1, cz + 1, seed), fx)
	return mix(mix(x00, x10, fy), mix(x01, x11, fy), fz)


func density(x: int, y: int, z: int, seed: int) -> float:
	var n := octave(x, y, z, 5, seed) * 0.5 + octave(x, y, z, 4, seed + 1) * 0.3 + octave(x, y, z, 3, seed + 2) * 0.2
	return n - 0.5 - (float(y) - float(HEIGHT) * 0.5) / float(HEIGHT * 2)


func solids(cx: int, cy: int, cz: int, seed: int) -> PackedByteArray:
	var o := PackedByteArray()
	o.resize(PAD * PAD * PAD)
	o.fill(0)
	var i := 0
	for y in PAD:
		for z in PAD:
			for x in PAD:
				if density(cx * SIZE + x - 1, cy * SIZE + y - 1, cz * SIZE + z - 1, seed) > 0.0:
					o[i] = 1
				i += 1
	return o


func mesh(cx: int, cy: int, cz: int, seed: int) -> ChunkMesh:
	var solid := solids(cx, cy, cz, seed)
	var m := ChunkMesh.new()
	for y in SIZE:
		for z in SIZE:
			for x in SIZE:
				var i: int = ((y + 1) * PAD + z + 1) * PAD + x + 1
				if solid[i] == 1:
					var p := Vector3(cx * SIZE + x, cy * SIZE + y, cz * SIZE + z)
					if solid[i + 1] == 0:
						m.quad(Vector3(1, 0, 0), p + Vector3(1, 0, 0), p + Vector3(1, 0, 1), p + Vector3(1, 1, 1), p + Vector3(1, 1, 0))
					if solid[i - 1] == 0:
						m.quad(Vector3(-1, 0, 0), p + Vector3(0, 0, 1), p, p + Vector3(0, 1, 0), p + Vector3(0, 1, 1))
					if solid[i + PAD * PAD] == 0:
						m.quad(Vector3(0, 1, 0), p + Vector3(0, 1, 0), p + Vector3(1, 1, 0), p + Vector3(1, 1, 1), p + Vector3(0, 1, 1))
					if solid[i - PAD * PAD] == 0:
						m.quad(Vector3(0, -1, 0), p, p + Vector3(0, 0, 1), p + Vector3(1, 0, 1), p + Vector3(1, 0, 0))
					if solid[i + PAD] == 0:
						m.quad(Vector3(0, 0, 1), p + Vector3(1, 0, 1), p + Vector3(0, 0, 1), p + Vector3(0, 1, 1), p + Vector3(1, 1, 1))
					if solid[i - PAD] == 0:
						m.quad(Vector3(0, 0, -1), p, p + Vector3(1, 0, 0), p + Vector3(1, 1, 0), p + Vector3(0, 1, 0))
	return m


func chunk_faces(cx: int, cy: int, cz: int, seed: int) -> int:
	return mesh(cx, cy, cz, seed).vertices.size() >> 2


func bench(seed: int, width: int) -> int:
	var faces := 0
	for cx in width:
		for cy in LAYERS:
			for cz in width:
				faces += chunk_faces(cx, cy, cz, seed)
	return faces
