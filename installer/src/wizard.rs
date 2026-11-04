//! The installer's window: a classic Windows setup wizard. It's made of real Windows controls,
//! so it looks at home on any version of Windows and needs no graphics card to open.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::mpsc::{Receiver, channel};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BF_TOP, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, COLOR_BTNFACE, COLOR_GRAYTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT, CreateFontIndirectW, CreateSolidBrush, DIB_RGB_COLORS, DT_CALCRECT, DT_LEFT, DT_NOPREFIX, DT_WORDBREAK, DrawEdge, DrawTextW, EDGE_ETCHED, EndPaint, FillRect, GetSysColor, GetSysColorBrush,
    HALFTONE, HBRUSH, HDC, HFONT, PAINTSTRUCT, RDW_ALLCHILDREN, RDW_ERASE, RDW_INVALIDATE, RedrawWindow, SRCCOPY, SelectObject, SetBkColor, SetBkMode, SetStretchBltMode, SetTextColor, StretchDIBits, TRANSPARENT,
};
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{ICC_PROGRESS_CLASS, ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, PBM_SETPOS, PBM_SETRANGE32, PROGRESS_CLASSW};
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows_sys::Win32::UI::Shell::{BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, SHBrowseForFolderW, SHGetPathFromIDListW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_GROUPBOX, BS_PUSHBUTTON, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DI_NORMAL, DefWindowProcW, DestroyWindow, DispatchMessageW, DrawIconEx, EN_CHANGE, ES_AUTOHSCROLL, ES_READONLY, GWLP_USERDATA, GetMessageW, GetSystemMetrics, GetWindowLongPtrW,
    GetWindowTextW, HICON, IDC_ARROW, IDCANCEL, IDI_WARNING, IDOK, IDYES, IMAGE_ICON, IsDialogMessageW, LR_DEFAULTCOLOR, LoadCursorW, LoadIconW, LoadImageW, MB_DEFBUTTON2, MB_ICONERROR, MB_ICONQUESTION, MB_OK, MB_YESNO, MSG, MessageBoxW, NONCLIENTMETRICSW, PostMessageW, PostQuitMessage, RegisterClassExW, SM_CXSCREEN, SM_CYSCREEN,
    SPI_GETNONCLIENTMETRICS, SW_HIDE, SW_SHOW, SW_SHOWNOACTIVATE, SendMessageW, SetWindowLongPtrW, SetWindowTextW, ShowWindow, SystemParametersInfoW, TranslateMessage, WM_APP, WM_CLOSE, WM_COMMAND, WM_CTLCOLORSTATIC, WM_DESTROY, WM_PAINT, WM_SETFONT, WNDCLASSEXW, WS_BORDER, WS_CAPTION, WS_CHILD,
    WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};

use crate::install::{self, PROGRAM};
use crate::pack::Payload;
use crate::system::wide;

// Sizes at 100% display scaling, which `px` turns into real pixels.
const WIDTH: i32 = 500;
const HEIGHT: i32 = 360;
const HEADER_HEIGHT: i32 = 58;
/// The line above the buttons.
const LINE_Y: i32 = 312;
const BANNER_WIDTH: i32 = 164;
const MARK_DRAWN: i32 = 120;
const BUTTON_WIDTH: i32 = 75;
const BUTTON_HEIGHT: i32 = 23;
const BUTTON_Y: i32 = 325;
const MARGIN: i32 = 24;
const HEADER_ICON: i32 = 48;
const WARNING_ICON: i32 = 32;

/// Keep these the same as in `build.rs`.
const MARK_SIZE: i32 = 240;
const BANNER_COLOR: u32 = 0x70 << 16 | 0x6B << 8 | 0xF3;
static MARK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/mark.bgra"));

const ID_BACK: i32 = 3;
const ID_BROWSE: i32 = 4;
const ID_FOLDER: i32 = 5;
const ID_DESKTOP: i32 = 6;
const ID_RUN: i32 = 7;
/// The thread doing the work has something to say.
const WM_WORKER: u32 = WM_APP + 1;
const PROGRESS_STEPS: f32 = 1000.0;
const CHECKED: usize = 1;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const TEST_BUILD_TEXT: &str = "This is a test build, so things can break.\n\nIf you're seeing this, you're probably here to find out what broke.";

