//! Kaye: a Blender-style animation tool built only for Roblox rigs.

pub mod animation;
pub mod app;
pub mod assets;
#[cfg(windows)]
pub mod association;
pub mod camera;
pub mod clothing;
pub mod dragdrop;
pub mod dynamics;
#[cfg(feature = "devtools")]
pub mod devscript;
pub mod export;
pub mod gizmo;
pub mod history;
pub mod import;
pub mod library;
pub mod materials;
pub mod math;
pub mod media;
pub mod modifiers;
pub mod onion;
pub mod plugins;
pub mod posing;
pub mod project;
pub mod reference;
pub mod rigging;
pub mod rbxmesh;
pub mod render;
pub mod scene;
pub mod secrets;
pub mod sequences;
pub mod settings;
pub mod update;
