//! Unit meshes shared by every part and bone. All are convex and centered, which
//! lets the builder fix winding on its own and keeps the definitions short.

use std::f32::consts::{PI, TAU};

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use crate::scene::BONE_WAIST;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    /// Reaches zero on edges that should be outlined.
    pub edge: [f32; 3],
    pub uv: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MeshKind {
    Cube,
    Sphere,
    /// A lighter ball for a joint drawn small, where the full sphere's detail can't be seen.
    Joint,
    Cylinder,
    Wedge,
    CornerWedge,
    Bone,
    /// A flat strip for lines: x from -0.5 to 0.5, y from 0 to 1, facing +Z.
    Quad,
    /// A picture laid flat: x and y from -0.5 to 0.5 facing +Z, with uv (0, 0) at the
    /// top left, for Decals and Textures.
    Decal,
    /// Boxes with faces laid out to read Roblox's clothing templates.
    ClothedTorso,
    ClothedRightLimb,
    ClothedLeftLimb,
    /// Roblox's classic rounded head, with UVs that lay a face decal over its front.
    Head,
    /// A mesh downloaded from Roblox, by its asset id.
    Asset(u64),
    /// One entry of `FrameInput::skinned`, bent to this frame's pose.
    Skinned(usize),
}

pub const MESH_KINDS: [MeshKind; 13] = [
    MeshKind::Cube,
    MeshKind::Sphere,
    MeshKind::Joint,
    MeshKind::Cylinder,
    MeshKind::Wedge,
    MeshKind::CornerWedge,
    MeshKind::Bone,
    MeshKind::Quad,
    MeshKind::ClothedTorso,
    MeshKind::ClothedRightLimb,
    MeshKind::ClothedLeftLimb,
    MeshKind::Head,
    MeshKind::Decal,
];

#[derive(Clone, Copy, Default)]
pub struct MeshRange {
    pub first_index: u32,
    pub index_count: u32,
    pub base_vertex: i32,
}

#[derive(Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    center: Vec3,
}

impl MeshData {
    fn tri(&mut self, p: [Vec3; 3], n: [Vec3; 3], e: [[f32; 3]; 3]) {
        let base = self.vertices.len() as u32;
        for i in 0..3 {
            self.vertices.push(Vertex {
                pos: p[i].into(),
                normal: n[i].into(),
                edge: e[i],
                uv: [0.0; 2],
            });
        }
        self.indices.extend([base, base + 1, base + 2]);
    }

    /// Flat convex polygon with its outline marked as edges and its internal
    /// triangle seams hidden.
    fn face(&mut self, corners: &[Vec3]) {
        let mid = corners.iter().sum::<Vec3>() / corners.len() as f32;
        let raw = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
        let flip = raw.dot(mid - self.center) < 0.0;
        let normal = if flip { -raw } else { raw }.normalize();
        let count = corners.len();
        for i in 1..count - 1 {
            let a = corners[0];
            let (mut b, mut c) = (corners[i], corners[i + 1]);
            // Component k is zero on the edge opposite vertex k. Pinning it to
            // one hides that edge.
            let mut hide = [false, i + 1 != count - 1, i != 1];
            if flip {
                std::mem::swap(&mut b, &mut c);
                hide.swap(1, 2);
            }
            let mut e = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
            for (k, hidden) in hide.iter().enumerate() {
                if *hidden {
                    e.iter_mut().for_each(|v| v[k] = 1.0);
                }
            }
            self.tri([a, b, c], [normal; 3], e);
        }
    }

    /// Triangle with per-vertex normals and no outline, for curved surfaces.
    fn smooth(&mut self, p: [Vec3; 3], n: [Vec3; 3]) {
        let raw = (p[1] - p[0]).cross(p[2] - p[0]);
        let mid = (p[0] + p[1] + p[2]) / 3.0;
        let e = [[1.0; 3]; 3];
        if raw.dot(mid - self.center) < 0.0 {
            self.tri([p[0], p[2], p[1]], [n[0], n[2], n[1]], e);
        } else {
            self.tri(p, n, e);
        }
    }
}

