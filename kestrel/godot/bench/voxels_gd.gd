extends RefCounted
## The same voxel mesh generator as demo/voxels.kes, in typed GDScript.
## (GDScript floats are 64-bit, so a height can round differently now and then.)

const SIZE := 32
const HEIGHT := 40


func hash_at(x: int, z: int, seed: int) -> int:
	var h: int = x * 73856093 + z * 19349663 + seed * 83492791
	return h % 1000003


func lattice(x: int, z: int, seed: int) -> float:
	return float(hash_at(x, z, seed)) / 1000003.0


func smooth(t: float) -> float:
	return t * t * (3.0 - 2.0 * t)


func noise(x: int, z: int, scale: int, seed: int) -> float:
	var cx: int = x / scale
	var cz: int = z / scale
	var fx := smooth(float(x - cx * scale) / float(scale))
	var fz := smooth(float(z - cz * scale) / float(scale))
	var a := lattice(cx, cz, seed)
	var b := lattice(cx + 1, cz, seed)
	var c := lattice(cx, cz + 1, seed)
	var d := lattice(cx + 1, cz + 1, seed)
	var top := a + (b - a) * fx
	var bottom := c + (d - c) * fx
	return top + (bottom - top) * fz


func height(x: int, z: int, seed: int) -> int:
	var n := noise(x, z, 64, seed) * 0.6 + noise(x, z, 16, seed + 1) * 0.3 + noise(x, z, 4, seed + 2) * 0.1
	return int(n * float(HEIGHT)) + 1


func quad(o: PackedVector3Array, normals: bool, n: Vector3, a: Vector3, b: Vector3, c: Vector3, d: Vector3) -> void:
	if normals:
		for k in 6:
			o.append(n)
	else:
		o.append(a); o.append(b); o.append(c)
		o.append(a); o.append(c); o.append(d)


func mesh(cx: int, cz: int, seed: int, normals: bool) -> PackedVector3Array:
	var o := PackedVector3Array()
	for i in SIZE * SIZE:
		var x: int = cx * SIZE + i % SIZE
		var z: int = cz * SIZE + i / SIZE
		var h := height(x, z, seed)
		var fx := float(x)
		var fz := float(z)
		var fh := float(h)
		quad(o, normals, Vector3(0, 1, 0), Vector3(fx, fh, fz), Vector3(fx + 1, fh, fz), Vector3(fx + 1, fh, fz + 1), Vector3(fx, fh, fz + 1))
		var east := float(height(x + 1, z, seed))
		if east < fh:
			quad(o, normals, Vector3(1, 0, 0), Vector3(fx + 1, east, fz), Vector3(fx + 1, east, fz + 1), Vector3(fx + 1, fh, fz + 1), Vector3(fx + 1, fh, fz))
		var west := float(height(x - 1, z, seed))
		if west < fh:
			quad(o, normals, Vector3(-1, 0, 0), Vector3(fx, west, fz + 1), Vector3(fx, west, fz), Vector3(fx, fh, fz), Vector3(fx, fh, fz + 1))
		var south := float(height(x, z + 1, seed))
		if south < fh:
			quad(o, normals, Vector3(0, 0, 1), Vector3(fx + 1, south, fz + 1), Vector3(fx, south, fz + 1), Vector3(fx, fh, fz + 1), Vector3(fx + 1, fh, fz + 1))
		var north := float(height(x, z - 1, seed))
		if north < fh:
			quad(o, normals, Vector3(0, 0, -1), Vector3(fx, north, fz), Vector3(fx + 1, north, fz), Vector3(fx + 1, fh, fz), Vector3(fx, fh, fz))
	return o


func vertices(cx: int, cz: int, seed: int) -> PackedVector3Array:
	return mesh(cx, cz, seed, false)


func normals(cx: int, cz: int, seed: int) -> PackedVector3Array:
	return mesh(cx, cz, seed, true)


func bench(seed: int, reps: int) -> int:
	var total := 0
	for r in reps:
		for cx in 4:
			for cz in 4:
				total += mesh(cx, cz, seed, false).size() + mesh(cx, cz, seed, true).size()
	return total
