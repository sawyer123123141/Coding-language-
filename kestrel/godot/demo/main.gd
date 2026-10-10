extends Node3D
## Builds voxel terrain with Kestrel: `voxels.kes` computes every chunk's
## mesh, and this script only hands the arrays to Godot. Save `voxels.kes`
## while the scene runs and the terrain rebuilds.

const CHUNKS := 4
const SEED := 1337

## Grass low down, rock above, snow on the peaks; walls a little darker.
const TERRAIN_SHADER := """
shader_type spatial;
varying float height;
varying float up;
void vertex() {
	height = VERTEX.y;
	up = NORMAL.y;
}
void fragment() {
	vec3 grass = vec3(0.32, 0.58, 0.26);
	vec3 rock = vec3(0.52, 0.48, 0.43);
	vec3 snow = vec3(0.94, 0.96, 1.0);
	vec3 c = mix(grass, rock, smoothstep(17.0, 23.0, height));
	c = mix(c, snow, smoothstep(27.0, 31.0, height));
	ALBEDO = c * mix(0.72, 1.0, up);
}
"""

var kestrel := Kestrel.new()
var since_check := 0.0

@onready var terrain: Node3D = $Terrain
@onready var material := ShaderMaterial.new()


func _ready() -> void:
	var shader := Shader.new()
	shader.code = TERRAIN_SHADER
	material.shader = shader
	var size := CHUNKS * 32.0
	$Camera3D.look_at_from_position(Vector3(size * 0.5, size * 0.55, size * 1.25), Vector3(size * 0.5, 8, size * 0.45))
	if kestrel.load("res://voxels.kes"):
		build()


func _process(delta: float) -> void:
	since_check += delta
	if since_check > 0.5:
		since_check = 0.0
		if kestrel.refresh():
			build()


func build() -> void:
	for child in terrain.get_children():
		child.queue_free()
	var started := Time.get_ticks_usec()
	var vertex_count := 0
	for cx in CHUNKS:
		for cz in CHUNKS:
			var vertices: PackedVector3Array = kestrel.run("vertices", [cx, cz, SEED])
			var normals: PackedVector3Array = kestrel.run("normals", [cx, cz, SEED])
			if vertices.is_empty():
				continue
			var arrays := []
			arrays.resize(Mesh.ARRAY_MAX)
			arrays[Mesh.ARRAY_VERTEX] = vertices
			arrays[Mesh.ARRAY_NORMAL] = normals
			var mesh := ArrayMesh.new()
			mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
			var chunk := MeshInstance3D.new()
			chunk.mesh = mesh
			chunk.material_override = material
			terrain.add_child(chunk)
			vertex_count += vertices.size()
	var ms := (Time.get_ticks_usec() - started) / 1000.0
	print("Kestrel built %d chunks (%d vertices) in %.1f ms" % [CHUNKS * CHUNKS, vertex_count, ms])
