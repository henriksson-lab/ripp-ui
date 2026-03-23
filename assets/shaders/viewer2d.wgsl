struct Uniforms {
    cam_x: f32,
    cam_y: f32,
    zoom:  f32,
    lo:    f32,
    hi:    f32,
    out_w: f32,
    out_h: f32,
    _pad:  f32,
}

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var t_image: texture_2d<f32>;
@group(0) @binding(2) var s_image: sampler;

struct VertOut {
    @builtin(position) pos: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VertOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var o: VertOut;
    o.pos = vec4<f32>(p[idx], 0.0, 1.0);
    return o;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let img = vec2<f32>(textureDimensions(t_image));
    let src_x = u.cam_x + (frag.x - 0.5 - u.out_w * 0.5) / u.zoom;
    let src_y = u.cam_y + (frag.y - 0.5 - u.out_h * 0.5) / u.zoom;
    let uv = vec2<f32>(src_x / img.x, src_y / img.y);
    if (uv.x < 0.0 || uv.x >= 1.0 || uv.y < 0.0 || uv.y >= 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let raw = textureSampleLevel(t_image, s_image, uv, 0.0);
    let range = max(u.hi - u.lo, 1.0 / 255.0);
    let v = clamp((raw.r * 255.0 - u.lo) / range, 0.0, 1.0);
    return vec4<f32>(v, v, v, 1.0);
}
