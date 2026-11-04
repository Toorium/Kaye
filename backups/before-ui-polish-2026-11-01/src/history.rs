//! Undo and redo for pose and animation edits.

use crate::animation::Animation;
use crate::math::Xf;
use crate::scene::{BoneId, RigState, Scene};

#[derive(Clone, Debug)]
pub struct PoseChange {
    pub bone: BoneId,
    pub before: Xf,
    pub after: Xf,
}

#[derive(Clone, Debug)]
pub struct Step {
    pub label: String,
    pub changes: Vec<PoseChange>,
    /// The whole animation before and after, when the edit touched it. Copies share
    /// the keys of every bone the edit left alone, so this stays small.
    pub animation: Option<Box<(Animation, Animation)>>,
    /// Modifiers and added objects before and after, when the edit touched them.
    pub rig: Option<Box<(RigState, RigState)>>,
}

pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// How many steps to keep. The oldest ones drop off.
    pub limit: usize,
}

impl Default for History {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit: 256,
        }
    }
}

impl History {
    pub fn push(&mut self, label: impl Into<String>, changes: Vec<PoseChange>) {
        if changes.is_empty() {
            return;
        }
        self.undo.push(Step {
            label: label.into(),
            changes,
            animation: None,
            rig: None,
        });
        self.redo.clear();
        self.trim();
    }

    pub fn push_edit(&mut self, label: impl Into<String>, changes: Vec<PoseChange>, before: Animation, after: Animation) {
        self.undo.push(Step {
            label: label.into(),
            changes,
            animation: Some(Box::new((before, after))),
            rig: None,
        });
        self.redo.clear();
        self.trim();
    }

    /// For edits to modifiers and added objects, which can touch keys as well.
    pub fn push_rig(&mut self, label: impl Into<String>, before: (RigState, Animation), after: (RigState, Animation)) {
        self.undo.push(Step {
            label: label.into(),
            changes: Vec::new(),
            animation: Some(Box::new((before.1, after.1))),
            rig: Some(Box::new((before.0, after.0))),
        });
        self.redo.clear();
        self.trim();
    }

    pub fn undo(&mut self, scene: &mut Scene, animation: &mut Animation) -> Option<String> {
        let step = self.undo.pop()?;
        for c in step.changes.iter().rev() {
            scene.bones[c.bone].pose = c.before;
        }
        if let Some(pair) = &step.animation {
            *animation = pair.0.clone();
        }
        if let Some(pair) = &step.rig {
            scene.restore_rig_state(&pair.0);
        }
        scene.evaluate();
        let label = step.label.clone();
        self.redo.push(step);
        Some(label)
    }

    pub fn redo(&mut self, scene: &mut Scene, animation: &mut Animation) -> Option<String> {
        let step = self.redo.pop()?;
        for c in &step.changes {
            scene.bones[c.bone].pose = c.after;
        }
        if let Some(pair) = &step.animation {
            *animation = pair.1.clone();
        }
        if let Some(pair) = &step.rig {
            scene.restore_rig_state(&pair.1);
        }
        scene.evaluate();
        let label = step.label.clone();
        self.undo.push(step);
        Some(label)
    }

    fn trim(&mut self) {
        if self.undo.len() > self.limit {
            let extra = self.undo.len() - self.limit;
            self.undo.drain(..extra);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.trim();
    }
}
