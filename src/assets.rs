//! Downloads the meshes and textures a model points at, and keeps them on disk.
//! Like the rig logic, this has no interface code.
//!
//! A model file only stores asset ids. Roblox hands out the real files through Open
//! Cloud, which needs an API key with the `legacy-asset:manage` scope, and answers with a
//! temporary link to fetch. Roblox's own material pictures and the Baseplate's grid are
//! public, so those come from its public link with no key. Nothing here ever gets in the
//! way of the work: a part whose asset can't be had simply stays a grey box, and it's all
//! done on other threads.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use crate::clothing::BodyAtlas;
use crate::rbxmesh::{self, RobloxMesh};

const DOWNLOAD_THREADS: usize = 4;
/// Larger than any asset Roblox allows, as a guard against a bad answer.
const MOST_BYTES: u64 = 256 * 1024 * 1024;
const LARGEST_TEXTURE: u32 = 2048;

/// A texture ready for the graphics card, largest size first.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureData {
    pub width: u32,
    pub height: u32,
    /// RGBA, with each level half the size of the one before.
    pub levels: Vec<Vec<u8>>,
}

/// Halves a picture, averaging each block of four pixels. With `weighted`, clear pixels
/// don't count toward the color, or clothing would go dark around its edges at a distance.
/// A material picture's alpha says where the part's color tints it instead, so there
/// every pixel counts the same.
fn halved(pixels: &[u8], width: u32, height: u32, weighted: bool) -> (Vec<u8>, u32, u32) {
    let (new_width, new_height) = ((width / 2).max(1), (height / 2).max(1));
    let mut out = Vec::with_capacity((new_width * new_height * 4) as usize);
    for y in 0..new_height {
        for x in 0..new_width {
            let (mut color, mut weight, mut alpha) = ([0u32; 3], 0u32, 0u32);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (px, py) = ((x * 2 + dx).min(width - 1), (y * 2 + dy).min(height - 1));
                let at = ((py * width + px) * 4) as usize;
                let cover = if weighted { pixels[at + 3] as u32 } else { 255 };
                for channel in 0..3 {
                    color[channel] += pixels[at + channel] as u32 * cover;
                }
                weight += cover;
                alpha += pixels[at + 3] as u32;
            }
            let mixed = color.map(|c| c.checked_div(weight).unwrap_or(0) as u8);
            out.extend([mixed[0], mixed[1], mixed[2], (alpha / 4) as u8]);
        }
    }
    (out, new_width, new_height)
}

pub fn decode_texture(bytes: &[u8]) -> Result<TextureData, String> {
    let image = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let image = if image.width() > LARGEST_TEXTURE || image.height() > LARGEST_TEXTURE { image.thumbnail(LARGEST_TEXTURE, LARGEST_TEXTURE) } else { image };
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(with_smaller_sizes(rgba.into_raw(), width, height))
}

/// One texture laid over another, which is what a torso wears: a shirt over pants, and both
/// over a body package's own texture. It's as big as the bigger of the two, so a body's
/// detail isn't lost under a smaller clothing atlas, which is stretched smoothly to fit.
pub fn layered_texture(top: &TextureData, under: &TextureData) -> TextureData {
    let (width, height) = if under.width * under.height > top.width * top.height { (under.width, under.height) } else { (top.width, top.height) };
    let mut pixels = vec![0; (width * height * 4) as usize];
    for (i, pixel) in pixels.chunks_exact_mut(4).enumerate() {
        let (x, y) = (i as u32 % width, i as u32 / width);
        let spot = [(x as f32 + 0.5) / width as f32, (y as f32 + 0.5) / height as f32];
        let (above, below) = (sample_smooth(top, spot), sample_smooth(under, spot));
        let cover = above[3] as u32;
        for channel in 0..3 {
            pixel[channel] = ((above[channel] as u32 * cover + below[channel] as u32 * (255 - cover)) / 255) as u8;
        }
        pixel[3] = above[3].max(below[3]);
    }
    with_smaller_sizes(pixels, width, height)
}

