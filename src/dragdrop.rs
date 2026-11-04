//! Dragging a file out of Kaye's window into another program, the way Explorer does, so a
//! rig can be dropped straight into Roblox Studio. No interface code.
//!
//! Windows does the dragging itself. The shell builds the file's data from its path, and
//! with no drop source of Kaye's own it supplies the usual one, so Kaye has no COM objects
//! to write. The drag runs its own message loop until the mouse is let go.

use std::path::Path;

/// How a drag out of the window ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dragged {
    /// Something took the file.
    Dropped,
    /// Let go over nothing that wanted it, or Esc.
    Cancelled,
}

/// Drags `path` from the window with this handle until the mouse is let go. The left
/// button has to be down when this starts, or Windows drops it on the spot.
#[cfg(windows)]
pub fn drag_file(window: isize, path: &Path) -> Result<Dragged, String> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::Ole::DROPEFFECT_COPY;
    use windows_sys::Win32::UI::Shell::{ILClone, ILCreateFromPathW, ILFindLastID, ILFree, ILRemoveLastID, SHCreateDataObject, SHDoDragDrop};
    use windows_sys::core::GUID;

    const IID_IDATAOBJECT: GUID = GUID::from_u128(0x0000010e_0000_0000_c000_000000000046);
    const DRAGDROP_S_DROP: i32 = 0x0004_0100;

    /// Every COM object starts with IUnknown's three functions.
    #[repr(C)]
    struct Unknown {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
    }

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // Safety: every item list is checked before use and freed once, and the data object is
    // released once, after the drag is over.
    unsafe {
        let full = ILCreateFromPathW(wide.as_ptr());
        if full.is_null() {
            return Err("Windows couldn't find the file to drag.".to_string());
        }
        // The shell wants the folder, then the file inside it.
        let file = ILClone(ILFindLastID(full));
        let folder = ILClone(full);
        ILRemoveLastID(folder);
        let mut data: *mut c_void = std::ptr::null_mut();
        let files = [file as *const _];
        let made = SHCreateDataObject(folder, 1, files.as_ptr(), std::ptr::null_mut(), &IID_IDATAOBJECT, &mut data);
        let result = if made >= 0 && !data.is_null() {
            let mut effect = 0;
            let ended = SHDoDragDrop(window as HWND, data, std::ptr::null_mut(), DROPEFFECT_COPY, &mut effect);
            let vtable = *(data as *const *const Unknown);
            ((*vtable).release)(data);
            Ok(if ended == DRAGDROP_S_DROP && effect != 0 { Dragged::Dropped } else { Dragged::Cancelled })
        } else {
            Err(format!("Windows couldn't start the drag ({made:#x})."))
        };
        ILFree(file);
        ILFree(folder);
        ILFree(full);
        result
    }
}

#[cfg(not(windows))]
pub fn drag_file(_window: isize, _path: &Path) -> Result<Dragged, String> {
    Err("Dragging out of Kaye only works on Windows so far.".to_string())
}
