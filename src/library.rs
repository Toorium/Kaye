//! The files the home page lists: what's been opened lately, and the copies Kaye keeps
//! of work in progress. Like the rig logic, this has no interface code.
//!
//! Recent files have their own file and not a place in the settings. Double-clicking
//! projects makes two windows common, and each would overwrite the other's list.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use image::imageops::{self, FilterType};
use serde::{Deserialize, Serialize};

use crate::project;

const MOST_RECENT: usize = 24;
const BACKUPS_PER_PROJECT: usize = 5;
const MOST_BACKUPS: usize = 40;
/// Twice the size of a card on the home page, so it stays sharp at higher UI scales.
const THUMBNAIL: (u32, u32) = (356, 268);

/// Seconds since 1970. Enough to say how long ago something was, with no date library.
pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_secs())
}

/// How long ago something happened, in the words a person would use.
pub fn time_ago(then: u64, now: u64) -> String {
    let plural = |count: u64, unit: &str| format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" });
    let minutes = now.saturating_sub(then) / 60;
    match minutes {
        0 => "Just now".to_string(),
        1..60 => plural(minutes, "minute"),
        60..1440 => plural(minutes / 60, "hour"),
        _ => plural(minutes / 1440, "day"),
    }
}

pub fn size_text(bytes: u64) -> String {
    let megabytes = bytes as f64 / 1_048_576.0;
    if megabytes >= 1.0 { format!("{megabytes:.1} MB") } else { format!("{} KB", (bytes / 1024).max(1)) }
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b || (cfg!(windows) && a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()))
}

/// A file opened lately, which is a project or a rig.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recent {
    pub path: PathBuf,
    pub opened: u64,
}

impl Recent {
    pub fn name(&self) -> String {
        self.path.file_stem().map_or_else(String::new, |name| name.to_string_lossy().into_owned())
    }

    pub fn is_project(&self) -> bool {
        project::is_project(&self.path)
    }
}

fn read_list(list: &Path) -> Vec<Recent> {
    std::fs::read_to_string(list).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// What's in the list, newest first, without files that have since gone.
pub fn recent_files(list: &Path) -> Vec<Recent> {
    read_list(list).into_iter().filter(|recent| recent.path.exists()).collect()
}

/// Puts a file at the top of the list. It's read again first, so a second window's
/// files stay.
pub fn remember(list: &Path, path: &Path, opened: u64) {
    let mut files = read_list(list);
    files.retain(|recent| !same_file(&recent.path, path));
    files.insert(0, Recent { path: path.to_path_buf(), opened });
    files.truncate(MOST_RECENT);
    if let Some(folder) = list.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    if let Ok(text) = serde_json::to_string_pretty(&files) {
        let _ = std::fs::write(list, text);
    }
}

/// A copy of the work, made while it was being worked on.
#[derive(Clone, Debug, PartialEq)]
pub struct Backup {
    pub path: PathBuf,
    pub name: String,
    pub made: u64,
    pub size: u64,
}

/// Where a backup goes. The name is the only record of what it's a copy of and when, so
/// it holds both, and anything a file name can't hold is swapped out.
pub fn backup_path(folder: &Path, name: &str, made: u64) -> PathBuf {
    let name: String = name.chars().map(|c| if c.is_alphanumeric() || " -_".contains(c) { c } else { '_' }).collect();
    folder.join(format!("{name}.{made}.{}", project::EXTENSION))
}

fn read_backup(path: PathBuf) -> Option<Backup> {
    if !project::is_project(&path) {
        return None;
    }
    let (name, made) = path.file_stem()?.to_str()?.rsplit_once('.')?;
    let (name, made) = (name.to_string(), made.parse().ok()?);
    let size = path.metadata().ok()?.len();
    Some(Backup { path, name, made, size })
}

/// Every backup, newest first.
pub fn backups(folder: &Path) -> Vec<Backup> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found: Vec<Backup> = entries.flatten().filter_map(|entry| read_backup(entry.path())).collect();
    found.sort_by_key(|backup| std::cmp::Reverse(backup.made));
    found
}

pub fn is_backup(folder: &Path, path: &Path) -> bool {
    path.parent().is_some_and(|parent| same_file(parent, folder))
}

/// Keeps the newest few of each project, within a limit overall.
pub fn prune_backups(folder: &Path) {
    let mut kept: HashMap<String, usize> = HashMap::new();
    for (index, backup) in backups(folder).into_iter().enumerate() {
        let count = kept.entry(backup.name).or_default();
        *count += 1;
        if *count > BACKUPS_PER_PROJECT || index >= MOST_BACKUPS {
            let _ = std::fs::remove_file(backup.path);
        }
    }
}

/// Where a file's picture is kept. Going by the path keeps pictures out of project files.
pub fn thumbnail_path(folder: &Path, file: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    file.to_string_lossy().to_lowercase().hash(&mut hasher);
    folder.join(format!("{:016x}.png", hasher.finish()))
}

/// Writes the middle of a picture of the 3D view as a thumbnail. Keeping to the middle
/// leaves out the corners, where the view has its labels and axis gizmo.
pub fn write_thumbnail(view: &image::RgbaImage, target: &Path) -> Result<(), String> {
    let (width, height) = view.dimensions();
    let (wide, tall) = (THUMBNAIL.0 as f32, THUMBNAIL.1 as f32);
    let fitted = (height as f32 * wide / tall) as u32;
    let (crop_width, crop_height) = if fitted <= width { (fitted, height) } else { (width, (width as f32 * tall / wide) as u32) };
    if crop_width == 0 || crop_height == 0 {
        return Err("the view has no size".to_string());
    }
    let middle = imageops::crop_imm(view, (width - crop_width) / 2, (height - crop_height) / 2, crop_width, crop_height).to_image();
    let small = imageops::resize(&middle, THUMBNAIL.0, THUMBNAIL.1, FilterType::Triangle);
    if let Some(folder) = target.parent() {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    }
    image::DynamicImage::ImageRgba8(small).to_rgb8().save(target).map_err(|e| e.to_string())
}
