//! Reference clips: a picture, GIF or video shown in its own window, playing along with
//! the timeline. This is where a clip sits on the timeline and how much of it shows. The
//! pictures themselves are read by `media.rs`. No interface code.
//!
//! A clip is placed like one in a video editor. `offset` is the timeline frame its own
//! start lands on, and `trim_in` and `trim_out` are how far into it, in seconds, it starts
//! and stops showing. Moving the strip changes `offset`, and dragging its ends changes the
//! trims, so the clip itself never slides under the strip.

use serde::{Deserialize, Serialize};

/// The shortest a strip can be trimmed to, in timeline frames.
pub const SHORTEST: f64 = 1.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reference {
    /// Stays the same for the reference's whole life, so its window can be found again.
    pub id: u64,
    pub path: String,
    pub name: String,
    pub offset: f64,
    pub trim_in: f64,
    pub trim_out: f64,
    /// The clip's own frame rate, for playing it one frame per timeline frame. 0 for a picture.
    #[serde(default)]
    pub fps: f32,
    /// The clip's length in seconds, 0 for a picture, which lasts as long as its strip.
    #[serde(default)]
    pub duration: f64,
}

/// Which end of a strip is being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}

impl Reference {
    pub fn new(id: u64, path: String, name: String, offset: f64) -> Self {
        Self { id, path, name, offset, trim_in: 0.0, trim_out: 0.0, fps: 0.0, duration: 0.0 }
    }

    pub fn is_picture(&self) -> bool {
        self.duration <= 0.0
    }

    /// Timeline frames for each second of the clip. Matching the frame rate plays it at its
    /// real speed. Otherwise each of its frames takes one timeline frame, so it plays slower
    /// or faster than it was filmed.
    pub fn rate(&self, animation_fps: f32, match_fps: bool) -> f64 {
        let own = if match_fps || self.fps <= 0.0 { animation_fps } else { self.fps };
        own.max(1.0) as f64
    }

    /// Where the strip starts and ends on the timeline.
    pub fn span(&self, rate: f64) -> (f64, f64) {
        (self.offset + self.trim_in * rate, self.offset + self.trim_out * rate)
    }

    /// The moment of the clip to show at a timeline frame. Outside the strip it holds the
    /// first or last frame shown.
    pub fn time_at(&self, frame: f64, rate: f64) -> f64 {
        ((frame - self.offset) / rate).clamp(self.trim_in, self.trim_out.max(self.trim_in))
    }

    /// Gives a new clip its whole length once that's known. A picture gets `frames` of the
    /// timeline instead.
    pub fn fill_length(&mut self, frames: f64, rate: f64) {
        if self.trim_out <= self.trim_in {
            self.trim_out = if self.is_picture() { self.trim_in + frames.max(SHORTEST) / rate } else { self.duration };
        }
    }

    /// Moves the strip by whole frames.
    pub fn move_by(&mut self, frames: f64) {
        self.offset += frames;
    }

    /// Drags one end of the strip to `frame`. A clip can't be trimmed past its own ends or
    /// shorter than a frame. A picture has no ends of its own, so its start moves the strip
    /// instead.
    pub fn drag_edge(&mut self, edge: Edge, frame: f64, rate: f64) {
        let shortest = SHORTEST / rate;
        match edge {
            Edge::Start if self.is_picture() => {
                let end = self.offset + self.trim_out * rate;
                let start = frame.min(end - SHORTEST);
                self.offset = start - self.trim_in * rate;
                self.trim_out = (end - self.offset) / rate;
            }
            Edge::Start => {
                self.trim_in = ((frame - self.offset) / rate).clamp(0.0, self.trim_out - shortest);
            }
            Edge::End => {
                let longest = if self.is_picture() { f64::MAX } else { self.duration.max(self.trim_in + shortest) };
                self.trim_out = ((frame - self.offset) / rate).clamp(self.trim_in + shortest, longest);
            }
        }
    }
}

/// An id no reference in the list has.
pub fn next_id(references: &[Reference]) -> u64 {
    references.iter().map(|r| r.id).max().map_or(1, |id| id + 1)
}

/// Where a reference's window was left, kept with the view rather than the work.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowPlace {
    pub id: u64,
    /// Left, top, width and height, in points.
    pub rect: [f32; 4],
    pub open: bool,
    /// In a window of its own outside Kaye's.
    pub popped: bool,
}
