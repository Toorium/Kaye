//! Finding a newer Kaye on GitHub, and putting it in place.
//!
//! The repository's `latest.json` names the newest version and every file it's made of, with
//! each one's size, SHA-256 and download link. Kaye reads it on launch. Updating runs this same
//! exe again with `--update`, in a small window of its own, while the main window closes:
//!
//! 1. Files whose hash differs from the release, or that are missing, download into
//!    `.kaye-update\new` beside the exe, and every one is checked against its hash.
//! 2. Once Kaye has closed, each is swapped in, and the file it replaces moves to
//!    `.kaye-update\old-<id>`. Files the last update installed and this one doesn't have move
//!    there too. Any failure moves everything back, so the old version still works.
//! 3. `kaye-files.json` records what's installed, which is how step 2 knows what was removed.
//!    The `.msi` doesn't write it, so the first update removes nothing.
//! 4. The new Kaye starts with `--updated`, and deletes `.kaye-update`.
//!
//! Windows lets a running exe be renamed, which is how the updater replaces the file it's
//! running from. Settings, the API key and backups are in `%APPDATA%\Kaye`, and nothing here
//! touches them.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The file on GitHub that says which version is newest.
pub const LATEST_URL: &str = "https://raw.githubusercontent.com/Toorium/Kaye/main/latest.json";
/// This build's version, from `Cargo.toml`.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// Beside the exe, holding downloads and the files they replaced.
const WORK_FOLDER: &str = ".kaye-update";
/// Beside the exe, listing what the last update installed.
const INSTALLED_LIST: &str = "kaye-files.json";
/// `latest.json` is a few hundred bytes, so anything near this isn't one.
const MOST_LIST_BYTES: u64 = 1 << 20;

/// What `latest.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    /// What's new, shown in the popup. Can be empty.
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub files: Vec<ReleaseFile>,
}

/// One file of a release, relative to the folder `kaye.exe` is in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReleaseFile {
    /// With `/` between folders, such as `kaye.exe`.
    pub path: String,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
    pub url: String,
}

/// What `kaye-files.json` holds.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Installed {
    version: String,
    files: Vec<String>,
}

/// What an update has to do to one folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub version: String,
    /// New files and changed ones, to download.
    pub fetch: Vec<ReleaseFile>,
    /// Files the last update installed that this one doesn't have.
    pub remove: Vec<PathBuf>,
    /// Every file of the release, for `kaye-files.json`.
    pub files: Vec<String>,
}

impl Plan {
    pub fn download_size(&self) -> u64 {
        self.fetch.iter().map(|f| f.size).sum()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    Cancelled,
    /// Said to the user as it is.
    Failed(String),
}

impl From<String> for Failure {
    fn from(text: String) -> Self {
        Self::Failed(text)
    }
}

/// How far a download has got, shared with the window showing it.
#[derive(Debug, Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    /// Set by the window to stop the download.
    pub cancel: AtomicBool,
}

/// Where `latest.json` is read from. A test build can be pointed at a local server.
pub fn latest_url() -> String {
    #[cfg(feature = "devtools")]
    if let Ok(url) = std::env::var("KAYE_UPDATE_URL") {
        return url;
    }
    LATEST_URL.to_string()
}

/// Real builds check on every launch. Debug and test builds don't, since an update would
/// replace the build being worked on, unless a test sets `KAYE_UPDATE_URL`.
pub fn checks_on_launch() -> bool {
    if cfg!(feature = "devtools") {
        return std::env::var_os("KAYE_UPDATE_URL").is_some();
    }
    !cfg!(debug_assertions)
}