/// A texture's color at a spot from 0 to 1 across and down, blended from the four pixels
/// around it. At its own size that's exactly the pixel there.
fn sample_smooth(texture: &TextureData, spot: [f32; 2]) -> [u8; 4] {
    let (w, h) = (texture.width, texture.height);
    let fx = (spot[0] * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
    let fy = (spot[1] * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (fx as u32, fy as u32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let pixel = |x: u32, y: u32| {
        let at = ((y * w + x) * 4) as usize;
        &texture.levels[0][at..at + 4]
    };
    let (a, b, c, d) = (pixel(x0, y0), pixel(x1, y0), pixel(x0, y1), pixel(x1, y1));
    std::array::from_fn(|i| {
        let top = a[i] as f32 * (1.0 - tx) + b[i] as f32 * tx;
        let bottom = c[i] as f32 * (1.0 - tx) + d[i] as f32 * tx;
        (top * (1.0 - ty) + bottom * ty).round() as u8
    })
}

/// What a body texture with clothes over it goes by. The top bit keeps it clear of real
/// asset ids, as with the other textures Kaye makes.
fn worn_over(own: u64, dressed: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    ("worn over", own, dressed).hash(&mut hasher);
    hasher.finish() | 1 << 63
}

pub(crate) fn with_smaller_sizes(pixels: Vec<u8>, width: u32, height: u32) -> TextureData {
    smaller_sizes(pixels, width, height, true)
}

/// Exactly `size` square, for a slot in the stack of material pictures, which all have
/// to match.
pub fn decode_material_map(bytes: &[u8], size: u32) -> Result<TextureData, String> {
    let image = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let rgba = image.resize_exact(size, size, image::imageops::FilterType::Triangle).to_rgba8();
    Ok(smaller_sizes(rgba.into_raw(), size, size, false))
}

fn smaller_sizes(pixels: Vec<u8>, width: u32, height: u32, weighted: bool) -> TextureData {
    let mut levels = vec![pixels];
    let (mut w, mut h) = (width, height);
    while w > 1 || h > 1 {
        let (next, next_w, next_h) = halved(levels.last().expect("starts with one level"), w, h, weighted);
        levels.push(next);
        (w, h) = (next_w, next_h);
    }
    TextureData { width, height, levels }
}

/// Where an asset has got to.
#[derive(Clone, Debug)]
pub enum Slot<T> {
    Waiting,
    Ready(Arc<T>),
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Mesh,
    Texture,
    /// A picture that ships with Roblox, read from wherever it's installed.
    ShippedTexture,
    /// One of Roblox's published material pictures, shrunk to this size.
    Map(u32),
}

enum Loaded {
    Mesh(Arc<RobloxMesh>),
    Texture(Arc<TextureData>),
    Missing,
    /// Roblox turned the key down, which is the one failure someone can fix.
    KeyRefused,
}

/// The text is the API key, or the path of a file that ships with Roblox.
type Job = (Kind, u64, String);
type Finished = (Kind, u64, Loaded);

pub struct AssetStore {
    folder: Option<PathBuf>,
    key: String,
    meshes: HashMap<u64, Slot<RobloxMesh>>,
    textures: HashMap<u64, Slot<TextureData>>,
    jobs: Option<Sender<Job>>,
    finished: Receiver<Finished>,
    finished_tx: Sender<Finished>,
    /// Called from other threads when something arrives, so a window at rest repaints.
    wake: Arc<dyn Fn() + Send + Sync>,
    /// Set once if Roblox refuses the key, for the app to say so.
    pub key_refused: bool,
}

impl AssetStore {
    pub fn new(folder: Option<PathBuf>, key: String, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (finished_tx, finished) = channel();
        Self { folder, key, meshes: HashMap::new(), textures: HashMap::new(), jobs: None, finished, finished_tx, wake, key_refused: false }
    }

    /// Hands the store a picture Kaye drew itself, ready under `id` from the start.
    pub fn provide(&mut self, id: u64, texture: TextureData) {
        self.textures.insert(id, Slot::Ready(Arc::new(texture)));
    }

    /// A new key might open up everything that was refused before.
    pub fn set_key(&mut self, key: &str) {
        if self.key != key {
            self.key = key.to_string();
            self.key_refused = false;
            self.meshes.retain(|_, slot| !matches!(slot, Slot::Missing));
            self.textures.retain(|_, slot| !matches!(slot, Slot::Missing));
        }
    }

    fn start(&mut self, kind: Kind, id: u64, text: String) {
        let (folder, finished, wake) = (self.folder.clone(), self.finished_tx.clone(), self.wake.clone());
        let jobs = self.jobs.get_or_insert_with(|| {
            let (tx, rx) = channel::<Job>();
            let rx = Arc::new(Mutex::new(rx));
            for _ in 0..DOWNLOAD_THREADS {
                let (rx, folder, finished, wake) = (rx.clone(), folder.clone(), finished.clone(), wake.clone());
                std::thread::spawn(move || {
                    loop {
                        // Held only for the wait, so the others can pick up work.
                        let job = rx.lock().ok().and_then(|rx| rx.recv().ok());
                        let Some((kind, id, text)) = job else {
                            return;
                        };
                        let loaded = load(kind, id, &text, folder.as_deref());
                        let _ = finished.send((kind, id, loaded));
                        wake();
                    }
                });
            }
            tx
        });
        let _ = jobs.send((kind, id, text));
    }

    /// Asks for a mesh, and answers with what's known about it so far.
    pub fn mesh(&mut self, id: u64) -> Slot<RobloxMesh> {
        if !self.meshes.contains_key(&id) {
            self.meshes.insert(id, Slot::Waiting);
            self.start(Kind::Mesh, id, self.key.clone());
        }
        self.meshes[&id].clone()
    }

    pub fn texture(&mut self, id: u64) -> Slot<TextureData> {
        if !self.textures.contains_key(&id) {
            self.textures.insert(id, Slot::Waiting);
            self.start(Kind::Texture, id, self.key.clone());
        }
        self.textures[&id].clone()
    }

    /// Asks for a material's color or normal map, shrunk to `size` square.
    pub fn material_map(&mut self, id: u64, size: u32) -> Slot<TextureData> {
        if !self.textures.contains_key(&id) {
            self.textures.insert(id, Slot::Waiting);
            self.start(Kind::Map(size), id, self.key.clone());
        }
        self.textures[&id].clone()
    }

    /// Asks for a texture by what a model calls it: an asset, or a file that ships with Roblox.
    pub fn texture_named(&mut self, text: &str) {
        if let Some(id) = rbxmesh::asset_id(text) {
            self.texture(id);
        } else if let Some(path) = rbxmesh::shipped_file(text) {
            let id = shipped_id(path);
            if !self.textures.contains_key(&id) {
                self.textures.insert(id, Slot::Waiting);
                self.start(Kind::ShippedTexture, id, path.to_string());
            }
        }
    }

    /// Draws an R15 body atlas from whichever of the clothes it wears have arrived, once
    /// for each set of them.
    pub fn dress(&mut self, atlas: BodyAtlas, shirt: Option<u64>, pants: Option<u64>) {
        let (shirt, pants) = (self.arrived(shirt.filter(|_| atlas.wears_shirt())), self.arrived(pants.filter(|_| atlas.wears_pants())));
        if shirt.is_none() && pants.is_none() {
            return;
        }
        let id = atlas.id(shirt, pants);
        if !self.textures.contains_key(&id) {
            let drawn = atlas.build(shirt.and_then(|s| self.loaded_texture(s)).map(Arc::as_ref), pants.and_then(|p| self.loaded_texture(p)).map(Arc::as_ref));
            self.textures.insert(id, Slot::Ready(Arc::new(drawn)));
        }
    }

    /// A body part's own texture with the clothes drawn over it, as Roblox shows a body
    /// package wearing a shirt. Made once the texture and the atlas `dress` draws are ready.
    pub fn dress_over(&mut self, own: u64, atlas: BodyAtlas, shirt: Option<u64>, pants: Option<u64>) {
        let Some(dressed) = self.dressed(atlas, shirt, pants) else { return };
        let id = worn_over(own, dressed);
        if self.textures.contains_key(&id) {
            return;
        }
        if let (Some(top), Some(under)) = (self.loaded_texture(dressed), self.loaded_texture(own)) {
            let both = layered_texture(top, under);
            self.textures.insert(id, Slot::Ready(Arc::new(both)));
        }
    }

    /// The id of what `dress_over` has made for this texture and these clothes, if it has.
    pub fn dressed_over(&self, own: u64, atlas: BodyAtlas, shirt: Option<u64>, pants: Option<u64>) -> Option<u64> {
        let dressed = self.dressed(atlas, shirt, pants)?;
        Some(worn_over(own, dressed)).filter(|id| self.textures.contains_key(id))
    }

    /// The id of the atlas `dress` has drawn from these clothes so far.
    pub fn dressed(&self, atlas: BodyAtlas, shirt: Option<u64>, pants: Option<u64>) -> Option<u64> {
        let (shirt, pants) = (self.arrived(shirt.filter(|_| atlas.wears_shirt())), self.arrived(pants.filter(|_| atlas.wears_pants())));
        Some(atlas.id(shirt, pants)).filter(|id| self.textures.contains_key(id))
    }

    fn arrived(&self, id: Option<u64>) -> Option<u64> {
        id.filter(|id| self.loaded_texture(*id).is_some())
    }

    /// The id a shirt and pants go by once they're laid together. Made the first time
    /// both are ready.
    pub fn layered(&mut self, shirt: u64, pants: u64) -> Option<u64> {
        let id = shirt ^ pants.rotate_left(17) | 1 << 63;
        if !self.textures.contains_key(&id) {
            let (top, under) = (self.loaded_texture(shirt)?, self.loaded_texture(pants)?);
            let both = layered_texture(top, under);
            self.textures.insert(id, Slot::Ready(Arc::new(both)));
        }
        Some(id)
    }

    /// What's known about a mesh, without asking for it.
    pub fn loaded_mesh(&self, id: u64) -> Option<&Arc<RobloxMesh>> {
        match self.meshes.get(&id) {
            Some(Slot::Ready(mesh)) => Some(mesh),
            _ => None,
        }
    }

    pub fn loaded_texture(&self, id: u64) -> Option<&Arc<TextureData>> {
        match self.textures.get(&id) {
            Some(Slot::Ready(texture)) => Some(texture),
            _ => None,
        }
    }

    pub fn is_missing_mesh(&self, id: u64) -> bool {
        matches!(self.meshes.get(&id), Some(Slot::Missing))
    }

    /// Takes in whatever has finished. Returns true when anything new is ready.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok((kind, id, loaded)) = self.finished.try_recv() {
            changed = true;
            match (kind, loaded) {
                (_, Loaded::Mesh(mesh)) => {
                    self.meshes.insert(id, Slot::Ready(mesh));
                }
                (_, Loaded::Texture(texture)) => {
                    self.textures.insert(id, Slot::Ready(texture));
                }
                (Kind::Mesh, other) => {
                    self.key_refused |= matches!(other, Loaded::KeyRefused);
                    self.meshes.insert(id, Slot::Missing);
                }
                (Kind::Texture | Kind::ShippedTexture | Kind::Map(_), other) => {
                    self.key_refused |= matches!(other, Loaded::KeyRefused);
                    self.textures.insert(id, Slot::Missing);
                }
            }
        }
        changed
    }
}

/// What a file that ships with Roblox goes by. A bit keeps it clear of real asset ids.
pub fn shipped_id(path: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish() | 1 << 62
}

/// Every `content` folder of a Roblox install on this machine, newest first.
fn roblox_content_folders() -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        bases.push(PathBuf::from(local));
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        bases.push(PathBuf::from(home).join("AppData").join("Local"));
    }
    for name in ["ProgramFiles(x86)", "ProgramFiles"] {
        bases.extend(std::env::var_os(name).map(PathBuf::from));
    }
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    // Many players start Roblox through Bloxstrap or one of its forks, which keep their
    // own copy of the install and leave Roblox's folder empty.
    let launchers = ["Roblox", "Bloxstrap", "Fishstrap", "Voidstrap"];
    for folder in bases.iter().flat_map(|base| launchers.map(|name| base.join(name).join("Versions"))) {
        let Ok(versions) = std::fs::read_dir(folder) else {
            continue;
        };
        for version in versions.flatten() {
            let content = version.path().join("content");
            if let Ok(when) = std::fs::metadata(&content).and_then(|m| m.modified()) {
                found.push((when, content));
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, folder)| folder).collect()
}

