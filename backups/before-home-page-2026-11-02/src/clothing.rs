//! Classic clothing on R15 bodies. Like the rig logic, this has no interface code.
//!
//! An R15 body part's UVs don't point at a shirt or pants template. They point into an
//! atlas Roblox draws first, one for each limb and one for the torso, by copying panels
//! out of the template. Sampling the template directly scrambles it. Every rectangle here
//! was read from `R15CompositLeftArmBase.mesh`, `R15CompositRightArmBase.mesh` and
//! `R15CompositTorsoBase.mesh` in Roblox's `content/avatar/compositing` folder. Legs use
//! the arm layouts, because pants templates are laid out like shirts.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::assets::{TextureData, with_smaller_sizes};

const TEMPLATE: [f32; 2] = [585.0, 559.0];
const LIMB_SIZE: [u32; 2] = [264, 284];
const TORSO_SIZE: [u32; 2] = [388, 272];
/// How far past a panel its edge color carries, so filtering at a seam finds no gap.
const BLEED: f32 = 2.0;
/// A template this many times the usual size keeps its detail, and no more.
const MOST_DETAIL: u32 = 4;

/// One rectangle copied from a template. `atlas` is left, bottom, right, top, counting up
/// from the atlas's bottom edge as Roblox does. `template` is the spot in the template
/// that lands on the bottom left corner, then the one on the top right, so a panel can
/// be flipped or squeezed down to a single row.
#[derive(Clone, Copy)]
struct Panel {
    atlas: [f32; 4],
    template: [f32; 4],
}

fn limb_panels(right: f32, front: f32, left: f32, back: f32) -> Vec<Panel> {
    let mut panels = Vec::new();
    for (i, column) in [right, front, left, back].into_iter().enumerate() {
        let x = 2.0 + 64.0 * i as f32;
        panels.push(Panel { atlas: [x, 106.0, x + 64.0, 218.0], template: [column, 467.0, column + 64.0, 355.0] });
    }
    // The hand or foot gets the template's last 16 rows, and its back sits apart.
    for (i, column) in [right, front, left].into_iter().enumerate() {
        let x = 2.0 + 64.0 * i as f32;
        panels.push(Panel { atlas: [x, 50.0, x + 64.0, 66.0], template: [column, 483.0, column + 64.0, 467.0] });
    }
    panels.push(Panel { atlas: [2.0, 30.0, 66.0, 46.0], template: [back, 483.0, back + 64.0, 467.0] });
    panels.push(Panel { atlas: [2.0, 218.0, 66.0, 282.0], template: [front + 64.0, 353.0, front, 289.0] });
    panels.push(Panel { atlas: [198.0, 2.0, 262.0, 66.0], template: [front, 549.0, front + 64.0, 485.0] });
    // The caps inside a joint, which only show when it bends. Roblox smears one row across them.
    for (atlas, row) in [([162.0, 14.0, 194.0, 46.0], 467.0), ([2.0, 70.0, 34.0, 102.0], 416.0), ([38.0, 70.0, 70.0, 102.0], 467.0), ([70.0, 222.0, 102.0, 254.0], 418.5)] {
        panels.push(Panel { atlas, template: [front, row, front + 64.0, row] });
    }
    panels
}

fn torso_panels() -> Vec<Panel> {
    let mut panels = vec![
        Panel { atlas: [2.0, 6.0, 130.0, 70.0], template: [231.0, 268.0, 359.0, 204.0] },
        Panel { atlas: [2.0, 198.0, 130.0, 262.0], template: [231.0, 72.0, 359.0, 8.0] },
    ];
    for (atlas, column, width) in [([2.0, 130.0], 231.0, 128.0), ([130.0, 194.0], 361.0, 64.0), ([194.0, 322.0], 427.0, 128.0), ([322.0, 386.0], 165.0, 64.0)] {
        panels.push(Panel { atlas: [atlas[0], 70.0, atlas[1], 198.0], template: [column, 202.0, column + width, 74.0] });
    }
    for atlas in [[134.0, 34.0, 198.0, 66.0], [134.0, 202.0, 198.0, 234.0]] {
        panels.push(Panel { atlas, template: [231.0, 170.0, 359.0, 170.0] });
    }
    panels
}

/// One of the textures Roblox draws for an R15 body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BodyAtlas {
    Torso,
    RightArm,
    LeftArm,
    RightLeg,
    LeftLeg,
}

impl BodyAtlas {
    /// The atlas an R15 body part draws from, by its name.
    pub fn of_part(name: &str) -> Option<Self> {
        Some(match name {
            "UpperTorso" | "LowerTorso" => BodyAtlas::Torso,
            "RightUpperArm" | "RightLowerArm" | "RightHand" => BodyAtlas::RightArm,
            "LeftUpperArm" | "LeftLowerArm" | "LeftHand" => BodyAtlas::LeftArm,
            "RightUpperLeg" | "RightLowerLeg" | "RightFoot" => BodyAtlas::RightLeg,
            "LeftUpperLeg" | "LeftLowerLeg" | "LeftFoot" => BodyAtlas::LeftLeg,
            _ => return None,
        })
    }

