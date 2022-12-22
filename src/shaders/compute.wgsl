struct Scene {
    time: f32,
    freq: f32,
    count: u32,
    width: u32,
    height: u32,
}

@group(0)
@binding(0)
var<uniform> scene: Scene;

@group(0)
@binding(1)
var<storage, write> output: array<f32>;

@compute
@workgroup_size(8, 8, 4)
fn step(@builtin(global_invocation_id) id: vec3<u32>) {
    let pos = vec2<f32>(f32(id.x), f32(id.y));
    output[id.x + id.y * scene.width] = 0.5;
}