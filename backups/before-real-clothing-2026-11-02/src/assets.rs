//! Downloads the meshes and textures a model points at, and keeps them on disk.
//! Like the rig logic, this has no interface code.
//!
//! A model file only stores asset ids. Roblox hands out the real files through Open
//! Cloud, which needs an API key with the `legacy-asset:manage` scope, and answers with a
//! temporary link to fetch. Nothing here ever gets in the way of the work: a part whose
//! asset can't be had simply stays a grey box, and it's all done on other threads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

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

/// Halves a picture, averaging each block of four pixels.
fn halved(pixels: &[u8], width: u32, height: u32) -> (Vec<u8>, u32, u32) {
    let (new_width, new_height) = ((width / 2).max(1), (height / 2).max(1));
    let mut out = Vec::with_capacity((new_width * new_height * 4) as usize);
    for y in 0..new_height {
        for x in 0..new_width {
            let mut sum = [0u32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (px, py) = ((x * 2 + dx).min(width - 1), (y * 2 + dy).min(height - 1));
                let at = ((py * width + px) * 4) as usize;
                for channel in 0..4 {
                    sum[channel] += pixels[at + channel] as u32;
                }
            }
            out.extend(sum.map(|s| (s / 4) as u8));
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

/// One texture laid over another, which is what a torso wears: a shirt over pants.
pub fn layered_texture(top: &TextureData, under: &TextureData) -> TextureData {
    let (width, height) = (top.width, top.height);
    let mut pixels = top.levels[0].clone();
    for (i, pixel) in pixels.chunks_exact_mut(4).enumerate() {
        let (x, y) = (i as u32 % width, i as u32 / width);
        let at = ((y * under.height / height * under.width + x * under.width / width) * 4) as usize;
        let below = &under.levels[0][at..at + 4];
        let cover = pixel[3] as u32;
        for channel in 0..3 {
            pixel[channel] = ((pixel[channel] as u32 * cover + below[channel] as u32 * (255 - cover)) / 255) as u8;
        }
        pixel[3] = pixel[3].max(below[3]);
    }
    with_smaller_sizes(pixels, width, height)
}

fn with_smaller_sizes(pixels: Vec<u8>, width: u32, height: u32) -> TextureData {
    let mut levels = vec![pixels];
    let (mut w, mut h) = (width, height);
    while w > 1 || h > 1 {
        let (next, next_w, next_h) = halved(levels.last().expect("starts with one level"), w, h);
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
}

enum Loaded {
    Mesh(Arc<RobloxMesh>),
    Texture(Arc<TextureData>),
    Missing,
    /// Roblox turned the key down, which is the one failure someone can fix.
    KeyRefused,
}

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

    /// A new key might open up everything that was refused before.
    pub fn set_key(&mut self, key: &str) {
        if self.key != key {
            self.key = key.to_string();
            self.key_refused = false;
            self.meshes.retain(|_, slot| !matches!(slot, Slot::Missing));
            self.textures.retain(|_, slot| !matches!(slot, Slot::Missing));
        }
    }

    fn start(&mut self, kind: Kind, id: u64) {
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
                        let Some((kind, id, key)) = job else {
                            return;
                        };
                        let loaded = load(kind, id, &key, folder.as_deref());
                        let _ = finished.send((kind, id, loaded));
                        wake();
                    }
                });
            }
            tx
        });
        let _ = jobs.send((kind, id, self.key.clone()));
    }

    /// Asks for a mesh, and answers with what's known about it so far.
    pub fn mesh(&mut self, id: u64) -> Slot<RobloxMesh> {
        if !self.meshes.contains_key(&id) {
            self.meshes.insert(id, Slot::Waiting);
            self.start(Kind::Mesh, id);
        }
        self.meshes[&id].clone()
    }

    pub fn texture(&mut self, id: u64) -> Slot<TextureData> {
        if !self.textures.contains_key(&id) {
            self.textures.insert(id, Slot::Waiting);
            self.start(Kind::Texture, id);
        }
        self.textures[&id].clone()
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
                (Kind::Texture, other) => {
                    self.key_refused |= matches!(other, Loaded::KeyRefused);
                    self.textures.insert(id, Slot::Missing);
                }
            }
        }
        changed
    }
}

fn load(kind: Kind, id: u64, key: &str, folder: Option<&Path>) -> Loaded {
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
        Kind::Texture => decode_texture(&bytes).map_or(Loaded::Missing, |texture| Loaded::Texture(Arc::new(texture))),
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

fn download(id: u64, key: &str) -> Result<Vec<u8>, Loaded> {
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