    pub fn wears_shirt(self) -> bool {
        matches!(self, BodyAtlas::Torso | BodyAtlas::RightArm | BodyAtlas::LeftArm)
    }

    pub fn wears_pants(self) -> bool {
        matches!(self, BodyAtlas::Torso | BodyAtlas::RightLeg | BodyAtlas::LeftLeg)
    }

    /// What this atlas goes by once these are drawn into it. The top bit keeps it clear
    /// of real asset ids.
    pub fn id(self, shirt: Option<u64>, pants: Option<u64>) -> u64 {
        let mut hasher = DefaultHasher::new();
        (self, shirt, pants).hash(&mut hasher);
        hasher.finish() | 1 << 63
    }

    fn size(self) -> [u32; 2] {
        if self == BodyAtlas::Torso { TORSO_SIZE } else { LIMB_SIZE }
    }

    fn panels(self) -> Vec<Panel> {
        match self {
            BodyAtlas::Torso => torso_panels(),
            BodyAtlas::RightArm | BodyAtlas::RightLeg => limb_panels(151.0, 217.0, 19.0, 85.0),
            BodyAtlas::LeftArm | BodyAtlas::LeftLeg => limb_panels(506.0, 308.0, 374.0, 440.0),
        }
    }

    /// Draws the atlas, with the shirt over the pants. Anything no panel covers stays
    /// clear, so the body's own color shows there.
    pub fn build(self, shirt: Option<&TextureData>, pants: Option<&TextureData>) -> TextureData {
        let top = shirt.filter(|_| self.wears_shirt());
        let under = pants.filter(|_| self.wears_pants());
        let detail = [top, under].iter().flatten().map(|t| (t.width as f32 / TEMPLATE[0]).round() as u32).max().unwrap_or(1).clamp(1, MOST_DETAIL);
        let [width, height] = self.size().map(|side| side * detail);
        let mut pixels = vec![0; (width * height * 4) as usize];
        for (spot, pixel) in template_spots(&self.panels(), self.size(), detail).into_iter().zip(pixels.chunks_exact_mut(4)) {
            if let Some(spot) = spot {
                let lower = under.map_or([0; 4], |t| sample(t, spot));
                pixel.copy_from_slice(&top.map_or(lower, |t| over(sample(t, spot), lower)));
            }
        }
        with_smaller_sizes(pixels, width, height)
    }
}

/// The spot in the template behind each pixel of the atlas, row by row from the top.
fn template_spots(panels: &[Panel], size: [u32; 2], detail: u32) -> Vec<Option<[f32; 2]>> {
    let (width, height) = (size[0] * detail, size[1] * detail);
    let scale = detail as f32;
    let mut spots = vec![None; (width * height) as usize];
    for margin in [BLEED, 0.0] {
        for panel in panels {
            let [left, bottom, right, top] = panel.atlas;
            let [template_left, template_bottom, template_right, template_top] = panel.template;
            let columns = ((left - margin).max(0.0) * scale) as u32..(((right + margin) * scale) as u32).min(width);
            let rows_up = ((bottom - margin).max(0.0) * scale) as u32..(((top + margin) * scale) as u32).min(height);
            for up in rows_up {
                let across_y = (((up as f32 + 0.5) / scale - bottom) / (top - bottom)).clamp(0.0, 1.0);
                let row = height - 1 - up;
                for column in columns.clone() {
                    let spot = &mut spots[(row * width + column) as usize];
                    // The bleed only fills what's empty. The panels themselves come after it.
                    if margin > 0.0 && spot.is_some() {
                        continue;
                    }
                    let across_x = (((column as f32 + 0.5) / scale - left) / (right - left)).clamp(0.0, 1.0);
                    *spot = Some([template_left + (template_right - template_left) * across_x, template_bottom + (template_top - template_bottom) * across_y]);
                }
            }
        }
    }
    spots
}

/// The template's pixel at a spot measured in its usual 585 by 559.
fn sample(texture: &TextureData, spot: [f32; 2]) -> [u8; 4] {
    let x = ((spot[0] / TEMPLATE[0] * texture.width as f32) as u32).min(texture.width - 1);
    let y = ((spot[1] / TEMPLATE[1] * texture.height as f32) as u32).min(texture.height - 1);
    let at = ((y * texture.width + x) * 4) as usize;
    texture.levels[0][at..at + 4].try_into().expect("four bytes make a pixel")
}

fn over(top: [u8; 4], under: [u8; 4]) -> [u8; 4] {
    let (cover, below) = (top[3] as u32, under[3] as u32 * (255 - top[3] as u32) / 255);
    let alpha = cover + below;
    if alpha == 0 {
        return [0; 4];
    }
    let mix = |channel: usize| ((top[channel] as u32 * cover + under[channel] as u32 * below) / alpha) as u8;
    [mix(0), mix(1), mix(2), alpha as u8]
}
