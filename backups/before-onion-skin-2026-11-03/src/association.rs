//! Tells Windows that `.kaye` files open with Kaye. The entries go under the current
//! user, which needs no administrator rights, and they're checked on every launch so
//! moving the exe can't leave them pointing at nothing.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegGetValueW, RegSetValueExW,
};
use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};

use crate::project;

const CLASSES: &str = r"Software\Classes";
const FILE_TYPE: &str = "Kaye.Project";

/// Each registry key and the value it needs, for an exe at this path.
pub fn entries(exe: &str) -> Vec<(String, String)> {
    let extension = project::EXTENSION;
    vec![
        (format!(r"{CLASSES}\.{extension}"), FILE_TYPE.to_string()),
        (format!(r"{CLASSES}\{FILE_TYPE}"), "Kaye Project".to_string()),
        (format!(r"{CLASSES}\{FILE_TYPE}\DefaultIcon"), format!(r#""{exe}",0"#)),
        (format!(r"{CLASSES}\{FILE_TYPE}\shell\open\command"), format!(r#""{exe}" "%1""#)),
    ]
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn read(key: &str) -> Option<String> {
    let key = wide(key);
    let mut text = [0u16; 2048];
    let mut bytes = size_of_val(&text) as u32;
    // SAFETY: the key is null-terminated, and `bytes` is the size of the buffer given.
    let result = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), null(), RRF_RT_REG_SZ, null_mut(), text.as_mut_ptr().cast(), &mut bytes) };
    if result != ERROR_SUCCESS {
        return None;
    }
    // The count includes the null on the end.
    let length = (bytes as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&text[..length]))
}

fn write(key: &str, value: &str) -> bool {
    let (key, value) = (wide(key), wide(value));
    let mut handle = null_mut();
    // SAFETY: both strings are null-terminated and outlive the calls, and `handle` is
    // only used and closed once the first call says it's valid.
    unsafe {
        let created = RegCreateKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, null(), &mut handle, null_mut());
        if created != ERROR_SUCCESS {
            return false;
        }
        let bytes = (value.len() * 2) as u32;
        let written = RegSetValueExW(handle, null(), 0, REG_SZ, value.as_ptr().cast(), bytes);
        RegCloseKey(handle);
        written == ERROR_SUCCESS
    }
}

/// Failing leaves `.kaye` files as they were, which isn't worth stopping a launch over.
pub fn register() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut changed = false;
    for (key, value) in entries(&exe.to_string_lossy()) {
        if read(&key).as_deref() != Some(value.as_str()) {
            changed |= write(&key, &value);
        }
    }
    if changed {
        // SAFETY: this event takes no items, so both can be null.
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, null(), null()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Registry::RegDeleteTreeW;

    /// Works on a key of its own, which it removes again.
    #[test]
    fn values_go_into_the_registry_and_come_back() {
        const TEST_KEY: &str = r"Software\KayeAssociationTest";
        let key = format!(r"{TEST_KEY}\shell\open\command");
        let value = r#""C:\Program Files\Käye\kaye.exe" "%1""#;
        assert_eq!(read(&key), None);
        assert!(write(&key, value));
        let read_back = read(&key);
        let root = wide(TEST_KEY);
        // SAFETY: the name is null-terminated and only this test uses the key.
        let removed = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, root.as_ptr()) };
        assert_eq!(removed, ERROR_SUCCESS);
        assert_eq!(read_back.as_deref(), Some(value));
        assert_eq!(read(&key), None);
    }

    #[test]
    fn a_path_with_spaces_stays_in_one_piece() {
        let entries = entries(r"C:\Program Files\Kaye\kaye.exe");
        assert_eq!(entries[0], (r"Software\Classes\.kaye".to_string(), "Kaye.Project".to_string()));
        let command = entries.iter().find(|(key, _)| key.ends_with(r"shell\open\command")).unwrap();
        assert_eq!(command.1, r#""C:\Program Files\Kaye\kaye.exe" "%1""#);
        let icon = entries.iter().find(|(key, _)| key.ends_with("DefaultIcon")).unwrap();
        assert_eq!(icon.1, r#""C:\Program Files\Kaye\kaye.exe",0"#);
    }
}
