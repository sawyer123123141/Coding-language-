extends SceneTree
## Headless check of the Kestrel add-on:
##   godot --headless --path kestrel/godot/demo --script res://tests/test_bridge.gd
## Exits with the number of failed checks.

var failures := 0


func check(what: String, got: Variant, want: Variant) -> void:
	if typeof(got) == typeof(want) and got == want:
		print("ok   ", what)
	else:
		failures += 1
		print("FAIL ", what, ": got ", got, ", want ", want)


func _init() -> void:
	var k := Kestrel.new()
	check("load", k.load("res://tests/bridge_test.kes"), true)
	check("functions skip ones Godot can't call", "hidden" in k.functions(), false)
	check("functions list the rest", "add" in k.functions(), true)
	check("int and float", k.run("add", [2, 0.5]), 2.5)
	check("u8 and bool", k.run("over", [4]), true)
	check("i64", k.run("big", [3000000]), 3000000000000)
	check("string", k.run("same", ["héllo"]), "héllo")
	check("a built string comes back", k.run("label", [7]), "hp 7")
	check("[int] in", k.run("sum", [PackedInt32Array([1, 2, 3, 4])]), 10)
	check("[vec3] in and out", k.run("scale", [PackedVector3Array([Vector3(1, 2, 3)]), 2.0]), PackedVector3Array([Vector3(2, 4, 6)]))
	check("vec3", k.run("up", [Vector3(1, 1, 1)]), Vector3(1, 2, 1))
	check("[u8] out", k.run("ramp", [4]), PackedByteArray([0, 1, 2, 3]))
	check("print goes to output and the result comes back", k.run("talk", [41]), 42)
	check("a runtime panic returns null", k.run("crash", [5]), null)
	check("the next call works", k.run("add", [1, 1.0]), 2.0)
	check("unknown function returns null", k.run("nope", []), null)
	check("wrong argument type returns null", k.run("add", ["x", 1.0]), null)
	check("a heap constant is built before the first call", k.run("weight_sum", []), 60)

	# Hot reload: change a file on disk and the next call sees it.
	var path := "user://hot.kes"
	var file := FileAccess.open(path, FileAccess.WRITE)
	file.store_string("pub pure value() -> int { 1 }\n")
	file.close()
	var hot := Kestrel.new()
	check("load hot", hot.load(path), true)
	check("first version", hot.run("value", []), 1)
	OS.delay_msec(1100)
	file = FileAccess.open(path, FileAccess.WRITE)
	file.store_string("pub pure value() -> int { 2 }\n")
	file.close()
	check("refresh sees the change", hot.refresh(), true)
	check("second version", hot.run("value", []), 2)
	OS.delay_msec(1100)
	file = FileAccess.open(path, FileAccess.WRITE)
	file.store_string("pub pure value() -> int { true }\n")
	file.close()
	check("a broken edit keeps the old version", hot.run("value", []), 2)

	var voxels := Kestrel.new()
	check("load voxels", voxels.load("res://voxels.kes"), true)
	var heights: PackedInt32Array = voxels.run("heights", [0, 0, 1337])
	check("heights per chunk", heights.size(), 32 * 32)
	var vertices: PackedVector3Array = voxels.run("vertices", [0, 0, 1337])
	var normals: PackedVector3Array = voxels.run("normals", [0, 0, 1337])
	check("vertices and normals match", vertices.size() == normals.size() and vertices.size() > 0, true)
	# window, draw, draw3d and input open Kestrel's own window, so the add-on won't load them.
	var windowed := Kestrel.new()
	check("a program using the window modules is refused", windowed.load("res://tests/builtin_test.kes"), false)
	check("and nothing is loaded", windowed.run("answer", []), null)
	var drawn := Kestrel.new()
	check("a program using only draw3d is refused too", drawn.load("res://tests/draw3d_test.kes"), false)
	check("and nothing is loaded for it", drawn.run("answer", []), null)
	print("%d failure(s)" % failures)
	quit(failures)
