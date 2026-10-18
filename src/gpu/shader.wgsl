// Shared CPU/GPU ABI is defined by prepare.rs; geometry remains indexed and instanced.
struct Uniforms {
    view: vec4<f32>,
    settings: vec4<u32>,
};
@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var samples_image: texture_2d<f32>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) face_normal: vec3<f32>,
    @location(3) color: vec4<f32>,
    @location(4) flags: vec4<f32>,
    @location(5) clip_x: vec4<f32>,
    @location(6) clip_y: vec4<f32>,
    @location(7) clip_z: vec4<f32>,
    @location(8) normal_x: vec4<f32>,
    @location(9) normal_y: vec4<f32>,
    @location(10) normal_z: vec4<f32>,
    @location(11) instance_flags: vec4<f32>,
};
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) @interpolate(flat) geometric: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) @interpolate(flat) flags: vec4<f32>,
};
fn unit_or_zero(v: vec3<f32>) -> vec3<f32> {
    let magnitude = length(v);
    if magnitude > 1e-14 { return v / magnitude; }
    return vec3<f32>(0.0);
}
@vertex fn mesh_vertex(input: VertexInput) -> VertexOutput {
    let p = vec4<f32>(input.position, 1.0);
    let n = vec4<f32>(input.normal, 0.0);
    let f = vec4<f32>(input.face_normal, 0.0);
    var output: VertexOutput;
    output.position = vec4<f32>(dot(input.clip_x, p), dot(input.clip_y, p), dot(input.clip_z, p), 1.0);
    output.normal = unit_or_zero(vec3<f32>(dot(input.normal_x, n), dot(input.normal_y, n), dot(input.normal_z, n)));
    output.geometric = unit_or_zero(vec3<f32>(dot(input.normal_x, f), dot(input.normal_y, f), dot(input.normal_z, f))) * input.instance_flags.x;
    output.color = input.color;
    output.flags = input.flags;
    return output;
}
fn linear_to_srgb(value: vec3<f32>) -> vec3<f32> {
    let clamped = clamp(value, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(clamped, vec3<f32>(1.0 / 2.4)) - 0.055, clamped * 12.92, clamped <= vec3<f32>(0.0031308));
}
fn srgb_to_linear(value: vec3<f32>) -> vec3<f32> {
    return select(pow((value + 0.055) / 1.055, vec3<f32>(2.4)), value / 12.92, value <= vec3<f32>(0.04045));
}
fn bytes(value: vec4<f32>) -> vec4<f32> {
    return floor(clamp(value, vec4<f32>(0.0), vec4<f32>(1.0)) * 255.0 + 0.5) / 255.0;
}
@fragment fn mesh_fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    let geometric = unit_or_zero(input.geometric);
    let facing = dot(geometric, uniforms.view.xyz);
    let light = normalize(vec3<f32>(-0.35, -0.6, 1.0));
    var normal = geometric;
    if input.flags.x > 0.5 {
        if input.flags.z < 0.5 && facing <= 0.0 { discard; }
        if input.flags.w > 0.5 {
            let authored = unit_or_zero(input.normal);
            if length(authored) > 0.0 { normal = authored; }
        }
        if input.flags.z > 0.5 && facing < 0.0 { normal = -normal; }
        var color = input.color.rgb;
        if input.flags.y < 0.5 {
            color *= 0.35 + 0.65 * max(dot(normal, light), 0.0);
        }
        return bytes(vec4<f32>(linear_to_srgb(color), 1.0));
    }
    if facing < 0.0 || (facing == 0.0 && dot(normal, light) < 0.0) { normal = -normal; }
    let intensity = 0.35 + 0.65 * max(dot(normal, light), 0.0);
    return bytes(vec4<f32>(input.color.rgb * intensity, 1.0));
}
@vertex fn resolve_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let positions = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(positions[index], 0.0, 1.0);
}
@fragment fn resolve_fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let samples = uniforms.settings.x;
    let origin = vec2<u32>(position.xy) * samples;
    if samples == 1u { return textureLoad(samples_image, vec2<i32>(origin), 0); }
    var rgb = vec3<f32>(0.0);
    var alpha = 0.0;
    for (var y = 0u; y < samples; y += 1u) {
        for (var x = 0u; x < samples; x += 1u) {
            let pixel = textureLoad(samples_image, vec2<i32>(origin + vec2<u32>(x, y)), 0);
            alpha += pixel.a;
            var color = pixel.rgb;
            if uniforms.settings.w != 0u { color = srgb_to_linear(color); }
            rgb += color * pixel.a;
        }
    }
    if alpha > 0.0 {
        rgb /= alpha;
        if uniforms.settings.w != 0u { rgb = linear_to_srgb(rgb); }
    }
    return bytes(vec4<f32>(rgb, alpha / f32(samples * samples)));
}
