extends SceneTree
## Times the 3D voxel world (bench/voxels3d: 8x4x8 chunks of 32^3 blocks,
## density from 3D noise, face-culled meshes) in Kestrel, C# and GDScript,
## all inside Godot:
##   godot --headless --path kestrel/godot/bench --script res://bench3d.gd
## GDScript only builds one column of 4 chunks (1/64 of the world); the
## whole world would take minutes.

const SEED := 1337
const WIDTH := 8
const SOURCE := "res://../../bench/voxels3d/voxels3d.kes"


func median_ms(runs: int, work: Callable) -> float:
	var times: Array[float] = []
	for i in runs:
		var start := Time.get_ticks_usec()
		work.call()
		times.append((Time.get_ticks_usec() - start) / 1000.0)
	times.sort()
	return times[runs / 2]


## Faces in the column of chunks at (0, 0), built with `chunk_faces`.
func column(chunk_faces: Callable) -> int:
	var faces := 0
	for cy in 4:
		faces += chunk_faces.call(0, cy, 0, SEED)
	return faces


func _init() -> void:
	var k := Kestrel.new()
	k.load(SOURCE)
	var fast := Kestrel.new()
	fast.checks = false
	fast.load(SOURCE)
	var cs: Object = load("res://Voxels3dGodot.cs").new()
	var gd: Object = load("res://voxels3d_gd.gd").new()
	var columns := {
		"Kestrel": func(): return column(func(x, y, z, s): return k.run("chunk_faces", [x, y, z, s])),
		"Kestrel (checks off)": func(): return column(func(x, y, z, s): return fast.run("chunk_faces", [x, y, z, s])),
		"C#": func(): return column(Callable(cs, "ChunkFaces")),
		"GDScript": func(): return column(Callable(gd, "chunk_faces")),
	}
	var worlds := {
		"Kestrel": func(): return k.run("bench", [SEED, WIDTH]),
		"Kestrel (checks off)": func(): return fast.run("bench", [SEED, WIDTH]),
		"C#": func(): return cs.Bench(SEED, WIDTH),
	}

	# Same output first.
	var outputs := {}
	for name in columns:
		outputs["column, " + name] = columns[name].call()
	for name in worlds:
		outputs["world, " + name] = worlds[name].call()
	for key in outputs:
		print("%s: %d faces" % [key, outputs[key]])
		var expected: int = outputs["column, C#" if key.begins_with("column") else "world, C#"]
		if outputs[key] != expected:
			push_error("%s made %d faces, but C# made %d" % [key, outputs[key], expected])
			quit(1)
			return

	# Warm up: .NET compiles hot methods again, optimized, after a few dozen calls.
	for i in 30:
		cs.ChunkFaces(0, 1, 0, SEED)
	OS.delay_msec(500)

	var names := ["Kestrel", "Kestrel (checks off)", "C#", "GDScript"]
	var world_ms := {}
	var column_ms := {}
	for name in names:
		column_ms[name] = median_ms(3 if name == "GDScript" else 9, columns[name])
		if name in worlds:
			world_ms[name] = median_ms(5, worlds[name])
	print("| | Whole world (256 chunks) | One column (4 chunks) |")
	print("|---|---:|---:|")
	for name in names:
		var world := "%.0f ms" % world_ms[name] if name in world_ms else "~%.0f ms (column x 64)" % (column_ms[name] * 64.0)
		print("| %s | %s | %.1f ms |" % [name, world, column_ms[name]])
	quit()
