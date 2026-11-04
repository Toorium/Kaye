//! Luau plugins, so other people can extend Kaye. Like the rig logic, this has no
//! interface code, and plugins call the same operations the interface does.
//!
//! A plugin is a `.luau` or `.lua` file in the plugins folder. Loading one runs it, and
//! it registers commands with `kaye.command`. Each command shows up in the Plugins menu.
//! Luau has no file or network access, and the sandbox keeps plugins from changing the
//! language under each other. The time limit is there so a loop that never ends only
//! costs a message, and doesn't freeze someone's work.
//!
//! While a command runs, it owns the `Workspace`, which is the app's own scene and keys
//! handed over for the moment. The app makes the whole run one undo step.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use glam::{EulerRot, Quat, Vec3};
use mlua::{Error, Function, Lua, RegistryKey, Table, VmState};

use crate::animation::{Animation, FrameNumber, angles_near};
use crate::math::Xf;
use crate::posing;
use crate::scene::{BoneId, RigId, Scene};

/// How long one command, or one file loading, gets.
const TIME_LIMIT: Duration = Duration::from_secs(5);

/// What a running command works on.
#[derive(Default)]
pub struct Workspace {
    pub scene: Scene,
    pub animation: Animation,
    pub selected: Vec<BoneId>,
    pub rig: Option<RigId>,
    pub frame: FrameNumber,
    /// Anything the command wanted to say.
    pub messages: Vec<String>,
}

impl Workspace {
    fn bone(&self, id: BoneId) -> Result<BoneId, Error> {
        match self.scene.bones.get(id) {
            Some(bone) if !bone.removed => Ok(id),
            _ => Err(Error::runtime(format!("there's no bone {id}"))),
        }
    }
}

struct Command {
    name: String,
    function: RegistryKey,
}

pub struct Plugins {
    lua: Lua,
    workspace: Rc<RefCell<Workspace>>,
    commands: Rc<RefCell<Vec<Command>>>,
    deadline: Rc<Cell<Instant>>,
}

fn pose_table(lua: &Lua, pose: Xf) -> mlua::Result<Table> {
    let angles = angles_near(pose.rot, [0.0; 3]);
    let table = lua.create_table()?;
    table.set("x", pose.pos.x)?;
    table.set("y", pose.pos.y)?;
    table.set("z", pose.pos.z)?;
    table.set("rx", angles[0])?;
    table.set("ry", angles[1])?;
    table.set("rz", angles[2])?;
    Ok(table)
}

/// A table of numbers as a pose. Anything left out stays as it is in `current`.
fn table_pose(table: &Table, current: Xf) -> mlua::Result<Xf> {
    let angles = angles_near(current.rot, [0.0; 3]);
    let number = |name: &str, kept: f32| -> mlua::Result<f32> { Ok(table.get::<Option<f32>>(name)?.unwrap_or(kept)) };
    let pos = Vec3::new(number("x", current.pos.x)?, number("y", current.pos.y)?, number("z", current.pos.z)?);
    let (rx, ry, rz) = (number("rx", angles[0])?, number("ry", angles[1])?, number("rz", angles[2])?);
    let rot = Quat::from_euler(EulerRot::XYZ, rx.to_radians(), ry.to_radians(), rz.to_radians()).normalize();
    Ok(Xf::new(rot, pos))
}

impl Plugins {
    pub fn new() -> mlua::Result<Self> {
        let lua = Lua::new();
        let workspace = Rc::new(RefCell::new(Workspace::default()));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let deadline = Rc::new(Cell::new(Instant::now()));
        let plugins = Self { lua, workspace, commands, deadline };
        plugins.install()?;
        Ok(plugins)
    }