fn cube(m: &mut MeshData) {
    let c = |x: f32, y: f32, z: f32| Vec3::new(x, y, z) * 0.5;
    let sign = [-1.0, 1.0];
    for s in sign {
        m.face(&[c(s, -1., -1.), c(s, 1., -1.), c(s, 1., 1.), c(s, -1., 1.)]);
        m.face(&[c(-1., s, -1.), c(1., s, -1.), c(1., s, 1.), c(-1., s, 1.)]);
        m.face(&[c(-1., -1., s), c(1., -1., s), c(1., 1., s), c(-1., 1., s)]);
    }
}

/// Sides around a joint ball. Up to `JOINT_MAX_PIXELS` across, 16 sides keep its outline
/// within a quarter pixel of a true circle.
const JOINT_SEGMENTS: u32 = 16;
const JOINT_RINGS: u32 = 8;
/// The widest a joint can be drawn, in pixels, with the lighter ball.
pub const JOINT_MAX_PIXELS: f32 = 24.0;

fn sphere(m: &mut MeshData, rings: u32, segs: u32) {
    let at = |r: u32, s: u32| {
        let (theta, phi) = (PI * r as f32 / rings as f32, TAU * s as f32 / segs as f32);
        Vec3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin())
    };
    for r in 0..rings {
        for s in 0..segs {
            let (a, b, c, d) = (at(r, s), at(r + 1, s), at(r + 1, s + 1), at(r, s + 1));
            if r != 0 {
                m.smooth([a * 0.5, b * 0.5, d * 0.5], [a, b, d]);
            }
            if r != rings - 1 {
                m.smooth([b * 0.5, c * 0.5, d * 0.5], [b, c, d]);
            }
        }
    }
}

/// Roblox cylinders run along the part's X axis.
fn cylinder(m: &mut MeshData) {
    const SEGS: u32 = 32;
    let ring = |i: u32, x: f32| {
        let a = TAU * i as f32 / SEGS as f32;
        Vec3::new(x, a.cos() * 0.5, a.sin() * 0.5)
    };
    let radial = |v: Vec3| Vec3::new(0.0, v.y, v.z).normalize();
    for i in 0..SEGS {
        let (a, b, c, d) = (ring(i, -0.5), ring(i, 0.5), ring(i + 1, 0.5), ring(i + 1, -0.5));
        m.smooth([a, b, c], [radial(a), radial(b), radial(c)]);
        m.smooth([a, c, d], [radial(a), radial(c), radial(d)]);
    }
    for x in [-0.5, 0.5] {
        let cap: Vec<Vec3> = (0..SEGS).map(|i| ring(i, x)).collect();
        m.face(&cap);
    }
}

/// Full bottom, vertical back at +Z, sloping down to the front at -Z.
fn wedge(m: &mut MeshData) {
    m.center = Vec3::new(0.0, -0.15, 0.15);
    let c = |x: f32, y: f32, z: f32| Vec3::new(x, y, z) * 0.5;
    m.face(&[c(-1., -1., -1.), c(1., -1., -1.), c(1., -1., 1.), c(-1., -1., 1.)]);
    m.face(&[c(-1., -1., 1.), c(1., -1., 1.), c(1., 1., 1.), c(-1., 1., 1.)]);
    m.face(&[c(-1., -1., -1.), c(1., -1., -1.), c(1., 1., 1.), c(-1., 1., 1.)]);
    m.face(&[c(-1., -1., -1.), c(-1., -1., 1.), c(-1., 1., 1.)]);
    m.face(&[c(1., -1., -1.), c(1., -1., 1.), c(1., 1., 1.)]);
    m.center = Vec3::ZERO;
}

fn corner_wedge(m: &mut MeshData) {
    m.center = Vec3::new(0.1, -0.2, -0.1);
    let c = |x: f32, y: f32, z: f32| Vec3::new(x, y, z) * 0.5;
    let apex = c(1., 1., -1.);
    let base = [c(-1., -1., -1.), c(1., -1., -1.), c(1., -1., 1.), c(-1., -1., 1.)];
    m.face(&base);
    for i in 0..4 {
        m.face(&[base[i], base[(i + 1) % 4], apex]);
    }
    m.center = Vec3::ZERO;
}

