//! Keyframes, tracks and easing. Like the rig logic, this has no interface code,
//! so tests and future plugins can use it directly.
//!
//! A key stores a bone's `Transform` at a frame. Its easing shapes the motion from
//! that key to the next one, which matches how Roblox stores easing on a `Pose`.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::f32::consts::{FRAC_PI_2, PI};
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, OnceLock};

use glam::{EulerRot, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::math::Xf;
use crate::scene::{BoneId, Scene};

pub type FrameNumber = i32;
pub type KeyId = (BoneId, FrameNumber);

/// What the Graph tab draws and edits: position in studs, rotation in degrees, and
/// the bone's extra number.
pub const CHANNELS: usize = 7;
pub const VALUE_CHANNEL: usize = 6;
pub const CHANNEL_NAMES: [&str; 6] = ["Position X", "Position Y", "Position Z", "Rotation X", "Rotation Y", "Rotation Z"];

/// Rotation keys further apart than this would land on the shorter way around.
const MAX_TURN: f32 = 179.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EaseStyle {
    /// Blender's default: curves that smooth through neighboring keys.
    Bezier,
    /// A hand-drawn curve for one span, edited on the Graph tab.
    Custom,
    Linear,
    Constant,
    Sine,
    Quad,
    Cubic,
    Quart,
    Quint,
    Expo,
    Circ,
    Back,
    Elastic,
    Bounce,
}

impl EaseStyle {
    pub const ALL: [EaseStyle; 14] = [
        EaseStyle::Bezier,
        EaseStyle::Custom,
        EaseStyle::Linear,
        EaseStyle::Constant,
        EaseStyle::Sine,
        EaseStyle::Quad,
        EaseStyle::Cubic,
        EaseStyle::Quart,
        EaseStyle::Quint,
        EaseStyle::Expo,
        EaseStyle::Circ,
        EaseStyle::Back,
        EaseStyle::Elastic,
        EaseStyle::Bounce,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EaseStyle::Bezier => "Bezier",
            EaseStyle::Custom => "Custom Curve",
            EaseStyle::Linear => "Linear",
            EaseStyle::Constant => "Constant",
            EaseStyle::Sine => "Sine",
            EaseStyle::Quad => "Quad",
            EaseStyle::Cubic => "Cubic",
            EaseStyle::Quart => "Quart",
            EaseStyle::Quint => "Quint",
            EaseStyle::Expo => "Expo",
            EaseStyle::Circ => "Circ",
            EaseStyle::Back => "Back",
            EaseStyle::Elastic => "Elastic",
            EaseStyle::Bounce => "Bounce",
        }
    }

    pub fn has_direction(self) -> bool {
        !matches!(self, EaseStyle::Bezier | EaseStyle::Custom | EaseStyle::Linear | EaseStyle::Constant)
    }
}

/// How a Bezier key's handles are worked out, named as in Blender.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandleType {
    /// Smooth through neighbors, but never overshoot. Blender's default.
    AutoClamped,
    /// Smooth through neighbors, even if that means overshooting.
    Auto,
    /// Aim straight at the neighboring keys, making a sharp corner.
    Vector,
}

impl HandleType {
    pub const ALL: [HandleType; 3] = [HandleType::AutoClamped, HandleType::Auto, HandleType::Vector];

    pub fn label(self) -> &'static str {
        match self {
            HandleType::AutoClamped => "Auto Clamped",
            HandleType::Auto => "Auto",
            HandleType::Vector => "Vector",
        }
    }
}

/// Blender's Auto Smoothing setting for automatic handles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Smoothing {
    /// Each key only looks at its two neighbors.
    #[default]
    None,
    /// Keys solve together so acceleration stays continuous through them.
    Continuous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EaseDirection {
    In,
    Out,
    InOut,
}

impl EaseDirection {
    pub const ALL: [EaseDirection; 3] = [EaseDirection::In, EaseDirection::Out, EaseDirection::InOut];