pub enum Mode {
    Install { setup: PathBuf, payload: Payload },
    Uninstall { folder: PathBuf },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Welcome,
    Folder,
    ConfirmRemoval,
    Working,
    Finished,
}

impl Page {
    /// The first and last pages have Kaye's color down the side, and the rest a white header.
    fn has_banner(self) -> bool {
        matches!(self, Page::Welcome | Page::Finished)
    }
}

/// What the thread doing the work has to say.
enum Update {
    Step(f32, &'static str),
    Finished(Result<(), String>),
}

struct Fonts {
    normal: HFONT,
    bold: HFONT,
    title: HFONT,
}

/// Every control, made once and shown or hidden as the pages change.
struct Controls {
    back: HWND,
    next: HWND,
    cancel: HWND,
    info: HWND,
    note: HWND,
    group: HWND,
    folder: HWND,
    browse: HWND,
    desktop: HWND,
    required: HWND,
    available: HWND,
    status: HWND,
    bar: HWND,
    run: HWND,
}

struct Wizard {
    window: HWND,
    mode: Mode,
    title: &'static str,
    dpi: i32,
    fonts: Fonts,
    controls: Controls,
    logo: HICON,
    warning: HICON,
    banner: HBRUSH,
    page: Cell<Page>,
    updates: RefCell<Option<Receiver<Update>>>,
    removed: Cell<bool>,
}

fn scaled(value: i32, dpi: i32) -> i32 {
    value * dpi / 96
}

fn set_text(window: HWND, text: &str) {
    let text = wide(text);
    // SAFETY: the text is null-terminated, and Windows copies it.
    unsafe { SetWindowTextW(window, text.as_ptr()) };
}

fn set_shown(window: HWND, shown: bool) {
    // SAFETY: any window handle can be asked, and a stale one only fails.
    unsafe { ShowWindow(window, if shown { SW_SHOW } else { SW_HIDE }) };
}

fn set_enabled(window: HWND, enabled: bool) {
    // SAFETY: as for `set_shown`.
    unsafe { EnableWindow(window, enabled as i32) };
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT { left, top, right, bottom }
}

/// A picked folder gets a Kaye folder of its own, unless that's what it already is.
fn with_own_folder(picked: PathBuf) -> PathBuf {
    if picked.file_name().is_some_and(|name| name.eq_ignore_ascii_case("Kaye")) { picked } else { picked.join("Kaye") }
}

fn size_text(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let megabytes = bytes as f64 / MB;
    if megabytes < 1024.0 { format!("{megabytes:.1} MB") } else { format!("{:.1} GB", megabytes / 1024.0) }
}

/// How much room is left where a folder is, or would be.
fn free_space(folder: &Path) -> Option<u64> {
    let existing = folder.ancestors().find(|path| path.exists())?;
    let path = wide(&existing.to_string_lossy());
    let mut free = 0u64;
    // SAFETY: the path is null-terminated, and the other two answers can be left out.
    let found = unsafe { GetDiskFreeSpaceExW(path.as_ptr(), &mut free, null_mut(), null_mut()) };
    (found != 0).then_some(free)
}

impl Wizard {
    fn px(&self, value: i32) -> i32 {
        scaled(value, self.dpi)
    }

    fn removing(&self) -> bool {
        matches!(self.mode, Mode::Uninstall { .. })
    }

    fn message(&self, text: &str, flags: u32) -> i32 {
        let (text, title) = (wide(text), wide(self.title));
        // SAFETY: both strings are null-terminated and outlive the call.
        unsafe { MessageBoxW(self.window, text.as_ptr(), title.as_ptr(), flags) }
    }

    fn folder_text(&self) -> String {
        let mut text = [0u16; 1024];
        // SAFETY: the length given is the buffer's own.
        let length = unsafe { GetWindowTextW(self.controls.folder, text.as_mut_ptr(), text.len() as i32) };
        String::from_utf16_lossy(&text[..length.max(0) as usize]).trim().to_string()
    }

    fn is_ticked(&self, checkbox: HWND) -> bool {
        // SAFETY: this message only reads the checkbox.
        unsafe { SendMessageW(checkbox, BM_GETCHECK, 0, 0) as usize == CHECKED }
    }

