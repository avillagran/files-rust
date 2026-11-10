//! Path editing, geometry and routing, independent of native input delivery.
use crate::{
    line_edit::{Cursor, Motion},
    model::Model,
    painter::{Painter, Rect},
};
use fltk::enums::Key;
use std::path::{Path, PathBuf};
#[derive(Default)]
pub struct State {
    pub cursor: Cursor,
    pub scroll: f64,
    area: Rect,
    points: Vec<(usize, f64)>,
    painted: u64,
    pub dragging: bool,
    /// Open Tab-completion suggestions, if any.
    pub completion: Option<Completion>,
}
/// Most directory entries one Tab completion scans before giving up on the
/// rest; keeps the read on the UI thread short even in huge folders.
pub(crate) const COMPLETION_SCAN: usize = 5000;
/// Most suggestions kept, and most symlinks resolved, per completion.
const COMPLETION_ITEMS: usize = 256;
const COMPLETION_LINKS: usize = 64;
/// Suggestions left by an ambiguous Tab: Up/Down or Tab cycle them.
#[derive(Debug, Default)]
pub struct Completion {
    /// Field text before the completed component (ends with '/' or is empty).
    base: String,
    /// Candidate names and whether each browses as a folder.
    pub items: Vec<(String, bool)>,
    pub index: Option<usize>,
    /// The `operation_message` text this completion shows and owns.
    notice: String,
}
impl Completion {
    fn text(&self, i: usize) -> String {
        let (name, dir) = &self.items[i];
        format!("{}{name}{}", self.base, if *dir { "/" } else { "" })
    }
    /// One line listing the candidates, the current one in brackets.
    pub(crate) fn render(&self) -> String {
        let mut out = String::from("Tab ·");
        for (i, (name, dir)) in self.items.iter().enumerate() {
            if out.len() > 400 {
                out.push_str(&format!(" … ({})", self.items.len()));
                break;
            }
            let shown = format!("{name}{}", if *dir { "/" } else { "" });
            if self.index == Some(i) {
                out.push_str(&format!(" [{shown}]"));
            } else {
                out.push_str(&format!(" {shown}"));
            }
        }
        out
    }
}
/// Splits the field text into the part kept and the last component typed.
fn split_component(text: &str) -> (&str, &str) {
    match text.rfind('/') {
        Some(i) => (&text[..=i], &text[i + 1..]),
        None => ("", text),
    }
}
fn completion_dir(base: &str, current: &Path) -> PathBuf {
    if base.is_empty() {
        current.to_path_buf()
    } else if let Some(rest) = base.strip_prefix("~/") {
        files_core::theme::home().join(rest)
    } else if base.starts_with('/') {
        PathBuf::from(base)
    } else {
        current.join(base)
    }
}
/// One bounded `read_dir`: at most [`COMPLETION_SCAN`] entries, names that are
/// not UTF-8 skipped, and only matching symlinks resolved (one `stat` each).
fn read_candidates(dir: &Path, partial: &str) -> std::io::Result<Vec<(String, bool)>> {
    let lower = partial.to_lowercase();
    let mut out = Vec::new();
    let mut links = 0;
    for item in std::fs::read_dir(dir)?.take(COMPLETION_SCAN) {
        let Ok(item) = item else { continue };
        let Ok(name) = item.file_name().into_string() else {
            continue;
        };
        if !name.to_lowercase().starts_with(&lower) {
            continue;
        }
        let folder = match item.file_type() {
            Ok(t) if t.is_dir() => true,
            Ok(t) if t.is_symlink() && links < COMPLETION_LINKS => {
                links += 1;
                std::fs::metadata(item.path()).is_ok_and(|m| m.is_dir())
            }
            _ => false,
        };
        out.push((name, folder));
    }
    Ok(out)
}
/// Names matching the typed component: case-sensitive first, otherwise
/// case-insensitive; hidden names only when the component starts with '.';
/// folders only when any folder matches.
pub(crate) fn completion_matches(
    names: impl IntoIterator<Item = (String, bool)>,
    partial: &str,
) -> Vec<(String, bool)> {
    let hidden = partial.starts_with('.');
    let all: Vec<_> = names
        .into_iter()
        .filter(|(n, _)| hidden || !n.starts_with('.'))
        .collect();
    let mut found: Vec<_> = all
        .iter()
        .filter(|(n, _)| n.starts_with(partial))
        .cloned()
        .collect();
    if found.is_empty() {
        let lower = partial.to_lowercase();
        found = all
            .into_iter()
            .filter(|(n, _)| n.to_lowercase().starts_with(&lower))
            .collect();
    }
    if found.iter().any(|(_, dir)| *dir) {
        found.retain(|(_, dir)| *dir);
    }
    found.sort_by(|a, b| {
        a.0.to_lowercase()
            .cmp(&b.0.to_lowercase())
            .then_with(|| a.0.cmp(&b.0))
    });
    found.dedup();
    found.truncate(COMPLETION_ITEMS);
    found
}
/// Longest prefix shared by every candidate, compared case-insensitively
/// when the matches were case-insensitive; spelled as the first candidate.
fn common_prefix(items: &[(String, bool)], insensitive: bool) -> &str {
    let first = &items[0].0;
    let mut end = first.len();
    for (name, _) in &items[1..] {
        let mut shared = 0;
        for ((i, a), b) in first.char_indices().zip(name.chars()) {
            let same = if insensitive {
                a.to_lowercase().eq(b.to_lowercase())
            } else {
                a == b
            };
            if !same || i >= end {
                break;
            }
            shared = i + a.len_utf8();
        }
        end = end.min(shared);
    }
    &first[..end]
}
pub enum KeyResult {
    Unhandled,
    Handled,
    Copy(String),
}
// Percent encoding expands each raw byte by at most three. Other editors and
// literal paths keep the 4096-byte limit; decoded paths are still validated.
pub(crate) const URI_LIMIT: usize = 3 * crate::omnibar_input::LIMIT + 7;
fn is_uri(text: &str) -> bool {
    text.get(..7)
        .is_some_and(|s| s.eq_ignore_ascii_case("file://"))
}
pub(crate) fn edit_limit(text: &str, range: std::ops::Range<usize>, input: &str) -> usize {
    let prefix = text[..range.start]
        .bytes()
        .chain(
            input
                .bytes()
                .filter(|b| !matches!(b, b'\n' | b'\r' | b'\0')),
        )
        .chain(text[range.end..].bytes());
    if prefix
        .take(7)
        .map(|b| b.to_ascii_lowercase())
        .eq(b"file://".iter().copied())
    {
        URI_LIMIT
    } else {
        crate::omnibar_input::LIMIT
    }
}
pub fn display(path: &Path) -> Result<String, &'static str> {
    files_core::navigation_history::validate(path)?;
    if let Some(s) = path.to_str().filter(|s| !s.chars().any(char::is_control)) {
        if s.len() > crate::omnibar_input::LIMIT {
            return Err(crate::omnibar_input::TOO_LONG);
        }
        return Ok(s.to_string());
    }
    use files_core::file_transfer::{self, Format, Intent};
    let mut bytes = file_transfer::encode_bounded(
        Format::UriList,
        Intent::Copy,
        std::slice::from_ref(&path.to_path_buf()),
        URI_LIMIT + 2,
    )
    .map_err(|_| "No se puede representar esta ruta como URI local válida")?;
    bytes.truncate(bytes.len() - 2);
    String::from_utf8(bytes).map_err(|_| "URI de ruta inválida")
}
pub fn destination(text: &str, current: &Path) -> Result<PathBuf, &'static str> {
    if text.chars().any(char::is_control) {
        return Err("La ruta debe ocupar una sola línea, sin controles");
    }
    if text.is_empty() {
        return Err("Escribe una ruta de carpeta");
    }
    if text.len()
        > if is_uri(text) {
            URI_LIMIT
        } else {
            crate::omnibar_input::LIMIT
        }
    {
        return Err(crate::omnibar_input::TOO_LONG);
    }
    if text
        .get(..7)
        .is_some_and(|s| s.eq_ignore_ascii_case("file://"))
    {
        let trimmed = text.trim_end_matches('/');
        if text.eq_ignore_ascii_case("file:///") || text.eq_ignore_ascii_case("file://localhost/") {
            return Ok("/".into());
        }
        let mut decoded = files_core::file_transfer::decode(
            files_core::file_transfer::Format::UriList,
            trimmed.as_bytes(),
        )
        .map_err(|_| "URI file:// local inválida o fuera de límites")?;
        if decoded.paths.len() != 1 {
            return Err("El campo admite una sola ruta");
        }
        return Ok(decoded.paths.remove(0));
    }
    if text.split_once("://").is_some_and(|(scheme, _)| {
        scheme
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
            && scheme
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b))
    }) {
        return Err(
            "Este campo admite rutas locales, file://, smb://, sftp://, ftp://, dav://, nfs:// y afp://",
        );
    }
    let path = if text == "~" {
        files_core::theme::home()
    } else if let Some(s) = text.strip_prefix("~/") {
        files_core::theme::home().join(s)
    } else {
        PathBuf::from(text)
    };
    Ok(if path.is_absolute() {
        path
    } else {
        current.join(path)
    })
}
impl Model {
    fn set_path_text(&mut self, text: &str) {
        let limit = edit_limit(&self.edit, 0..self.edit.len(), text);
        if self
            .path_edit
            .cursor
            .insert_bounded(&mut self.edit, text, true, limit)
            .is_ok()
        {
            self.replace_edit = false;
        }
    }
    fn show_completion_notice(&mut self, text: String) {
        let owned = self
            .path_edit
            .completion
            .as_ref()
            .map(|c| c.notice.as_str());
        if self.operation_message.is_none() || self.operation_message.as_deref() == owned {
            self.operation_message = Some(text.clone());
        }
        if let Some(c) = self.path_edit.completion.as_mut() {
            c.notice = text;
        }
    }
    /// Closes Tab suggestions, removing only the notice they showed.
    pub fn close_path_completion(&mut self) -> bool {
        let Some(c) = self.path_edit.completion.take() else {
            return false;
        };
        if self.operation_message.as_deref() == Some(c.notice.as_str()) {
            self.operation_message = None;
        }
        true
    }
    /// Tab in the location field: completes the last component like a shell.
    /// Never runs for the network password prompt, network or file:// URIs,
    /// catalog prefixes, or folders on network/FUSE mounts not already listed.
    pub fn complete_path(&mut self, backwards: bool) {
        if !self.editing || self.network.prompt.is_some() {
            return;
        }
        if let Some(c) = self.path_edit.completion.as_mut()
            && c.items.len() > 1
        {
            let n = c.items.len();
            let i = match (c.index, backwards) {
                (None, false) => 0,
                (None, true) => n - 1,
                (Some(i), false) => (i + 1) % n,
                (Some(i), true) => (i + n - 1) % n,
            };
            c.index = Some(i);
            let (text, notice) = (c.text(i), c.render());
            self.set_path_text(&text);
            self.show_completion_notice(notice);
            return;
        }
        self.close_path_completion();
        let text = self.edit.clone();
        if files_core::network::is_network_uri(&text)
            || is_uri(&text)
            || matches!(text.as_bytes().first(), Some(b'>' | b':'))
            || text.chars().any(char::is_control)
        {
            return;
        }
        if text == "~" {
            self.set_path_text("~/");
            return;
        }
        let (base, partial) = split_component(&text);
        let dir = completion_dir(base, &self.path);
        let names: Vec<(String, bool)> = if dir == self.path && self.loaded {
            // Already listed: no filesystem access at all.
            self.entries
                .iter()
                .filter_map(|e| Some((e.name.to_str()?.to_owned(), e.browses_as_dir())))
                .collect()
        } else if files_core::network::gvfs_dir().is_some_and(|g| dir.starts_with(g))
            || !crate::automatic_sizes::background_location(&dir, &self.mounts, false)
        {
            self.path_edit.completion = Some(Completion::default());
            self.show_completion_notice("Tab: sin autocompletar en ubicaciones de red".into());
            return;
        } else {
            read_candidates(&dir, partial).unwrap_or_default()
        };
        let found = completion_matches(names, partial);
        if found.is_empty() {
            self.path_edit.completion = Some(Completion::default());
            self.show_completion_notice("Tab: sin coincidencias".into());
            return;
        }
        let base = base.to_owned();
        if let [(name, dir)] = found.as_slice() {
            let text = format!("{base}{name}{}", if *dir { "/" } else { "" });
            self.set_path_text(&text);
            return;
        }
        let insensitive = !found.iter().all(|(n, _)| n.starts_with(partial));
        let prefix = common_prefix(&found, insensitive);
        if prefix.len() >= partial.len() && prefix != partial {
            let text = format!("{base}{prefix}");
            self.set_path_text(&text);
        }
        let c = Completion {
            base,
            items: found,
            index: None,
            notice: String::new(),
        };
        let notice = c.render();
        self.path_edit.completion = Some(c);
        self.show_completion_notice(notice);
    }
    /// Text export only: never decode a presentation label or substitute raw bytes.
    pub(crate) fn copy_path_with(&mut self, sink: impl FnOnce(&str)) {
        let result = (|| {
            if self.closing || !self.loaded || self.selection_refreshing() {
                return Err("Espera la carga de la carpeta");
            }
            let i = self.selected.ok_or("Enfoca un elemento primero")?;
            let entry = self
                .entries
                .get(i)
                .ok_or("El elemento ya no está disponible")?;
            if !files_core::visibility::accepts(&entry.name, self.hidden_visible())
                || (!crate::selection_tools::is_map(self) && !self.order.contains(&i))
            {
                return Err("El elemento ya no está visible");
            }
            display(&self.path.join(&entry.name))
        })();
        match result {
            Ok(text) => {
                sink(&text);
                self.launch_notice = Some(if text.starts_with("file://") {
                    "URI file:// copiada · Ctrl+L para abrirla"
                } else {
                    "Ruta copiada"
                });
            }
            Err(why) => self.error = format!("Copiar ruta: {why}"),
        }
    }
    pub fn begin_path_edit(&mut self) {
        self.end_filter_edit();
        self.text_paste.invalidate();
        self.drag = None;
        self.session.restore_allowed = false;
        match display(&self.path) {
            Ok(text) => {
                self.edit = text;
                self.editing = true;
                self.replace_edit = true;
                self.path_edit = State::default();
                self.path_edit.cursor.all(&self.edit);
            }
            Err(e) => self.error = format!("Ruta: {e}"),
        }
    }
    pub fn end_path_edit(&mut self) {
        self.close_path_completion();
        if self.network.prompt.take().is_some() {
            // A password never outlives its prompt.
            self.edit.clear();
            self.operation_message = None;
        }
        self.end_filter_edit();
        self.text_paste.invalidate();
        self.editing = false;
        self.replace_edit = false;
        self.path_edit = State::default();
        if self.error.starts_with("Ruta:") || self.error == crate::omnibar_input::TOO_LONG {
            self.error.clear();
        }
    }
    pub fn commit_path_edit(&mut self) {
        if self.network.prompt.is_some() {
            self.submit_network_password();
            return;
        }
        if files_core::network::is_network_uri(&self.edit) {
            let text = self.edit.clone();
            if files_core::network::parse(&text).is_ok() {
                self.end_path_edit();
            }
            self.connect_network(&text);
            return;
        }
        if matches!(self.edit.as_bytes().first(), Some(b'>' | b':')) {
            let prefix = self.edit.as_bytes()[0];
            let query = self.edit[1..].trim();
            // Preflight before leaving the editor or cloning any input. A prefix
            // selects a catalog only; it never executes a command or a path.
            if query.len() > 256 || self.edit[1..].chars().any(char::is_control) {
                self.error = "Ruta: catálogo admite 256 bytes sin controles".into();
                return;
            }
            let query = query.to_owned();
            self.end_path_edit();
            if prefix == b'>' {
                self.open_commands();
                if let Some(state) = self.commands.as_mut() {
                    state.input(&query);
                }
            } else {
                self.open_bookmarks();
                if let Some(panel) = self.bookmarks.panel.as_mut()
                    && let Err(error) = panel.input(&query)
                {
                    panel.local_error = error.into();
                }
            }
            return;
        }
        match destination(&self.edit, &self.path) {
            Ok(path) => {
                if let Err(e) = files_core::navigation_history::validate(&path) {
                    self.error = e.into();
                    return;
                }
                let same = path == self.path;
                self.navigate(path);
                if same {
                    self.end_path_edit();
                }
            }
            Err(e) => self.error = format!("Ruta: {e}"),
        }
    }
    pub fn path_input(&mut self, text: &str) -> Result<(), &'static str> {
        self.close_path_completion();
        let range = if self.replace_edit {
            0..self.edit.len()
        } else {
            self.path_edit.cursor.range(&self.edit)
        };
        let limit = edit_limit(&self.edit, range, text);
        self.path_edit
            .cursor
            .insert_bounded(&mut self.edit, text, self.replace_edit, limit)?;
        self.replace_edit = false;
        if self.error.starts_with("Ruta:") || self.error == crate::omnibar_input::TOO_LONG {
            self.error.clear();
        }
        Ok(())
    }
    pub fn path_compose(&mut self, text: &str, bytes: usize) -> Result<(), &'static str> {
        self.text_paste.invalidate();
        self.close_path_completion();
        let range = self
            .path_edit
            .cursor
            .composition_range(&self.edit, bytes, self.replace_edit);
        let limit = edit_limit(&self.edit, range, text);
        self.path_edit.cursor.compose_bounded(
            &mut self.edit,
            text,
            bytes,
            self.replace_edit,
            limit,
        )?;
        self.replace_edit = false;
        if self.error.starts_with("Ruta:") || self.error == crate::omnibar_input::TOO_LONG {
            self.error.clear();
        }
        Ok(())
    }
    pub fn path_key(&mut self, key: Key, ctrl: bool, shift: bool, alt: bool) -> KeyResult {
        if !self.editing || alt && matches!(key, Key::Left | Key::Right) {
            return KeyResult::Unhandled;
        }
        self.text_paste.invalidate();
        let suggesting = self
            .path_edit
            .completion
            .as_ref()
            .is_some_and(|c| !c.items.is_empty());
        match key {
            Key::Tab if !ctrl => {
                self.complete_path(shift);
                return KeyResult::Handled;
            }
            Key::Up | Key::Down if suggesting => {
                self.complete_path(key == Key::Up);
                return KeyResult::Handled;
            }
            Key::Escape if self.close_path_completion() => return KeyResult::Handled,
            Key::Enter | Key::KPEnter
                if self
                    .path_edit
                    .completion
                    .as_ref()
                    .is_some_and(|c| c.index.is_some()) =>
            {
                // Accept the highlighted suggestion; a second Enter navigates.
                self.close_path_completion();
                return KeyResult::Handled;
            }
            Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Tab => {}
            // Pressing a modifier (Shift for Shift+Tab) keeps the suggestions.
            Key::ShiftL
            | Key::ShiftR
            | Key::ControlL
            | Key::ControlR
            | Key::AltL
            | Key::AltR
            | Key::MetaL
            | Key::MetaR
            | Key::CapsLock => {}
            _ => {
                self.close_path_completion();
            }
        }
        if self.replace_edit {
            self.path_edit.cursor.all(&self.edit);
            self.replace_edit = false;
        }
        let c = &mut self.path_edit.cursor;
        match key {
            Key::Left => c.motion(
                &self.edit,
                if ctrl { Motion::WordLeft } else { Motion::Left },
                shift,
            ),
            Key::Right => c.motion(
                &self.edit,
                if ctrl {
                    Motion::WordRight
                } else {
                    Motion::Right
                },
                shift,
            ),
            Key::Home => c.motion(&self.edit, Motion::Start, shift),
            Key::End => c.motion(&self.edit, Motion::End, shift),
            Key::BackSpace => c.delete(&mut self.edit, true, ctrl, false),
            Key::Delete => c.delete(&mut self.edit, false, ctrl, false),
            Key::Escape => self.end_path_edit(),
            Key::Enter | Key::KPEnter => self.commit_path_edit(),
            Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Tab => {}
            Key::F2 | Key::F3 | Key::F5 | Key::F6 | Key::F7 | Key::F8 => {}
            k if ctrl && k == Key::from_char('a') => c.all(&self.edit),
            k if ctrl && (k == Key::from_char('z') || k == Key::from_char('y')) => {
                c.undo(&mut self.edit, shift || k == Key::from_char('y'))
            }
            k if ctrl
                && (k == Key::from_char('c') || k == Key::from_char('x'))
                && self.network.prompt.is_some() => {}
            k if ctrl && (k == Key::from_char('c') || k == Key::from_char('x')) => {
                let range = c.range(&self.edit);
                let copy = self.edit[range].to_string();
                if k == Key::from_char('x') && !copy.is_empty() {
                    c.delete(&mut self.edit, false, false, false);
                }
                return KeyResult::Copy(copy);
            }
            k if ctrl
                && !matches!(k, k if k==Key::from_char('l') || k==Key::from_char('f') || k==Key::from_char(',')) =>
                {}
            _ => return KeyResult::Unhandled,
        }
        KeyResult::Handled
    }
}
pub fn paint(p: &mut dyn Painter, m: &mut Model, area: Rect) {
    // A network password is drawn as one dot per character.
    let secret = m.network.prompt.is_some();
    let state = &mut m.path_edit;
    if m.replace_edit && state.cursor.range(&m.edit) != (0..m.edit.len()) {
        state.cursor.all(&m.edit);
    }
    state.area = area;
    state.points.clear();
    let wanted = m.edit.chars().count() + 1;
    if state.points.capacity() < wanted {
        state.points.reserve_exact(wanted);
    }
    state.points.push((0, 0.));
    let mut width = 0f64;
    let dot = f64::from(p.text_width("•", 13, true).max(0.));
    for (i, c) in m.edit.char_indices() {
        let mut buf = [0; 4];
        width += if secret {
            dot
        } else {
            f64::from(p.text_width(c.encode_utf8(&mut buf), 13, true).max(0.))
        };
        state.points.push((i + c.len_utf8(), width));
    }
    let caret = state
        .points
        .iter()
        .find(|(i, _)| *i == state.cursor.caret)
        .map(|(_, x)| *x)
        .unwrap_or(width);
    let available = (area.w - 12).max(1) as f64;
    if caret < state.scroll {
        state.scroll = caret;
    }
    if caret > state.scroll + available {
        state.scroll = caret - available;
    }
    state.scroll = state.scroll.clamp(0., (width - available).max(0.));
    let x = area.x + 4 - state.scroll.round() as i32;
    let range = state.cursor.range(&m.edit);
    if !range.is_empty() {
        let a = state
            .points
            .iter()
            .find(|(i, _)| *i == range.start)
            .map(|(_, x)| *x)
            .unwrap_or(0.);
        let b = state
            .points
            .iter()
            .find(|(i, _)| *i == range.end)
            .map(|(_, x)| *x)
            .unwrap_or(width);
        p.rect(
            Rect::new(
                x + a.round() as i32,
                area.y + 6,
                (b - a).round() as i32,
                area.h - 12,
            ),
            m.palette.selection,
        );
    }
    if secret && m.edit.is_empty() {
        p.literal_text(
            Rect::new(area.x + 12, area.y, (area.w - 12).max(1), area.h),
            "Contraseña",
            13,
            m.palette.muted,
        );
    }
    let shown = if secret {
        std::borrow::Cow::Owned("•".repeat(m.edit.chars().count()))
    } else {
        std::borrow::Cow::Borrowed(m.edit.as_str())
    };
    p.literal_text(
        Rect::new(x, area.y, (width.ceil() as i32 + 8).max(1), area.h),
        &shown,
        13,
        m.palette.accent,
    );
    p.rect(
        Rect::new(x + caret.round() as i32, area.y + 6, 1, area.h - 12),
        m.palette.accent,
    );
    state.painted = state.cursor.revision;
}
pub fn painted_position(m: &Model, byte: usize) -> Option<(i32, i32)> {
    let s = &m.path_edit;
    if s.painted != s.cursor.revision {
        return None;
    }
    let x = s.points.iter().find(|(i, _)| *i == byte)?.1;
    let x = s.area.x + 4 + (x - s.scroll).round() as i32;
    s.area
        .contains(x, s.area.y + s.area.h / 2)
        .then_some((x, s.area.y + s.area.h / 2))
}
pub fn pointer(
    m: &mut Model,
    x: i32,
    y: i32,
    down: bool,
    drag: bool,
    shift: bool,
    double: bool,
) -> bool {
    if !m.editing {
        return false;
    }
    if !down && !drag {
        m.path_edit.dragging = false;
        return false;
    }
    let s = &mut m.path_edit;
    let outer = Rect::new(s.area.x - 8, s.area.y - 2, s.area.w + 16, s.area.h + 4);
    if down && !outer.contains(x, y) {
        s.dragging = false;
        return false;
    }
    if drag && !s.dragging {
        return false;
    }
    if s.painted != s.cursor.revision {
        return true;
    }
    let position = (x - s.area.x - 4) as f64 + s.scroll;
    let i = s.points.partition_point(|(_, p)| *p < position);
    let index = if i == 0 {
        0
    } else if i >= s.points.len() {
        s.points.last().map(|(i, _)| *i).unwrap_or(0)
    } else {
        if position - s.points[i - 1].1 <= s.points[i].1 - position {
            s.points[i - 1].0
        } else {
            s.points[i].0
        }
    };
    m.text_paste.invalidate();
    m.replace_edit = false;
    if down && double {
        s.cursor.component(&m.edit, index);
    } else {
        let anchor = if shift || drag {
            s.cursor.anchor
        } else {
            index
        };
        s.cursor.select(&m.edit, anchor, index);
    }
    s.dragging = true;
    true
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn copied_raw_path_roundtrips_without_replacement_or_label_decoding() {
        use std::os::unix::ffi::OsStringExt;
        let mut m = Model::new("/fixture-only-copy-path".into());
        m.loaded = true;
        m.entries = [
            b"raw\xff\n%2F\\xFF".as_slice(),
            "raw�".as_bytes(),
            b"literal\\xFF",
        ]
        .into_iter()
        .map(|name| files_core::listing::Entry {
            name: std::ffi::OsString::from_vec(name.to_vec()),
            kind: files_core::listing::Kind::File,
            metadata: None,
            stat_attempted: false,
            link: None,
        })
        .collect();
        m.order = vec![0, 1, 2];
        let mut outputs = Vec::new();
        for i in 0..3 {
            m.selected = Some(i);
            m.copy_path_with(|text| outputs.push(text.to_owned()));
            assert_eq!(
                destination(&outputs[i], &m.path).unwrap(),
                m.path.join(&m.entries[i].name)
            );
        }
        assert!(outputs[0].contains("%FF%0A%252F%5CxFF"));
        assert!(!outputs[1].starts_with("file://"));
        assert!(outputs[2].ends_with("literal\\xFF"));
        assert_ne!(outputs[0], outputs[1]);
        m.selected = Some(usize::MAX);
        m.copy_path_with(|_| panic!("stale selection must not reach clipboard"));
        m.selected = Some(0);
        m.error = "operación pendiente de rescate".into();
        m.copy_path_with(|_| {});
        assert_eq!(m.error, "operación pendiente de rescate");
        assert!(m.launch_notice.unwrap().contains("URI file://"));
        m.order.clear();
        m.copy_path_with(|_| panic!("filtered focus must not fall back"));
        m.order.push(0);
        m.loaded = false;
        m.copy_path_with(|_| panic!("loading must not reach clipboard"));
    }
    #[cfg(unix)]
    #[test]
    fn expanded_uri_roundtrips_at_raw_limit_and_oversize_refuses_copy() {
        use std::os::unix::ffi::OsStringExt;
        let raw = PathBuf::from(std::ffi::OsString::from_vec(
            [b"/".as_slice(), &vec![0xff; 4095]].concat(),
        ));
        assert!(files_core::navigation_history::validate(&raw).is_ok());
        let encoded = display(&raw).unwrap();
        assert!(encoded.len() > crate::omnibar_input::LIMIT);
        assert_eq!(destination(&encoded, Path::new("/base")).unwrap(), raw);
        let mut editor = Model::new("/fixture-only-expanded-uri".into());
        editor.begin_path_edit();
        editor.path_input(&encoded).unwrap();
        let before = (
            editor.edit.clone(),
            editor.edit.capacity(),
            editor.path_edit.cursor.revision,
        );
        assert!(editor.path_input(&"X".repeat(1_000_000)).is_err());
        assert_eq!(
            (
                &editor.edit,
                editor.edit.capacity(),
                editor.path_edit.cursor.revision
            ),
            (&before.0, before.1, before.2)
        );
        editor.path_edit.cursor.all(&editor.edit);
        assert!(editor.path_compose(&"a".repeat(4097), 0).is_err());
        assert_eq!(editor.edit, encoded);
        editor.path_compose("file:///small%FF", 0).unwrap();
        assert_eq!(editor.edit, "file:///small%FF");
        assert!(destination(&format!("file:///{}", "%FF".repeat(4096)), Path::new("/")).is_err());
        // A failed copy never reaches the native clipboard sink.
        let mut m = Model::new(raw);
        m.loaded = true;
        m.entries.push(files_core::listing::Entry {
            name: "leaf".into(),
            kind: files_core::listing::Kind::File,
            metadata: None,
            stat_attempted: false,
            link: None,
        });
        m.order.push(0);
        m.selected = Some(0);
        m.copy_path_with(|_| panic!("failed encoding must not reach clipboard"));
        assert!(m.error.starts_with("Copiar ruta:"));
    }
    struct WidthOnly;
    impl crate::painter::Painter for WidthOnly {
        fn rect(&mut self, _: Rect, _: files_core::theme::Color) {}
        fn rounded_rect(&mut self, _: Rect, _: i32, _: files_core::theme::Color) {}
        fn text(
            &mut self,
            _: Rect,
            _: &str,
            _: i32,
            _: files_core::theme::Color,
            _: bool,
            _: bool,
        ) {
        }
        fn image(&mut self, _: Rect, _: &crate::painter::Image<'_>) {}
        fn clip(&mut self, _: Rect) {}
        fn unclip(&mut self) {}
    }
    #[test]
    fn switching_to_search_releases_path_geometry_and_invalidates_pending_paste() {
        let mut m = Model::new("/fixture-only-path-search".into());
        m.begin_path_edit();
        m.path_input("changed").unwrap();
        let mut p = WidthOnly;
        paint(&mut p, &mut m, Rect::new(100, 10, 120, 34));
        assert!(m.path_edit.points.capacity() > 0);
        let context = m.text_context();
        assert!(m.text_paste.request(context).is_some());
        m.action(crate::model::Action::Search);
        assert!(m.search.visible && !m.editing && m.path_edit.points.capacity() == 0);
        assert!(!m.receive_text_paste("late") && m.search.query.is_empty());
    }
    #[test]
    fn paint_preserves_paste_revision_and_short_replacement_resets_horizontal_scroll() {
        let mut m = Model::new("/fixture-only-path-paint".into());
        m.begin_path_edit();
        m.path_input(&"long".repeat(100)).unwrap();
        let mut p = WidthOnly;
        paint(&mut p, &mut m, Rect::new(100, 10, 120, 34));
        assert!(m.path_edit.scroll > 0.);
        let revision = m.path_edit.cursor.revision;
        paint(&mut p, &mut m, Rect::new(100, 10, 120, 34));
        assert_eq!(m.path_edit.cursor.revision, revision);
        m.path_key(Key::from_char('a'), true, false, false);
        m.path_input("é/abc").unwrap();
        paint(&mut p, &mut m, Rect::new(100, 10, 120, 34));
        assert_eq!(m.path_edit.scroll, 0.);
        assert!(painted_position(&m, 0).is_some());
        let text = m.edit.clone();
        assert!(pointer(&mut m, 96, 24, true, false, false, false));
        assert_eq!(m.edit, text);
        assert_eq!(m.path_edit.cursor.caret, 0);
    }
    #[test]
    fn uri_is_explicit_local_single_decoded_once_and_raw_roundtrips() {
        use std::os::unix::ffi::OsStringExt;
        let raw = PathBuf::from(std::ffi::OsString::from_vec(
            b"/private/a%2F-\xff\n".to_vec(),
        ));
        let text = display(&raw).unwrap();
        assert!(text.contains("%252F") && text.contains("%FF") && text.contains("%0A"));
        assert_eq!(destination(&text, Path::new("/base")).unwrap(), raw);
        assert_eq!(
            destination("100%20literal", Path::new("/base")).unwrap(),
            Path::new("/base/100%20literal")
        );
        assert_eq!(
            destination("file:///", Path::new("/base")).unwrap(),
            Path::new("/")
        );
        assert!(destination("file://remote/private", Path::new("/base")).is_err());
        assert!(destination("a\0b", Path::new("/base")).is_err());
    }
    #[test]
    fn field_keys_and_late_paste_do_not_mutate_browser_or_accept_old_cursor() {
        let mut m = Model::new("/fixture-only-path-editor".into());
        m.begin_path_edit();
        m.path_key(Key::End, false, false, false);
        crate::input(&mut m, "/日本/old");
        let context = m.text_context();
        assert!(m.text_paste.request(context).is_some());
        m.path_key(Key::Left, true, false, false);
        assert!(!m.receive_text_paste("stale"));
        assert!(m.edit.ends_with("/日本/old"));
        m.path_key(Key::End, false, false, false);
        m.path_key(Key::BackSpace, true, false, false);
        assert!(m.edit.ends_with("/日本/"));
        m.can_undo = true;
        m.path_key(Key::from_char('z'), true, false, false);
        assert!(m.edit.ends_with("/日本/old") && !m.mutation_applied);
        m.path_key(Key::F5, false, false, false);
        assert!(m.copy_form.is_none());
        m.path_key(Key::Escape, false, false, false);
        assert!(!m.editing);
    }
    /// A fresh completion fixture under the private test TMPDIR; retained.
    fn completion_fixture() -> (PathBuf, Model) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static ID: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "files-path-complete-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        for dir in [
            "Documents",
            "Downloads",
            "Music",
            "proj-alpha",
            "proj-beta",
            ".hidden",
        ] {
            std::fs::create_dir(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("Dockerfile"), b"x").unwrap();
        std::fs::write(root.join("notes.txt"), b"x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("Music", root.join("tunes")).unwrap();
        // The model browses a different private folder: completion must read
        // the typed folder, not the listing.
        let elsewhere = root.join("Documents");
        let mut m = Model::new(elsewhere);
        let (tx, _) = std::sync::mpsc::sync_channel(4);
        let (_, rx) = std::sync::mpsc::channel();
        m.aux = crate::aux_io::AuxIo { tx, rx };
        m.begin_path_edit();
        (root, m)
    }
    fn typed(m: &mut Model, text: &str) {
        m.path_key(Key::from_char('a'), true, false, false);
        m.path_input(text).unwrap();
    }
    fn tab(m: &mut Model, shift: bool) {
        assert!(matches!(
            m.path_key(Key::Tab, false, shift, false),
            KeyResult::Handled
        ));
    }
    #[test]
    fn tab_completes_unique_folders_files_case_and_hidden_like_a_shell() {
        let (root, mut m) = completion_fixture();
        let base = format!("{}/", root.display());
        typed(&mut m, &format!("{base}Doc"));
        tab(&mut m, false);
        assert_eq!(m.edit, format!("{base}Documents/"));
        assert!(m.path_edit.completion.is_none());
        // Case-insensitive only when nothing matches case-sensitively.
        typed(&mut m, &format!("{base}mus"));
        tab(&mut m, false);
        assert_eq!(m.edit, format!("{base}Music/"));
        // A file completes without a slash when no folder matches.
        typed(&mut m, &format!("{base}Dockerf"));
        tab(&mut m, false);
        assert_eq!(m.edit, format!("{base}Dockerfile"));
        // A link to a folder completes as a folder.
        typed(&mut m, &format!("{base}tu"));
        tab(&mut m, false);
        assert_eq!(m.edit, format!("{base}tunes/"));
        // Hidden entries only when the component starts with '.'.
        typed(&mut m, &format!("{base}hid"));
        tab(&mut m, false);
        assert_eq!(m.edit, format!("{base}hid"));
        assert_eq!(
            m.operation_message.as_deref(),
            Some("Tab: sin coincidencias")
        );
        typed(&mut m, &format!("{base}.h"));
        assert!(m.operation_message.is_none(), "typing drops the notice");
        tab(&mut m, false);
        assert_eq!(m.edit, format!("{base}.hidden/"));
        typed(&mut m, "~");
        tab(&mut m, false);
        assert_eq!(m.edit, "~/");
        assert!(m.editing);
    }
    #[test]
    fn ambiguous_tab_extends_prefix_lists_and_cycles_then_enter_accepts() {
        let (root, mut m) = completion_fixture();
        let base = format!("{}/", root.display());
        typed(&mut m, &format!("{base}pr"));
        tab(&mut m, false);
        assert_eq!(m.edit, format!("{base}proj-"));
        let listed = m.operation_message.clone().unwrap();
        assert!(listed.contains("proj-alpha/") && listed.contains("proj-beta/"));
        tab(&mut m, false);
        assert_eq!(m.edit, format!("{base}proj-alpha/"));
        m.path_key(Key::ShiftL, false, true, false);
        tab(&mut m, false);
        assert_eq!(m.edit, format!("{base}proj-beta/"));
        tab(&mut m, true);
        assert_eq!(m.edit, format!("{base}proj-alpha/"));
        m.path_key(Key::Down, false, false, false);
        assert_eq!(m.edit, format!("{base}proj-beta/"));
        m.path_key(Key::Up, false, false, false);
        assert_eq!(m.edit, format!("{base}proj-alpha/"));
        assert!(
            m.operation_message
                .as_deref()
                .unwrap_or_default()
                .contains("[proj-alpha/]")
        );
        let path = m.path.clone();
        m.path_key(Key::Enter, false, false, false);
        assert!(m.editing && m.path_edit.completion.is_none());
        assert_eq!(m.path, path, "Enter first accepts the suggestion");
        assert!(m.operation_message.is_none());
        // Folders are preferred over files sharing the prefix.
        typed(&mut m, &format!("{base}Do"));
        tab(&mut m, false);
        assert_eq!(m.edit, format!("{base}Do"));
        let items = &m.path_edit.completion.as_ref().unwrap().items;
        assert_eq!(
            items,
            &[
                ("Documents".to_owned(), true),
                ("Downloads".to_owned(), true)
            ]
        );
        // Esc closes the suggestions first, then cancels editing.
        m.path_key(Key::Escape, false, false, false);
        assert!(m.editing && m.path_edit.completion.is_none());
        m.path_key(Key::Escape, false, false, false);
        assert!(!m.editing);
    }
    #[test]
    fn completion_uses_listing_and_never_runs_for_network_or_password() {
        let (root, mut m) = completion_fixture();
        // The current folder is answered from the listing, not the disk.
        m.path = root.clone();
        m.loaded = true;
        m.entries = vec![files_core::listing::Entry {
            name: "only-listed".into(),
            kind: files_core::listing::Kind::Directory,
            metadata: None,
            stat_attempted: false,
            link: None,
        }];
        typed(&mut m, "only");
        tab(&mut m, false);
        assert_eq!(m.edit, "only-listed/");
        typed(&mut m, "smb://host/sha");
        tab(&mut m, false);
        assert_eq!(m.edit, "smb://host/sha");
        typed(&mut m, &format!("file://{}/Doc", root.display()));
        tab(&mut m, false);
        assert!(m.edit.ends_with("/Doc") && m.path_edit.completion.is_none());
        m.network.prompt = Some(crate::network_ui::Prompt {
            location: files_core::network::parse("smb://host/share").unwrap(),
        });
        m.edit = "Doc".into();
        m.path_edit = State::default();
        tab(&mut m, false);
        assert_eq!(m.edit, "Doc");
        assert!(m.path_edit.completion.is_none());
    }
    #[test]
    fn completion_matching_and_common_prefix_rules() {
        let names = |v: &[(&str, bool)]| {
            v.iter()
                .map(|(n, d)| ((*n).to_owned(), *d))
                .collect::<Vec<_>>()
        };
        let found = completion_matches(
            names(&[("Alpha", false), ("alps", false), (".al", true)]),
            "al",
        );
        assert_eq!(found, names(&[("alps", false)]));
        let found = completion_matches(names(&[("Alpha", false), ("ALPS", false)]), "al");
        assert_eq!(common_prefix(&found, true), "Alp");
        assert_eq!(split_component("/a/b"), ("/a/", "b"));
        assert_eq!(split_component("b"), ("", "b"));
    }
    #[test]
    fn ime_replaces_bytes_at_cursor_atomically_and_undo_restores_cursor() {
        let mut c = Cursor::default();
        let mut text = "préTAIL".to_string();
        c.select(&text, 4, 4);
        c.compose(&mut text, "e", 2, false).unwrap();
        assert_eq!(text, "preTAIL");
        assert_eq!(c.caret, 3);
        c.undo(&mut text, false);
        assert_eq!(text, "préTAIL");
        assert_eq!(c.caret, 4);
        let before = (text.clone(), c.caret, c.anchor, c.revision);
        assert!(c.compose(&mut text, &"x".repeat(5000), 2, false).is_err());
        assert_eq!((text, c.caret, c.anchor, c.revision), before);
    }
}