    pub fn label(self) -> &'static str {
        match self {
            EaseDirection::In => "In",
            EaseDirection::Out => "Out",
            EaseDirection::InOut => "In Out",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Easing {
    pub style: EaseStyle,
    pub direction: EaseDirection,
    /// Custom Curve control points as x1, y1, x2, y2, from (0,0) to (1,1).
    pub handles: [f32; 4],
    pub handle_type: HandleType,
}

impl Default for Easing {
    /// Bezier with auto-clamped handles is Kaye's default, as in Blender.
    fn default() -> Self {
        Self {
            style: EaseStyle::Bezier,
            direction: EaseDirection::InOut,
            handles: [0.42, 0.0, 0.58, 1.0],
            handle_type: HandleType::AutoClamped,
        }
    }
}

impl Easing {
    /// Maps progress between two keys, from 0 to 1, to how far the pose has moved.
    /// Back and Elastic deliberately leave the 0 to 1 range to overshoot.
    pub fn apply(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self.style {
            // A lone span, drawn as the flat-ended curve auto handles give it.
            EaseStyle::Bezier => bezier([AUTO_HANDLE, 0.0, 1.0 - AUTO_HANDLE, 1.0], t),
            EaseStyle::Custom => bezier(self.handles, t),
            EaseStyle::Linear => t,
            EaseStyle::Constant => {
                if t >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            style => match self.direction {
                EaseDirection::In => ease_in(style, t),
                EaseDirection::Out => 1.0 - ease_in(style, 1.0 - t),
                EaseDirection::InOut if t < 0.5 => ease_in(style, t * 2.0) * 0.5,
                EaseDirection::InOut => 1.0 - ease_in(style, (1.0 - t) * 2.0) * 0.5,
            },
        }
    }
}

fn ease_in(style: EaseStyle, t: f32) -> f32 {
    match style {
        EaseStyle::Sine => 1.0 - (t * FRAC_PI_2).cos(),
        EaseStyle::Quad => t * t,
        EaseStyle::Cubic => t.powi(3),
        EaseStyle::Quart => t.powi(4),
        EaseStyle::Quint => t.powi(5),
        EaseStyle::Expo => {
            if t <= 0.0 {
                0.0
            } else {
                2f32.powf(10.0 * t - 10.0)
            }
        }
        EaseStyle::Circ => 1.0 - (1.0 - t * t).max(0.0).sqrt(),
        EaseStyle::Back => 2.70158 * t.powi(3) - 1.70158 * t * t,
        EaseStyle::Elastic => {
            if t <= 0.0 {
                0.0
            } else if t >= 1.0 {
                1.0
            } else {
                -(2f32.powf(10.0 * t - 10.0)) * ((10.0 * t - 10.75) * (2.0 * PI / 3.0)).sin()
            }
        }
        EaseStyle::Bounce => 1.0 - bounce_out(1.0 - t),
        EaseStyle::Bezier | EaseStyle::Custom | EaseStyle::Linear | EaseStyle::Constant => t,
    }
}

fn bounce_out(t: f32) -> f32 {
    const N: f32 = 7.5625;
    const D: f32 = 2.75;
    if t < 1.0 / D {
        N * t * t
    } else if t < 2.0 / D {
        let t = t - 1.5 / D;
        N * t * t + 0.75
    } else if t < 2.5 / D {
        let t = t - 2.25 / D;
        N * t * t + 0.9375
    } else {
        let t = t - 2.625 / D;
        N * t * t + 0.984375
    }
}

/// Evaluates a cubic bezier at time `t`. Time is the curve's x, so this first
/// searches for the curve position with that x. X is monotonic while the handles
/// stay inside 0 to 1, which the editor enforces.
fn bezier(handles: [f32; 4], t: f32) -> f32 {
    let [x1, y1, x2, y2] = handles;
    let u = solve_bezier_time(x1, x2, t);
    let v = 1.0 - u;
    3.0 * v * v * u * y1 + 3.0 * v * u * u * y2 + u * u * u
}

fn solve_bezier_time(x1: f32, x2: f32, t: f32) -> f32 {
    let curve = |u: f32| {
        let v = 1.0 - u;
        3.0 * v * v * u * x1 + 3.0 * v * u * u * x2 + u * u * u
    };
    let (mut low, mut high) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (low + high) * 0.5;
        if curve(mid) < t {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) * 0.5
}

/// How far along a span an automatic handle reaches. Blender's value for plain
/// auto handles, which comes from its 2.5614 factor.
const AUTO_HANDLE: f32 = 1.0 / 2.5614;
/// Continuous acceleration needs a true cubic, which has thirds for its handles.
const SMOOTH_HANDLE: f32 = 1.0 / 3.0;

/// The XYZ angles of a rotation in degrees, nearest to `near`. One rotation has two
/// sets of angles, each repeating every 360 degrees. Taking the closest keeps
/// curves continuous and rotations going the short way.
pub fn angles_near(rot: Quat, near: [f32; 3]) -> [f32; 3] {
    let (x, y, z) = rot.to_euler(EulerRot::XYZ);
    let (x, y, z) = (x.to_degrees(), y.to_degrees(), z.to_degrees());
    let settle = |angles: [f32; 3]| std::array::from_fn::<f32, 3, _>(|i| angles[i] + ((near[i] - angles[i]) / 360.0).round() * 360.0);
    let distance = |angles: &[f32; 3]| (0..3).map(|i| (angles[i] - near[i]).powi(2)).sum::<f32>();
    let (a, b) = (settle([x, y, z]), settle([x + 180.0, 180.0 - y, z + 180.0]));
    if distance(&a) <= distance(&b) { a } else { b }
}

fn pose_from_channels(c: &[f32; CHANNELS]) -> Xf {
    Xf::new(
        Quat::from_euler(EulerRot::XYZ, c[3].to_radians(), c[4].to_radians(), c[5].to_radians()).normalize(),
        Vec3::new(c[0], c[1], c[2]),
    )
}

/// A handle's slope. Nearly vertical handles still need a finite one.
fn handle_slope(handle: [f32; 2]) -> f32 {
    let run = if handle[0].abs() < 1e-3 { 1e-3f32.copysign(handle[0]) } else { handle[0] };
    handle[1] / run
}

/// Handles set by hand for one channel of one key, as offsets from the key in
/// frames and value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Handles {
    pub left: [f32; 2],
    pub right: [f32; 2],
    /// Moving one handle swings the other, keeping the curve smooth through the key.
    pub aligned: bool,
}

/// One key on one channel, with both handles worked out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CurvePoint {
    pub value: f32,
    pub left: [f32; 2],
    pub right: [f32; 2],
}

/// A handle can't reach past the next key, or time would run backward.
fn limit_handle(handle: [f32; 2], gap: f32) -> [f32; 2] {
    let run = handle[0].abs();
    if run <= gap {
        handle
    } else {
        [handle[0] * gap / run, handle[1] * gap / run]
    }
}

fn span_value(a: &CurvePoint, b: &CurvePoint, gap: f32, t: f32) -> f32 {
    let (out, into) = (limit_handle(a.right, gap), limit_handle(b.left, gap));
    let u = solve_bezier_time(out[0] / gap, 1.0 + into[0] / gap, t);
    let v = 1.0 - u;
    let (p1, p2) = (a.value + out[1], b.value + into[1]);
    v * v * v * a.value + 3.0 * v * v * u * p1 + 3.0 * v * u * u * p2 + u * u * u * b.value
}

/// The curve's slope leaving each side of every key, for one channel. Follows
/// how Blender calculates automatic handles. Handles set by hand are fixed points
/// that the automatic ones work around.
fn channel_slopes(times: &[f32], values: &[f32], handles: &[HandleType], custom: &[Option<Handles>], smoothing: Smoothing) -> Vec<(f32, f32)> {
    let n = values.len();
    if n < 2 {
        return vec![(0.0, 0.0); n];
    }
    let gaps: Vec<f32> = times.windows(2).map(|w| w[1] - w[0]).collect();
    let secants: Vec<f32> = (0..n - 1).map(|i| (values[i + 1] - values[i]) / gaps[i]).collect();
    let reach = match smoothing {
        Smoothing::None => AUTO_HANDLE,
        Smoothing::Continuous => SMOOTH_HANDLE,
    };

    // Keys whose slope is already fixed, which smoothing has to work around.
    let mut locked = vec![false; n];
    let mut slopes = vec![(0.0, 0.0); n];
    for i in 0..n {
        if let Some(set) = custom[i] {
            slopes[i] = (handle_slope(set.left), handle_slope(set.right));
            locked[i] = true;
            continue;
        }
        let left = i.checked_sub(1).map(|p| secants[p]);
        let right = secants.get(i).copied();
        match (handles[i], left, right) {
            (HandleType::Vector, _, _) => {
                slopes[i] = (left.or(right).unwrap_or(0.0), right.or(left).unwrap_or(0.0));
                locked[i] = true;
            }
            // Clamped curves come to rest on the first and last keys.
            (HandleType::AutoClamped, None, _) | (HandleType::AutoClamped, _, None) => locked[i] = true,
            (HandleType::Auto, None, Some(s)) | (HandleType::Auto, Some(s), None) => {
                slopes[i] = (s, s);
                locked[i] = true;
            }
            (HandleType::AutoClamped, Some(a), Some(b)) => {
                if a * b <= 0.0 {
                    // A peak or valley stays flat, which stops overshoot.
                    locked[i] = true;
                } else {
                    let s = (a + b) * 0.5;
                    slopes[i] = (s, s);
                }
            }
            (HandleType::Auto, Some(a), Some(b)) => {
                let s = (a + b) * 0.5;
                slopes[i] = (s, s);
            }
            (_, None, None) => {}
        }
    }

    if smoothing == Smoothing::Continuous {
        solve_smooth_runs(&gaps, &secants, &locked, &mut slopes);
    }

    // Clamped handles can't reach past the value of the neighboring key.
    for i in 1..n - 1 {
        if handles[i] == HandleType::AutoClamped && !locked[i] {
            let limit = (secants[i - 1].abs().min(secants[i].abs())) / reach;
            let s = slopes[i].0.clamp(-limit, limit);
            slopes[i] = (s, s);
        }
    }
    slopes
}

/// Solves each run of free keys between locked ones so acceleration is continuous
/// across them. This is a standard cubic spline, solved with the Thomas algorithm.
fn solve_smooth_runs(gaps: &[f32], secants: &[f32], locked: &[bool], slopes: &mut [(f32, f32)]) {
    let n = locked.len();
    let mut i = 1;
    while i < n - 1 {
        if locked[i] {
            i += 1;
            continue;
        }
        let first = i;
        while i < n - 1 && !locked[i] {
            i += 1;
        }
        let count = i - first;
        let (mut upper, mut rhs) = (vec![0.0f32; count], vec![0.0f32; count]);
        for j in 0..count {
            let k = first + j;
            let (h0, h1) = (gaps[k - 1], gaps[k]);
            let mut b = 2.0 * (h0 + h1);
            let mut d = 3.0 * (h1 * secants[k - 1] + h0 * secants[k]);
            if j == 0 {
                d -= h1 * slopes[k - 1].1;
            } else {
                b -= h1 * upper[j - 1];
                d -= h1 * rhs[j - 1];
            }
            if j == count - 1 {
                d -= h0 * slopes[k + 1].0;
            }
            upper[j] = h0 / b;
            rhs[j] = d / b;
        }
        let mut next = 0.0;
        for j in (0..count).rev() {
            let m = if j == count - 1 { rhs[j] } else { rhs[j] - upper[j] * next };
            slopes[first + j] = (m, m);
            next = m;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Key {
    pub frame: FrameNumber,
    pub pose: Xf,
    /// The bone's extra number at this key, see `Bone::value`.
    pub value: f32,
    pub easing: Easing,
    /// Handles set by hand, per channel. The rest follow `easing.handle_type`.
    #[serde(default, skip_serializing_if = "no_handles")]
    pub handles: [Option<Handles>; CHANNELS],
}

fn no_handles(handles: &[Option<Handles>; CHANNELS]) -> bool {
    handles.iter().all(Option::is_none)
}

/// Every handle on every channel, indexed by channel, then key.
type Points = [Vec<CurvePoint>; CHANNELS];

#[derive(Default)]
struct KeyData {
    keys: Vec<Key>,
    /// Worked out when first asked for, one per `Smoothing`.
    points: [OnceLock<Points>; 2],
}

impl Clone for KeyData {
    /// Only runs when the keys are about to change, so the points wouldn't apply.
    fn clone(&self) -> Self {
        Self { keys: self.keys.clone(), points: Default::default() }
    }
}

/// A track's keys, read and changed like a `Vec`. Copies share one list until one of
/// them changes, which is what keeps undo small, since an edit normally touches one
/// bone. Changing the keys is also the only way to drop the points worked out from
/// them, so those can't go stale.
#[derive(Clone, Default)]
pub struct Keys(Arc<KeyData>);

impl Deref for Keys {
    type Target = Vec<Key>;

    fn deref(&self) -> &Vec<Key> {
        &self.0.keys
    }
}

impl DerefMut for Keys {
    fn deref_mut(&mut self) -> &mut Vec<Key> {
        let data = Arc::make_mut(&mut self.0);
        data.points = Default::default();
        &mut data.keys
    }
}

impl<'a> IntoIterator for &'a Keys {
    type Item = &'a Key;
    type IntoIter = std::slice::Iter<'a, Key>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.keys.iter()
    }
}

impl PartialEq for Keys {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.keys == other.0.keys
    }
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.keys.fmt(f)
    }
}

impl Serialize for Keys {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.keys.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Keys {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let keys = Vec::deserialize(deserializer)?;
        Ok(Self(Arc::new(KeyData { keys, points: Default::default() })))
    }
}

/// One bone's keys, always sorted by frame with no duplicates.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub keys: Keys,
}

