struct Uniforms { mvp: mat4x4<f32> }
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VertIn  { @location(0) pos: vec3<f32>, @location(1) normal: vec3<f32> }
struct VertOut { @builtin(position) clip: vec4<f32>, @location(0) normal: vec3<f32> }

@vertex
fn vs_main(v: VertIn) -> VertOut {
    return VertOut(u.mvp * vec4<f32>(v.pos, 1.0), v.normal);
}

@fragment
fn fs_main(f: VertOut) -> @location(0) vec4<f32> {
    let light = normalize(vec3<f32>(1.0, 2.0, 3.0));
    let diffuse = max(dot(normalize(f.normal), light), 0.0);
    let color = vec3<f32>(0.8, 0.5, 0.2) * (0.2 + 0.8 * diffuse);
    return vec4<f32>(color, 1.0);
}
