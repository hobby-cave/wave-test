@group(0)
@binding(0)
var<storage, write> v_output: array<u32>;

@compute
@workgroup_size(2, 3, 4)
fn forward(@builtin(global_invocation_id) global_id: vec3<u32>) {
}