/// A track with every handle worked out, ready to read from many times.
pub struct Curves<'a> {
    track: &'a Track,
    /// Indexed by channel, then key.
    pub points: &'a Points,
}

impl Curves<'_> {
    fn span(&self, i: usize, t: f32) -> [f32; CHANNELS] {
        let keys = &self.track.keys;
        let gap = (keys[i + 1].frame - keys[i].frame) as f32;
        std::array::from_fn(|c| span_value(&self.points[c][i], &self.points[c][i + 1], gap, t))
    }

    /// Every channel at any point in time, continuous with the keys on either side.
    /// This is what the Graph tab draws, so it has to match `Track::sample_with`.
    pub fn at(&self, frame: f32) -> Option<[f32; CHANNELS]> {
        let keys = &self.track.keys;
        let next = keys.partition_point(|k| k.frame as f32 <= frame);
        let key_channels = |i: usize| std::array::from_fn::<f32, CHANNELS, _>(|c| self.points[c][i].value);
        let (i, a, b) = match (next.checked_sub(1), keys.get(next)) {
            (Some(i), Some(b)) => (i, &keys[i], b),
            (Some(i), None) => return Some(key_channels(i)),
            (None, Some(_)) => return Some(key_channels(0)),
            (None, None) => return None,
        };
        let t = (frame - a.frame as f32) / (b.frame - a.frame) as f32;
        if a.easing.style == EaseStyle::Bezier {
            return Some(self.span(i, t));
        }
        let (pose, value) = eased(a, b, t);
        let start = key_channels(i);
        let angles = angles_near(pose.rot, [start[3], start[4], start[5]]);
        Some([pose.pos.x, pose.pos.y, pose.pos.z, angles[0], angles[1], angles[2], value])
    }
}

