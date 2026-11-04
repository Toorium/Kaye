//! Roblox's part materials, and where their pictures come from. No interface code.
//!
//! Roblox publishes the color and normal map of every 2022 material as ordinary assets,
//! listed on its Creator Hub materials page, for creators to build their own materials
//! from. Kaye downloads them the same way it downloads clothing, so nothing of Roblox's
//! ships with Kaye. They're public, so they come down without an API key too.
//!
//! Plastic, Smooth Plastic, Neon and Force Field have no published pictures. Roblox draws
//! those itself, so Kaye does too.

use serde::{Deserialize, Serialize};

use crate::assets::{TextureData, with_smaller_sizes};

/// How big each material's color picture is kept. Roblox's are 1024, which costs four
/// times the graphics memory for detail that only shows up close.
pub const COLOR_SIZE: u32 = 512;
/// Normal maps only shape the light, so they can be smaller.
pub const NORMAL_SIZE: u32 = 256;
/// Every 2022 material repeats every 8 studs, measured in Studio against its picture.
pub const TILE_STUDS: f32 = 8.0;

macro_rules! materials {
    ($($name:ident = $value:literal, $label:literal, $maps:expr;)*) => {
        /// A Roblox `Enum.Material`, kept by name in projects.
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub enum Material {
            #[default]
            $($name,)*
        }

        impl Material {
            /// Every material a part can have, in Roblox's order.
            pub const ALL: &[Material] = &[$(Material::$name,)*];

            /// The value Roblox stores in a file.
            pub fn roblox_value(self) -> u32 {
                match self {
                    $(Material::$name => $value,)*
                }
            }

            /// What Roblox Studio calls it.
            pub fn label(self) -> &'static str {
                match self {
                    $(Material::$name => $label,)*
                }
            }

            /// The asset ids of its color map and normal map, from Roblox's materials page.
            pub fn maps(self) -> Option<(u64, u64)> {
                match self {
                    $(Material::$name => $maps,)*
                }
            }

            /// Air and Water belong to terrain, so a part with them shows as Plastic, which is
            /// also what an unknown newer value falls back to.
            pub fn from_roblox(value: u32) -> Material {
                match value {
                    $($value => Material::$name,)*
                    _ => Material::Plastic,
                }
            }
        }
    };
}

// https://create.roblox.com/docs/parts/materials, "Current Base": ColorMap, then Normal.
materials! {
    Plastic = 256, "Plastic", None;
    SmoothPlastic = 272, "Smooth Plastic", None;
    Neon = 288, "Neon", None;
    Wood = 512, "Wood", Some((9920625290, 9439641376));
    WoodPlanks = 528, "Wood Planks", Some((9920626778, 9439650689));
    Marble = 784, "Marble", Some((9439430596, 9439431240));
    Basalt = 788, "Basalt", Some((9920482056, 9438412214));
    Slate = 800, "Slate", Some((9920599782, 9439612514));
    CrackedLava = 804, "Cracked Lava", Some((9920484943, 9438508790));
    Concrete = 816, "Concrete", Some((9920484153, 9466554006));
    Limestone = 820, "Limestone", Some((9920561437, 9439415191));
    Granite = 832, "Granite", Some((9920550238, 9438882935));
    Pavement = 836, "Pavement", Some((9920579943, 9439519281));
    Brick = 848, "Brick", Some((9920482813, 9438453152));
    Pebble = 864, "Pebble", Some((9920581082, 9439528644));
    Cobblestone = 880, "Cobblestone", Some((9919718991, 9438457162));
    Rock = 896, "Rock", Some((9920587470, 9439538417));
    Sandstone = 912, "Sandstone", Some((9920596120, 9439596530));
    CorrodedMetal = 1040, "Corroded Metal", Some((9920589327, 9439548484));
    DiamondPlate = 1056, "Diamond Plate", Some((10237720195, 9438583222));
    Foil = 1072, "Foil", Some((9466552117, 9424786192));
    Metal = 1088, "Metal", Some((9920574687, 9873295432));
    Grass = 1280, "Grass", Some((9920551868, 9438955773));
    LeafyGrass = 1284, "Leafy Grass", Some((9920557906, 9439080781));
    Sand = 1296, "Sand", Some((9920591683, 9439577084));
    Fabric = 1312, "Fabric", Some((9920517696, 9873280412));
    Snow = 1328, "Snow", Some((9920620284, 9439632006));
    Mud = 1344, "Mud", Some((9920578473, 9439509827));
    Ground = 1360, "Ground", Some((9920554482, 9439043558));
    Asphalt = 1376, "Asphalt", Some((9930003046, 9429449876));
    Salt = 1392, "Salt", Some((9920590225, 9439565809));
    Ice = 1536, "Ice", Some((9920555943, 9467301039));
    Glacier = 1552, "Glacier", Some((9920518732, 9438812958));
    Glass = 1568, "Glass", Some((9438868521, 7547304785));
    ForceField = 1584, "Force Field", None;
    Cardboard = 2304, "Cardboard", Some((14108651729, 14108654002));
    Carpet = 2305, "Carpet", Some((14108662587, 14108663154));
    CeramicTiles = 2306, "Ceramic Tiles", Some((17429425079, 17429425915));
    ClayRoofTiles = 2307, "Clay Roof Tiles", Some((18147681935, 18147683410));
    RoofShingles = 2308, "Roof Shingles", Some((119722544879522, 77534750680073));
    Leather = 2309, "Leather", Some((14108670073, 14108670486));
    Plaster = 2310, "Plaster", Some((14108671255, 14108671870));
    Rubber = 2311, "Rubber", Some((14108673018, 14108674698));
}

