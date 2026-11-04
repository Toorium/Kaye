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

// The texture on what's being drawn. Parts with none get one clear pixel, so their own
// color shows, which is also what clear parts of a real texture do in Roblox.
@group(1) @binding(0) var part_texture: texture_2d<f32>;
@group(1) @binding(1) var part_sampler: sampler;

// Roblox's material pictures, one slot each: colors (whose alpha says where the part's
// own color tints them), and normal maps.
@group(0) @binding(4) var material_colors: texture_2d_array<f32>;
@group(0) @binding(5) var material_normals: texture_2d_array<f32>;
@group(0) @binding(6) var material_sampler: sampler;

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

    // The two center lines are the axes' colors on a dark background, as in Blender. Those
    // wash out on a light background, so there they go near black, as in Maya.
    let light = dot(g.background_top.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)) > 0.2;
    let x_axis = axis_line(p.y, 1.4);
    let z_axis = axis_line(p.x, 1.4);
    if (x_axis > 0.01) {
        color = mix(color, select(vec3<f32>(0.62, 0.10, 0.12), vec3<f32>(0.004), light), x_axis);
        line_alpha = max(line_alpha, x_axis * 0.7);
    }
    if (z_axis > 0.01) {
        color = mix(color, select(vec3<f32>(0.10, 0.22, 0.60), vec3<f32>(0.004), light), z_axis);
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
    @location(9) uv: vec2<f32>,
    // x: material layer or -1, y: one over the studs a picture covers, z: finish
    @location(10) surface: vec4<f32>,
    // Scale then offset for uv, for a Texture that repeats.
    @location(11) uv_xf: vec4<f32>,
};

struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) edge: vec3<f32>,
    @location(3) color: vec4<f32>,
    @location(4) edge_color: vec4<f32>,
    @location(5) uv: vec2<f32>,
    // Where on the part, in studs from its middle, and which way its face points there.
    @location(6) stud: vec3<f32>,
    @location(7) local_n: vec3<f32>,
    @location(8) @interpolate(flat) surface: vec4<f32>,
    // Which ways a material picture's right and up run here, in the world.
    @location(9) tangent: vec3<f32>,
    @location(10) bitangent: vec3<f32>,
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
    out.uv = v.uv * v.uv_xf.xy + v.uv_xf.zw;
    out.stud = v.pos * vec3<f32>(sqrt(sx), sqrt(sy), sqrt(sz));
    out.local_n = v.normal;
    out.surface = v.surface;
    // Matches the sides `material_uv` lays the picture on.
    let a = abs(v.normal);
    var right = vec3<f32>(1.0, 0.0, 0.0);
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (a.x >= a.y && a.x >= a.z) {
        right = vec3<f32>(0.0, 0.0, 1.0);
    } else if (a.y >= a.z) {
        up = vec3<f32>(0.0, 0.0, -1.0);
    }
    out.tangent = v.m0.xyz * right.x + v.m1.xyz * right.y + v.m2.xyz * right.z;
    out.bitangent = v.m0.xyz * up.x + v.m1.xyz * up.y + v.m2.xyz * up.z;
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

// Where in a material's picture this point is. The picture is laid on whichever side of
// the part's box the point faces most, in studs, so it keeps its size however the part
// is stretched, as Roblox's materials do.
fn material_uv(stud: vec3<f32>, local_n: vec3<f32>, surface: vec4<f32>) -> vec2<f32> {
    let a = abs(local_n);
    let side_x = vec2<f32>(stud.z, -stud.y);
    let side_z = vec2<f32>(stud.x, -stud.y);
    let flat_y = stud.xz;
    return select(select(side_z, flat_y, a.y >= a.z), side_x, a.x >= a.y && a.x >= a.z) * surface.y;
}

@fragment
fn fs_mesh(in: VertexOut) -> @location(0) vec4<f32> {
    let has_material = in.surface.x >= 0.0;
    let muv = material_uv(in.stud, in.local_n, in.surface);
    let layer = i32(max(in.surface.x, 0.0));
    // Sampled every time, since a texture lookup has to happen in step across neighbors.
    let mcolor = textureSample(material_colors, material_sampler, muv, layer);
    let bump = textureSample(material_normals, material_sampler, muv, layer).xyz * 2.0 - 1.0;
    var n = normalize(in.normal);
    if (in.surface.w > 0.5) {
        n = normalize(normalize(in.tangent) * bump.x + normalize(in.bitangent) * bump.y + n * bump.z);
    }
    let view = normalize(g.eye.xyz - in.world);
    let texel = textureSample(part_texture, part_sampler, in.uv);
    // Roblox tints a material's picture with the part's color where the picture's alpha
    // says so, and leaves the rest, such as the grout between bricks, its own color.
    let base = select(in.color.rgb, mcolor.rgb * mix(vec3<f32>(1.0), in.color.rgb, mcolor.a), has_material);
    let finish = in.surface.z;
    var albedo = mix(base, texel.rgb, texel.a);
    var alpha = in.color.a;
    if (finish > 2.5) {
        // A Decal or Texture: its picture tinted by its color, clear where the picture is.
        albedo = texel.rgb * in.color.rgb;
        alpha = texel.a * in.color.a;
    }
    var color: vec3<f32>;
    if (g.params.z > 0.5) {
        color = roblox_light(n, in.world, albedo);
    } else {
        color = studio_light(n, view, albedo);
    }
    if (finish > 0.5 && finish < 1.5) {
        // Neon is lit from within, so light and shadow barely touch it.
        color = mix(albedo * 1.45, color, 0.12);
    } else if (finish > 1.5 && finish < 2.5) {
        // Metals, glass and ice catch a sharp highlight from the light.
        let to_light = select(normalize(vec3<f32>(0.45, 0.8, -0.55)), g.sun_dir.xyz, g.params.z > 0.5);
        let shine = pow(max(dot(n, normalize(view + to_light)), 0.0), 48.0);
        color += vec3<f32>(shine * 0.32);
    }

    let width = fwidth(in.edge);
    let e = in.edge / max(width, vec3<f32>(1e-5));
    let edge = 1.0 - min(min(min(e.x, e.y), e.z) / 1.2, 1.0);
    color = mix(color, in.edge_color.rgb, edge * in.edge_color.a);
    // Real meshes have no box edges to outline, so selection lights their rim.
    if (in.edge.x > 1.5) {
        let rim = pow(1.0 - max(dot(n, view), 0.0), 2.5);
        color = mix(color, in.edge_color.rgb, rim * in.edge_color.a);
    }
    return vec4<f32>(color, alpha);
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