    /// The text that depends on which folder is typed in.
    fn folder_changed(&self) {
        let folder = PathBuf::from(self.folder_text());
        let info = if install::is_installed(&folder) {
            "Kaye is already installed in this folder. Setup will replace it and keep your settings."
        } else {
            "Setup will install Kaye in the folder below. To use a different folder, click Browse and pick another one."
        };
        set_text(self.controls.info, info);
        let available = free_space(&folder).map_or_else(String::new, |free| format!("Space available: {}", size_text(free)));
        set_text(self.controls.available, &available);
    }

    fn show(&self, page: Page) {
        self.page.set(page);
        let c = &self.controls;
        let removing = self.removing();
        let on_folder = page == Page::Folder;
        let on_confirm = page == Page::ConfirmRemoval;
        let working = page == Page::Working;
        set_shown(c.info, on_folder || on_confirm);
        set_shown(c.group, on_folder || on_confirm);
        set_shown(c.folder, on_folder || on_confirm);
        set_shown(c.note, on_confirm);
        set_shown(c.browse, on_folder);
        set_shown(c.desktop, on_folder);
        set_shown(c.required, on_folder);
        set_shown(c.available, on_folder);
        set_shown(c.status, working);
        set_shown(c.bar, working);
        set_shown(c.run, page == Page::Finished && !removing);
        set_enabled(c.back, on_folder);
        set_enabled(c.next, !working);
        set_enabled(c.cancel, !working && page != Page::Finished);
        set_text(
            c.next,
            match page {
                Page::Welcome | Page::Working => "Next >",
                Page::Folder => "Install",
                Page::ConfirmRemoval => "Uninstall",
                Page::Finished => "Finish",
            },
        );
        if on_folder {
            self.folder_changed();
        }
        // The controls go too, since painting a page draws over wherever they are. A group box
        // has no fill of its own, so it can't be left out of the window's painting.
        // SAFETY: the window is alive, and leaving the area out means all of it.
        unsafe {
            SetFocus(c.next);
            RedrawWindow(self.window, null(), null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
        }
    }

    /// Runs `work` on another thread, so the window keeps drawing.
    fn start(&self, work: impl FnOnce(install::Report) -> Result<(), String> + Send + 'static) {
        let (sender, receiver) = channel();
        *self.updates.borrow_mut() = Some(receiver);
        // SAFETY: this only sets a number on the bar.
        unsafe { SendMessageW(self.controls.bar, PBM_SETPOS, 0, 0) };
        self.show(Page::Working);
        let window = self.window as isize;
        std::thread::spawn(move || {
            // SAFETY: posting to a window that's gone only fails, and the window can't close
            // while this is running anyway.
            let wake = move || unsafe {
                PostMessageW(window as HWND, WM_WORKER, 0, 0);
            };
            let steps = sender.clone();
            let outcome = work(&mut move |share, step| {
                let _ = steps.send(Update::Step(share, step));
                wake();
                // Real installs are over too fast for a test run to see this page.
                if cfg!(feature = "devtools") {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            });
            let _ = sender.send(Update::Finished(outcome));
            wake();
        });
    }

    fn poll(&self) {
        loop {
            let update = self.updates.borrow().as_ref().and_then(|updates| updates.try_recv().ok());
            match update {
                None => return,
                Some(Update::Step(share, step)) => {
                    // SAFETY: this only sets a number on the bar.
                    unsafe { SendMessageW(self.controls.bar, PBM_SETPOS, (share * PROGRESS_STEPS) as usize, 0) };
                    set_text(self.controls.status, step);
                }
                Some(Update::Finished(outcome)) => {
                    *self.updates.borrow_mut() = None;
                    match outcome {
                        Ok(()) => {
                            self.removed.set(self.removing());
                            self.show(Page::Finished);
                        }
                        Err(problem) => {
                            self.show(if self.removing() { Page::ConfirmRemoval } else { Page::Folder });
                            self.message(&problem, MB_OK | MB_ICONERROR);
                        }
                    }
                    return;
                }
            }
        }
    }

    fn start_install(&self) {
        let Mode::Install { setup, payload } = &self.mode else {
            return;
        };
        let (setup, payload) = (setup.clone(), *payload);
        let folder = PathBuf::from(self.folder_text());
        if !folder.is_absolute() {
            self.message("Enter a full path, like C:\\Kaye.", MB_OK | MB_ICONERROR);
            return;
        }
        let Some(places) = install::real_places(folder) else {
            self.message("Windows wouldn't say where shortcuts go.", MB_OK | MB_ICONERROR);
            return;
        };
        let desktop_shortcut = self.is_ticked(self.controls.desktop);
        self.start(move |report| install::install(&setup, &payload, &places, desktop_shortcut, report));
    }

    fn start_removal(&self) {
        let Mode::Uninstall { folder } = &self.mode else {
            return;
        };
        let Some(places) = install::real_places(folder.clone()) else {
            self.message("Windows wouldn't say where shortcuts go.", MB_OK | MB_ICONERROR);
            return;
        };
        self.start(move |report| install::uninstall(&places, report));
    }

    fn browse(&self) {
        let title = wide("Select the folder to install Kaye in:");
        // SAFETY: an all-zero BROWSEINFOW means no extras. The title outlives the call, the
        // path buffer is the size Windows asks for, and the list it returns is freed here.
        unsafe {
            let mut info: BROWSEINFOW = std::mem::zeroed();
            info.hwndOwner = self.window;
            info.lpszTitle = title.as_ptr();
            info.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE;
            let list = SHBrowseForFolderW(&info);
            if list.is_null() {
                return;
            }
            let mut path = [0u16; 260];
            let found = SHGetPathFromIDListW(list, path.as_mut_ptr());
            CoTaskMemFree(list as *const c_void);
            if found != 0 {
                let length = path.iter().position(|c| *c == 0).unwrap_or(path.len());
                let picked = PathBuf::from(String::from_utf16_lossy(&path[..length]));
                set_text(self.controls.folder, &with_own_folder(picked).to_string_lossy());
            }
        }
    }

    fn next(&self) {
        match self.page.get() {
            Page::Welcome => self.show(Page::Folder),
            Page::Folder => self.start_install(),
            Page::ConfirmRemoval => self.start_removal(),
            Page::Working => {}
            Page::Finished => {
                if !self.removing() && self.is_ticked(self.controls.run) {
                    let _ = std::process::Command::new(Path::new(&self.folder_text()).join(PROGRAM)).spawn();
                }
                // SAFETY: this is the wizard's own window, closed from its own thread.
                unsafe { DestroyWindow(self.window) };
            }
        }
    }

    /// Cancel and the window's X both end up here.
    fn cancel(&self) {
        let page = self.page.get();
        if page == Page::Working {
            return;
        }
        let question = format!("Are you sure you want to quit {}?", self.title);
        if page == Page::Finished || self.message(&question, MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) == IDYES {
            // SAFETY: this is the wizard's own window, closed from its own thread.
            unsafe { DestroyWindow(self.window) };
        }
    }

    fn command(&self, id: i32, notification: u32) {
        match id {
            IDOK => self.next(),
            IDCANCEL => self.cancel(),
            ID_BACK => self.show(Page::Welcome),
            ID_BROWSE => self.browse(),
            ID_FOLDER if notification == EN_CHANGE && self.page.get() == Page::Folder => self.folder_changed(),
            _ => {}
        }
    }

    /// Writes a paragraph inside `area` and moves `area` down past it.
    fn paragraph(&self, hdc: HDC, text: &str, font: HFONT, area: &mut RECT, gap: i32) {
        let mut text = wide(text);
        // SAFETY: the text is null-terminated, as a length of -1 says it is, and both calls
        // only write inside the rectangles given.
        unsafe {
            SelectObject(hdc, font as *mut c_void);
            let mut measured = *area;
            DrawTextW(hdc, text.as_mut_ptr(), -1, &mut measured, DT_WORDBREAK | DT_LEFT | DT_NOPREFIX | DT_CALCRECT);
            DrawTextW(hdc, text.as_mut_ptr(), -1, area, DT_WORDBREAK | DT_LEFT | DT_NOPREFIX);
            area.top = measured.bottom + self.px(gap);
        }
    }

    fn paint_banner_page(&self, hdc: HDC) {
        let (width, line, banner) = (self.px(WIDTH), self.px(LINE_Y), self.px(BANNER_WIDTH));
        let drawn = self.px(MARK_DRAWN);
        // SAFETY: the brushes are alive, and the mark holds `MARK_SIZE` by `MARK_SIZE` pixels
        // of four bytes each, as the header says.
        unsafe {
            FillRect(hdc, &rect(0, 0, width, line), GetSysColorBrush(COLOR_WINDOW));
            FillRect(hdc, &rect(0, 0, banner, line), self.banner);
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: MARK_SIZE, biHeight: -MARK_SIZE, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB, ..std::mem::zeroed() };
            SetStretchBltMode(hdc, HALFTONE);
            StretchDIBits(hdc, (banner - drawn) / 2, (line - drawn) / 2, drawn, drawn, 0, 0, MARK_SIZE, MARK_SIZE, MARK.as_ptr().cast(), &info, DIB_RGB_COLORS, SRCCOPY);
        }
        let mut area = rect(banner + self.px(16), self.px(18), width - self.px(16), line - self.px(8));
        let removing = self.removing();
        match (self.page.get(), removing) {
            (Page::Welcome, _) => {
                self.paragraph(hdc, "Welcome to the Kaye Setup Wizard", self.fonts.title, &mut area, 14);
                self.paragraph(hdc, &format!("This will install Kaye {VERSION} on your computer."), self.fonts.normal, &mut area, 14);
                let icon = self.px(WARNING_ICON);
                // SAFETY: the icon is one Windows owns, and stays alive.
                unsafe { DrawIconEx(hdc, area.left, area.top, self.warning, icon, icon, 0, null_mut(), DI_NORMAL) };
                let mut warning = rect(area.left + icon + self.px(10), area.top, area.right, area.bottom);
                self.paragraph(hdc, TEST_BUILD_TEXT, self.fonts.normal, &mut warning, 14);
                area.top = warning.top;
                self.paragraph(hdc, "Click Next to continue, or Cancel to exit Setup.", self.fonts.normal, &mut area, 0);
            }
            (_, false) => {
                self.paragraph(hdc, "Completing the Kaye Setup Wizard", self.fonts.title, &mut area, 14);
                self.paragraph(hdc, "Kaye has been installed on your computer.", self.fonts.normal, &mut area, 10);
                self.paragraph(hdc, "Click Finish to close this wizard.", self.fonts.normal, &mut area, 0);
            }
            (_, true) => {
                self.paragraph(hdc, "Completing the Kaye Uninstall Wizard", self.fonts.title, &mut area, 14);
                self.paragraph(hdc, "Kaye has been removed from your computer.", self.fonts.normal, &mut area, 10);
                self.paragraph(hdc, "Click Finish to close this wizard.", self.fonts.normal, &mut area, 0);
            }
        }
    }

    fn paint_header_page(&self, hdc: HDC) {
        let (width, header) = (self.px(WIDTH), self.px(HEADER_HEIGHT));
        let icon = self.px(HEADER_ICON);
        // SAFETY: the brush is one Windows owns, and the icon stays alive with the wizard.
        unsafe {
            FillRect(hdc, &rect(0, 0, width, header), GetSysColorBrush(COLOR_WINDOW));
            DrawIconEx(hdc, width - icon - self.px(6), (header - icon) / 2, self.logo, icon, icon, 0, null_mut(), DI_NORMAL);
        }
        let (title, subtitle) = match (self.page.get(), self.removing()) {
            (Page::Folder, _) => ("Choose Install Location", "Choose the folder Kaye will be installed in."),
            (Page::ConfirmRemoval, _) => ("Uninstall Kaye", "Remove Kaye from your computer."),
            (_, false) => ("Installing", "Please wait while Kaye is being installed."),
            (_, true) => ("Uninstalling", "Please wait while Kaye is being removed."),
        };
        let mut area = rect(self.px(16), self.px(10), width - icon - self.px(16), header);
        self.paragraph(hdc, title, self.fonts.bold, &mut area, 3);
        area.left = self.px(28);
        self.paragraph(hdc, subtitle, self.fonts.normal, &mut area, 0);
        let mut edge = rect(0, header, width, header + 2);
        // SAFETY: the rectangle lives for the call.
        unsafe { DrawEdge(hdc, &mut edge, EDGE_ETCHED, BF_TOP) };
    }

    fn paint(&self) {
        // SAFETY: an all-zero PAINTSTRUCT is what BeginPaint fills in, and each BeginPaint
        // is matched by this EndPaint.
        unsafe {
            let mut paint: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(self.window, &mut paint);
            SetBkMode(hdc, TRANSPARENT as i32);
            SetTextColor(hdc, GetSysColor(COLOR_WINDOWTEXT));
            if self.page.get().has_banner() {
                self.paint_banner_page(hdc);
            } else {
                self.paint_header_page(hdc);
            }
            let width = self.px(WIDTH);
            let mut edge = rect(0, self.px(LINE_Y), width, self.px(LINE_Y) + 2);
            DrawEdge(hdc, &mut edge, EDGE_ETCHED, BF_TOP);
            SetTextColor(hdc, GetSysColor(COLOR_GRAYTEXT));
            let mut brand = rect(self.px(12), self.px(BUTTON_Y) + self.px(4), self.px(240), self.px(BUTTON_Y + BUTTON_HEIGHT));
            let text = format!("Kaye {VERSION} test build");
            self.paragraph(hdc, &text, self.fonts.normal, &mut brand, 0);
            EndPaint(self.window, &paint);
        }
    }

    /// Controls sitting on a white page need telling, or they bring a grey box with them.
    fn control_color(&self, hdc: HDC, control: HWND) -> Option<LRESULT> {
        if control != self.controls.run {
            return None;
        }
        // SAFETY: the device context is the one Windows handed over for this.
        unsafe {
            SetBkColor(hdc, GetSysColor(COLOR_WINDOW));
            Some(GetSysColorBrush(COLOR_WINDOW) as LRESULT)
        }
    }

    fn handle(&self, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match message {
            WM_PAINT => self.paint(),
            WM_COMMAND => self.command((wparam & 0xFFFF) as i32, (wparam >> 16) as u32 & 0xFFFF),
            WM_CTLCOLORSTATIC => return self.control_color(wparam as HDC, lparam as HWND),
            WM_WORKER => self.poll(),
            WM_CLOSE => self.cancel(),
            // SAFETY: this only tells the message loop to stop.
            WM_DESTROY => unsafe { PostQuitMessage(0) },
            _ => return None,
        }
        Some(0)
    }
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: the pointer is the wizard `run` keeps alive until this window is gone, or null
    // before it's been set. It's only shared, since a message box can bring another message
    // here while one's being handled.
    unsafe {
        let wizard = GetWindowLongPtrW(window, GWLP_USERDATA) as *const Wizard;
        if let Some(handled) = wizard.as_ref().and_then(|wizard| wizard.handle(message, wparam, lparam)) {
            return handled;
        }
        DefWindowProcW(window, message, wparam, lparam)
    }
}

fn message_fonts() -> Fonts {
    // SAFETY: the size field says which version of the structure this is, and each font
    // is made from a filled-in description.
    unsafe {
        let mut metrics: NONCLIENTMETRICSW = std::mem::zeroed();
        metrics.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
        SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, metrics.cbSize, (&raw mut metrics).cast(), 0);
        let normal = metrics.lfMessageFont;
        let bold = windows_sys::Win32::Graphics::Gdi::LOGFONTW { lfWeight: 700, ..normal };
        let title = windows_sys::Win32::Graphics::Gdi::LOGFONTW { lfWeight: 700, lfHeight: normal.lfHeight * 3 / 2, ..normal };
        Fonts { normal: CreateFontIndirectW(&normal), bold: CreateFontIndirectW(&bold), title: CreateFontIndirectW(&title) }
    }
}

/// Places for the controls, all at 100% display scaling.
struct Maker {
    window: HWND,
    instance: *mut c_void,
    dpi: i32,
    font: HFONT,
}

impl Maker {
    fn make(&self, class: *const u16, text: &str, style: u32, id: i32, [x, y, width, height]: [i32; 4]) -> HWND {
        let text = wide(text);
        let scale = |value| scaled(value, self.dpi);
        // SAFETY: the strings are null-terminated, the parent is alive, and a child window's
        // menu slot holds its id.
        unsafe {
            let control = CreateWindowExW(0, class, text.as_ptr(), WS_CHILD | style, scale(x), scale(y), scale(width), scale(height), self.window, id as isize as *mut c_void, self.instance, null());
            SendMessageW(control, WM_SETFONT, self.font as usize, 1);
            control
        }
    }