fn eased(a: &Key, b: &Key, t: f32) -> (Xf, f32) {
    let e = a.easing.apply(t);
    let pose = Xf::new(a.pose.rot.slerp(b.pose.rot, e).normalize(), a.pose.pos.lerp(b.pose.pos, e));
    (pose, a.value + (b.value - a.value) * e)
}

impl Track {
    fn index_of(&self, frame: FrameNumber) -> Result<usize, usize> {
        self.keys.binary_search_by_key(&frame, |k| k.frame)
    }

    pub fn index(&self, frame: FrameNumber) -> Option<usize> {
        self.index_of(frame).ok()
    }

    pub fn key_at(&self, frame: FrameNumber) -> Option<&Key> {
        self.index_of(frame).ok().map(|i| &self.keys[i])
    }

    /// Adds a key, or updates the pose of one already on this frame.
    pub fn set_key(&mut self, frame: FrameNumber, pose: Xf) {
        self.set_key_with(frame, pose, Easing::default());
    }

    /// Like `set_key`, with the easing a brand new key should start with.
    pub fn set_key_with(&mut self, frame: FrameNumber, pose: Xf, easing: Easing) {
        self.set_key_full(frame, pose, 1.0, easing);
    }

    /// Like `set_key_with`, also storing the bone's extra number.
    pub fn set_key_full(&mut self, frame: FrameNumber, pose: Xf, value: f32, easing: Easing) {
        match self.index_of(frame) {
            Ok(i) => {
                self.keys[i].pose = pose;
                self.keys[i].value = value;
            }
            Err(i) => self.keys.insert(i, Key { frame, pose, value, easing, handles: [None; CHANNELS] }),
        }
    }

    pub fn insert(&mut self, key: Key) {
        match self.index_of(key.frame) {
            Ok(i) => self.keys[i] = key,
            Err(i) => self.keys.insert(i, key),
        }
    }

    pub fn remove(&mut self, frame: FrameNumber) -> Option<Key> {
        self.index_of(frame).ok().map(|i| self.keys.remove(i))
    }

    /// Stops the key on this frame from easing into the next one.
    fn hold(&mut self, frame: FrameNumber) {
        if let Ok(i) = self.index_of(frame) {
            self.keys[i].easing.style = EaseStyle::Constant;
        }
    }

    /// The last key at or before `frame`, whose easing shapes the current motion.
    pub fn key_before(&self, frame: f32) -> Option<&Key> {
        let next = self.keys.partition_point(|k| k.frame as f32 <= frame);
        next.checked_sub(1).map(|i| &self.keys[i])
    }

