//! The files behind the home page: the recent list, backups and thumbnails.

use std::path::{Path, PathBuf};

use kaye::library::{self, Recent};

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let folder = std::env::temp_dir().join(format!("kaye-library-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        Self(folder)
    }

    fn touch(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, "x").unwrap();
        path
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn names(list: &[Recent]) -> Vec<String> {
    list.iter().map(Recent::name).collect()
}

#[test]
fn the_last_file_opened_comes_first_and_only_once() {
    let folder = Folder::new("order");
    let list = folder.0.join("recent.json");
    let (a, b) = (folder.touch("a.kaye"), folder.touch("b.rbxm"));
    library::remember(&list, &a, 10);
    library::remember(&list, &b, 20);
    library::remember(&list, &a, 30);
    let recent = library::recent_files(&list);
    assert_eq!(names(&recent), ["a", "b"]);
    assert_eq!(recent[0].opened, 30);
    assert!(recent[0].is_project());
    assert!(!recent[1].is_project());
}

#[test]
fn the_list_has_a_limit_and_keeps_the_newest() {
    let folder = Folder::new("limit");
    let list = folder.0.join("recent.json");
    for index in 0..30 {
        library::remember(&list, &folder.touch(&format!("{index}.kaye")), index);
    }
    let recent = library::recent_files(&list);
    assert_eq!(recent.len(), 24);
    assert_eq!(recent[0].name(), "29");
}

#[test]
fn a_file_that_has_gone_stops_showing() {
    let folder = Folder::new("gone");
    let list = folder.0.join("recent.json");
    let (kept, gone) = (folder.touch("kept.kaye"), folder.touch("gone.kaye"));
    library::remember(&list, &kept, 1);
    library::remember(&list, &gone, 2);
    std::fs::remove_file(&gone).unwrap();
    assert_eq!(names(&library::recent_files(&list)), ["kept"]);
}

#[test]
fn a_damaged_or_missing_list_is_just_empty() {
    let folder = Folder::new("damaged");
    let list = folder.0.join("recent.json");
    assert!(library::recent_files(&list).is_empty());
    std::fs::write(&list, "not json").unwrap();
    assert!(library::recent_files(&list).is_empty());
    let file = folder.touch("a.kaye");
    library::remember(&list, &file, 1);
    assert_eq!(library::recent_files(&list).len(), 1);
}

fn write_backup(folder: &Path, name: &str, made: u64) {
    std::fs::write(library::backup_path(folder, name, made), "backup").unwrap();
}

#[test]
fn a_backup_names_its_project_and_when_it_was_made() {
    let folder = Folder::new("names");
    write_backup(&folder.0, "Walk Cycle", 100);
    write_backup(&folder.0, "Walk Cycle", 300);
    write_backup(&folder.0, "Rig: v1/final", 200);
    let found = library::backups(&folder.0);
    assert_eq!(found.iter().map(|b| b.made).collect::<Vec<_>>(), [300, 200, 100], "newest first");
    assert_eq!(found[0].name, "Walk Cycle");
    assert_eq!(found[1].name, "Rig_ v1_final", "anything a file name can't hold is swapped out");
    assert_eq!(found[0].size, 6);
}

#[test]
fn only_kaye_files_with_a_time_count_as_backups() {
    let folder = Folder::new("others");
    write_backup(&folder.0, "Real", 100);
    folder.touch("Real.100.kaye.tmp");
    folder.touch("Plain.kaye");
    folder.touch("notes.txt");
    assert_eq!(library::backups(&folder.0).len(), 1);
    assert!(library::backups(&folder.0.join("no such folder")).is_empty());
}

#[test]
fn cleaning_up_keeps_the_newest_of_each_project() {
    let folder = Folder::new("prune");
    for made in 0..8 {
        write_backup(&folder.0, "Long", made);
    }
    write_backup(&folder.0, "Short", 3);
    library::prune_backups(&folder.0);
    let found = library::backups(&folder.0);
    let long: Vec<u64> = found.iter().filter(|b| b.name == "Long").map(|b| b.made).collect();
    assert_eq!(long, [7, 6, 5, 4, 3]);
    assert_eq!(found.iter().filter(|b| b.name == "Short").count(), 1, "one project can't push out another");
}

#[test]
fn cleaning_up_has_a_limit_across_projects() {
    let folder = Folder::new("total");
    for project in 0..50 {
        write_backup(&folder.0, &format!("Project {project}"), project);
    }
    library::prune_backups(&folder.0);
    let found = library::backups(&folder.0);
    assert_eq!(found.len(), 40);
    assert_eq!(found[0].made, 49);
}

#[test]
fn only_a_file_in_the_folder_is_a_backup() {
    let folder = Path::new("C:/Kaye/backups");
    assert!(library::is_backup(folder, &folder.join("Walk.100.kaye")));
    assert!(!library::is_backup(folder, Path::new("C:/Kaye/Walk.kaye")));
    assert!(!library::is_backup(folder, &folder.join("old").join("Walk.100.kaye")));
}

#[test]
fn times_read_the_way_a_person_would_say_them() {
    let at = 1_000_000;
    let ago = |seconds: u64| library::time_ago(at - seconds, at);
    assert_eq!(ago(10), "Just now");
    assert_eq!(ago(60), "1 minute ago");
    assert_eq!(ago(59 * 60), "59 minutes ago");
    assert_eq!(ago(60 * 60), "1 hour ago");
    assert_eq!(ago(5 * 3600), "5 hours ago");
    assert_eq!(ago(24 * 3600), "1 day ago");
    assert_eq!(ago(3 * 24 * 3600), "3 days ago");
    assert_eq!(library::time_ago(at + 500, at), "Just now", "a clock that went backward isn't a problem");
    assert_eq!(library::size_text(2048), "2 KB");
    assert_eq!(library::size_text(3 * 1_048_576 / 2), "1.5 MB");
}

#[test]
fn a_thumbnail_is_the_middle_of_the_view_at_a_fixed_size() {
    let folder = Folder::new("thumbnail");
    // Wide, with red on the left and right edges and green in the middle.
    let view = image::RgbaImage::from_fn(1200, 400, |x, _| {
        if (300..900).contains(&x) { image::Rgba([0, 255, 0, 255]) } else { image::Rgba([255, 0, 0, 255]) }
    });
    let target = library::thumbnail_path(&folder.0.join("made"), Path::new("C:/a project.kaye"));
    library::write_thumbnail(&view, &target).unwrap();
    let saved = image::open(&target).unwrap().to_rgb8();
    assert_eq!(saved.dimensions(), (356, 268));
    assert_eq!(saved.get_pixel(2, 134).0, [0, 255, 0], "the sides are left out");
    assert_eq!(saved.get_pixel(353, 134).0, [0, 255, 0]);
}

#[test]
fn a_view_with_no_size_makes_no_thumbnail() {
    let folder = Folder::new("empty");
    let target = folder.0.join("empty.png");
    assert!(library::write_thumbnail(&image::RgbaImage::new(0, 0), &target).is_err());
    assert!(!target.exists());
}

#[test]
fn each_file_has_its_own_thumbnail_in_any_case() {
    let folder = Path::new("C:/thumbnails");
    let one = library::thumbnail_path(folder, Path::new("C:/a.kaye"));
    assert_eq!(one, library::thumbnail_path(folder, Path::new("C:/A.KAYE")), "Windows paths ignore case");
    assert_ne!(one, library::thumbnail_path(folder, Path::new("C:/b.kaye")));
}