/// How a material changes lighting, besides its pictures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Matte,
    /// Lit from within, like Roblox's Neon.
    Glow,
    /// A bright highlight from the sun, for metals, glass and ice.
    Shiny,
}

impl Material {
    /// Which slot in the stack of material pictures it uses, if it has pictures.
    pub fn layer(self) -> Option<u32> {
        textured().iter().position(|m| *m == self).map(|i| i as u32)
    }

    pub fn look(self) -> Look {
        use Material::*;
        match self {
            Neon => Look::Glow,
            Metal | DiamondPlate | Foil | Glass | Ice | Glacier | CeramicTiles => Look::Shiny,
            _ => Look::Matte,
        }
    }

    /// Force Field is drawn see-through in Roblox, whatever the part's Transparency says.
    pub fn least_transparency(self) -> f32 {
        if self == Material::ForceField { 0.55 } else { 0.0 }
    }
}

/// The materials with pictures, in the order they're stacked.
pub fn textured() -> &'static [Material] {
    static TEXTURED: std::sync::OnceLock<Vec<Material>> = std::sync::OnceLock::new();
    TEXTURED.get_or_init(|| Material::ALL.iter().copied().filter(|m| m.maps().is_some()).collect())
}

/// Whether an asset is one Roblox hands to anyone, so it can be fetched with no API key:
/// the material pictures and the Baseplate's grid.
pub fn is_public(id: u64) -> bool {
    id == BASEPLATE_ASSET || Material::ALL.iter().any(|m| m.maps().is_some_and(|(color, normal)| id == color || id == normal))
}

// ---- The Baseplate's grid ---------------------------------------------------------------

/// The asset id of the grid on Studio's Baseplate.
pub const BASEPLATE_TEXTURE: &str = "rbxassetid://6372755229";
const BASEPLATE_ASSET: u64 = 6372755229;

/// What Kaye's own grid is kept under in the asset store. Bit 61 keeps it clear of real
/// asset ids and of the store's own made-up ones.
pub const GRID_ID: u64 = 1 << 61 | 1;

/// Kaye's own picture to show while one of Roblox's can't be had.
pub fn stand_in(text: &str) -> Option<u64> {
    (crate::rbxmesh::asset_id(text) == Some(BASEPLATE_ASSET)).then_some(GRID_ID)
}

/// Kaye's own grid for a baseplate, for when the real one can't be downloaded. Roblox
/// draws it in black at 80% transparency, so only how solid each pixel is matters: a
/// light fill with a firmer line round each 8 stud square.
pub fn baseplate_grid() -> TextureData {
    let size = 128usize;
    let mut pixels = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let (u, v) = ((x as f32 + 0.5) / size as f32, (y as f32 + 0.5) / size as f32);
            let edge = u.min(1.0 - u).min(v.min(1.0 - v));
            let line = 1.0 - ((edge - 0.012) / 0.012).clamp(0.0, 1.0);
            let bevel = 1.0 - ((edge - 0.03) / 0.04).clamp(0.0, 1.0);
            let alpha = 0.28 + 0.32 * bevel + 0.4 * line;
            pixels.extend([255, 255, 255, (alpha * 255.0).round() as u8]);
        }
    }
    with_smaller_sizes(pixels, size as u32, size as u32)
}