/// A picture that ships with Roblox isn't ever downloaded, so with no Roblox here it's just
/// missing.
fn load_shipped(path: &str) -> Loaded {
    let bytes = roblox_content_folders().into_iter().find_map(|folder| std::fs::read(folder.join(path)).ok());
    bytes.and_then(|bytes| decode_texture(&bytes).ok()).map_or(Loaded::Missing, |texture| Loaded::Texture(Arc::new(texture)))
}

fn load(kind: Kind, id: u64, text: &str, folder: Option<&Path>) -> Loaded {
    if kind == Kind::ShippedTexture {
        return load_shipped(text);
    }
    let key = text;
    let path = folder.map(|f| f.join(id.to_string()));
    let cached = path.as_deref().and_then(|p| std::fs::read(p).ok());
    let bytes = match cached {
        Some(bytes) => bytes,
        None => match download(id, key) {
            Ok(bytes) => {
                if let Some(path) = &path {
                    keep(path, &bytes);
                }
                bytes
            }
            Err(failure) => return failure,
        },
    };
    match kind {
        Kind::Mesh => rbxmesh::parse(&bytes).map_or(Loaded::Missing, |mesh| Loaded::Mesh(Arc::new(mesh))),
        Kind::Texture | Kind::ShippedTexture => decode_texture(&bytes).map_or(Loaded::Missing, |texture| Loaded::Texture(Arc::new(texture))),
        Kind::Map(size) => decode_material_map(&bytes, size).map_or(Loaded::Missing, |texture| Loaded::Texture(Arc::new(texture))),
    }
}

