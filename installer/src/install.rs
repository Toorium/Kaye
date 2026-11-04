//! Putting Kaye on a computer, and taking it off. Everything goes under the current user, so
//! nothing needs administrator rights.
//!
//! The installer never touches `%APPDATA%\Kaye`, where settings, the API key and backups
//! live. A new computer starts with Kaye's defaults, and an update or an uninstall leaves
//! whatever the person already has.

use std::fs::File;
use std::io::{self, BufWriter, ErrorKind, Write};
use std::path::{Path, PathBuf};

use crate::pack::{self, Payload};
use crate::system::{self, Value};

pub const PROGRAM: &str = "kaye.exe";
pub const UNINSTALLER: &str = "Uninstall Kaye.exe";
const SHORTCUT: &str = "Kaye.lnk";
const UNINSTALL_KEY: &str = r"Microsoft\Windows\CurrentVersion\Uninstall\Kaye";
/// The registry keys for `.kaye` files, which go when Kaye does.
const FILE_TYPE_KEYS: [&str; 2] = [r"Classes\.kaye", r"Classes\Kaye.Project"];
const OPEN_COMMAND_KEY: &str = r"Classes\Kaye.Project\shell\open\command";

/// How far each step gets the progress bar.
const UNPACKED: f32 = 0.9;
const COPIED: f32 = 0.95;

pub type Report<'a> = &'a mut dyn FnMut(f32, &'static str);

/// Everywhere an install touches.
#[derive(Clone, Debug)]
pub struct Places {
    pub folder: PathBuf,
    pub start_menu: PathBuf,
    pub desktop: PathBuf,
    /// The registry key everything goes under. It's `Software`, or a key of its own in tests.
    pub registry: String,
}

impl Places {
    fn program(&self) -> PathBuf {
        self.folder.join(PROGRAM)
    }

    fn uninstaller(&self) -> PathBuf {
        self.folder.join(UNINSTALLER)
    }

    fn uninstall_key(&self) -> String {
        format!(r"{}\{UNINSTALL_KEY}", self.registry)
    }
}

/// Where the real locations are. With `devtools`, `KAYE_SETUP_ROOT` swaps all of them for
/// folders inside it and a registry key of their own, so a test can't touch the real ones.
pub fn real_places(folder: PathBuf) -> Option<Places> {
    #[cfg(feature = "devtools")]
    if let Some(root) = std::env::var_os("KAYE_SETUP_ROOT") {
        let root = PathBuf::from(root);
        return Some(Places { folder, start_menu: root.join("Start Menu"), desktop: root.join("Desktop"), registry: r"Software\KayeSetupScript".to_string() });
    }
    Some(Places { folder, start_menu: system::start_menu_folder()?, desktop: system::desktop_folder()?, registry: "Software".to_string() })
}

/// The folder to offer: where Kaye already is, or else where a program for one user goes.
pub fn suggested_folder() -> Option<PathBuf> {
    #[cfg(feature = "devtools")]
    if let Some(root) = std::env::var_os("KAYE_SETUP_ROOT") {
        return Some(PathBuf::from(root).join("Programs").join("Kaye"));
    }
    let places_key = format!(r"Software\{UNINSTALL_KEY}");
    installed_at(&places_key).or_else(|| Some(system::programs_folder()?.join("Kaye")))
}

fn installed_at(key: &str) -> Option<PathBuf> {
    system::text_value(key, Some("InstallLocation")).map(PathBuf::from).filter(|folder| folder.join(PROGRAM).exists())
}

/// Whether installing here would replace a copy of Kaye.
pub fn is_installed(folder: &Path) -> bool {
    folder.join(PROGRAM).exists()
}

/// Writes a file to one side, and only puts it in place once it's all there.
fn write_in_place(path: &Path, write: impl FnOnce(&mut BufWriter<File>) -> io::Result<()>) -> Result<(), String> {
    let temporary = path.with_added_extension("new");
    let written = File::create(&temporary).map(BufWriter::new).and_then(|mut file| {
        write(&mut file)?;
        file.flush()
    });
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(match error.kind() {
            ErrorKind::InvalidData | ErrorKind::UnexpectedEof => "This installer is damaged. Download it again.".to_string(),
            _ => format!("Couldn't write to this folder: {error}"),
        });
    }
    if std::fs::rename(&temporary, path).is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err("Kaye is open. Close it, then try again.".to_string());
    }
    Ok(())
}

