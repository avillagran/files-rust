//! Desktop identity and a live title for compositor-managed decorations.
use crate::model::Model;
use fltk::{enums::ColorDepth, image::RgbImage, prelude::*, window::Window};

#[derive(Default)]
pub struct State {
    last: Option<(u64, (u64, u64), u8)>,
}
impl State {
    pub fn sync(&mut self, m: &Model, win: &mut Window) {
        let key = (m.generation, m.pane_owner(), status(m));
        if self.last != Some(key) {
            win.set_label(&title(m));
            self.last = Some(key);
        }
    }
}
fn status(m: &Model) -> u8 {
    if m.closing {
        1
    } else if !m.error.is_empty() {
        2
    } else if m.mutations.busy || m.copy_busy() {
        3
    } else if !m.loaded {
        4
    } else if m.sizes.has_work() {
        5
    } else {
        0
    }
}
pub fn title(m: &Model) -> String {
    display_title(m.path.as_os_str(), status(m))
}
fn display_title(raw: &std::ffi::OsStr, state: u8) -> String {
    let path = files_core::name_label::label(raw);
    // The desktop entry supplies the app name and icon. Keep the client title
    // descriptive so compositor decorations can style their own app prefix.
    let mut title = String::new();
    for ch in path.chars().take(1024) {
        title.push(if ch.is_control() { ' ' } else { ch });
    }
    let status = match state {
        1 => "Cerrando…",
        2 => "Error",
        3 => "Operación en curso…",
        4 => "Cargando…",
        5 => "Calculando tamaños…",
        _ => "",
    };
    if !status.is_empty() {
        title.push_str(" · ");
        title.push_str(status);
    }
    title
}

pub fn set_icon(win: &mut Window) {
    // Match assets/files.svg without loading an image engine at startup.
    let mut rgba = vec![0u8; 64 * 64 * 4];
    for y in 0..64usize {
        for x in 0..64usize {
            let color = if (8..32).contains(&x) && (13..23).contains(&y) {
                Some([214, 144, 27, 255])
            } else if (8..56).contains(&x) && (21..52).contains(&y) {
                Some(if y < 27 {
                    [239, 178, 55, 255]
                } else {
                    [224, 157, 36, 255]
                })
            } else {
                None
            };
            if let Some(color) = color {
                rgba[(y * 64 + x) * 4..(y * 64 + x + 1) * 4].copy_from_slice(&color);
            }
        }
    }
    if let Ok(image) = RgbImage::new(&rgba, 64, 64, ColorDepth::Rgba8) {
        win.set_icon(Some(image));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn title_publishes_the_path_and_status_for_native_decorations() {
        let title = display_title(std::ffi::OsStr::new("/home/example"), 0);
        assert_eq!(title, "/home/example");
        assert_eq!(
            display_title(std::ffi::OsStr::new("/tmp"), 4),
            "/tmp · Cargando…"
        );
    }
}
