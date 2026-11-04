// Viewport shaders. Output is linear; the target view applies sRGB encoding.

struct Globals {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    shadow_matrix: mat4x4<f32>,
    eye: vec4<f32>,
    // x: grid height, y: grid fade distance, z: 1 when shaded, w: 1 with shadows
    params: vec4<f32>,
    // xyz: direction toward the sun, w: shadow softness in map texels
    sun_dir: vec4<f32>,
    // rgb: sun light, w: how dark shadows get
    sun_color: vec4<f32>,
    // rgb: ambient from above, w: 1 when the floor catches shadows
    sky: vec4<f32>,
    // rgb: ambient from below, w: world distance receivers are pushed off themselves
    ground: vec4<f32>,
    // rgb: background at the top, w: background alpha
    background_top: vec4<f32>,
    // rgb: background at the bottom, w: grid opacity
    background_bottom: vec4<f32>,
    // rgb: grid lines, w: size of one shadow map texel in uv
    grid_color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;

// ---- Shadows ---------------------------------------------------------------

// How lit a point is, from 0 in full shadow to 1 in full sun. Averaging a small
// disc of samples gives the soft edge Roblox shadows have.
fn sunlight(world: vec3<f32>, normal: vec3<f32>) -> f32 {
    if (g.params.w < 0.5) {
        return 1.0;
    }
    let lifted = world + normal * g.ground.w;
    let clip = g.shadow_matrix * vec4<f32>(lifted, 1.0);
    let uv = clip.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || clip.z > 1.0) {
        return 1.0;
    }
    const OFFSETS = array<vec2<f32>, 13>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(0.53, 0.13), vec2<f32>(-0.13, 0.53), vec2<f32>(-0.53, -0.13), vec2<f32>(0.13, -0.53),
        vec2<f32>(0.71, 0.71), vec2<f32>(-0.71, 0.71), vec2<f32>(-0.71, -0.71), vec2<f32>(0.71, -0.71),
        vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0), vec2<f32>(-1.0, 0.0), vec2<f32>(0.0, -1.0),
    );
    let radius = g.sun_dir.w * g.grid_color.w;
    var total = 0.0;
    for (var i = 0; i < 13; i++) {
        total += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + OFFSETS[i] * radius, clip.z);
    }
    return total / 13.0;
}

@vertex
fn vs_shadow(v: VertexIn) -> @builtin(position) vec4<f32> {
    let world = v.m0 * v.pos.x + v.m1 * v.pos.y + v.m2 * v.pos.z + v.m3;
    return g.shadow_matrix * world;
}

// ---- Background ------------------------------------------------------------

struct FullscreenOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> FullscreenOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: FullscreenOut;
    out.pos = vec4<f32>(p, 0.0, 1.0);
    out.ndc = p;
    return out;
}

@fragment
fn fs_background(in: FullscreenOut) -> @location(0) vec4<f32> {
    let alpha = g.background_top.w;
    let color = mix(g.background_bottom.rgb, g.background_top.rgb, in.ndc.y * 0.5 + 0.5);
    return vec4<f32>(color * alpha, alpha);
}

// ---- OpenGL ----------------------------------------------------------------

// OpenGL can't view one texture as both Unorm and sRGB, and its copies decode sRGB
// without encoding it again. So there the finished picture passes through here,
// which reads it as linear and encodes it for the texture egui shows.
@group(0) @binding(3) var finished: texture_2d<f32>;

@fragment
fn fs_encode(in: FullscreenOut) -> @location(0) vec4<f32> {
    let c = textureLoad(finished, vec2<i32>(in.pos.xy), 0);
    let low = c.rgb * 12.92;
    let high = 1.055 * pow(c.rgb, vec3<f32>(1.0 / 2.4)) - 0.055;
    return vec4<f32>(select(high, low, c.rgb <= vec3<f32>(0.0031308)), c.a);
}

// ---- Floor grid ------------------------------------------------------------

struct GridOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

fn grid_lines(p: vec2<f32>, spacing: f32, width: f32) -> f32 {
    let c = p / spacing;
    let d = fwidth(c);
    let g2 = abs(fract(c - 0.5) - 0.5) / max(d, vec2<f32>(1e-5));
    return 1.0 - min(min(g2.x, g2.y) / width, 1.0);
}

fn axis_line(v: f32, width: f32) -> f32 {
    return 1.0 - min(abs(v) / (max(fwidth(v), 1e-5) * width), 1.0);
}