    fn install(&self) -> mlua::Result<()> {
        let lua = &self.lua;
        let kaye = lua.create_table()?;
        let commands = self.commands.clone();
        kaye.set(
            "command",
            lua.create_function(move |lua, (name, function): (String, Function)| {
                let function = lua.create_registry_value(function)?;
                commands.borrow_mut().push(Command { name, function });
                Ok(())
            })?,
        )?;
        let ws = self.workspace.clone();
        kaye.set("selected", lua.create_function(move |_, ()| Ok(ws.borrow().selected.clone()))?)?;
        let ws = self.workspace.clone();
        kaye.set(
            "bones",
            lua.create_function(move |_, ()| {
                let ws = ws.borrow();
                Ok(ws.rig.map_or_else(Vec::new, |rig| ws.scene.joints(rig)))
            })?,
        )?;
        let ws = self.workspace.clone();
        kaye.set(
            "findBone",
            lua.create_function(move |_, name: String| {
                let ws = ws.borrow();
                let Some(rig) = ws.rig else {
                    return Ok(None);
                };
                Ok(ws.scene.live_bones(rig).into_iter().find(|b| ws.scene.bones[*b].name == name))
            })?,
        )?;
        let ws = self.workspace.clone();
        kaye.set(
            "boneName",
            lua.create_function(move |_, bone: BoneId| {
                let ws = ws.borrow();
                Ok(ws.scene.bones[ws.bone(bone)?].name.clone())
            })?,
        )?;
        let ws = self.workspace.clone();
        kaye.set(
            "getPose",
            lua.create_function(move |lua, bone: BoneId| {
                let ws = ws.borrow();
                pose_table(lua, ws.scene.bones[ws.bone(bone)?].pose)
            })?,
        )?;
        let ws = self.workspace.clone();
        kaye.set(
            "setPose",
            lua.create_function(move |_, (bone, pose): (BoneId, Table)| {
                let mut ws = ws.borrow_mut();
                let bone = ws.bone(bone)?;
                let pose = table_pose(&pose, ws.scene.bones[bone].pose)?;
                ws.scene.set_bone_pose(bone, pose);
                Ok(())
            })?,
        )?;
        let ws = self.workspace.clone();
        kaye.set(
            "setKey",
            lua.create_function(move |_, bone: BoneId| {
                let mut ws = ws.borrow_mut();
                let bone = ws.bone(bone)?;
                let (pose, value, frame) = (ws.scene.bones[bone].pose, ws.scene.bones[bone].value, ws.frame);
                ws.animation.key_shown(bone, frame, pose, value);
                Ok(())
            })?,
        )?;
        let ws = self.workspace.clone();
        kaye.set("getFrame", lua.create_function(move |_, ()| Ok(ws.borrow().frame))?)?;
        let ws = self.workspace.clone();
        kaye.set(
            "setFrame",
            lua.create_function(move |_, frame: FrameNumber| {
                let ws = &mut *ws.borrow_mut();
                ws.frame = frame;
                ws.animation.apply(&mut ws.scene, frame as f32);
                Ok(())
            })?,
        )?;
        let ws = self.workspace.clone();
        kaye.set(
            "getRange",
            lua.create_function(move |_, ()| {
                let ws = ws.borrow();
                Ok((ws.animation.start, ws.animation.end))
            })?,
        )?;
        let ws = self.workspace.clone();
        kaye.set(
            "mirror",
            lua.create_function(move |_, bones: Vec<BoneId>| {
                let ws = &mut *ws.borrow_mut();
                for bone in &bones {
                    ws.bone(*bone)?;
                }
                for (bone, pose) in posing::mirror_pose(&ws.scene, &bones) {
                    ws.scene.bones[bone].pose = pose;
                }
                ws.scene.evaluate();
                Ok(())
            })?,
        )?;
        let ws = self.workspace.clone();
        kaye.set(
            "message",
            lua.create_function(move |_, text: String| {
                ws.borrow_mut().messages.push(text);
                Ok(())
            })?,
        )?;
        lua.globals().set("kaye", kaye)?;
        let deadline = self.deadline.clone();
        lua.set_interrupt(move |_| {
            if Instant::now() > deadline.get() {
                return Err(Error::runtime("it took too long and was stopped"));
            }
            Ok(VmState::Continue)
        });
        // mlua always adds a require that reads files, which a plugin must not have.
        lua.globals().raw_remove("require")?;
        lua.sandbox(true)
    }

    /// Runs one plugin file, which is how its commands get registered.
    pub fn load_source(&mut self, name: &str, source: &str) -> Result<(), String> {
        self.deadline.set(Instant::now() + TIME_LIMIT);
        self.lua.load(source).set_name(format!("@{name}")).exec().map_err(|e| format!("{name}: {e}"))
    }

    /// Loads every plugin in a folder, in name order. A plugin that fails doesn't stop
    /// the others, and each failure comes back to be shown.
    pub fn load_folder(&mut self, folder: &Path) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return Vec::new();
        };
        let mut files: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("luau") || e.eq_ignore_ascii_case("lua"))).collect();
        files.sort();
        let mut failures = Vec::new();
        for file in files {
            let name = file.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let loaded = std::fs::read_to_string(&file).map_err(|e| format!("{name}: {e}")).and_then(|source| self.load_source(&name, &source));
            failures.extend(loaded.err());
        }
        failures
    }

    pub fn command_names(&self) -> Vec<String> {
        self.commands.borrow().iter().map(|c| c.name.clone()).collect()
    }

    /// Hands the workspace to a command, and gets it back with what it did and any
    /// error. Whatever it finished before an error stays in the workspace, so the caller
    /// decides whether to keep it.
    pub fn run(&mut self, command: usize, workspace: Workspace) -> (Workspace, Result<(), String>) {
        *self.workspace.borrow_mut() = workspace;
        self.deadline.set(Instant::now() + TIME_LIMIT);
        let commands = self.commands.borrow();
        let result = match commands.get(command) {
            Some(command) => self.lua.registry_value::<Function>(&command.function).and_then(|f| f.call::<()>(())).map_err(|e| format!("{}: {e}", command.name)),
            None => Err("That command is gone.".to_string()),
        };
        drop(commands);
        (self.workspace.take(), result)
    }
}
