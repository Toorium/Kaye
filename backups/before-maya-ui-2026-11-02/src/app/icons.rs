//! Phosphor icons (MIT, see assets/icons/LICENSE). Each is rasterized at the exact
//! pixel size it is drawn at, then tinted, so they stay crisp at any display scale.

use std::cell::RefCell;
use std::collections::HashMap;

use egui::{Color32, ColorImage, Painter, Rect, TextureHandle, TextureOptions, Vec2, pos2};
use resvg::{tiny_skia, usvg};

macro_rules! icons {
    ($($variant:ident => $file:literal,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Icon { $($variant,)* }

        impl Icon {
            #[cfg(test)]
            const ALL: &[Icon] = &[$(Icon::$variant,)*];

            fn svg(self) -> &'static [u8] {
                match self {
                    $(Icon::$variant => include_bytes!(concat!("../../assets/icons/", $file, ".svg")),)*
                }
            }
        }
    };
}

icons! {
    Select => "cursor",
    Move => "arrows-out-cardinal",
    Rotate => "arrow-clockwise",
    Bone => "bone",
    Rig => "person-arms-spread",
    Part => "cube",
    Eye => "eye",
    EyeOff => "eye-slash",
    Reset => "arrow-counter-clockwise",
    Undo => "arrow-u-up-left",
    Redo => "arrow-u-up-right",
    Import => "download-simple",
    Key => "diamond",
    KeyOutline => "diamond-outline",
    Pause => "pause",
    Loop => "repeat",
    AutoKey => "record",
    PrevKey => "caret-double-left",
    NextKey => "caret-double-right",
    Chain => "link-simple",
    Mirror => "flip-horizontal",
    Play => "play",
    SkipStart => "skip-back",
    SkipEnd => "skip-forward",
    StepBack => "caret-left",
    StepForward => "caret-right",
    Search => "magnifying-glass",
    ChevronRight => "caret-right",
    ChevronDown => "caret-down",
    Folder => "folder-simple",
    Perspective => "perspective",
    Frame => "corners-out",
    OnTop => "stack",
    Colors => "palette",
    Modifiers => "wrench",
    Settings => "gear-six",
    Shaded => "sun",
    Studio => "circle-half",
    Camera => "video-camera",
    Unparent => "link-break",
    Target => "target",
    Close => "x",
}

thread_local! {
    static CACHE: RefCell<HashMap<(Icon, u32), TextureHandle>> = RefCell::new(HashMap::new());
}

/// Coverage of the icon at `px` by `px`, one byte per pixel.
fn rasterize(icon: Icon, px: u32) -> Vec<u8> {
    let tree = usvg::Tree::from_data(icon.svg(), &usvg::Options::default()).expect("bundled icon is valid SVG");
    let mut pixmap = tiny_skia::Pixmap::new(px, px).expect("icon size is not zero");
    let scale = px as f32 / tree.size().width();
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.pixels().iter().map(|p| p.alpha()).collect()
}

const LOGO: &[u8] = include_bytes!("../../assets/kaye.ico");
const MARK: &[u8] = include_bytes!("../../assets/logo_mark.png");

/// The full-color logo for the window and taskbar, from the largest size in the
/// .ico file, as straight RGBA with its width and height.
pub fn logo_full() -> (Vec<u8>, u32, u32) {
    let image = image::load_from_memory_with_format(LOGO, image::ImageFormat::Ico).expect("bundled logo is a valid icon");
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    (rgba.into_raw(), width, height)
}

/// Kaye's single-color mark at `px` by `px`, as straight RGBA.
fn mark_rgba(px: u32) -> Vec<u8> {
    let image = image::load_from_memory_with_format(MARK, image::ImageFormat::Png).expect("bundled mark is a valid image");
    image::imageops::resize(&image.to_rgba8(), px, px, image::imageops::FilterType::Lanczos3).into_raw()
}

thread_local! {
    static MARK_CACHE: RefCell<HashMap<u32, TextureHandle>> = RefCell::new(HashMap::new());
}

/// Draws the mark in the top bar, tinted so it stays visible on any panel color.
pub fn paint_logo(painter: &Painter, rect: Rect, color: Color32) {
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let px = (rect.width().min(rect.height()) * ppp).round().max(1.0) as u32;
    let texture = MARK_CACHE.with_borrow_mut(|cache| {
        cache
            .entry(px)
            .or_insert_with(|| {
                let image = ColorImage::from_rgba_unmultiplied([px as usize, px as usize], &mark_rgba(px));
                ctx.load_texture(format!("mark {px}"), image, TextureOptions::LINEAR)
            })
            .id()
    });
    let size = px as f32 / ppp;
    let min = rect.center() - Vec2::splat(size * 0.5);
    let min = pos2((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp);
    painter.image(
        texture,
        Rect::from_min_size(min, Vec2::splat(size)),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        color,
    );
}

/// Draws an icon centered in `rect`, tinted with `color`.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let px = (rect.width().min(rect.height()) * ppp).round().max(1.0) as u32;
    let texture = CACHE.with_borrow_mut(|cache| {
        cache
            .entry((icon, px))
            .or_insert_with(|| {
                let pixels: Vec<Color32> = rasterize(icon, px).into_iter().map(Color32::from_white_alpha).collect();
                let image = ColorImage::new([px as usize, px as usize], pixels);
                ctx.load_texture(format!("icon {icon:?} {px}"), image, TextureOptions::LINEAR)
            })
            .id()
    });
    // Landing on whole physical pixels keeps the raster mapped one to one.
    let size = px as f32 / ppp;
    let min = rect.center() - Vec2::splat(size * 0.5);
    let min = pos2((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp);
    painter.image(
        texture,
        Rect::from_min_size(min, Vec2::splat(size)),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_and_mark_load() {
        let (rgba, width, height) = logo_full();
        assert_eq!(rgba.len() as u32, width * height * 4);
        assert!(width >= 32);

        let mark = mark_rgba(24);
        assert_eq!(mark.len(), 24 * 24 * 4);
        let solid = mark.chunks(4).filter(|p| p[3] > 200).count();
        let clear = mark.chunks(4).filter(|p| p[3] < 40).count();
        assert!(solid > 0 && clear > 0, "the mark should have a shape on transparency");
    }

    #[test]
    fn every_bundled_icon_renders() {
        for icon in Icon::ALL {
            let pixels = rasterize(*icon, 16);
            assert_eq!(pixels.len(), 16 * 16);
            assert!(pixels.iter().any(|a| *a > 0), "{icon:?} rendered empty");
        }
    }
}
