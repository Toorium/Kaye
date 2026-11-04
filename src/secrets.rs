//! Keeps the Roblox API key out of `settings.json`. No interface code.
//!
//! A key in a plain file can be read by anything that copies the folder: a backup, a
//! sync service, or someone zipping up their settings to share a theme. On Windows it
//! lives in Credential Manager instead, which encrypts it under the Windows account and
//! never roams it to another machine. Elsewhere it stays in the settings file.

/// The name the key goes by in Credential Manager.
#[cfg(windows)]
const TARGET: &str = "Kaye/Roblox API key";

/// Whether there's a secure place for the key on this system.
pub fn available() -> bool {
    cfg!(windows)
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The saved key, or `None` when there isn't one or it can't be read.
#[cfg(windows)]
pub fn load_key() -> Option<String> {
    read(TARGET)
}

/// Saves the key, or removes it when it's empty. Returns false if Windows refused.
#[cfg(windows)]
pub fn save_key(key: &str) -> bool {
    write(TARGET, key)
}

#[cfg(windows)]
fn read(target: &str) -> Option<String> {
    use windows_sys::Win32::Security::Credentials::{CREDENTIALW, CRED_TYPE_GENERIC, CredFree, CredReadW};
    let name = wide(target);
    let mut found: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: `name` is null terminated and outlives the call. On success Windows hands
    // back one allocation, read here and then freed with CredFree.
    unsafe {
        if CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut found) == 0 || found.is_null() {
            return None;
        }
        let credential = &*found;
        let bytes = if credential.CredentialBlob.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(credential.CredentialBlob, credential.CredentialBlobSize as usize).to_vec()
        };
        CredFree(found as *const _);
        String::from_utf8(bytes).ok()
    }
}

#[cfg(windows)]
fn write(target: &str, key: &str) -> bool {
    use windows_sys::Win32::Security::Credentials::{CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CredDeleteW, CredWriteW};
    let mut name = wide(target);
    if key.is_empty() {
        // SAFETY: `name` is null terminated and outlives the call.
        let deleted = unsafe { CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0;
        return deleted || read(target).is_none();
    }
    let mut user = wide("Kaye");
    let mut blob = key.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: name.as_mut_ptr(),
        UserName: user.as_mut_ptr(),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        // This machine only: the key isn't copied to other PCs signed into the account.
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    // SAFETY: every pointer in `credential` points into a buffer that outlives the call.
    unsafe { CredWriteW(&credential, 0) != 0 }
}

#[cfg(not(windows))]
pub fn load_key() -> Option<String> {
    None
}

#[cfg(not(windows))]
pub fn save_key(_key: &str) -> bool {
    false
}

#[cfg(all(test, windows))]
mod tests {
    use super::{read, write};

    /// Under a name of its own, so the real key is never touched, and removed after.
    #[test]
    fn a_key_goes_in_comes_back_and_goes_away() {
        let target = "Kaye/Test probe";
        assert!(write(target, "first-key"));
        assert!(write(target, "second-key"), "saving again replaces it");
        assert_eq!(read(target).as_deref(), Some("second-key"));
        assert!(write(target, ""));
        assert_eq!(read(target), None);
    }
}
