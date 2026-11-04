//! Dragging rigs, cameras, shapes and folders out of the Workspace and into Roblox Studio.
//! Past the edge of the window it becomes a real file drag, of one `.rbxm` holding
//! everything picked up, each rig as its model with every animation in its AnimSaves.
//! Studio inserts it where it's dropped, and dropping it in Explorer saves the file.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::side::Dragged;
use super::{KayeApp, ToastKind};
use crate::dragdrop;
use crate::export::{self, Item};

/// What `export::items_file` gives back: the file, and anything left out of it.
type Made = Result<(Vec<u8>, Vec<String>), String>;

/// Things on their way out of the window.
#[derive(Default)]
pub struct DragOut {
    /// The file, made on another thread from when the rows are picked up, so it's usually
    /// ready by the time the mouse reaches the edge.
    making: Option<(Vec<Item>, Receiver<Made>)>,
    /// Where the drag left the window. Windows kept the mouse until it was let go, so
    /// egui never heard the button come up, and has to be told.
    released_at: Option<egui::Pos2>,
}

/// Starts on the file as soon as the rows are picked up, since baking a long animation
/// takes a moment.
pub fn prepare(app: &mut KayeApp, items: &[Item]) {
    app.drag_out.making = None;
    if items.is_empty() {
        return;
    }
    app.animation.store_range();
    let (scene, animation, wanted) = (app.scene.clone(), app.animation.clone(), items.to_vec());
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(export::items_file(&scene, &animation, &wanted));
    });
    app.drag_out.making = Some((items.to_vec(), rx));
}

/// Hands the drag to Windows once something being dragged in the Workspace leaves the window.
pub fn check(app: &mut KayeApp, ctx: &egui::Context, frame: &eframe::Frame) {
    let Some(Dragged(items)) = egui::DragAndDrop::payload::<Dragged>(ctx).map(|d| (*d).clone()) else {
        app.drag_out.making = None;
        return;
    };
    let (down, pointer) = ctx.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos()));
    let window = ctx.viewport_rect();
    if !down || pointer.is_some_and(|p| window.contains(p)) {
        return;
    }
    egui::DragAndDrop::clear_payload(ctx);
    let name = file_name(app, &items);
    let Some((_, made)) = app.drag_out.making.take().filter(|(made_for, _)| *made_for == items) else {
        return;
    };
    let (bytes, left_out) = match made.recv() {
        Ok(Ok(made)) => made,
        Ok(Err(problem)) => {
            app.toast(ToastKind::Error, format!("{name} couldn't be dragged out: {problem}"));
            return;
        }
        Err(_) => return,
    };
    if !left_out.is_empty() {
        app.toast(ToastKind::Warning, format!("Left out of the drag: {}", left_out.join(" ")));
    }
    let dragged = drag_file_path(&name).and_then(|path| {
        std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
        dragdrop::drag_file(window_handle(frame).ok_or("Kaye's window can't start a drag.")?, &path)
    });
    if let Err(problem) = dragged {
        app.toast(ToastKind::Error, format!("{name} couldn't be dragged out: {problem}"));
    }
    app.drag_out.released_at = Some(pointer.unwrap_or(window.right_center() + egui::vec2(50.0, 0.0)));
    ctx.request_repaint();
}

/// The file's name, which Windows shows while it's dragged and Explorer saves it under:
/// what's being dragged, or the first of them and how many more.
fn file_name(app: &KayeApp, items: &[Item]) -> String {
    let name = |item: &Item| match *item {
        Item::Rig(r) => app.scene.rigs[r].name.clone(),
        Item::Folder(f) => app.scene.folders[f].name.clone(),
    };
    match items {
        [only] => name(only),
        [first, rest @ ..] => format!("{} and {} more", name(first), rest.len()),
        [] => "Kaye".to_string(),
    }
}

/// Tells egui the button came up, after a drag that ended outside the window.
pub fn finish(app: &mut KayeApp, raw: &mut egui::RawInput) {
    if let Some(at) = app.drag_out.released_at.take() {
        raw.events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::default() });
        raw.events.push(egui::Event::PointerGone);
    }
}

/// Whether a dropped file is one of Kaye's own drags, which shouldn't be imported again
/// when it's let go over Kaye.
pub fn is_drag_file(path: &std::path::Path) -> bool {
    drag_folder().is_some_and(|folder| path.starts_with(folder))
}

fn drag_folder() -> Option<PathBuf> {
    Some(std::env::temp_dir().join("Kaye").join("Drag"))
}

/// Where the file goes, named so Windows can show it.
fn drag_file_path(name: &str) -> Result<PathBuf, String> {
    let folder = drag_folder().ok_or("There's no folder for temporary files.")?;
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let safe: String = name.chars().filter(|c| !c.is_control() && !r#"<>:"/\|?*"#.contains(*c)).collect();
    let safe = safe.trim().trim_end_matches('.');
    let stem = if safe.is_empty() { "Rig" } else { safe };
    Ok(folder.join(format!("{stem}.{}", export::EXTENSION)))
}

fn window_handle(frame: &eframe::Frame) -> Option<isize> {
    match frame.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}