/// A slim spindle: base at the origin, tip at +Y, widest just past the base. Width
/// and length are scaled separately, so the proportions don't change with length.
fn bone(m: &mut MeshData) {
    const WIDTH: f32 = 0.5;
    m.center = Vec3::new(0.0, BONE_WAIST, 0.0);
    let base = Vec3::ZERO;
    let tip = Vec3::Y;
    let waist = [
        Vec3::new(WIDTH, BONE_WAIST, WIDTH),
        Vec3::new(-WIDTH, BONE_WAIST, WIDTH),
        Vec3::new(-WIDTH, BONE_WAIST, -WIDTH),
        Vec3::new(WIDTH, BONE_WAIST, -WIDTH),
    ];
    for i in 0..4 {
        let (a, b) = (waist[i], waist[(i + 1) % 4]);
        m.face(&[base, a, b]);
        m.face(&[tip, a, b]);
    }
    m.center = Vec3::ZERO;
}

/// One rectangle of Roblox's clothing template, in its pixels: left, top, width, height.
type Panel = [f32; 4];
const TEMPLATE_SIZE: [f32; 2] = [585.0, 559.0];

/// The panels of a box, as +X, -X, +Y, -Y, +Z, -Z. The template unfolds around the front
/// (-Z, the way a character faces), so each panel's edge meets the one beside it.
const TORSO_PANELS: [Panel; 6] = [[165., 74., 64., 128.], [361., 74., 64., 128.], [231., 8., 128., 64.], [231., 204., 128., 64.], [427., 74., 128., 128.], [231., 74., 128., 128.]];
const RIGHT_LIMB_PANELS: [Panel; 6] = [[151., 355., 64., 128.], [19., 355., 64., 128.], [217., 289., 64., 64.], [217., 485., 64., 64.], [85., 355., 64., 128.], [217., 355., 64., 128.]];
const LEFT_LIMB_PANELS: [Panel; 6] = [[506., 355., 64., 128.], [374., 355., 64., 128.], [308., 289., 64., 64.], [308., 485., 64., 64.], [440., 355., 64., 128.], [308., 355., 64., 128.]];

/// How far across and down a panel a point on the box is. Seen from outside, as the
/// template is drawn, +X is on the left of the front because a character faces the viewer.
fn panel_spot(face: usize, p: Vec3) -> [f32; 2] {
    match face {
        0 => [0.5 - p.z, 0.5 - p.y],
        1 => [0.5 + p.z, 0.5 - p.y],
        2 => [0.5 - p.x, 0.5 - p.z],
        3 => [0.5 - p.x, 0.5 + p.z],
        4 => [0.5 + p.x, 0.5 - p.y],
        _ => [0.5 - p.x, 0.5 - p.y],
    }
}

fn clothed(m: &mut MeshData, panels: [Panel; 6]) {
    let c = |x: f32, y: f32, z: f32| Vec3::new(x, y, z) * 0.5;
    let faces: [[Vec3; 4]; 6] = [
        [c(1., -1., -1.), c(1., 1., -1.), c(1., 1., 1.), c(1., -1., 1.)],
        [c(-1., -1., -1.), c(-1., 1., -1.), c(-1., 1., 1.), c(-1., -1., 1.)],
        [c(-1., 1., -1.), c(1., 1., -1.), c(1., 1., 1.), c(-1., 1., 1.)],
        [c(-1., -1., -1.), c(1., -1., -1.), c(1., -1., 1.), c(-1., -1., 1.)],
        [c(-1., -1., 1.), c(1., -1., 1.), c(1., 1., 1.), c(-1., 1., 1.)],
        [c(-1., -1., -1.), c(1., -1., -1.), c(1., 1., -1.), c(-1., 1., -1.)],
    ];
    for (face, corners) in faces.iter().enumerate() {
        let first = m.vertices.len();
        m.face(corners);
        let [left, top, width, height] = panels[face];
        for vertex in &mut m.vertices[first..] {
            let [across, down] = panel_spot(face, Vec3::from_array(vertex.pos));
            vertex.uv = [(left + across * width) / TEMPLATE_SIZE[0], (top + down * height) / TEMPLATE_SIZE[1]];
        }
    }
}