@fragment
fn fs_grid(in: FullscreenOut) -> GridOut {
    let near_h = g.inv_view_proj * vec4<f32>(in.ndc, 0.0, 1.0);
    let far_h = g.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let near = near_h.xyz / near_h.w;
    let far = far_h.xyz / far_h.w;
    let t = (g.params.x - near.y) / (far.y - near.y);
    if (t <= 0.0 || t >= 1.0) {
        discard;
    }
    let world = near + (far - near) * t;
    let clip = g.view_proj * vec4<f32>(world, 1.0);
    let p = world.xz;

    let cell = max(fwidth(p).x, fwidth(p).y);
    let minor_fade = 1.0 - smoothstep(0.12, 0.4, cell);
    let mid_fade = 1.0 - smoothstep(0.8, 3.0, cell);
    let minor = grid_lines(p, 1.0, 1.0) * 0.16 * minor_fade;
    let mid = grid_lines(p, 10.0, 1.1) * 0.24 * mid_fade;
    let major = grid_lines(p, 100.0, 1.2) * 0.34;
    var line_alpha = max(max(minor, mid), major);
    var color = g.grid_color.rgb;

    let x_axis = axis_line(p.y, 1.4);
    let z_axis = axis_line(p.x, 1.4);
    if (x_axis > 0.01) {
        color = mix(color, vec3<f32>(0.62, 0.10, 0.12), x_axis);
        line_alpha = max(line_alpha, x_axis * 0.7);
    }
    if (z_axis > 0.01) {
        color = mix(color, vec3<f32>(0.10, 0.22, 0.60), z_axis);
        line_alpha = max(line_alpha, z_axis * 0.7);
    }

    let dist = distance(world, g.eye.xyz);
    let fade = 1.0 - smoothstep(g.params.y * 0.35, g.params.y, dist);
    line_alpha *= fade * g.background_bottom.w;

    // Shadows on the floor are a dark layer underneath the lines.
    var shadow_alpha = 0.0;
    if (g.params.z > 0.5 && g.sky.w > 0.5) {
        shadow_alpha = clamp((1.0 - sunlight(world, vec3<f32>(0.0, 1.0, 0.0))) * g.sun_color.w * 1.3, 0.0, 1.0) * fade;
    }
    let alpha = 1.0 - (1.0 - line_alpha) * (1.0 - shadow_alpha);
    if (alpha < 0.004) {
        discard;
    }
    var out: GridOut;
    out.color = vec4<f32>(color * line_alpha / alpha, alpha);
    out.depth = clip.z / clip.w;
    return out;
}

// ---- Parts and bones -------------------------------------------------------

struct VertexIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) edge: vec3<f32>,
    @location(3) m0: vec4<f32>,
    @location(4) m1: vec4<f32>,
    @location(5) m2: vec4<f32>,
    @location(6) m3: vec4<f32>,
    @location(7) color: vec4<f32>,
    @location(8) edge_color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) edge: vec3<f32>,
    @location(3) color: vec4<f32>,
    @location(4) edge_color: vec4<f32>,
};

@vertex
fn vs_mesh(v: VertexIn) -> VertexOut {
    let world = v.m0 * v.pos.x + v.m1 * v.pos.y + v.m2 * v.pos.z + v.m3;
    // Dividing by squared scale keeps normals correct on stretched boxes.
    let sx = max(dot(v.m0.xyz, v.m0.xyz), 1e-8);
    let sy = max(dot(v.m1.xyz, v.m1.xyz), 1e-8);
    let sz = max(dot(v.m2.xyz, v.m2.xyz), 1e-8);
    let n = v.m0.xyz * (v.normal.x / sx) + v.m1.xyz * (v.normal.y / sy) + v.m2.xyz * (v.normal.z / sz);
    var out: VertexOut;
    out.pos = g.view_proj * world;
    out.world = world.xyz;
    out.normal = n;
    out.edge = v.edge;
    out.color = v.color;
    out.edge_color = v.edge_color;
    return out;
}

// A fixed studio rig, like Blender's solid viewport: soft sky and ground fill, a
// key from above-front, and a weak kicker from the camera side.
fn studio_light(n: vec3<f32>, view: vec3<f32>, albedo: vec3<f32>) -> vec3<f32> {
    let hemi = mix(vec3<f32>(0.20), vec3<f32>(0.64), n.y * 0.5 + 0.5);
    let to_key = normalize(vec3<f32>(0.45, 0.8, -0.55));
    let key = max(dot(n, to_key), 0.0);
    let fill = max(dot(n, view), 0.0);
    let spec = pow(max(dot(n, normalize(view + to_key)), 0.0), 40.0) * 0.08;
    return albedo * (hemi + vec3<f32>(0.46) * key + vec3<f32>(0.20) * fill) + vec3<f32>(spec);
}

// Close to how Roblox lights parts: a sun with soft shadows over a generous ambient,
// with no reflections or tone mapping, so colors stay flat and readable.
fn roblox_light(n: vec3<f32>, world: vec3<f32>, albedo: vec3<f32>) -> vec3<f32> {
    let ambient = mix(g.ground.rgb, g.sky.rgb, n.y * 0.5 + 0.5);
    let facing = max(dot(n, g.sun_dir.xyz), 0.0);
    var lit = 0.0;
    if (facing > 0.0) {
        lit = sunlight(world, n);
    }
    // Shadows take away the sun, and dim the ambient by the darkness setting.
    let shade = mix(1.0 - g.sun_color.w * 0.6, 1.0, lit);
    return albedo * (ambient * shade + g.sun_color.rgb * facing * lit);
}

@fragment
fn fs_mesh(in: VertexOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let view = normalize(g.eye.xyz - in.world);
    var color: vec3<f32>;
    if (g.params.z > 0.5) {
        color = roblox_light(n, in.world, in.color.rgb);
    } else {
        color = studio_light(n, view, in.color.rgb);
    }

    let width = fwidth(in.edge);
    let e = in.edge / max(width, vec3<f32>(1e-5));
    let edge = 1.0 - min(min(min(e.x, e.y), e.z) / 1.2, 1.0);
    color = mix(color, in.edge_color.rgb, edge * in.edge_color.a);
    return vec4<f32>(color, in.color.a);
}

// Bones and joints: almost flat, like a UI overlay, with a dark rim so they hold
// their edge against light and dark surfaces alike.
@fragment
fn fs_overlay(in: VertexOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let ndv = max(dot(n, normalize(g.eye.xyz - in.world)), 0.0);
    let shade = mix(0.42, 1.0, smoothstep(0.0, 0.45, ndv));
    return vec4<f32>(in.color.rgb * shade, 1.0);
}