    /// The pose at any point in time, without continuous smoothing.
    pub fn sample(&self, frame: f32) -> Option<Xf> {
        self.sample_with(frame, Smoothing::None)
    }

    /// The pose at any point in time. Holds the first and last keys beyond the ends.
    pub fn sample_with(&self, frame: f32, smoothing: Smoothing) -> Option<Xf> {
        self.sample_full(frame, smoothing).map(|(pose, _)| pose)
    }

    /// The pose and the extra number at any point in time.
    pub fn sample_full(&self, frame: f32, smoothing: Smoothing) -> Option<(Xf, f32)> {
        let next = self.keys.partition_point(|k| k.frame as f32 <= frame);
        let (a, b) = match (next.checked_sub(1).map(|i| &self.keys[i]), self.keys.get(next)) {
            (Some(a), Some(b)) => (a, b),
            (Some(only), None) | (None, Some(only)) => return Some((only.pose, only.value)),
            (None, None) => return None,
        };
        if a.frame as f32 == frame {
            return Some((a.pose, a.value));
        }
        let t = (frame - a.frame as f32) / (b.frame - a.frame) as f32;
        if a.easing.style == EaseStyle::Bezier {
            let c = self.curves(smoothing).span(next - 1, t);
            return Some((pose_from_channels(&c), c[VALUE_CHANNEL]));
        }
        Some(eased(a, b, t))
    }

    /// Every key's channels. Each rotation is written close to the key before it,
    /// which is what makes curves take the short way around.
    pub fn key_channels(&self) -> Vec<[f32; CHANNELS]> {
        let mut near = [0.0; 3];
        self.keys
            .iter()
            .map(|key| {
                near = angles_near(key.pose.rot, near);
                let p = key.pose.pos;
                [p.x, p.y, p.z, near[0], near[1], near[2], key.value]
            })
            .collect()
    }

    /// Every handle on every channel, worked out once until the keys change.
    pub fn curves(&self, smoothing: Smoothing) -> Curves<'_> {
        let points = self.keys.0.points[smoothing as usize].get_or_init(|| self.work_out_points(smoothing));
        Curves { track: self, points }
    }

    fn work_out_points(&self, smoothing: Smoothing) -> Points {
        let times: Vec<f32> = self.keys.iter().map(|k| k.frame as f32).collect();
        let types: Vec<HandleType> = self.keys.iter().map(|k| k.easing.handle_type).collect();
        let channels = self.key_channels();
        let reach = match smoothing {
            Smoothing::None => AUTO_HANDLE,
            Smoothing::Continuous => SMOOTH_HANDLE,
        };
        std::array::from_fn(|c| {
            let column: Vec<f32> = channels.iter().map(|k| k[c]).collect();
            let custom: Vec<Option<Handles>> = self.keys.iter().map(|k| k.handles[c]).collect();
            let slopes = channel_slopes(&times, &column, &types, &custom, smoothing);
            (0..column.len())
                .map(|i| {
                    let after = times.get(i + 1).map(|t| t - times[i]);
                    let before = i.checked_sub(1).map(|p| times[i] - times[p]);
                    let (left, right) = (before.or(after).unwrap_or(1.0), after.or(before).unwrap_or(1.0));
                    let (left, right) = match custom[i] {
                        Some(set) => (set.left, set.right),
                        None => ([-left * reach, -slopes[i].0 * left * reach], [right * reach, slopes[i].1 * right * reach]),
                    };
                    CurvePoint { value: column[i], left, right }
                })
                .collect()
        })
    }

    /// Changes one channel of one key and leaves the rest of its pose alone.
    pub fn set_channel(&mut self, index: usize, channel: usize, value: f32) {
        let mut channels = self.key_channels();
        let mut value = value;
        if (3..VALUE_CHANNEL).contains(&channel) {
            // A pose can't store more than the short way to the next key.
            let neighbors = [index.checked_sub(1), Some(index + 1).filter(|i| *i < channels.len())];
            for near in neighbors.into_iter().flatten().map(|i| channels[i][channel]) {
                value = value.clamp(near - MAX_TURN, near + MAX_TURN);
            }
        }
        channels[index][channel] = value;
        let key = &mut self.keys[index];
        match channel {
            VALUE_CHANNEL => key.value = value,
            3..VALUE_CHANNEL => key.pose.rot = pose_from_channels(&channels[index]).rot,
            axis => key.pose.pos[axis] = value,
        }
    }

    /// Sets one handle, keeping it on its own side of the key. An aligned key swings
    /// the other handle to the same slope, keeping that handle's length in time.
    pub fn set_handle(&mut self, index: usize, channel: usize, right: bool, offset: [f32; 2], aligned: bool, smoothing: Smoothing) {
        let point = self.curves(smoothing).points[channel][index];
        let run = if right { offset[0].max(0.01) } else { offset[0].min(-0.01) };
        let moved = [run, offset[1]];
        let mut set = Handles { left: point.left, right: point.right, aligned };
        if right {
            set.right = moved;
        } else {
            set.left = moved;
        }
        if aligned {
            let slope = handle_slope(moved);
            if right {
                set.left[1] = set.left[0] * slope;
            } else {
                set.right[1] = set.right[0] * slope;
            }
        }
        self.keys[index].handles[channel] = Some(set);
    }

    /// Turns a key's automatic handles into ones that stay put, so they can be edited.
    pub fn pin_handles(&mut self, index: usize, channel: usize, aligned: bool, smoothing: Smoothing) {
        let point = self.curves(smoothing).points[channel][index];
        self.keys[index].handles[channel] = Some(Handles { left: point.left, right: point.right, aligned });
    }

    /// Whether this channel of this key has handles set by hand.
    pub fn handle_mode(&self, index: usize, channel: usize) -> Option<bool> {
        self.keys[index].handles[channel].map(|h| h.aligned)
    }
}

