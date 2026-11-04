//! Unit meshes shared by every part and bone. All are convex and centered, which
//! lets the builder fix winding on its own and keeps the definitions short.

use std::f32::consts::{PI, TAU};

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    /// Reaches zero on edges that should be outlined.
    pub edge: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MeshKind {
    Cube,
    Sphere,
    Cylinder,
    Wedge,
    CornerWedge,
    Bone,
    /// A flat strip for lines: x from -0.5 to 0.5, y from 0 to 1, facing +Z.
    Quad,
}

pub const MESH_KINDS: [MeshKind; 7] = [
    MeshKind::Cube,
    MeshKind::Sphere,
    MeshKind::Cylinder,
    MeshKind::Wedge,
    MeshKind::CornerWedge,
    MeshKind::Bone,
    MeshKind::Quad,
];

#[derive(Clone, Copy, Default)]
pub struct MeshRange {
    pub first_index: u32,
    pub index_count: u32,
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

fn sphere(m: &mut MeshData) {
    const RINGS: u32 = 18;
    const SEGS: u32 = 32;
    let at = |r: u32, s: u32| {
        let (theta, phi) = (PI * r as f32 / RINGS as f32, TAU * s as f32 / SEGS as f32);
        Vec3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin())
    };
    for r in 0..RINGS {
        for s in 0..SEGS {
            let (a, b, c, d) = (at(r, s), at(r + 1, s), at(r + 1, s + 1), at(r, s + 1));
            if r != 0 {
                m.smooth([a * 0.5, b * 0.5, d * 0.5], [a, b, d]);
            }
            if r != RINGS - 1 {
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
    const WAIST_Y: f32 = 0.05;
    const WIDTH: f32 = 0.5;
    m.center = Vec3::new(0.0, WAIST_Y, 0.0);
    let base = Vec3::ZERO;
    let tip = Vec3::Y;
    let waist = [
        Vec3::new(WIDTH, WAIST_Y, WIDTH),
        Vec3::new(-WIDTH, WAIST_Y, WIDTH),
        Vec3::new(-WIDTH, WAIST_Y, -WIDTH),
        Vec3::new(WIDTH, WAIST_Y, -WIDTH),
    ];
    for i in 0..4 {
        let (a, b) = (waist[i], waist[(i + 1) % 4]);
        m.face(&[base, a, b]);
        m.face(&[tip, a, b]);
    }
    m.center = Vec3::ZERO;
}

fn quad(m: &mut MeshData) {
    let corner = |x: f32, y: f32| Vec3::new(x, y, 0.0);
    let (a, b, c, d) = (corner(-0.5, 0.0), corner(0.5, 0.0), corner(0.5, 1.0), corner(-0.5, 1.0));
    m.tri([a, b, c], [Vec3::Z; 3], [[1.0; 3]; 3]);
    m.tri([a, c, d], [Vec3::Z; 3], [[1.0; 3]; 3]);
}

pub fn build_all() -> (MeshData, Vec<(MeshKind, MeshRange)>) {
    let mut m = MeshData::default();
    let mut ranges = Vec::new();
    for kind in MESH_KINDS {
        let first_index = m.indices.len() as u32;
        match kind {
            MeshKind::Cube => cube(&mut m),
            MeshKind::Sphere => sphere(&mut m),
            MeshKind::Cylinder => cylinder(&mut m),
            MeshKind::Wedge => wedge(&mut m),
            MeshKind::CornerWedge => corner_wedge(&mut m),
            MeshKind::Bone => bone(&mut m),
            MeshKind::Quad => quad(&mut m),
        }
        ranges.push((
            kind,
            MeshRange {
                first_index,
                index_count: m.indices.len() as u32 - first_index,
            },
        ));
    }
    (m, ranges)
}
