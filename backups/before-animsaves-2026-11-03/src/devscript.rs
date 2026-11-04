//! Drives the real app from a text file by injecting input events, so the UI can
//! be tested end to end without touching the actual mouse. Only built with the
//! `devtools` feature.
//!
//! Set `KAYE_DEV_SCRIPT` to a file of commands, one per line:
//!   wait <frames>        move <x> <y>       click <x> <y>
//!   down <l|m|r>         up <l|m|r>         key <Name> [ctrl] [shift] [alt]
//!   hold <shift|alt|ctrl|none>              scroll <dy>
//!   drop <file>          shot <file.png>
//!   copy                 paste              what real Ctrl+C and Ctrl+V arrive as
//!   close               asks to close, as the window's X button does
//!   quit                 closes without asking about unsaved work
//!
//! `KAYE_DEV_SAVE_AS` answers the Save As dialog, which a script can't reach, and
//! `KAYE_DEV_EXPORT_AS` the Export Animation one.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, pos2};

/// A file dropped on the window by a script.
#[derive(Debug)]
struct ScriptedFile(PathBuf);

impl egui::DroppedFile for ScriptedFile {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|err| err.to_string())
    }
}

pub struct DevScript {
    lines: VecDeque<String>,
    wait: u32,
    pos: Pos2,
    mods: Modifiers,
    pending_shot: Arc<Mutex<bool>>,
    /// A `quit` is under way, which shouldn't stop to ask about unsaved work.
    pub quitting: bool,
}

impl DevScript {
    pub fn from_env() -> Option<Self> {
        let path = std::env::var("KAYE_DEV_SCRIPT").ok()?;
        let text = std::fs::read_to_string(&path).expect("dev script not readable");
        Some(Self {
            lines: text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_owned)
                .collect(),
            wait: 5,
            pos: pos2(0.0, 0.0),
            mods: Modifiers::NONE,
            pending_shot: Arc::new(Mutex::new(false)),
            quitting: false,
        })
    }

    pub fn step(&mut self, ctx: &egui::Context, raw: &mut RawInput) {
        ctx.request_repaint();
        // Holds for one frame, so a covered window needs asking every time.
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
        if self.wait > 0 {
            self.wait -= 1;
            return;
        }
        if *self.pending_shot.lock().unwrap() {
            return;
        }
        let Some(line) = self.lines.pop_front() else {
            return;
        };
        println!("[script] {line}");
        let parts: Vec<&str> = line.split_whitespace().collect();
        let number = |i: usize| parts.get(i).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
        let button = |s: Option<&&str>| match s.copied() {
            Some("m") => PointerButton::Middle,
            Some("r") => PointerButton::Secondary,
            _ => PointerButton::Primary,
        };
        let pointer = |pos: Pos2, button: PointerButton, pressed: bool, modifiers: Modifiers| Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers,
        };
        match parts[0] {
            "wait" => self.wait = number(1) as u32,
            "move" => {
                self.pos = pos2(number(1), number(2));
                raw.events.push(Event::PointerMoved(self.pos));
            }
            "click" => {
                self.pos = pos2(number(1), number(2));
                raw.events.push(Event::PointerMoved(self.pos));
                raw.events.push(pointer(self.pos, PointerButton::Primary, true, self.mods));
                raw.events.push(pointer(self.pos, PointerButton::Primary, false, self.mods));
                self.wait = 2;
            }
            "down" => raw.events.push(pointer(self.pos, button(parts.get(1)), true, self.mods)),
            "up" => raw.events.push(pointer(self.pos, button(parts.get(1)), false, self.mods)),
            "hold" => {
                self.mods = match parts.get(1).copied() {
                    Some("shift") => Modifiers::SHIFT,
                    Some("alt") => Modifiers::ALT,
                    Some("ctrl") => Modifiers::CTRL | Modifiers::COMMAND,
                    _ => Modifiers::NONE,
                };
                raw.events.push(Event::ModifiersChanged(self.mods));
            }
            "scroll" => raw.events.push(Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, number(1)),
                phase: egui::TouchPhase::Move,
                modifiers: self.mods,
                source: egui::MouseWheelSource::Wheel,
            }),
            "key" => {
                let key = Key::from_name(parts[1]).expect("unknown key name");
                let mut modifiers = self.mods;
                for m in &parts[2..] {
                    match *m {
                        "ctrl" => modifiers = modifiers | Modifiers::CTRL | Modifiers::COMMAND,
                        "shift" => modifiers = modifiers | Modifiers::SHIFT,
                        "alt" => modifiers = modifiers | Modifiers::ALT,
                        _ => {}
                    }
                }
                raw.events.push(Event::ModifiersChanged(modifiers));
                for pressed in [true, false] {
                    raw.events.push(Event::Key {
                        key,
                        physical_key: Some(key),
                        pressed,
                        repeat: false,
                        modifiers,
                    });
                }
                raw.events.push(Event::ModifiersChanged(self.mods));
                self.wait = 2;
            }
            "copy" => raw.events.push(Event::Copy),
            "paste" => raw.events.push(Event::Paste("script".to_string())),
            "drop" => raw.dropped_files.push(Arc::new(ScriptedFile(parts[1].into()))),
            "shot" => {
                let path = parts[1].to_string();
                let pending = self.pending_shot.clone();
                *pending.lock().unwrap() = true;
                let repaint = ctx.clone();
                ctx.request_screenshot(move |image| {
                    let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                    image::save_buffer(
                        &path,
                        &bytes,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ColorType::Rgba8,
                    )
                    .expect("could not save screenshot");
                    println!("[script] saved {path}");
                    *pending.lock().unwrap() = false;
                    repaint.request_repaint();
                });
            }
            "close" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            "quit" => {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            other => panic!("unknown script command: {other}"),
        }
    }
}