/// A cylinder with its ends rounded over, as wide and as tall as a unit box, turned from
/// a profile with the proportions of Roblox's own `head.mesh`. A curved surface has no box
/// edges to outline, so it lights its rim when selected, like a real mesh.
fn head(m: &mut MeshData) {
    const SEGS: u32 = 32;
    const STEPS: u32 = 6;
    const CORNER: f32 = 0.25;
    let mut profile = vec![(0.0, -0.5, 0.0, -1.0)];
    for i in 0..=STEPS {
        let (sin, cos) = (PI * 0.5 * i as f32 / STEPS as f32).sin_cos();
        profile.push((CORNER + CORNER * sin, -CORNER - CORNER * cos, sin, -cos));
    }
    for i in 0..=STEPS {
        let (sin, cos) = (PI * 0.5 * i as f32 / STEPS as f32).sin_cos();
        profile.push((CORNER + CORNER * cos, CORNER + CORNER * sin, cos, sin));
    }
    profile.push((0.0, 0.5, 0.0, 1.0));
    let at = |row: (f32, f32, f32, f32), s: u32| {
        let (sin, cos) = (TAU * s as f32 / SEGS as f32).sin_cos();
        (Vec3::new(row.0 * cos, row.1, row.0 * sin), Vec3::new(row.2 * cos, row.3, row.2 * sin))
    };
    let first = m.vertices.len();
    for rows in profile.windows(2) {
        for s in 0..SEGS {
            let (a, b, c, d) = (at(rows[0], s), at(rows[1], s), at(rows[1], s + 1), at(rows[0], s + 1));
            if rows[0].0 > 0.0 {
                m.smooth([a.0, b.0, d.0], [a.1, b.1, d.1]);
            }
            if rows[1].0 > 0.0 {
                m.smooth([b.0, c.0, d.0], [b.1, c.1, d.1]);
            }
        }
    }
    for vertex in &mut m.vertices[first..] {
        let p = Vec3::from_array(vertex.pos);
        vertex.edge = [2.0; 3];
        // The back of the head points at a clear corner, or a face would show there too.
        vertex.uv = if p.z <= 0.0 { [0.5 - p.x, 0.5 - p.y] } else { [0.01, 0.01] };
    }
}

fn quad(m: &mut MeshData) {
    let corner = |x: f32, y: f32| Vec3::new(x, y, 0.0);
    let (a, b, c, d) = (corner(-0.5, 0.0), corner(0.5, 0.0), corner(0.5, 1.0), corner(-0.5, 1.0));
    m.tri([a, b, c], [Vec3::Z; 3], [[1.0; 3]; 3]);
    m.tri([a, c, d], [Vec3::Z; 3], [[1.0; 3]; 3]);
}

fn decal(m: &mut MeshData) {
    let base = m.vertices.len() as u32;
    for (x, y) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
        m.vertices.push(Vertex { pos: [x, y, 0.0], normal: [0.0, 0.0, 1.0], edge: [1.0; 3], uv: [x + 0.5, 0.5 - y] });
    }
    m.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

pub fn build_all() -> (MeshData, Vec<(MeshKind, MeshRange)>) {
    let mut m = MeshData::default();
    let mut ranges = Vec::new();
    for kind in MESH_KINDS {
        let first_index = m.indices.len() as u32;
        match kind {
            MeshKind::Cube => cube(&mut m),
            MeshKind::Sphere => sphere(&mut m, 18, 32),
            MeshKind::Joint => sphere(&mut m, JOINT_RINGS, JOINT_SEGMENTS),
            MeshKind::Cylinder => cylinder(&mut m),
            MeshKind::Wedge => wedge(&mut m),
            MeshKind::CornerWedge => corner_wedge(&mut m),
            MeshKind::Bone => bone(&mut m),
            MeshKind::Quad => quad(&mut m),
            MeshKind::Decal => decal(&mut m),
            MeshKind::ClothedTorso => clothed(&mut m, TORSO_PANELS),
            MeshKind::ClothedRightLimb => clothed(&mut m, RIGHT_LIMB_PANELS),
            MeshKind::ClothedLeftLimb => clothed(&mut m, LEFT_LIMB_PANELS),
            MeshKind::Head => head(&mut m),
            MeshKind::Asset(_) | MeshKind::Skinned(_) => {}
        }
        ranges.push((
            kind,
            MeshRange {
                first_index,
                index_count: m.indices.len() as u32 - first_index,
                base_vertex: 0,
            },
        ));
    }
    (m, ranges)
}
