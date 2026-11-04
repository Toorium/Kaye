//! Onion skin: see-through copies of a rig at the keys or frames either side of the one
//! showing, so a pose can be judged against where it came from and where it goes. The
//! setting lives on `Rig::onion`, and this works out where each copy stands. No interface
//! code.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::animation::{Animation, FrameNumber};
use crate::math::Xf;
use crate::scene::{RigId, RigKind, Scene};

/// The most copies on one side. Past this they only crowd the view.
pub const MAX_GHOSTS: u8 = 8;
/// The longest gap between copies in Frames mode.
pub const MAX_STEP: u16 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnionMode {
    /// The keys either side, for working pose to pose.
    Keys,
    /// Every `step` frames either side, for checking spacing.
    Frames,
}

impl OnionMode {
    pub const ALL: [OnionMode; 2] = [OnionMode::Keys, OnionMode::Frames];

    pub fn label(self) -> &'static str {
        match self {
            OnionMode::Keys => "Keyframes",
            OnionMode::Frames => "Frames",
        }
    }
}

/// A rig's onion skin, and how it looks.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OnionSkin {
    /// Hidden from its eye in the Workspace, without losing the settings.
    pub shown: bool,
    pub mode: OnionMode,
    /// How many copies before the frame showing.
    pub before: u8,
    /// How many copies after it.
    pub after: u8,
    /// Frames between copies, in Frames mode.
    pub step: u16,
    /// How strongly the nearest copy shows, from 0 to 1. Each one further out is fainter.
    pub opacity: f32,
    pub before_color: [u8; 3],
    pub after_color: [u8; 3],
}

impl Default for OnionSkin {
    fn default() -> Self {
        Self {
            shown: true,
            mode: OnionMode::Keys,
            before: 1,
            after: 1,
            step: 2,
            opacity: 0.4,
            // Warm for where it came from, cool for where it goes, as most tools have it.
            before_color: [0xF0, 0x5A, 0x4A],
            after_color: [0x3E, 0xC8, 0x7A],
        }
    }
}

impl OnionSkin {
    /// Keeps values typed into a saved file or a plugin in a range that can be drawn.
    pub fn clamped(mut self) -> Self {
        self.before = self.before.min(MAX_GHOSTS);
        self.after = self.after.min(MAX_GHOSTS);
        self.step = self.step.clamp(1, MAX_STEP);
        self.opacity = if self.opacity.is_finite() { self.opacity.clamp(0.0, 1.0) } else { 0.4 };
        self
    }
}

/// Whether a copy shows where the rig was or where it's going.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Side {
    Before,
    After,
}

/// One copy of a rig, standing as it does on another frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Ghost {
    pub rig: RigId,
    pub frame: FrameNumber,
    pub side: Side,
    /// 1 for the nearest copy on its side, less for each one further out.
    pub strength: f32,
    /// Where each of the rig's parts stands, in `Rig::parts` order.
    pub parts: Vec<Xf>,
    /// Where each of the rig's bones stands, in `Rig::bones` order. Skinned meshes bend by these.
    pub bones: Vec<Xf>,
}

/// Whether this rig can have an onion skin. A camera is a wire drawing with no parts to copy.
pub fn can_have(scene: &Scene, rig: RigId) -> bool {
    scene.rigs.get(rig).is_some_and(|r| !r.removed && r.kind != RigKind::Camera)
}

/// The frames that get a copy, nearest first on each side, each with its strength. Only
/// frames inside `range` count, since that's what plays. `keys` is every frame the rig
/// has a key on.
pub fn ghost_frames(onion: &OnionSkin, keys: &BTreeSet<FrameNumber>, frame: FrameNumber, range: (FrameNumber, FrameNumber)) -> Vec<(FrameNumber, Side, f32)> {
    let onion = onion.clamped();
    let (start, end) = (range.0.min(range.1), range.0.max(range.1));
    let fade = |index: usize, count: u8| 1.0 - index as f32 / count.max(1) as f32;
    let mut out = Vec::new();
    match onion.mode {
        OnionMode::Keys => {
            // A range that runs backward panics, which a frame outside the range would make.
            let below = frame.min(end + 1);
            if below > start {
                let before = keys.range(start..below).rev().take(onion.before as usize);
                out.extend(before.enumerate().map(|(i, f)| (*f, Side::Before, fade(i, onion.before))));
            }
            let above = (frame + 1).max(start);
            if above <= end {
                let after = keys.range(above..=end).take(onion.after as usize);
                out.extend(after.enumerate().map(|(i, f)| (*f, Side::After, fade(i, onion.after))));
            }
        }
        OnionMode::Frames => {
            let step = onion.step as FrameNumber;
            for i in 0..onion.before {
                let at = frame - step * (i as FrameNumber + 1);
                if (start..=end).contains(&at) {
                    out.push((at, Side::Before, fade(i as usize, onion.before)));
                }
            }
            for i in 0..onion.after {
                let at = frame + step * (i as FrameNumber + 1);
                if (start..=end).contains(&at) {
                    out.push((at, Side::After, fade(i as usize, onion.after)));
                }
            }
        }
    }
    out
}

/// Every copy every onion skin in the scene wants, around `frame`. Each frame is played
/// once on a copy of the scene, whichever rigs want it, so modifiers and physics show
/// in the copies just as they will on that frame.
pub fn ghosts(scene: &Scene, animation: &Animation, frame: FrameNumber) -> Vec<Ghost> {
    let mut wanted: BTreeMap<FrameNumber, Vec<(RigId, Side, f32)>> = BTreeMap::new();
    for (rig, details) in scene.rigs.iter().enumerate() {
        let Some(onion) = details.onion.filter(|o| o.shown && can_have(scene, rig)) else {
            continue;
        };
        let keys = animation.key_frames(&scene.live_bones(rig));
        for (at, side, strength) in ghost_frames(&onion, &keys, frame, animation.range_of(rig)) {
            wanted.entry(at).or_default().push((rig, side, strength));
        }
    }
    if wanted.is_empty() {
        return Vec::new();
    }
    let mut copy = scene.clone();
    let mut out = Vec::new();
    for (at, rigs) in wanted {
        animation.apply(&mut copy, at as f32);
        for (rig, side, strength) in rigs {
            let details = &copy.rigs[rig];
            out.push(Ghost {
                rig,
                frame: at,
                side,
                strength,
                parts: details.parts.iter().map(|p| copy.parts[*p].world).collect(),
                bones: details.bones.iter().map(|b| copy.bones[*b].frame).collect(),
            });
        }
    }
    out
}