/// Written to one side first, so a crash can't leave half a file looking like an asset.
fn keep(path: &Path, bytes: &[u8]) {
    let Some(folder) = path.parent() else {
        return;
    };
    let temporary = path.with_extension("tmp");
    let written = std::fs::create_dir_all(folder).and_then(|()| std::fs::write(&temporary, bytes)).and_then(|()| std::fs::rename(&temporary, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
}

/// Roblox's public delivery link, which hands out the assets anyone may use, with no key.
fn download_public(id: u64) -> Option<Vec<u8>> {
    let url = format!("https://assetdelivery.roblox.com/v1/asset/?id={id}");
    let mut answer = ureq::get(&url).call().ok()?;
    answer.body_mut().with_config().limit(MOST_BYTES).read_to_vec().ok()
}

fn download(id: u64, key: &str) -> Result<Vec<u8>, Loaded> {
    // Roblox's own material pictures come down for anyone, so they don't need a key.
    if crate::materials::is_public(id) {
        if let Some(bytes) = download_public(id) {
            return Ok(bytes);
        }
    }
    if key.is_empty() {
        return Err(Loaded::Missing);
    }
    let url = format!("https://apis.roblox.com/asset-delivery-api/v1/assetId/{id}");
    let failed = |error: ureq::Error| match error {
        ureq::Error::StatusCode(401 | 403) => Loaded::KeyRefused,
        _ => Loaded::Missing,
    };
    let mut answer = ureq::get(&url).header("x-api-key", key).call().map_err(failed)?;
    let text = answer.body_mut().read_to_string().map_err(failed)?;
    let location = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v["location"].as_str().map(str::to_string)).ok_or(Loaded::Missing)?;
    let mut file = ureq::get(&location).call().map_err(failed)?;
    file.body_mut().with_config().limit(MOST_BYTES).read_to_vec().map_err(failed)
}
