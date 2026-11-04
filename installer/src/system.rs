//! The parts of installing that only Windows can do: the registry, shortcuts, and finding
//! its folders. Nothing here asks for administrator rights.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::{null, null_mut};

use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile};
use windows::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Programs, FOLDERID_UserProgramFiles, IShellLinkW, KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath, ShellLink};
use windows::core::{GUID, HSTRING, Interface};
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, KEY_SET_VALUE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetValueExW,
};
use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// One value in the registry, under the current user.
pub enum Value<'a> {
    Text(&'a str),
    Number(u32),
}

/// Sets a value. With no name it's the key's own value.
pub fn set_value(key: &str, name: Option<&str>, value: Value) -> Result<(), String> {
    let (wide_key, wide_name) = (wide(key), name.map(wide));
    let (kind, data): (_, Vec<u8>) = match value {
        Value::Text(text) => (REG_SZ, wide(text).into_iter().flat_map(u16::to_le_bytes).collect()),
        Value::Number(number) => (REG_DWORD, number.to_le_bytes().to_vec()),
    };
    let mut handle = null_mut();
    // SAFETY: the strings are null-terminated and outlive the calls, and `handle` is only
    // used and closed once the first call says it's valid.
    let result = unsafe {
        let created = RegCreateKeyExW(HKEY_CURRENT_USER, wide_key.as_ptr(), 0, null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, null(), &mut handle, null_mut());
        if created != ERROR_SUCCESS {
            created
        } else {
            let written = RegSetValueExW(handle, wide_name.as_ref().map_or(null(), |n| n.as_ptr()), 0, kind, data.as_ptr(), data.len() as u32);
            RegCloseKey(handle);
            written
        }
    };
    if result == ERROR_SUCCESS { Ok(()) } else { Err(format!("Windows wouldn't save the setting {key} (error {result}).")) }
}

pub fn text_value(key: &str, name: Option<&str>) -> Option<String> {
    let (key, name) = (wide(key), name.map(wide));
    let mut text = [0u16; 2048];
    let mut bytes = size_of_val(&text) as u32;
    // SAFETY: the strings are null-terminated, and `bytes` is the size of the buffer given.
    let result = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ref().map_or(null(), |n| n.as_ptr()), RRF_RT_REG_SZ, null_mut(), text.as_mut_ptr().cast(), &mut bytes) };
    if result != ERROR_SUCCESS {
        return None;
    }
    // The count includes the null on the end.
    let length = (bytes as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&text[..length]))
}

/// Removes a key and everything under it. One that's already gone is fine.
pub fn delete_key(key: &str) -> Result<(), String> {
    let name = wide(key);
    // SAFETY: the name is null-terminated.
    let result = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, name.as_ptr()) };
    if result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND { Ok(()) } else { Err(format!("Windows wouldn't remove the setting {key} (error {result}).")) }
}

/// Tells Explorer file types changed, so `.kaye` files get their icon.
pub fn file_types_changed() {
    // SAFETY: this event takes no items, so both can be null.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, null(), null()) };
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    // A new Windows account has no user programs folder until something makes one, and by
    // default Windows won't say where a missing folder would be. Installing makes it.
    // SAFETY: on success Windows hands over a string this has to free, which is only read
    // and freed then.
    unsafe {
        let path = SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, None).ok()?;
        let text = path.to_string().ok();
        CoTaskMemFree(Some(path.as_ptr().cast()));
        text.map(PathBuf::from)
    }
}

/// Where a program for just this user goes, which needs no administrator.
pub fn programs_folder() -> Option<PathBuf> {
    known_folder(&FOLDERID_UserProgramFiles)
}

pub fn start_menu_folder() -> Option<PathBuf> {
    known_folder(&FOLDERID_Programs)
}

/// This isn't always under the user's folder, since OneDrive can move it.
pub fn desktop_folder() -> Option<PathBuf> {
    known_folder(&FOLDERID_Desktop)
}

/// Makes a shortcut to `target`, starting in the folder it's in.
pub fn make_shortcut(shortcut: &Path, target: &Path, about: &str) -> Result<(), String> {
    let folder = target.parent().unwrap_or(target);
    // SAFETY: COM is set up for this thread and taken down again here, and the strings
    // outlive the calls using them.
    let made = unsafe {
        let started = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let made = (|| -> windows::core::Result<()> {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&HSTRING::from(target.as_os_str()))?;
            link.SetWorkingDirectory(&HSTRING::from(folder.as_os_str()))?;
            link.SetDescription(&HSTRING::from(about))?;
            link.SetIconLocation(&HSTRING::from(target.as_os_str()), 0)?;
            link.cast::<IPersistFile>()?.Save(&HSTRING::from(shortcut.as_os_str()), true)
        })();
        if started.is_ok() {
            CoUninitialize();
        }
        made
    };
    made.map_err(|error| format!("Couldn't make the shortcut: {error}"))
}

/// Deletes a running program and the folder it's in, once it's had time to close. The folder
/// only goes if nothing else is in it.
pub fn remove_after_exit(program: &Path, folder: &Path) {
    let command = format!("/c ping -n 3 127.0.0.1 >nul & del /f /q \"{}\" & rmdir \"{}\"", program.display(), folder.display());
    let _ = Command::new("cmd.exe").raw_arg(command).creation_flags(CREATE_NO_WINDOW).spawn();
}

/// For a problem that stops the window from opening at all.
pub fn message_box(text: &str) {
    let (text, title) = (wide(text), wide("Kaye Setup"));
    // SAFETY: both strings are null-terminated.
    unsafe { MessageBoxW(null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_go_into_the_registry_and_come_back() {
        const KEY: &str = r"Software\KayeSetupRegistryTest";
        let inner = format!(r"{KEY}\inner");
        let path = r"C:\Program Files\Käye\kaye.exe";
        assert_eq!(text_value(&inner, Some("Path")), None);
        set_value(&inner, Some("Path"), Value::Text(path)).unwrap();
        set_value(&inner, None, Value::Text("default")).unwrap();
        set_value(&inner, Some("Size"), Value::Number(1234)).unwrap();
        let (read, default) = (text_value(&inner, Some("Path")), text_value(&inner, None));
        delete_key(KEY).unwrap();
        assert_eq!(read.as_deref(), Some(path));
        assert_eq!(default.as_deref(), Some("default"));
        assert_eq!(text_value(&inner, Some("Path")), None);
        delete_key(KEY).unwrap();
    }

    #[test]
    fn windows_has_a_place_for_everything() {
        assert!(programs_folder().is_some_and(|folder| folder.is_absolute()));
        assert!(start_menu_folder().is_some());
        assert!(desktop_folder().is_some());
    }
}