/// Whether `latest` is a later version than `current`. Anything unreadable counts as not
/// newer, so a broken `latest.json` never asks anyone to update.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let read = |text: &str| semver::Version::parse(text.trim().trim_start_matches('v'));
    match (read(latest), read(current)) {
        (Ok(latest), Ok(current)) => latest > current,
        _ => false,
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        // Per read, so a big file on a slow line is fine as long as it keeps coming.
        .timeout_per_read(Some(Duration::from_secs(30)))
        .user_agent(concat!("Kaye/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Reads `latest.json`, and checks every file in it could be installed.
pub fn fetch_latest(url: &str) -> Result<Release, String> {
    let mut answer = agent().get(url).call().map_err(|e| format!("GitHub couldn't be reached: {e}"))?;
    let text = answer
        .body_mut()
        .with_config()
        .limit(MOST_LIST_BYTES)
        .read_to_string()
        .map_err(|e| format!("The update list didn't download: {e}"))?;
    let release: Release = serde_json::from_str(&text).map_err(|e| format!("The update list couldn't be read: {e}"))?;
    check_release(&release)?;
    Ok(release)
}

/// The release to offer, if there's a newer one. Run off the main thread, since it waits on
/// the network.
pub fn check() -> Option<Release> {
    let release = fetch_latest(&latest_url()).ok()?;
    (is_newer(&release.version, CURRENT) && !release.files.is_empty()).then_some(release)
}

fn check_release(release: &Release) -> Result<(), String> {
    let mut seen = HashSet::new();
    for file in &release.files {
        if place(&file.path).is_none() {
            return Err(format!("The update has a file Kaye won't write: {}", file.path));
        }
        if file.sha256.len() != 64 || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("The update has no proper hash for {}", file.path));
        }
        if !seen.insert(same_file_key(&file.path)) {
            return Err(format!("The update lists {} twice", file.path));
        }
    }
    Ok(())
}

/// Where a release's file goes inside Kaye's folder. Nothing that could reach outside it, or
/// onto the updater's own files, gets a place.
pub fn place(path: &str) -> Option<PathBuf> {
    let mut place = PathBuf::new();
    for part in path.split(['/', '\\']) {
        let bad = part.is_empty() || part == "." || part == ".." || part.contains([':', '*', '?', '"', '<', '>', '|']) || part.ends_with(['.', ' ']);
        if bad {
            return None;
        }
        place.push(part);
    }
    let key = same_file_key(path);
    let own = key == INSTALLED_LIST || key == WORK_FOLDER || key.starts_with(&format!("{WORK_FOLDER}/"));
    (!own).then_some(place)
}

/// Windows doesn't care about case or which slash, so two names that differ only there are
/// the same file.
fn same_file_key(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

/// The folder Kaye runs from, which is the one an update changes.
pub fn exe_folder() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.to_path_buf())
}

/// SHA-256 of a file, as lowercase hex, or None if it can't be read.
pub fn hash_file(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = vec![0; 1 << 16];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            return Some(hex(hasher.finish().as_ref()));
        }
        hasher.update(&buffer[..read]);
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn read_installed(folder: &Path) -> Installed {
    std::fs::read_to_string(folder.join(INSTALLED_LIST)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// Works out what has to change in `folder` to make it `release`. Files that already match
/// their hash are left alone.
pub fn plan(folder: &Path, release: &Release) -> Result<Plan, String> {
    check_release(release)?;
    let fetch = release
        .files
        .iter()
        .filter(|file| {
            let local = place(&file.path).map(|p| folder.join(p));
            local.and_then(|p| hash_file(&p)).as_deref() != Some(file.sha256.to_lowercase().as_str())
        })
        .cloned()
        .collect();
    let keep: HashSet<String> = release.files.iter().map(|f| same_file_key(&f.path)).collect();
    let mut remove = Vec::new();
    for old in read_installed(folder).files {
        let Some(relative) = place(&old) else { continue };
        if !keep.contains(&same_file_key(&old)) && folder.join(&relative).is_file() {
            remove.push(relative);
        }
    }
    Ok(Plan {
        version: release.version.clone(),
        fetch,
        remove,
        files: release.files.iter().map(|f| f.path.clone()).collect(),
    })
}

fn staging(folder: &Path) -> PathBuf {
    folder.join(WORK_FOLDER).join("new")
}

/// Clears what an earlier try left, so a download starts clean.
pub fn clear_work_folder(folder: &Path) {
    let _ = std::fs::remove_dir_all(folder.join(WORK_FOLDER));
}

/// Downloads every file the plan fetches into the work folder, checking each one's size and
/// hash. Nothing in Kaye's folder changes yet.
pub fn download(plan: &Plan, folder: &Path, progress: &Progress) -> Result<(), Failure> {
    let staging = staging(folder);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| format!("Kaye's folder can't be written to: {e}"))?;
    progress.total.store(plan.download_size(), Ordering::SeqCst);
    progress.done.store(0, Ordering::SeqCst);
    let agent = agent();
    for file in &plan.fetch {
        let target = staging.join(place(&file.path).ok_or_else(|| format!("Kaye won't write {}", file.path))?);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Kaye's folder can't be written to: {e}"))?;
        }
        download_one(&agent, file, &target, progress)?;
    }
    Ok(())
}

fn download_one(agent: &ureq::Agent, file: &ReleaseFile, target: &Path, progress: &Progress) -> Result<(), Failure> {
    let failed = |e: &dyn std::fmt::Display| Failure::Failed(format!("{} didn't download: {e}", file.path));
    let mut answer = agent.get(&file.url).call().map_err(|e| failed(&e))?;
    let mut body = answer.body_mut().as_reader();
    let mut out = std::fs::File::create(target).map_err(|e| Failure::Failed(format!("Kaye's folder can't be written to: {e}")))?;
    let mut hasher = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = vec![0; 1 << 16];
    let mut got = 0u64;
    loop {
        if progress.cancel.load(Ordering::SeqCst) {
            return Err(Failure::Cancelled);
        }
        let read = body.read(&mut buffer).map_err(|e| failed(&e))?;
        if read == 0 {
            break;
        }
        got += read as u64;
        if got > file.size {
            return Err(failed(&"it was bigger than the update list says"));
        }
        hasher.update(&buffer[..read]);
        out.write_all(&buffer[..read]).map_err(|e| failed(&e))?;
        progress.done.fetch_add(read as u64, Ordering::SeqCst);
    }
    out.flush().map_err(|e| failed(&e))?;
    if got != file.size {
        return Err(failed(&"it stopped partway"));
    }
    if hex(hasher.finish().as_ref()) != file.sha256.to_lowercase() {
        return Err(failed(&"it doesn't match the update list, so it may be damaged"));
    }
    Ok(())
}

