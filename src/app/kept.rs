//! Drawing that only changes when what it shows does, kept as finished triangles. The
//! timeline and Graph tab hold thousands of keys, and while the animation plays nothing
//! about them changes, so a frame then only copies the triangles instead of building
//! every key's shape again.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use egui::epaint::Tessellator;
use egui::{Color32, Mesh, Rect, Shape};

/// One kept drawing, and what it was drawn from.
#[derive(Default)]
pub struct Kept {
    signature: Option<u64>,
    mesh: Arc<Mesh>,
}

impl Kept {
    /// The drawing for `signature`, built by `build` only when the signature differs from
    /// last time. `build` must not add text, since text's triangles point into the font
    /// atlas, which can change size.
    pub fn shape(&mut self, ctx: &egui::Context, signature: u64, build: impl FnOnce(&mut Vec<Shape>)) -> Shape {
        let signature = {
            // The same shapes come out as different triangles at another scale or quality.
            let mut h = Signature::new();
            h.add(signature);
            h.add_f32(ctx.pixels_per_point());
            ctx.tessellation_options(|o| {
                h.add(o.feathering);
                h.add_f32(o.feathering_size_in_pixels);
            });
            h.finish()
        };
        if self.signature != Some(signature) {
            let mut shapes = Vec::new();
            build(&mut shapes);
            let options = ctx.tessellation_options(|o| *o);
            let atlas = ctx.fonts(|f| f.font_image_size());
            // No prepared discs: they point into the font atlas too.
            let mut tessellator = Tessellator::new(ctx.pixels_per_point(), options, atlas, Vec::new());
            let mut mesh = Mesh::default();
            for shape in shapes {
                tessellator.tessellate_shape(shape, &mut mesh);
            }
            self.mesh = Arc::new(mesh);
            self.signature = Some(signature);
        }
        Shape::Mesh(self.mesh.clone())
    }
}

/// Adds up everything a kept drawing depends on.
pub struct Signature(DefaultHasher);

impl Signature {
    pub fn new() -> Self {
        Self(DefaultHasher::new())
    }

    pub fn add(&mut self, value: impl Hash) {
        value.hash(&mut self.0);
    }

    pub fn add_f32(&mut self, value: f32) {
        value.to_bits().hash(&mut self.0);
    }

    pub fn add_rect(&mut self, rect: Rect) {
        for v in [rect.min.x, rect.min.y, rect.max.x, rect.max.y] {
            self.add_f32(v);
        }
    }

    pub fn add_color(&mut self, color: Color32) {
        color.to_array().hash(&mut self.0);
    }

    /// A set's contents in any order, since a `HashSet` iterates in no fixed one.
    pub fn add_set<T: Hash>(&mut self, items: impl IntoIterator<Item = T>) {
        let (mut sum, mut count) = (0u64, 0u64);
        for item in items {
            let mut h = DefaultHasher::new();
            item.hash(&mut h);
            sum = sum.wrapping_add(h.finish());
            count += 1;
        }
        self.add((sum, count));
    }

    pub fn finish(&self) -> u64 {
        self.0.finish()
    }
}

impl Default for Signature {
    fn default() -> Self {
        Self::new()
    }
}
