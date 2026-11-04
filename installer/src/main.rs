#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kaye_setup::install::{self, UNINSTALLER};
use kaye_setup::pack::{self, Payload};
use kaye_setup::system;
use kaye_setup::wizard::{self, Mode};

/// What this copy of the program is holding, if anything.
fn own_payload(exe: &Path) -> std::io::Result<Option<Payload>> {
    pack::find(&mut File::open(exe)?)
}

/// `--check` is for the build script: it proves the packed program is all there.
fn check(exe: &Path) -> ExitCode {
    let unpacked = own_payload(exe).ok().flatten().is_some_and(|payload| {
        let Ok(mut file) = File::open(exe) else {
            return false;
        };
        pack::unpack(&mut file, &payload, &mut std::io::sink(), |_| {}).is_ok()
    });
    if unpacked { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn main() -> ExitCode {
    let Ok(exe) = std::env::current_exe() else {
        system::message_box("Windows wouldn't say where this program is.");
        return ExitCode::FAILURE;
    };
    if std::env::args().any(|arg| arg == "--check") {
        return check(&exe);
    }
    // With Kaye on the end it's an installer. Without, it's the copy left beside Kaye.
    let mode = match own_payload(&exe) {
        Ok(Some(payload)) => Mode::Install { setup: exe.clone(), payload },
        Ok(None) => Mode::Uninstall { folder: exe.parent().map(PathBuf::from).unwrap_or_default() },
        Err(error) => {
            system::message_box(&format!("Couldn't read this installer: {error}"));
            return ExitCode::FAILURE;
        }
    };
    let Some(suggested) = install::suggested_folder() else {
        system::message_box("Windows wouldn't say where programs go.");
        return ExitCode::FAILURE;
    };
    let removed = wizard::run(mode, &suggested);
    if removed && exe.file_name().is_some_and(|name| name == UNINSTALLER) {
        if let Some(folder) = exe.parent() {
            system::remove_after_exit(&exe, folder);
        }
    }
    ExitCode::SUCCESS
}
