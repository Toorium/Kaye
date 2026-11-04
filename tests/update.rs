//! Checking for, downloading and installing updates, against a small local server that stands
//! in for GitHub.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kaye::update::{self, Failure, Plan, Progress, Release, ReleaseFile};

enum Reply {
    Body(Vec<u8>),
    /// Where GitHub sends a release download, before the file itself.
    Redirect(String),
}

/// Serves each path's reply, and returns the address to ask.
fn serve(routes: HashMap<String, Reply>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let routes = Arc::new(routes);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let routes = routes.clone();
            std::thread::spawn(move || {
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                    request.push(byte[0]);
                }
                let text = String::from_utf8_lossy(&request);
                let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
                let head = match routes.get(&path) {
                    Some(Reply::Body(body)) => {
                        let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(body);
                        return;
                    }
                    Some(Reply::Redirect(to)) => format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                    None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
                };
                let _ = stream.write_all(head.as_bytes());
            });
        }
    });
    address
}

fn sha256(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn folder(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("kaye-update-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

fn file(path: &str, bytes: &[u8], url: String) -> ReleaseFile {
    ReleaseFile { path: path.to_string(), size: bytes.len() as u64, sha256: sha256(bytes), url }
}

fn read(path: impl AsRef<Path>) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

#[test]
fn only_a_later_version_is_newer() {
    assert!(update::is_newer("0.1.1", "0.1.0"));
    assert!(update::is_newer("v0.2.0", "0.1.9"));
    assert!(update::is_newer("0.10.0", "0.9.0"));
    assert!(update::is_newer("1.0.0", "1.0.0-beta"));
    assert!(!update::is_newer("0.1.0", "0.1.0"));
    // A newer build being worked on is never asked to go back.
    assert!(!update::is_newer("0.1.0", "0.1.1"));
    assert!(!update::is_newer("soon", "0.1.0"));
    assert!(!update::is_newer("", "0.1.0"));
}

#[test]
fn files_stay_inside_kayes_folder() {
    assert_eq!(update::place("kaye.exe"), Some(PathBuf::from("kaye.exe")));
    assert_eq!(update::place("plugins/example.luau"), Some(PathBuf::from("plugins").join("example.luau")));
    assert_eq!(update::place("plugins\\example.luau"), Some(PathBuf::from("plugins").join("example.luau")));
    for bad in ["", "../kaye.exe", "plugins/../../x", "C:/Windows/x.dll", "/kaye.exe", "\\\\server\\x", "a//b", "./kaye.exe", "kaye.exe:hidden", "kaye.exe.", "kaye-files.json", "KAYE-FILES.JSON", ".kaye-update/new/kaye.exe", ".Kaye-Update"] {
        assert_eq!(update::place(bad), None, "{bad:?} should be refused");
    }
}

#[test]
fn the_published_list_reads() {
    // Kept in the repository, and read by every copy of Kaye from GitHub.
    let ours: Release = serde_json::from_str(include_str!("../latest.json")).unwrap();
    assert!(semver::Version::parse(&ours.version).is_ok());
    for file in &ours.files {
        assert!(update::place(&file.path).is_some(), "{}", file.path);
    }
    // What scripts/publish-update.ps1 writes, as Windows PowerShell 5 lays it out.
    let written = r#"{
    "version":  "0.1.1",
    "notes":  "Adds update notifications.\nIt's quick.",
    "files":  [
                  {
                      "path":  "kaye.exe",
                      "size":  123,
                      "sha256":  "abababababababababababababababababababababababababababababababab",
                      "url":  "https://github.com/Toorium/Kaye/releases/download/v0.1.1/kaye.exe"
                  }
              ]
}"#;
    let release: Release = serde_json::from_str(written).unwrap();
    assert_eq!(release.notes, "Adds update notifications.\nIt's quick.");
    assert_eq!(release.files[0].size, 123);
}

/// Downloads everything GitHub's `latest.json` lists right now, through GitHub's own redirects,
/// and checks every hash. Nothing is installed. It needs the internet, so it only runs when
/// asked: `cargo test --test update -- --ignored`
#[test]
#[ignore]
fn the_published_release_downloads() {
    let release = update::fetch_latest(update::LATEST_URL).unwrap();
    let dir = folder("github");
    let plan = update::plan(&dir, &release).unwrap();
    assert_eq!(plan.fetch.len(), release.files.len());
    let progress = Progress::default();
    update::download(&plan, &dir, &progress).unwrap();
    assert_eq!(progress.done.load(std::sync::atomic::Ordering::SeqCst), plan.download_size());
    println!("Kaye {} downloads from GitHub: {} files, {} bytes", release.version, release.files.len(), plan.download_size());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_bad_release_is_refused() {
    let base = serve(HashMap::from([
        ("/outside.json".to_string(), Reply::Body(br#"{"version":"9.0.0","files":[{"path":"../evil.exe","size":1,"sha256":"00","url":"x"}]}"#.to_vec())),
        ("/garbage.json".to_string(), Reply::Body(b"<html>not json</html>".to_vec())),
    ]));
    assert!(update::fetch_latest(&format!("{base}/outside.json")).is_err());
    assert!(update::fetch_latest(&format!("{base}/garbage.json")).is_err());
    assert!(update::fetch_latest(&format!("{base}/missing.json")).is_err());
}

#[test]
fn an_update_replaces_adds_and_removes_files() {
    let dir = folder("full");
    std::fs::write(dir.join("kaye.exe"), b"old kaye").unwrap();
    std::fs::write(dir.join("same.txt"), b"unchanged").unwrap();
    std::fs::create_dir_all(dir.join("old-plugins")).unwrap();
    std::fs::write(dir.join("old-plugins").join("gone.dll"), b"dropped by the new version").unwrap();
    std::fs::write(dir.join("mine.txt"), b"put here by someone, not by an update").unwrap();
    std::fs::write(dir.join("kaye-files.json"), r#"{"version":"0.1.0","files":["kaye.exe","same.txt","old-plugins/gone.dll"]}"#).unwrap();

    let (new_kaye, data) = (b"new kaye, quite a bit longer".to_vec(), vec![7u8; 300_000]);
    let base = serve(HashMap::from([
        ("/download/kaye.exe".to_string(), Reply::Redirect("/files/kaye.exe".to_string())),
        ("/files/kaye.exe".to_string(), Reply::Body(new_kaye.clone())),
        ("/files/data.bin".to_string(), Reply::Body(data.clone())),
    ]));
    let release = Release {
        version: "9.0.0".into(),
        notes: String::new(),
        files: vec![
            file("kaye.exe", &new_kaye, format!("{base}/download/kaye.exe")),
            file("same.txt", b"unchanged", format!("{base}/never-asked-for")),
            file("assets/data.bin", &data, format!("{base}/files/data.bin")),
        ],
    };
    let plan = update::plan(&dir, &release).unwrap();
    let fetched: Vec<&str> = plan.fetch.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(fetched, ["kaye.exe", "assets/data.bin"], "a file that already matches isn't downloaded");
    assert_eq!(plan.remove, [PathBuf::from("old-plugins").join("gone.dll")]);
    assert_eq!(plan.download_size(), (new_kaye.len() + data.len()) as u64);

    let progress = Progress::default();
    update::download(&plan, &dir, &progress).unwrap();
    assert_eq!(progress.done.load(std::sync::atomic::Ordering::SeqCst), plan.download_size());
    assert_eq!(read(dir.join("kaye.exe")), b"old kaye", "downloading changes nothing yet");

    update::apply(&plan, &dir).unwrap();
    assert_eq!(read(dir.join("kaye.exe")), new_kaye);
    assert_eq!(read(dir.join("assets").join("data.bin")), data);
    assert_eq!(read(dir.join("same.txt")), b"unchanged");
    assert!(!dir.join("old-plugins").exists(), "the removed file's emptied folder goes too");
    assert!(dir.join("mine.txt").exists(), "files no update installed are never removed");
    let installed: serde_json::Value = serde_json::from_slice(&read(dir.join("kaye-files.json"))).unwrap();
    assert_eq!(installed["version"], "9.0.0");
    assert_eq!(installed["files"], serde_json::json!(["kaye.exe", "same.txt", "assets/data.bin"]));
    // The old exe is kept aside until the new Kaye deletes it.
    let old = dir.join(".kaye-update").join(format!("old-{}", std::process::id()));
    assert_eq!(read(old.join("kaye.exe")), b"old kaye");

    // Asked again, there's nothing left to do.
    let again = update::plan(&dir, &release).unwrap();
    assert!(again.fetch.is_empty() && again.remove.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_first_update_removes_nothing() {
    // The .msi doesn't write kaye-files.json, so nothing is known to be Kaye's but the release.
    let dir = folder("first");
    std::fs::write(dir.join("kaye.exe"), b"installed by the msi").unwrap();
    std::fs::write(dir.join("other.txt"), b"someone's").unwrap();
    let release = Release { version: "9.0.0".into(), notes: String::new(), files: vec![file("kaye.exe", b"new", "http://unused".into())] };
    let plan = update::plan(&dir, &release).unwrap();
    assert!(plan.remove.is_empty());
    assert_eq!(plan.fetch.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_damaged_download_changes_nothing() {
    let dir = folder("damaged");
    std::fs::write(dir.join("kaye.exe"), b"old kaye").unwrap();
    let base = serve(HashMap::from([("/kaye.exe".to_string(), Reply::Body(b"tampered!".to_vec()))]));
    // Same length as what's served, so only the hash can catch it.
    let release = Release { version: "9.0.0".into(), notes: String::new(), files: vec![file("kaye.exe", b"expected!", format!("{base}/kaye.exe"))] };
    let plan = update::plan(&dir, &release).unwrap();
    let failure = update::download(&plan, &dir, &Progress::default()).unwrap_err();
    assert!(matches!(&failure, Failure::Failed(text) if text.contains("damaged")), "{failure:?}");
    assert_eq!(read(dir.join("kaye.exe")), b"old kaye");

    let short = Release { version: "9.0.0".into(), notes: String::new(), files: vec![file("kaye.exe", b"longer than what is served", format!("{base}/kaye.exe"))] };
    let plan = update::plan(&dir, &short).unwrap();
    assert!(update::download(&plan, &dir, &Progress::default()).is_err());

    let missing = Release { version: "9.0.0".into(), notes: String::new(), files: vec![file("kaye.exe", b"x", format!("{base}/nowhere"))] };
    let plan = update::plan(&dir, &missing).unwrap();
    assert!(update::download(&plan, &dir, &Progress::default()).is_err());
    assert_eq!(read(dir.join("kaye.exe")), b"old kaye");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancelling_stops_the_download() {
    let dir = folder("cancel");
    let data = vec![1u8; 200_000];
    let base = serve(HashMap::from([("/kaye.exe".to_string(), Reply::Body(data.clone()))]));
    let release = Release { version: "9.0.0".into(), notes: String::new(), files: vec![file("kaye.exe", &data, format!("{base}/kaye.exe"))] };
    let plan = update::plan(&dir, &release).unwrap();
    let progress = Progress::default();
    progress.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(update::download(&plan, &dir, &progress), Err(Failure::Cancelled));
    assert!(!dir.join("kaye.exe").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file another program holds open without sharing can't be moved, which stands in for any
/// failure partway through the swap.
#[cfg(windows)]
#[test]
fn a_failed_swap_puts_everything_back() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = folder("rollback");
    std::fs::write(dir.join("kaye.exe"), b"old kaye").unwrap();
    std::fs::write(dir.join("locked.dll"), b"old dll").unwrap();
    std::fs::write(dir.join("kaye-files.json"), r#"{"version":"0.1.0","files":["kaye.exe","locked.dll"]}"#).unwrap();
    let base = serve(HashMap::from([
        ("/kaye.exe".to_string(), Reply::Body(b"new kaye".to_vec())),
        ("/locked.dll".to_string(), Reply::Body(b"new dll".to_vec())),
    ]));
    let release = Release {
        version: "9.0.0".into(),
        notes: String::new(),
        files: vec![file("kaye.exe", b"new kaye", format!("{base}/kaye.exe")), file("locked.dll", b"new dll", format!("{base}/locked.dll"))],
    };
    let plan: Plan = update::plan(&dir, &release).unwrap();
    update::download(&plan, &dir, &Progress::default()).unwrap();
    let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(dir.join("locked.dll")).unwrap();
    assert!(update::apply(&plan, &dir).is_err());
    drop(held);
    assert_eq!(read(dir.join("kaye.exe")), b"old kaye", "the file swapped before the failure went back");
    assert_eq!(read(dir.join("locked.dll")), b"old dll");
    let installed: serde_json::Value = serde_json::from_slice(&read(dir.join("kaye-files.json"))).unwrap();
    assert_eq!(installed["version"], "0.1.0");
    let _ = std::fs::remove_dir_all(&dir);
}