/// Keys that add on top of the layers under them. The base layer has no weight of its
/// own, and every other layer starts from no change at all.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    pub weight: f32,
    pub muted: bool,
    /// Empty while this is the active layer, whose keys are in `Animation::tracks`.
    pub tracks: BTreeMap<BoneId, Track>,
}

/// A layer's change, scaled by how strongly it's mixed in.
fn weighted(change: Xf, weight: f32) -> Xf {
    Xf::new(Quat::IDENTITY.slerp(change.rot, weight).normalize(), change.pos * weight)
}

/// How far a merged curve is from what it should show. Over 1 is too far to leave:
/// a hundredth of a stud or a tenth of a degree, each small enough not to be seen.
fn merge_miss(shown: (Xf, f32), wanted: (Xf, f32)) -> f32 {
    let moved = shown.0.pos.distance(wanted.0.pos) / 0.01;
    let turned = shown.0.rot.angle_between(wanted.0.rot).to_degrees() / 0.1;
    let value = (shown.1 - wanted.1).abs() / 0.01;
    moved.max(turned).max(value)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Animation {
    /// The keys of the active layer, so everything that edits keys works on any layer.
    pub tracks: BTreeMap<BoneId, Track>,
    /// Every layer, base first. Empty until a second layer is added.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub active_layer: usize,
    pub start: FrameNumber,
    pub end: FrameNumber,
    pub fps: f32,
    pub looping: bool,
    pub smoothing: Smoothing,
    /// What a brand new key starts with.
    pub default_easing: Easing,
}

impl Default for Animation {
    fn default() -> Self {
        Self {
            tracks: BTreeMap::new(),
            layers: Vec::new(),
            active_layer: 0,
            start: 0,
            end: 120,
            fps: 30.0,
            looping: true,
            smoothing: Smoothing::None,
            default_easing: Easing::default(),
        }
    }
}

impl Animation {
    pub fn key_at(&self, bone: BoneId, frame: FrameNumber) -> Option<&Key> {
        self.tracks.get(&bone).and_then(|t| t.key_at(frame))
    }

    pub fn set_key(&mut self, bone: BoneId, frame: FrameNumber, pose: Xf) {
        self.set_key_full(bone, frame, pose, 1.0);
    }

    pub fn set_key_full(&mut self, bone: BoneId, frame: FrameNumber, pose: Xf, value: f32) {
        let easing = self.default_easing;
        self.tracks.entry(bone).or_default().set_key_full(frame, pose, value, easing);
    }

    pub fn sample(&self, bone: BoneId, frame: f32) -> Option<Xf> {
        self.tracks.get(&bone).and_then(|t| t.sample_with(frame, self.smoothing))
    }

    pub fn remove_key(&mut self, bone: BoneId, frame: FrameNumber) {
        if let Some(track) = self.tracks.get_mut(&bone) {
            track.remove(frame);
            if track.keys.is_empty() {
                self.tracks.remove(&bone);
            }
        }
    }

    /// Keys an edit made on `frame`. A bone's very first key can also store where it
    /// was on the start frame, so posing partway along animates toward the new pose
    /// instead of snapping the whole animation to it.
    pub fn record_edit(&mut self, bone: BoneId, frame: FrameNumber, before: Xf, after: Xf, value: f32, key_start_pose: bool) {
        self.keep_in_base(bone, frame, before, value);
        let (before, start_value) = self.to_layer(bone, frame, before, value);
        if key_start_pose && !self.tracks.contains_key(&bone) && frame != self.start {
            self.set_key_full(bone, self.start, before, start_value);
        }
        self.key_shown(bone, frame, after, value);
    }

    /// Keys a bone as it's showing, whichever layer is active.
    pub fn key_shown(&mut self, bone: BoneId, frame: FrameNumber, pose: Xf, value: f32) {
        self.keep_in_base(bone, frame, pose, value);
        let (pose, value) = self.to_layer(bone, frame, pose, value);
        self.set_key_full(bone, frame, pose, value);
    }

    /// Keys a change as one step on `frame`, with `before` holding right up to it, for
    /// switches. Easing into a switch would drag the bone there over the frames before.
    pub fn key_step(&mut self, bone: BoneId, frame: FrameNumber, before: Xf, before_value: f32, after: Xf, after_value: f32) {
        let held = frame - 1;
        let earlier = self.tracks.get(&bone).and_then(|t| t.keys.first()).is_some_and(|k| k.frame < frame);
        if frame > self.start || earlier {
            if self.key_at(bone, held).is_none() {
                let (pose, value) = match self.tracks.get(&bone).and_then(|t| t.sample_full(held as f32, self.smoothing)) {
                    Some(keyed) => keyed,
                    None => (before, before_value),
                };
                self.keep_in_base(bone, held, before, before_value);
                self.set_key_full(bone, held, pose, value);
            }
            if let Some(track) = self.tracks.get_mut(&bone) {
                track.hold(held);
            }
        }
        self.key_shown(bone, frame, after, after_value);
    }

    /// Carries a switch forward through the bone's later keys, which were made for the
    /// old setting, as far as the next key that wasn't. Each key's pose turns by `change`,
    /// and it takes on the new value.
    pub fn carry_switch(&mut self, bone: BoneId, frame: FrameNumber, old_value: f32, new_value: f32, change: Xf) {
        if let Some(track) = self.tracks.get_mut(&bone) {
            let first = track.keys.partition_point(|k| k.frame <= frame);
            let count = track.keys[first..].iter().take_while(|k| (k.value - old_value).abs() < 1e-3).count();
            for key in track.keys[first..first + count].iter_mut() {
                key.pose = change * key.pose;
                key.value = new_value;
            }
        }
    }

    /// Sets the value of every key before `frame`, for a modifier added partway along,
    /// which shouldn't reach back into the keys already there.
    pub fn set_value_before(&mut self, bone: BoneId, frame: FrameNumber, value: f32) {
        if let Some(track) = self.tracks.get_mut(&bone) {
            let last = track.keys.partition_point(|k| k.frame < frame);
            track.keys[..last].iter_mut().for_each(|k| k.value = value);
        }
    }

    pub fn layer_count(&self) -> usize {
        self.layers.len().max(1)
    }

    /// One layer's keys, wherever they're being kept.
    fn layer_tracks(&self, index: usize) -> &BTreeMap<BoneId, Track> {
        if index == self.active_layer { &self.tracks } else { &self.layers[index].tracks }
    }

    /// Every bone with keys on any layer.
    pub fn keyed_bones(&self) -> BTreeSet<BoneId> {
        (0..self.layer_count()).flat_map(|i| self.layer_tracks(i).keys().copied()).collect()
    }

    /// Every key on every layer, for looking over a whole animation.
    pub fn all_tracks(&self) -> impl Iterator<Item = (&BoneId, &Track)> {
        (0..self.layer_count()).flat_map(|i| self.layer_tracks(i).iter())
    }

    /// Adds a layer on top and makes it the active one.
    pub fn add_layer(&mut self) {
        if self.layers.is_empty() {
            self.layers.push(Layer { name: "Base".to_string(), weight: 1.0, muted: false, tracks: BTreeMap::new() });
        }
        let name = format!("Layer {}", self.layers.len());
        self.layers.push(Layer { name, weight: 1.0, muted: false, tracks: BTreeMap::new() });
        self.set_active_layer(self.layers.len() - 1);
    }

    pub fn set_active_layer(&mut self, index: usize) {
        if index == self.active_layer || index >= self.layers.len() {
            return;
        }
        std::mem::swap(&mut self.tracks, &mut self.layers[self.active_layer].tracks);
        self.active_layer = index;
        std::mem::swap(&mut self.tracks, &mut self.layers[index].tracks);
    }

    /// Takes a layer and its keys away. The base layer stays.
    pub fn remove_layer(&mut self, index: usize) {
        if index == 0 || index >= self.layers.len() {
            return;
        }
        if index == self.active_layer {
            self.set_active_layer(index - 1);
        } else if index < self.active_layer {
            self.active_layer -= 1;
        }
        self.layers.remove(index);
        if self.layers.len() == 1 {
            self.layers.clear();
        }
    }

    /// Flattens a layer into the one under it, leaving what plays as it was. The result
    /// keeps both layers' keys, then gets more only where the curves between them need it
    /// to stay true, so it stays something a person can edit. The weights and hidden
    /// states go into the keys, which leaves the merged layer at full weight and showing.
    pub fn merge_down(&mut self, index: usize) {
        if index == 0 || index >= self.layers.len() {
            return;
        }
        let below = index - 1;
        let bones: BTreeSet<BoneId> = self.layer_tracks(below).keys().chain(self.layer_tracks(index).keys()).copied().collect();
        let merged: BTreeMap<BoneId, Track> = bones.into_iter().map(|bone| (bone, self.merged_track(bone, below, index))).collect();
        if below == self.active_layer {
            self.tracks = merged;
        } else {
            self.layers[below].tracks = merged;
        }
        self.layers[below].weight = 1.0;
        self.layers[below].muted = false;
        self.remove_layer(index);
    }

    /// What two neighboring layers come to together, as keys for the lower one.
    fn merged_track(&self, bone: BoneId, below: usize, index: usize) -> Track {
        let wanted = |frame: f32| -> (Xf, f32) {
            let lower = if below == 0 { self.base_at(bone, frame) } else { self.layer_change(below, bone, frame) };
            let (pose, value) = lower.unwrap_or((Xf::IDENTITY, 0.0));
            let (change, added) = self.layer_change(index, bone, frame).unwrap_or((Xf::IDENTITY, 0.0));
            (pose * change, value + added)
        };
        let (lower, upper) = (self.layer_tracks(below).get(&bone), self.layer_tracks(index).get(&bone));
        let easing_at = |frame: FrameNumber| {
            let key = |track: Option<&Track>| track.and_then(|t| t.key_at(frame)).map(|k| k.easing);
            key(lower).or(key(upper)).unwrap_or(self.default_easing)
        };
        let frames: BTreeSet<FrameNumber> = [lower, upper].into_iter().flatten().flat_map(|t| t.keys.iter().map(|k| k.frame)).collect();
        let mut track = Track::default();
        for frame in &frames {
            let (pose, value) = wanted(*frame as f32);
            track.set_key_full(*frame, pose, value, easing_at(*frame));
        }
        let (Some(first), Some(last)) = (frames.first().copied(), frames.last().copied()) else {
            return track;
        };
        // Each key added reshapes the curve near it, so this goes back over the whole
        // stretch every time. It can't run past one key per frame.
        for _ in first..=last {
            let mut worst: Option<(FrameNumber, f32)> = None;
            for frame in first..=last {
                let Some(shown) = track.sample_full(frame as f32, self.smoothing) else {
                    continue;
                };
                let miss = merge_miss(shown, wanted(frame as f32));
                if miss > 1.0 && worst.is_none_or(|(_, m)| miss > m) {
                    worst = Some((frame, miss));
                }
            }
            let Some((frame, _)) = worst else {
                break;
            };
            let (pose, value) = wanted(frame as f32);
            track.set_key_full(frame, pose, value, easing_at(frame));
        }
        track
    }

    /// What one layer adds to a bone at this point in time.
    fn layer_change(&self, index: usize, bone: BoneId, frame: f32) -> Option<(Xf, f32)> {
        let layer = &self.layers[index];
        if layer.muted {
            return None;
        }
        let (change, value) = self.layer_tracks(index).get(&bone)?.sample_full(frame, self.smoothing)?;
        Some((weighted(change, layer.weight), value * layer.weight))
    }

    /// The layers in a range, added up.
    fn changes(&self, layers: std::ops::Range<usize>, bone: BoneId, frame: f32) -> (Xf, f32) {
        layers.filter_map(|i| self.layer_change(i, bone, frame)).fold((Xf::IDENTITY, 0.0), |(pose, value), (change, added)| (pose * change, value + added))
    }

    /// The base layer's keys at this point in time.
    fn base_at(&self, bone: BoneId, frame: f32) -> Option<(Xf, f32)> {
        self.layer_tracks(0).get(&bone).and_then(|t| t.sample_full(frame, self.smoothing))
    }

    /// Every layer added up. Only for bones with keys on a layer.
    fn layered(&self, bone: BoneId, frame: f32, value: f32) -> (Xf, f32) {
        let (base, base_value) = self.base_at(bone, frame).unwrap_or((Xf::IDENTITY, value));
        let (change, added) = self.changes(1..self.layers.len(), bone, frame);
        (base * change, base_value + added)
    }

    /// A key on any layer but the base is a change from what's under it, so there has
    /// to be something under it. A bone posed but never keyed gets that here.
    fn keep_in_base(&mut self, bone: BoneId, frame: FrameNumber, pose: Xf, value: f32) {
        if self.active_layer > 0 && !self.layers[0].tracks.contains_key(&bone) {
            let easing = self.default_easing;
            self.layers[0].tracks.entry(bone).or_default().set_key_full(frame, pose, value, easing);
        }
    }

    /// What the active layer has to hold for a bone to show this pose and value.
    pub fn to_layer(&self, bone: BoneId, frame: FrameNumber, pose: Xf, value: f32) -> (Xf, f32) {
        if self.layers.is_empty() {
            return (pose, value);
        }
        let (time, active) = (frame as f32, self.active_layer);
        let (above, above_value) = self.changes(active + 1..self.layers.len(), bone, time);
        if active == 0 {
            return (pose * above.inverse(), value - above_value);
        }
        let (base, base_value) = self.base_at(bone, time).unwrap_or((Xf::IDENTITY, value));
        let (between, between_value) = self.changes(1..active, bone, time);
        let change = (base * between).inverse() * pose * above.inverse();
        let layer = &self.layers[active];
        // Nothing keyed here would show, so it doesn't matter what it holds.
        if layer.muted || layer.weight < 1e-3 {
            return (Xf::IDENTITY, 0.0);
        }
        (weighted(change, 1.0 / layer.weight), (value - base_value - between_value - above_value) / layer.weight)
    }

    /// Poses every keyed bone for this point in time. Bones with no keys keep
    /// whatever pose they have.
    pub fn apply(&self, scene: &mut Scene, frame: f32) {
        if self.layers.is_empty() {
            for (bone, track) in &self.tracks {
                if let Some((pose, value)) = track.sample_full(frame, self.smoothing) {
                    scene.bones[*bone].pose = pose;
                    scene.bones[*bone].value = value;
                }
            }
        } else {
            for bone in self.keyed_bones() {
                let (pose, value) = self.layered(bone, frame, scene.bones[bone].value);
                scene.bones[bone].pose = pose;
                scene.bones[bone].value = value;
            }
        }
        scene.evaluate();
    }

    /// Takes a bone off every layer.
    pub fn forget_bone(&mut self, bone: BoneId) {
        self.tracks.remove(&bone);
        self.layers.iter_mut().for_each(|l| {
            l.tracks.remove(&bone);
        });
    }

    /// A copy with these keys shifted in time. A moved key replaces any key already
    /// on the frame it lands on.
    pub fn moved(&self, keys: &HashSet<KeyId>, delta: FrameNumber) -> Animation {
        let mut out = self.clone();
        let mut lifted = Vec::with_capacity(keys.len());
        for (bone, frame) in keys {
            if let Some(track) = out.tracks.get_mut(bone) {
                if let Some(key) = track.remove(*frame) {
                    lifted.push((*bone, key));
                }
            }
        }
        for (bone, mut key) in lifted {
            key.frame += delta;
            out.tracks.entry(bone).or_default().insert(key);
        }
        out.tracks.retain(|_, t| !t.keys.is_empty());
        out
    }

    /// Every frame holding a key on any of these bones.
    pub fn key_frames(&self, bones: &[BoneId]) -> BTreeSet<FrameNumber> {
        bones
            .iter()
            .filter_map(|b| self.tracks.get(b))
            .flat_map(|t| t.keys.iter().map(|k| k.frame))
            .collect()
    }

    /// Moves time forward. Returns the new frame and whether playback continues.
    pub fn advance(&self, frame: f32, seconds: f32) -> (f32, bool) {
        let (start, end) = (self.start as f32, self.end as f32);
        let next = frame.max(start) + seconds * self.fps;
        if next <= end {
            (next, true)
        } else if self.looping && end > start {
            (start + (next - start) % (end - start), true)
        } else {
            (end, false)
        }
    }
}