/// Swaps the downloaded files in. If anything fails, every file goes back where it was and
/// the error says what happened, so the old version still runs.
pub fn apply(plan: &Plan, folder: &Path) -> Result<(), String> {
    let new = staging(folder);
    let old = folder.join(WORK_FOLDER).join(format!("old-{}", std::process::id()));
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let result = swap(plan, folder, &new, &old, &mut moved);
    if result.is_err() {
        for (from, to) in moved.iter().rev() {
            let _ = std::fs::rename(to, from);
        }
    } else {
        for relative in &plan.remove {
            remove_empty_parents(folder, relative);
        }
    }
    result
}

fn swap(plan: &Plan, folder: &Path, new: &Path, old: &Path, moved: &mut Vec<(PathBuf, PathBuf)>) -> Result<(), String> {
    let mut move_file = |from: PathBuf, to: PathBuf| -> Result<(), String> {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Kaye's folder can't be written to: {e}"))?;
        }
        std::fs::rename(&from, &to).map_err(|e| format!("{} couldn't be replaced: {e}", from.display()))?;
        moved.push((from, to));
        Ok(())
    };
    for file in &plan.fetch {
        let relative = place(&file.path).ok_or_else(|| format!("Kaye won't write {}", file.path))?;
        let live = folder.join(&relative);
        if live.exists() {
            move_file(live.clone(), old.join(&relative))?;
        }
        move_file(new.join(&relative), live)?;
    }
    for relative in &plan.remove {
        let live = folder.join(relative);
        if live.exists() {
            move_file(live, old.join(relative))?;
        }
    }
    let list = Installed { version: plan.version.clone(), files: plan.files.clone() };
    let text = serde_json::to_string_pretty(&list).map_err(|e| e.to_string())?;
    let temporary = folder.join(format!("{INSTALLED_LIST}.tmp"));
    std::fs::write(&temporary, text)
        .and_then(|()| std::fs::rename(&temporary, folder.join(INSTALLED_LIST)))
        .map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            format!("The list of Kaye's files couldn't be saved: {e}")
        })
}

/// A folder a removed file leaves empty goes too, but never Kaye's own folder.
fn remove_empty_parents(folder: &Path, relative: &Path) {
    let mut parent = relative.parent();
    while let Some(dir) = parent.filter(|d| !d.as_os_str().is_empty()) {
        if std::fs::remove_dir(folder.join(dir)).is_err() {
            break;
        }
        parent = dir.parent();
    }
}

/// Deletes the work folder after an update, on another thread. The updater runs from its old
/// copy in there and may take a moment to close, so this keeps trying for a while.
pub fn clean_up_after_update() {
    let Some(folder) = exe_folder() else { return };
    let work = folder.join(WORK_FOLDER);
    std::thread::spawn(move || {
        for _ in 0..40 {
            if !work.exists() || std::fs::remove_dir_all(&work).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

/// Starts this exe as the updater. Kaye closes straight after.
pub fn start_updater() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut command = std::process::Command::new(&exe);
    command.arg("--update").arg(std::process::id().to_string());
    if let Some(folder) = exe.parent() {
        command.current_dir(folder);
    }
    command.spawn().map(drop)
}

/// Starts Kaye from `exe`. `updated` tells it an update just finished.
pub fn launch(exe: &Path, updated: bool) -> std::io::Result<()> {
    let mut command = std::process::Command::new(exe);
    if updated {
        command.arg("--updated");
    }
    if let Some(folder) = exe.parent() {
        command.current_dir(folder);
    }
    // A scripted test run must not run its script again in the Kaye that comes back, or it
    // would click Update forever. It can give that Kaye a script of its own.
    #[cfg(feature = "devtools")]
    match std::env::var_os("KAYE_DEV_RELAUNCH_SCRIPT") {
        Some(script) => command.env("KAYE_DEV_SCRIPT", script),
        None => command.env_remove("KAYE_DEV_SCRIPT"),
    };
    command.spawn().map(drop)
}

/// Waits for the Kaye that started the updater to close, for up to `limit`. Returns false if
/// it's still open.
pub fn wait_for_exit(pid: u32, limit: Duration) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
        // SAFETY: the handle is checked before use and closed once.
        unsafe {
            let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if handle.is_null() {
                return true;
            }
            let waited = WaitForSingleObject(handle, limit.as_millis().min(u32::MAX as u128) as u32);
            CloseHandle(handle);
            waited != WAIT_TIMEOUT
        }
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        std::thread::sleep(limit.min(Duration::from_secs(2)));
        true
    }
}