fn add_shortcut(folder: &Path, program: &Path) -> Result<(), String> {
    std::fs::create_dir_all(folder).map_err(|error| format!("Couldn't make the shortcut: {error}"))?;
    system::make_shortcut(&folder.join(SHORTCUT), program, "Animate Roblox rigs")
}

/// The same entries Kaye checks on every launch in `association.rs`. A test holds the two
/// together, so a Kaye that's been installed and never opened still owns its files.
fn file_type_entries(registry: &str, program: &str) -> [(String, String); 4] {
    [
        (format!(r"{registry}\Classes\.kaye"), "Kaye.Project".to_string()),
        (format!(r"{registry}\Classes\Kaye.Project"), "Kaye Project".to_string()),
        (format!(r"{registry}\Classes\Kaye.Project\DefaultIcon"), format!(r#""{program}",0"#)),
        (format!(r"{registry}\{OPEN_COMMAND_KEY}"), format!(r#""{program}" "%1""#)),
    ]
}

/// Everything Windows has to be told, so Kaye shows in Installed Apps and `.kaye` files open
/// with it.
fn register(places: &Places, size: u64) -> Result<(), String> {
    let program = places.program();
    let program = program.to_string_lossy();
    let key = places.uninstall_key();
    let folder = places.folder.to_string_lossy();
    let uninstaller = format!("\"{}\"", places.uninstaller().display());
    let icon = format!("\"{program}\",0");
    system::set_value(&key, Some("DisplayName"), Value::Text("Kaye"))?;
    system::set_value(&key, Some("DisplayVersion"), Value::Text(env!("CARGO_PKG_VERSION")))?;
    system::set_value(&key, Some("Publisher"), Value::Text("Kaye"))?;
    system::set_value(&key, Some("DisplayIcon"), Value::Text(&icon))?;
    system::set_value(&key, Some("InstallLocation"), Value::Text(&folder))?;
    system::set_value(&key, Some("UninstallString"), Value::Text(&uninstaller))?;
    system::set_value(&key, Some("EstimatedSize"), Value::Number((size / 1024) as u32))?;
    system::set_value(&key, Some("NoModify"), Value::Number(1))?;
    system::set_value(&key, Some("NoRepair"), Value::Number(1))?;
    for (file_key, value) in file_type_entries(&places.registry, &program) {
        system::set_value(&file_key, None, Value::Text(&value))?;
    }
    system::file_types_changed();
    Ok(())
}

/// Installs Kaye from `setup`, the installer file itself. Running it again over an install
/// is how an update works, and a failure partway leaves the old copy working.
pub fn install(setup: &Path, payload: &Payload, places: &Places, desktop_shortcut: bool, report: Report) -> Result<(), String> {
    report(0.0, "Unpacking Kaye");
    std::fs::create_dir_all(&places.folder).map_err(|error| match error.kind() {
        ErrorKind::PermissionDenied => "Windows won't let Kaye go in this folder. Pick another one.".to_string(),
        _ => format!("Couldn't use this folder: {error}"),
    })?;
    let mut file = File::open(setup).map_err(|error| format!("Couldn't read the installer: {error}"))?;
    write_in_place(&places.program(), |out| pack::unpack(&mut file, payload, out, |share| report(share * UNPACKED, "Unpacking Kaye")))?;

    report(UNPACKED, "Adding the uninstaller");
    let stub = pack::stub_bytes(&mut file, payload).map_err(|error| format!("Couldn't read the installer: {error}"))?;
    write_in_place(&places.uninstaller(), |out| out.write_all(&stub))?;

    report(COPIED, "Adding shortcuts");
    let program = places.program();
    add_shortcut(&places.start_menu, &program)?;
    if desktop_shortcut {
        add_shortcut(&places.desktop, &program)?;
    } else {
        let _ = std::fs::remove_file(places.desktop.join(SHORTCUT));
    }
    register(places, payload.size + stub.len() as u64)?;
    report(1.0, "Done");
    Ok(())
}

/// Whether `.kaye` files open with the copy being removed, and not one somewhere else.
fn owns_file_type(places: &Places) -> bool {
    let key = format!(r"{}\{OPEN_COMMAND_KEY}", places.registry);
    system::text_value(&key, None).is_some_and(|command| command.to_lowercase().contains(&places.program().to_string_lossy().to_lowercase()))
}

/// Takes Kaye off the computer, apart from the uninstaller itself, which is still running.
/// The program goes first because it's the one thing that can refuse, so nothing's half gone
/// if Kaye is open. Anything else someone put in the folder stays.
pub fn uninstall(places: &Places, report: Report) -> Result<(), String> {
    report(0.0, "Removing Kaye");
    if let Err(error) = std::fs::remove_file(places.program())
        && error.kind() != ErrorKind::NotFound
    {
        return Err("Kaye is open. Close it, then try again.".to_string());
    }
    report(0.5, "Removing shortcuts");
    for folder in [&places.start_menu, &places.desktop] {
        let _ = std::fs::remove_file(folder.join(SHORTCUT));
    }
    report(0.75, "Cleaning up");
    if owns_file_type(places) {
        for key in FILE_TYPE_KEYS {
            system::delete_key(&format!(r"{}\{key}", places.registry))?;
        }
        system::file_types_changed();
    }
    system::delete_key(&places.uninstall_key())?;
    report(1.0, "Done");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real install into folders and a registry key that go away with the test.
    struct Bench {
        root: PathBuf,
        setup: PathBuf,
        payload: Payload,
        places: Places,
    }

    impl Bench {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("kaye-setup-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let setup = root.join("setup.exe");
            std::fs::write(&setup, pack::pack(b"the setup program", &b"the program".repeat(5000)).unwrap()).unwrap();
            let payload = pack::find(&mut File::open(&setup).unwrap()).unwrap().unwrap();
            let places = Places {
                folder: root.join("Programs").join("Kaye"),
                start_menu: root.join("Start Menu"),
                desktop: root.join("Desktop"),
                registry: format!(r"Software\KayeSetupTest{}{name}", std::process::id()),
            };
            Self { root, setup, payload, places }
        }

        fn install(&self, desktop_shortcut: bool) -> Result<(), String> {
            install(&self.setup, &self.payload, &self.places, desktop_shortcut, &mut |_, _| {})
        }
    }

    impl Drop for Bench {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
            let _ = system::delete_key(&self.places.registry);
        }
    }

    #[test]
    fn file_types_match_what_kaye_registers_itself() {
        let program = r"C:\Program Files\Kaye\kaye.exe";
        assert_eq!(file_type_entries("Software", program).to_vec(), kaye::association::entries(program));
    }

    #[test]
    fn an_install_puts_everything_in_place() {
        let bench = Bench::new("install");
        let places = &bench.places;
        let mut steps = Vec::new();
        install(&bench.setup, &bench.payload, places, true, &mut |share, _| steps.push(share)).unwrap();

        assert_eq!(std::fs::read(places.program()).unwrap(), b"the program".repeat(5000));
        assert_eq!(std::fs::read(places.uninstaller()).unwrap(), b"the setup program", "the uninstaller has no copy of Kaye in it");
        assert!(places.start_menu.join(SHORTCUT).exists());
        assert!(places.desktop.join(SHORTCUT).exists());
        assert!(steps.windows(2).all(|w| w[0] <= w[1]) && steps.last() == Some(&1.0));

        let key = places.uninstall_key();
        assert_eq!(system::text_value(&key, Some("DisplayName")).as_deref(), Some("Kaye"));
        assert_eq!(installed_at(&key).as_ref(), Some(&places.folder));
        let uninstall = system::text_value(&key, Some("UninstallString")).unwrap();
        assert!(uninstall.starts_with('"') && uninstall.ends_with(&format!("{UNINSTALLER}\"")), "a path with spaces needs quotes: {uninstall}");
        let command = system::text_value(&format!(r"{}\{OPEN_COMMAND_KEY}", places.registry), None).unwrap();
        assert!(command.contains(PROGRAM) && command.ends_with("\"%1\""), "{command}");
        assert!(!std::fs::read_dir(&places.folder).unwrap().flatten().any(|f| f.path().extension().is_some_and(|e| e == "new")), "nothing's left over");
    }

    #[test]
    fn the_desktop_shortcut_is_optional_and_an_update_can_take_it_away() {
        let bench = Bench::new("desktop");
        bench.install(false).unwrap();
        assert!(!bench.places.desktop.join(SHORTCUT).exists());
        bench.install(true).unwrap();
        assert!(bench.places.desktop.join(SHORTCUT).exists());
        bench.install(false).unwrap();
        assert!(!bench.places.desktop.join(SHORTCUT).exists());
    }

    #[test]
    fn an_update_keeps_what_isnt_ours() {
        let bench = Bench::new("update");
        bench.install(false).unwrap();
        let notes = bench.places.folder.join("my notes.txt");
        std::fs::write(&notes, "mine").unwrap();
        std::fs::write(bench.places.program(), "an older Kaye").unwrap();
        bench.install(false).unwrap();
        assert_eq!(std::fs::read(bench.places.program()).unwrap(), b"the program".repeat(5000));
        assert_eq!(std::fs::read_to_string(&notes).unwrap(), "mine");
    }

    #[test]
    fn a_running_kaye_stops_an_update_and_the_old_copy_stays() {
        let bench = Bench::new("running");
        bench.install(false).unwrap();
        std::fs::write(bench.places.program(), "the old one").unwrap();
        // Holding the file open with no sharing locks it the way a running program does.
        let held = std::os::windows::fs::OpenOptionsExt::share_mode(std::fs::OpenOptions::new().read(true), 0).open(bench.places.program()).unwrap();
        let problem = bench.install(false).unwrap_err();
        assert!(problem.contains("Kaye is open"), "{problem}");
        drop(held);
        assert_eq!(std::fs::read_to_string(bench.places.program()).unwrap(), "the old one");
    }

    #[test]
    fn a_damaged_installer_leaves_the_old_copy_alone() {
        let bench = Bench::new("damaged");
        bench.install(false).unwrap();
        std::fs::write(bench.places.program(), "the old one").unwrap();
        let mut bytes = std::fs::read(&bench.setup).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0xFF;
        std::fs::write(&bench.setup, bytes).unwrap();
        let problem = bench.install(false).unwrap_err();
        assert!(problem.contains("damaged"), "{problem}");
        assert_eq!(std::fs::read_to_string(bench.places.program()).unwrap(), "the old one");
    }

    #[test]
    fn an_uninstall_takes_everything_back_off() {
        let bench = Bench::new("uninstall");
        let places = &bench.places;
        bench.install(true).unwrap();
        uninstall(places, &mut |_, _| {}).unwrap();

        assert!(!places.program().exists());
        assert!(!places.start_menu.join(SHORTCUT).exists());
        assert!(!places.desktop.join(SHORTCUT).exists());
        assert_eq!(system::text_value(&places.uninstall_key(), Some("DisplayName")), None);
        assert_eq!(system::text_value(&format!(r"{}\Classes\.kaye", places.registry), None), None);
        assert!(places.uninstaller().exists(), "it's still running, so it goes after the window closes");
    }

    #[test]
    fn an_uninstall_only_removes_files_it_put_there() {
        let bench = Bench::new("own-files");
        bench.install(false).unwrap();
        let notes = bench.places.folder.join("my notes.txt");
        std::fs::write(&notes, "mine").unwrap();
        uninstall(&bench.places, &mut |_, _| {}).unwrap();
        assert_eq!(std::fs::read_to_string(&notes).unwrap(), "mine");
    }

    #[test]
    fn an_uninstall_leaves_kaye_project_files_with_another_copy() {
        let bench = Bench::new("other-copy");
        bench.install(false).unwrap();
        let key = format!(r"{}\{OPEN_COMMAND_KEY}", bench.places.registry);
        system::set_value(&key, None, Value::Text(r#""D:\Elsewhere\kaye.exe" "%1""#)).unwrap();
        uninstall(&bench.places, &mut |_, _| {}).unwrap();
        assert!(system::text_value(&key, None).is_some(), "that copy still needs it");
    }

    #[test]
    fn a_running_kaye_stops_an_uninstall_with_nothing_half_gone() {
        let bench = Bench::new("uninstall-running");
        bench.install(true).unwrap();
        let held = std::os::windows::fs::OpenOptionsExt::share_mode(std::fs::OpenOptions::new().read(true), 0).open(bench.places.program()).unwrap();
        let problem = uninstall(&bench.places, &mut |_, _| {}).unwrap_err();
        drop(held);
        assert!(problem.contains("Kaye is open"), "{problem}");
        assert!(bench.places.start_menu.join(SHORTCUT).exists());
    }

    #[test]
    fn a_shortcut_points_at_the_program() {
        let bench = Bench::new("shortcut");
        bench.install(false).unwrap();
        let bytes = std::fs::read(bench.places.start_menu.join(SHORTCUT)).unwrap();
        let wanted: Vec<u8> = "kaye.exe".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let found = bytes.windows(wanted.len()).any(|w| w == wanted) || bytes.windows(8).any(|w| w == b"kaye.exe");
        assert!(found, "the shortcut should name the program");
    }
}