    fn button(&self, text: &str, style: u32, id: i32, place: [i32; 4]) -> HWND {
        self.make(wide("BUTTON").as_ptr(), text, style, id, place)
    }

    fn label(&self, text: &str, place: [i32; 4]) -> HWND {
        self.make(wide("STATIC").as_ptr(), text, 0, 0, place)
    }
}

fn make_controls(maker: &Maker, mode: &Mode, suggested: &Path) -> Controls {
    let cancel_x = WIDTH - 12 - BUTTON_WIDTH;
    let next_x = cancel_x - 11 - BUTTON_WIDTH;
    let back_x = next_x - BUTTON_WIDTH;
    let wide_label = WIDTH - MARGIN * 2;
    let (folder, editable) = match mode {
        Mode::Install { .. } => (suggested.to_path_buf(), true),
        Mode::Uninstall { folder } => (folder.clone(), false),
    };
    let edit_style = WS_TABSTOP | ES_AUTOHSCROLL as u32 | if editable { 0 } else { ES_READONLY as u32 };
    let edit_width = if editable { 340 } else { 428 };
    let group_caption = if editable { "Destination Folder" } else { "Uninstalling From" };
    let info = if editable { "" } else { "Kaye will be removed from the folder below. Click Uninstall to start." };
    let edit = maker.make(wide("EDIT").as_ptr(), &folder.to_string_lossy(), edit_style | WS_BORDER, ID_FOLDER, [36, 142, edit_width, 22]);
    let controls = Controls {
        back: maker.button("< Back", WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32, ID_BACK, [back_x, BUTTON_Y, BUTTON_WIDTH, BUTTON_HEIGHT]),
        next: maker.button("Next >", WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON as u32, IDOK, [next_x, BUTTON_Y, BUTTON_WIDTH, BUTTON_HEIGHT]),
        cancel: maker.button("Cancel", WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32, IDCANCEL, [cancel_x, BUTTON_Y, BUTTON_WIDTH, BUTTON_HEIGHT]),
        info: maker.label(info, [MARGIN, 72, wide_label, 40]),
        note: maker.label("Your projects stay where you saved them. Your settings, API key and backups stay in your user folder.", [MARGIN, 190, wide_label, 40]),
        group: maker.button(group_caption, BS_GROUPBOX as u32, 0, [MARGIN, 120, wide_label, 56]),
        folder: edit,
        browse: maker.button("Browse...", WS_TABSTOP | BS_PUSHBUTTON as u32, ID_BROWSE, [384, 141, 80, BUTTON_HEIGHT]),
        desktop: maker.button("Create a desktop shortcut", WS_TABSTOP | BS_AUTOCHECKBOX as u32, ID_DESKTOP, [MARGIN, 186, wide_label, 20]),
        required: maker.label("", [MARGIN, 252, wide_label, 16]),
        available: maker.label("", [MARGIN, 270, wide_label, 16]),
        status: maker.label("", [MARGIN, 96, wide_label, 18]),
        bar: maker.make(PROGRESS_CLASSW, "", 0, 0, [MARGIN, 120, wide_label, 20]),
        run: maker.button("Run Kaye", WS_TABSTOP | BS_AUTOCHECKBOX as u32, ID_RUN, [BANNER_WIDTH + 16, 150, 300, 20]),
    };
    // SAFETY: these messages only set a tick and a range.
    unsafe {
        SendMessageW(controls.desktop, BM_SETCHECK, CHECKED, 0);
        SendMessageW(controls.run, BM_SETCHECK, CHECKED, 0);
        SendMessageW(controls.bar, PBM_SETRANGE32, 0, PROGRESS_STEPS as isize);
    }
    controls
}

/// Shows the wizard until it's closed. True means Kaye was uninstalled.
pub fn run(mode: Mode, suggested: &Path) -> bool {
    let title = if matches!(mode, Mode::Uninstall { .. }) { "Kaye Uninstall" } else { "Kaye Setup" };
    let required = match &mode {
        Mode::Install { payload, .. } => payload.size + payload.stub,
        Mode::Uninstall { .. } => 0,
    };
    let page = if matches!(mode, Mode::Uninstall { .. }) { Page::ConfirmRemoval } else { Page::Welcome };
    let class = wide("KayeSetupWizard");
    let window_title = wide(title);
    // SAFETY: every string outlives the calls using it. The wizard stays on this stack frame
    // until the message loop has ended, which is after the window's gone, so the pointer
    // kept in the window is good for as long as `window_proc` can see it.
    unsafe {
        CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
        let common = INITCOMMONCONTROLSEX { dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_STANDARD_CLASSES | ICC_PROGRESS_CLASS };
        InitCommonControlsEx(&common);
        let instance = GetModuleHandleW(null());
        // The .exe's own icon, which build.rs put in as number 1.
        let own_icon = 1usize as *const u16;
        let dpi = GetDpiForSystem() as i32;
        let logo = LoadImageW(instance, own_icon, IMAGE_ICON, scaled(HEADER_ICON, dpi), scaled(HEADER_ICON, dpi), LR_DEFAULTCOLOR);
        let class_info = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: LoadIconW(instance, own_icon),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
            hIconSm: LoadIconW(instance, own_icon),
        };
        RegisterClassExW(&class_info);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut frame = rect(0, 0, scaled(WIDTH, dpi), scaled(HEIGHT, dpi));
        AdjustWindowRectEx(&mut frame, style, 0, 0);
        let (width, height) = (frame.right - frame.left, frame.bottom - frame.top);
        let (x, y) = ((GetSystemMetrics(SM_CXSCREEN) - width) / 2, (GetSystemMetrics(SM_CYSCREEN) - height) / 2);
        let window = CreateWindowExW(0, class.as_ptr(), window_title.as_ptr(), style, x, y, width, height, null_mut(), null_mut(), instance, null());

        let fonts = message_fonts();
        let maker = Maker { window, instance, dpi, font: fonts.normal };
        let controls = make_controls(&maker, &mode, suggested);
        if required > 0 {
            set_text(controls.required, &format!("Space required: {}", size_text(required)));
        }
        let wizard = Wizard {
            window,
            mode,
            title,
            dpi,
            fonts,
            controls,
            logo,
            warning: LoadIconW(null_mut(), IDI_WARNING),
            banner: CreateSolidBrush(BANNER_COLOR),
            page: Cell::new(page),
            updates: RefCell::new(None),
            removed: Cell::new(false),
        };
        SetWindowLongPtrW(window, GWLP_USERDATA, (&raw const wizard) as isize);
        // A test run shouldn't take focus from whatever is in front.
        ShowWindow(window, if cfg!(feature = "devtools") { SW_SHOWNOACTIVATE } else { SW_SHOW });
        // After it's showing, since a hidden window can't be repainted.
        wizard.show(page);

        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            // This is what gives Tab, Enter and Escape their usual meanings.
            if IsDialogMessageW(window, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        SetWindowLongPtrW(window, GWLP_USERDATA, 0);
        wizard.removed.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picked_folder_gets_a_kaye_folder_of_its_own() {
        assert_eq!(with_own_folder(PathBuf::from(r"D:\Apps")), PathBuf::from(r"D:\Apps\Kaye"));
        assert_eq!(with_own_folder(PathBuf::from(r"D:\Apps\Kaye")), PathBuf::from(r"D:\Apps\Kaye"));
        assert_eq!(with_own_folder(PathBuf::from(r"D:\Apps\kaye")), PathBuf::from(r"D:\Apps\kaye"));
    }

    #[test]
    fn the_test_build_warning_says_what_it_should() {
        assert!(TEST_BUILD_TEXT.contains("test build") && TEST_BUILD_TEXT.contains("can break") && TEST_BUILD_TEXT.contains("find out what broke"));
        assert!(!TEST_BUILD_TEXT.contains('\u{2014}'), "no em dashes in Kaye's text");
    }

    #[test]
    fn sizes_read_like_an_installer_would_say_them() {
        assert_eq!(size_text(41 * 1024 * 1024), "41.0 MB");
        assert_eq!(size_text(5 * 1024 * 1024 * 1024), "5.0 GB");
    }

    #[test]
    fn free_space_looks_up_a_folder_that_doesnt_exist_yet() {
        let later = std::env::temp_dir().join("kaye-not-made-yet").join("deeper");
        assert!(free_space(&later).is_some_and(|free| free > 0));
    }

    #[test]
    fn the_mark_is_the_size_the_window_expects() {
        assert_eq!(MARK.len(), (MARK_SIZE * MARK_SIZE * 4) as usize);
    }
}
