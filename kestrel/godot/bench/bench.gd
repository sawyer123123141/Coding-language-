extends SceneTree
## Times the voxel mesh generator (a 4x4 world: 16 chunks, vertices and
## normals) in Kestrel, C# and GDScript, all inside Godot:
##   godot --headless --path kestrel/godot/bench --script res://bench.gd

const SEED := 1337


func median_ms(runs: int, work: Callable) -> float:
	var times: Array[float] = []
	for i in runs:
		var start := Time.get_ticks_usec()
		work.call()
		times.append((Time.get_ticks_usec() - start) / 1000.0)
	times.sort()
	return times[runs / 2]


func _init() -> void:
	var start := Time.get_ticks_usec()
	var k := Kestrel.new()
	k.load("res://../demo/voxels.kes")
	var kestrel_load := (Time.get_ticks_usec() - start) / 1000.0
	var fast := Kestrel.new()
	fast.checks = false
	fast.load("res://../demo/voxels.kes")
	var cs: Object = load("res://VoxelsCs.cs").new()
	var gd: Object = load("res://voxels_gd.gd").new()

	# Same output first.
	var counts := [k.run("bench", [SEED, 1]), cs.Bench(SEED, 1), gd.bench(SEED, 1)]
	print("vectors made: Kestrel %d, C# %d, GDScript %d" % counts)

	# Warm up: .NET compiles hot methods again, optimized, after a few dozen calls.
	for i in 30:
		cs.Bench(SEED, 1)
	OS.delay_msec(500)
	for i in 3:
		k.run("bench", [SEED, 1])
		fast.run("bench", [SEED, 1])

	var compute := {
		"Kestrel": median_ms(9, func(): k.run("bench", [SEED, 1])),
		"Kestrel (checks off)": median_ms(9, func(): fast.run("bench", [SEED, 1])),
		"C#": median_ms(9, func(): cs.Bench(SEED, 1)),
		"GDScript": median_ms(3, func(): gd.bench(SEED, 1)),
	}
	var fetch := {
		"Kestrel": median_ms(9, func():
			for cx in 4:
				for cz in 4:
					var v: PackedVector3Array = k.run("vertices", [cx, cz, SEED])
					var n: PackedVector3Array = k.run("normals", [cx, cz, SEED])),
		"Kestrel (checks off)": median_ms(9, func():
			for cx in 4:
				for cz in 4:
					var v: PackedVector3Array = fast.run("vertices", [cx, cz, SEED])
					var n: PackedVector3Array = fast.run("normals", [cx, cz, SEED])),
		"C#": median_ms(9, func():
			for cx in 4:
				for cz in 4:
					var v: PackedVector3Array = cs.Vertices(cx, cz, SEED)
					var n: PackedVector3Array = cs.Normals(cx, cz, SEED)),
		"GDScript": median_ms(3, func():
			for cx in 4:
				for cz in 4:
					var v: PackedVector3Array = gd.vertices(cx, cz, SEED)
					var n: PackedVector3Array = gd.normals(cx, cz, SEED)),
	}
	print("Kestrel load + compile: %.1f ms" % kestrel_load)
	print("| | build all 16 chunks | fetched from GDScript as arrays |")
	print("|---|---:|---:|")
	for name in ["Kestrel", "Kestrel (checks off)", "C#", "GDScript"]:
		print("| %s | %.1f ms | %.1f ms |" % [name, compute[name], fetch[name]])
	quit()
