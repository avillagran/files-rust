//! Flat, bounded treemap of measured immediate children. No filesystem work.
use crate::{
    model::{Action, Hit, Model},
    painter::{Painter, Rect},
};
use files_core::{
    format_size,
    space_map::{self, Area, NodeKind, NodeStatus, SpaceReport},
};
use std::collections::HashMap;
#[derive(Default)]
pub struct Geometry {
    pub painted: Option<(u64, u64, &'static str, i32, i32)>,
    key: Option<(u64, u64, usize, i32, i32, bool)>,
    pub areas: Vec<Area>,
    mapped: Vec<Option<usize>>,
    excluded: Vec<usize>,
    navigation: Vec<usize>,
    membership: Vec<usize>,
    container_key: Option<(u64, u64, usize, i32, i32, bool)>,
    map_height: i32,
    pub list_rect: Rect,
    error: Option<String>,
    /// Entry focused through the row list (a tiny tile is both a tile and a
    /// row): arrows keep moving within the rows while it stays focused.
    pub list_focus: Option<usize>,
    /// Entry and painted rectangle of every tile of the last paint.
    #[cfg(test)]
    pub drawn: Vec<(Option<usize>, Rect)>,
}
impl Geometry {
    pub fn navigation_indices(&self) -> &[usize] {
        &self.navigation
    }
    pub fn contains_entry(&self, i: usize) -> bool {
        self.membership.binary_search(&i).is_ok()
    }
    pub fn current(&self, generation: u64, revision: u64, entries: usize, hidden: bool) -> bool {
        self.key.is_some_and(|k| {
            k.0 == revision && k.1 == generation && k.2 == entries && k.5 == hidden
        })
    }
    pub fn excluded_entries(&self) -> impl Iterator<Item = usize> + '_ {
        self.excluded.iter().filter_map(|&node| self.mapped[node])
    }
    pub fn max_scroll(&self) -> i32 {
        if self.list_rect.h < 26 {
            return 0;
        }
        (self.excluded.len() as i32 * 28 - self.list_rect.h).max(0)
    }
    pub fn page_rows(&self) -> i32 {
        (self.list_rect.h.saturating_add(2) / 28).max(1)
    }
    pub fn clamp_scroll(&self, scroll: i32) -> i32 {
        scroll.clamp(0, self.max_scroll())
    }
    pub fn reveal_scroll(&self, selected: Option<usize>, scroll: i32) -> i32 {
        let scroll = self.clamp_scroll(scroll);
        if self.list_rect.h < 26 {
            return scroll;
        }
        let Some(rank) = selected.and_then(|i| {
            self.excluded
                .iter()
                .position(|&node| self.mapped[node] == Some(i))
        }) else {
            return scroll;
        };
        let top = rank as i32 * 28;
        self.clamp_scroll(if top < scroll {
            top
        } else if top + 26 > scroll + self.list_rect.h {
            top + 26 - self.list_rect.h
        } else {
            scroll
        })
    }
    fn prepare(
        &mut self,
        report: &SpaceReport,
        entries: &[files_core::listing::Entry],
        generation: u64,
        revision: u64,
        size: (i32, i32),
        show_hidden: bool,
    ) -> bool {
        let key = (
            revision,
            generation,
            entries.len(),
            size.0,
            size.1,
            show_hidden,
        );
        if self.container_key == Some(key) {
            return false;
        }
        let mut rows = 0;
        loop {
            let reserve = if rows > 0 { 56 + 28 * rows } else { 26 };
            let height = (size.1 - reserve).max(24);
            self.rebuild(
                report,
                entries,
                generation,
                revision,
                (size.0, height),
                show_hidden,
            );
            // Keep a compact diagnostic and usable rows when exact geometry
            // cannot be represented; do not discard positive measured items.
            self.map_height = if self.error.is_some() {
                height.min(50)
            } else {
                height
            };
            let needed = self.excluded.len().min(3) as i32;
            if needed <= rows {
                break;
            }
            rows = needed;
        }
        self.container_key = Some(key);
        true
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    fn rebuild(
        &mut self,
        report: &SpaceReport,
        entries: &[files_core::listing::Entry],
        generation: u64,
        revision: u64,
        size: (i32, i32),
        show_hidden: bool,
    ) {
        let (w, h) = size;
        let key = (revision, generation, entries.len(), w, h, show_hidden);
        if self.key == Some(key) {
            return;
        }
        self.clear();
        self.key = Some(key);
        match space_map::layout_where(&report.nodes, w as f64, h as f64, |n| {
            show_hidden || n.name.first() != Some(&b'.')
        }) {
            Ok(areas) => self.areas = areas,
            Err(e) => self.error = Some(e),
        }
        self.mapped.resize(report.nodes.len(), None);
        let names = report
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| show_hidden || n.name.first() != Some(&b'.'))
            .map(|(i, n)| (n.name.as_slice(), i))
            .collect::<HashMap<_, _>>();
        for (i, entry) in entries.iter().enumerate() {
            if let Some(&node) = names.get(entry.name.as_encoded_bytes()) {
                self.mapped[node] = Some(i);
            }
        }
        let layout_failed = self.error.is_some();
        self.excluded = report
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                (show_hidden || n.name.first() != Some(&b'.'))
                    && (layout_failed
                        || n.status != NodeStatus::Complete
                        || n.bytes.is_none_or(|b| b == 0))
            })
            .map(|(i, _)| i)
            .collect();
        for a in &self.areas {
            if a.w < 65.0 || a.h < 45.0 {
                self.excluded.push(a.node);
            }
        }
        let mut visited = vec![false; report.nodes.len()];
        for node in self
            .areas
            .iter()
            .map(|a| a.node)
            .chain(self.excluded.iter().copied())
        {
            if !visited[node] {
                visited[node] = true;
                if let Some(i) = self.mapped[node] {
                    self.navigation.push(i);
                }
            }
        }
        self.membership.extend_from_slice(&self.navigation);
        self.membership.sort_unstable();
    }
}
/// Result of an arrow key in the treemap.
#[derive(Debug, PartialEq)]
pub enum Arrow {
    /// No usable focus: the shared rank movement applies (Down starts at the
    /// first block, Up at the last).
    Linear,
    /// Nothing lies in that direction.
    Stay,
    /// Focus this entry; `true` when it is focused as a row of the list.
    To(usize, bool),
}
/// Spatial arrow navigation: the nearest block whose edge faces the focused
/// block in the arrow's direction, preferring blocks that share its row or
/// column span. Down from the bottom of the map enters the list of blocks
/// without area; Up from its first row returns to the bottom of the map.
pub fn arrow_target(m: &Model, key: fltk::enums::Key) -> Arrow {
    use fltk::enums::Key;
    let g = &m.space_geometry;
    let Some(selected) = m.selected.filter(|&i| g.contains_entry(i)) else {
        return Arrow::Linear;
    };
    let rows = g.excluded_entries().collect::<Vec<_>>();
    let tiles = g
        .areas
        .iter()
        .filter_map(|a| g.mapped[a.node].map(|i| (i, a)))
        .collect::<Vec<_>>();
    let current = tiles.iter().find(|(i, _)| *i == selected).map(|(_, a)| *a);
    let rank = rows.iter().position(|&i| i == selected);
    if let Some(rank) = rank
        && (current.is_none() || g.list_focus == Some(selected))
    {
        return match key {
            Key::Up if rank == 0 => tiles
                .iter()
                .max_by(|(_, a), (_, b)| {
                    (a.y + a.h)
                        .round()
                        .total_cmp(&(b.y + b.h).round())
                        .then(b.x.total_cmp(&a.x))
                })
                .map_or(Arrow::Stay, |(i, _)| Arrow::To(*i, false)),
            Key::Up | Key::Left => rank
                .checked_sub(1)
                .map_or(Arrow::Stay, |r| Arrow::To(rows[r], true)),
            _ => rows
                .get(rank + 1)
                .map_or(Arrow::Stay, |&i| Arrow::To(i, true)),
        };
    }
    let Some(cur) = current else {
        return Arrow::Linear;
    };
    // Express every rectangle so the arrow points along +main.
    let project = |a: &Area| -> (f64, f64, f64, f64) {
        match key {
            Key::Right => (a.x, a.x + a.w, a.y, a.y + a.h),
            Key::Left => (-(a.x + a.w), -a.x, a.y, a.y + a.h),
            Key::Down => (a.y, a.y + a.h, a.x, a.x + a.w),
            _ => (-(a.y + a.h), -a.y, a.x, a.x + a.w),
        }
    };
    let (_, c_hi, c_lo_p, c_hi_p) = project(cur);
    let centre = (c_lo_p + c_hi_p) / 2.;
    let best = tiles
        .iter()
        .filter(|(i, _)| *i != selected)
        .filter_map(|(i, a)| {
            let (lo, _, lo_p, hi_p) = project(a);
            let gap = lo - c_hi;
            if gap < -0.5 {
                return None;
            }
            let overlap = hi_p.min(c_hi_p) - lo_p.max(c_lo_p);
            let score = if overlap > 0.5 {
                // Facing blocks: nearest first, then the one on the focused
                // block's centre line, then the widest shared span.
                (
                    0,
                    gap.max(0.).round(),
                    u8::from(!(lo_p <= centre && centre <= hi_p)),
                    -overlap,
                )
            } else {
                (1, (gap.max(0.) - overlap).round(), 0, 0.)
            };
            Some((score, *i))
        })
        .min_by(|(a, _), (b, _)| {
            a.0.cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
                .then(a.3.total_cmp(&b.3))
        });
    match best {
        Some((_, i)) => Arrow::To(i, false),
        None if key == Key::Down => rows
            .iter()
            .find(|&&i| i != selected)
            .map_or(Arrow::Stay, |&i| Arrow::To(i, true)),
        None => Arrow::Stay,
    }
}
/// Shared keyboard/wheel/button scroll geometry; no filesystem work or new scan.
pub fn reveal(m: &mut Model) {
    m.scroll = m.space_geometry.reveal_scroll(m.selected, m.scroll);
}
pub fn scroll(m: &mut Model, rows: i32) {
    m.scroll = m
        .space_geometry
        .clamp_scroll(m.scroll.saturating_add(rows.saturating_mul(28)));
}
fn text(
    p: &mut dyn Painter,
    r: Rect,
    s: &str,
    color: files_core::theme::Color,
    size: i32,
    bold: bool,
) {
    p.text(r, s, size, color, bold, false);
}
fn button(p: &mut dyn Painter, m: &mut Model, r: Rect, label: &str, a: Action) {
    p.rounded_rect(r, 6 * m.radius, m.palette.lighter_background);
    text(
        p,
        Rect::new(r.x + 8, r.y, r.w - 16, r.h),
        label,
        m.palette.accent,
        12,
        false,
    );
    m.hits.push(Hit {
        rect: r,
        action: a,
        generation: m.generation,
    });
}
pub fn paint(p: &mut dyn Painter, m: &mut Model, body: Rect) {
    use crate::space_map_state::View;
    m.content_height = body.h;
    let state = match m.space_map.view() {
        View::Closed => "closed",
        View::Loading => "loading",
        View::Ready(_) => "ready",
        View::Error(_) => "error",
    };
    let stamp = Some((m.space_map.revision(), m.generation, state, body.w, body.h));
    m.space_geometry.painted = stamp;
    let c = m.palette.clone();
    let (title, total) = match m.space_map.view() {
        View::Ready(report) => {
            let bytes = report
                .nodes
                .iter()
                .filter(|n| m.hidden_visible() || n.name.first() != Some(&b'.'))
                .filter_map(|n| n.bytes)
                .sum::<u64>();
            (
                format!(
                    "Espacio · {} visibles{}",
                    format_size(bytes),
                    if report.complete { "" } else { " · parcial" }
                ),
                format!(
                    "Total aparente {}{} · incl. ocultos",
                    if report.complete { "" } else { "≥" },
                    format_size(report.observed_bytes)
                ),
            )
        }
        View::Loading => ("Espacio · calculando…".into(), String::new()),
        View::Error(_) => ("Espacio · no disponible".into(), String::new()),
        View::Closed => (
            if m.space_requested {
                "Espacio · esperando…"
            } else {
                "Espacio · detenido"
            }
            .into(),
            String::new(),
        ),
    };
    text(
        p,
        Rect::new(body.x, body.y + 2, (body.w - 194).max(1), 28),
        &title,
        c.foreground,
        13,
        true,
    );
    button(
        p,
        m,
        Rect::new(body.x + (body.w - 186).max(0), body.y, 184, 28),
        if m.hidden_visible() {
            "  Ocultar ocultos"
        } else {
            "  Mostrar ocultos"
        },
        Action::ToggleHidden,
    );
    let y = body.y + 34;
    if let Some(parent) = m.path.parent().map(|p| p.to_path_buf()) {
        button(
            p,
            m,
            Rect::new(body.x, y, 82, 28),
            "  Volver",
            Action::Navigate(parent),
        );
    }
    button(
        p,
        m,
        Rect::new(body.x + 88, y, 112, 28),
        " Recalcular",
        Action::SpaceRefresh,
    );
    let controls = if !m.space_map.is_drained() {
        button(
            p,
            m,
            Rect::new(body.x + 206, y, 36, 28),
            "",
            Action::SpaceCancel,
        );
        248
    } else {
        206
    };
    text(
        p,
        Rect::new(body.x + controls, y, (body.w - controls).max(0), 28),
        &total,
        c.dark_foreground,
        10,
        false,
    );
    let status = if m.space_map.watch_gap() {
        "Vigilancia interrumpida · F5 recalcula"
    } else if m.space_map.changed() {
        "Cambios detectados · F5 recalcula"
    } else {
        "Observación · F5 o Recalcular actualiza"
    };
    text(
        p,
        Rect::new(body.x, body.y + 66, (body.w - 74).max(0), 18),
        status,
        if m.space_map.changed() || m.space_map.watch_gap() {
            c.yellow
        } else {
            c.dark_foreground
        },
        10,
        false,
    );
    text(
        p,
        Rect::new(body.x, body.y + 88, body.w, 18),
        "Subcarpetas sin vigilancia · no es una instantánea",
        c.dark_foreground,
        10,
        false,
    );
    button(
        p,
        m,
        Rect::new(body.x + (body.w - 66).max(0), body.y + 64, 66, 24),
        "Marcas",
        Action::SelectionMenu(body.x + body.w, body.y + body.h),
    );
    let map_y = body.y + 108;
    let View::Ready(report) = m.space_map.view() else {
        // The scan of a freshly opened folder is asynchronous: the zoom
        // starts anyway, on the retained layouts, and never waits for it.
        let full = Rect::new(
            body.x,
            map_y,
            body.w.max(1),
            (body.y + body.h - map_y - 26).max(24),
        );
        if let Some(plan) = m.space_zoom.plan(&m.path, full, false) {
            if plan.moving {
                p.clip(full);
                m.space_zoom.paint_back(p, &c, m.radius, &plan, false);
                m.space_zoom.paint_front(p, &c, m.radius, &plan);
                p.unclip();
                return;
            }
            if plan.outline && !matches!(m.space_map.view(), View::Error(_)) {
                m.space_zoom.paint_outline(p, &c, m.radius, full);
            }
        }
        let message = match m.space_map.view() {
            View::Error(e) => e.as_str(),
            View::Loading => "Calculando tamaños…",
            _ => "Cálculo detenido · Actualizar para volver a calcular",
        };
        text(
            p,
            Rect::new(body.x, map_y, body.w, 50),
            message,
            c.dark_foreground,
            12,
            false,
        );
        if matches!(m.space_map.view(), View::Loading) {
            let size = 72.min(body.w).min((body.y + body.h - map_y - 60).max(0));
            let pointer = m.pointer;
            m.omi.paint(
                p,
                Rect::new(body.x + (body.w - size) / 2, map_y + 60, size, size),
                c.accent,
                pointer,
                std::time::Instant::now(),
            );
        }
        return;
    };
    let geometry_changed = m.space_geometry.prepare(
        report,
        &m.entries,
        m.generation,
        m.space_map.revision(),
        (body.w.max(1), body.y + body.h - map_y),
        m.hidden_visible(),
    );
    let map = Rect::new(body.x, map_y, body.w.max(1), m.space_geometry.map_height);
    let list_top = map.y + map.h + 32;
    m.space_geometry.list_rect = Rect::new(
        body.x,
        list_top,
        body.w,
        (body.y + body.h - list_top - 24).max(0),
    );
    m.scroll = if geometry_changed {
        m.space_geometry.reveal_scroll(m.selected, m.scroll)
    } else {
        m.space_geometry.clamp_scroll(m.scroll)
    };
    m.space_geometry.painted = stamp;
    if let Some(error) = &m.space_geometry.error {
        text(p, map, error, c.red, 12, false);
    } else if m.space_geometry.areas.is_empty() {
        text(
            p,
            map,
            if report.nodes.is_empty() && report.complete {
                "Carpeta vacía · 0 B"
            } else {
                "Sin bytes completos positivos para representar"
            },
            c.dark_foreground,
            13,
            false,
        );
    }
    // Retain this layout for later zooms, then apply any pending return
    // focus now that the tiles it needs exist (Backspace / Alt+Up).
    m.space_zoom.remember(
        &m.path,
        (m.space_map.revision(), m.generation),
        map,
        &m.space_geometry.areas,
        report,
    );
    if m.pump_parent_return_focus() {
        reveal(m);
    }
    let View::Ready(report) = m.space_map.view() else {
        unreachable!("space map stayed ready during paint");
    };
    let plan = m.space_zoom.plan(&m.path, map, true);
    #[cfg(test)]
    m.space_geometry.drawn.clear();
    p.clip(map);
    if let Some(plan) = &plan {
        m.space_zoom.paint_back(p, &c, m.radius, plan, true);
    }
    let mut tile_focus = false;
    // Tiles a tooltip may describe: (node, final rect).
    let mut hover_tip: Option<(usize, Rect)> = None;
    let mut focus_tip: Option<(usize, Rect)> = None;
    for area in &m.space_geometry.areas {
        let node = &report.nodes[area.node];
        let x0 = area.x.round() as i32;
        let x1 = (area.x + area.w).round() as i32;
        let y0 = area.y.round() as i32;
        let y1 = (area.y + area.h).round() as i32;
        let r = Rect::new(map.x + x0, map.y + y0, x1 - x0, y1 - y0);
        if r.w <= 0 || r.h <= 0 {
            continue;
        }
        let entry = m.space_geometry.mapped[area.node];
        let marks = Marks {
            selected: entry.is_some_and(|i| m.selection.contains(&i)),
            hovered: entry.is_some_and(|i| m.hovered == Some(i)),
            focused: entry.is_some_and(|i| m.selected == Some(i)),
        };
        if marks.hovered {
            hover_tip = Some((area.node, r));
        }
        if marks.focused {
            focus_tip = Some((area.node, r));
        }
        // Hits and keyboard targets always use the final rectangle; only the
        // painted rectangle follows the zoom.
        let painted = plan.as_ref().map_or(r, |plan| plan.live(r));
        #[cfg(test)]
        m.space_geometry.drawn.push((entry, painted));
        draw_tile(
            p,
            &c,
            m.radius,
            painted,
            TileInfo {
                name: &node.name,
                kind: node.kind,
                bytes: node.bytes,
                files: node.files,
            },
            marks,
        );
        if marks.focused {
            tile_focus = true;
            let ring = Rect::new(
                painted.x + 1,
                painted.y + 1,
                (painted.w - 2).max(0),
                (painted.h - 2).max(0),
            );
            if plan.as_ref().is_some_and(|plan| plan.transforms()) {
                // Mid-zoom the ring rides the tile instead of gliding behind it.
                crate::motion::outline(p, ring, 7 * m.radius, 2, c.accent);
            } else {
                m.motion.focus(
                    (m.generation, crate::model::View::SpaceMap as u64),
                    ring,
                    (0, 0),
                    7 * m.radius,
                    c.accent,
                );
            }
        }
        if let Some(i) = entry {
            m.hits.push(Hit {
                rect: r,
                action: Action::Entry(i),
                generation: m.generation,
            });
            m.visible.push(i);
        }
    }
    if let Some(plan) = &plan {
        m.space_zoom.paint_front(p, &c, m.radius, plan);
    }
    // A focused tile's ring stays inside the map clip, like List keeps it
    // under its pinned header; a focused row queues its ring below.
    if tile_focus {
        m.motion.flush(p);
    }
    p.unclip();
    let list_y = map.y + map.h + 8;
    if !m.space_geometry.excluded.is_empty() {
        let overflow = m.space_geometry.max_scroll() > 0;
        text(
            p,
            Rect::new(
                body.x,
                list_y,
                (body.w - if overflow { 68 } else { 0 }).max(0),
                22,
            ),
            &format!(
                "Sin área/etiqueta · {} · rueda / ↑↓",
                m.space_geometry.excluded.len()
            ),
            c.dark_foreground,
            10,
            false,
        );
        if overflow {
            for (offset, direction, label) in [(64, -1, "↑"), (30, 1, "↓")] {
                let rect = Rect::new(body.x + body.w - offset, list_y, 28, 22);
                p.rounded_rect(rect, 4, c.lighter_background);
                text(p, rect, label, c.foreground, 12, false);
                m.hits.push(Hit {
                    rect,
                    action: Action::SpaceListScroll(direction * m.space_geometry.page_rows()),
                    generation: m.generation,
                });
            }
        }
        let clip = m.space_geometry.list_rect;
        let list_top = clip.y;
        let list_h = clip.h;
        let scroll = m.scroll;
        m.content_height = body.h + m.space_geometry.max_scroll();
        p.clip(clip);
        for (rank, &node) in m.space_geometry.excluded.iter().enumerate() {
            let y = list_top + rank as i32 * 28 - scroll;
            if y < list_top || y + 26 > list_top + list_h {
                continue;
            }
            let n = &report.nodes[node];
            let state = match (n.status, n.bytes) {
                (NodeStatus::Complete, Some(0)) => "0 B · sin área".into(),
                (NodeStatus::Complete, Some(b)) => {
                    format!("{} · etiqueta en lista", format_size(b))
                }
                (NodeStatus::Partial, _) => format!("PARCIAL ≥{}", format_size(n.observed_bytes)),
                (NodeStatus::Error, _) => "ERROR · tamaño desconocido".into(),
                _ => "Tamaño desconocido".into(),
            };
            let row = Rect::new(body.x, y, body.w, 26);
            let index = m.space_geometry.mapped[node];
            if index.is_some_and(|i| m.selection.contains(&i)) {
                p.rect(row, c.selection);
            }
            if index.is_some_and(|i| m.selected == Some(i)) {
                p.rect(Rect::new(row.x + 1, row.y + 4, 2, row.h - 8), c.accent);
                m.motion.focus(
                    (m.generation, crate::model::View::SpaceMap as u64),
                    row,
                    (0, scroll),
                    4,
                    c.accent,
                );
            }
            text(
                p,
                Rect::new(body.x + 6, y, body.w - 12, 26),
                &format!("{} · {state}", files_core::name_label::bytes_label(&n.name)),
                c.foreground,
                11,
                false,
            );
            if let Some(i) = index {
                m.hits.push(Hit {
                    rect: row,
                    action: Action::Entry(i),
                    generation: m.generation,
                });
                m.visible.push(i);
            }
        }
        p.unclip();
    }
    let footer = report.reason.as_deref().unwrap_or(if report.complete {
        "Las carpetas incluyen descendientes ocultos"
    } else {
        "Recorrido parcial: total de áreas excluye tamaños desconocidos"
    });
    text(
        p,
        Rect::new(body.x, body.y + body.h - 22, body.w, 22),
        footer,
        if report.complete {
            c.dark_foreground
        } else {
            c.yellow
        },
        10,
        false,
    );
    // Pointer hover always explains its block; keyboard focus does so only
    // when the block is too small to show its own name and size, and never
    // while a zoom is still moving the tiles.
    let zooming = plan.as_ref().is_some_and(|plan| plan.transforms());
    let tip = hover_tip.map(|t| (t, m.pointer)).or_else(|| {
        focus_tip
            .filter(|&(node, r)| !zooming && !label_fits(p, r, &report.nodes[node].name))
            .map(|t| (t, None))
    });
    if let Some(((node, r), pointer)) = tip {
        let lines = tip_lines(&report.nodes[node], report.total_bytes);
        let bounds = Rect::new(body.x, body.y, body.w, body.h);
        paint_tip(p, &c, m.radius, &lines, r, pointer, bounds);
    }
}
/// Whether tile `r` shows the whole name and its size row (see `draw_tile`).
fn label_fits(p: &mut dyn Painter, r: Rect, name: &[u8]) -> bool {
    if r.w < 65 || r.h < 45 {
        return false;
    }
    let label = files_core::name_label::bytes_label(name);
    // Leave room for the small glyph prefix drawn when no icon fits.
    let room = r.w - 20 - if icon_layout(r).is_some() { 0 } else { 26 };
    p.text_width(&label, 13, true).ceil() as i32 <= room
}
/// Tooltip rows of a block: full name, kind and size, share of the folder.
pub(crate) fn tip_lines(n: &space_map::SpaceNode, total: u64) -> Vec<String> {
    let name = files_core::name_label::bytes_label(&n.name).into_owned();
    let kind: std::borrow::Cow<'_, str> = match n.kind {
        NodeKind::Directory => "Carpeta".into(),
        NodeKind::Symlink => "Enlace".into(),
        NodeKind::Other => "Especial".into(),
        NodeKind::File => crate::view::kind_label(
            false,
            std::path::Path::new(&*String::from_utf8_lossy(&n.name)),
        ),
    };
    let size = match (n.status, n.bytes) {
        (NodeStatus::Complete, Some(b)) => format_size(b),
        (NodeStatus::Partial, _) => format!("parcial ≥{}", format_size(n.observed_bytes)),
        (NodeStatus::Error, _) => "tamaño desconocido (error)".into(),
        _ => "tamaño desconocido".into(),
    };
    let mut detail = format!("{kind} · {size}");
    if n.kind == NodeKind::Directory {
        detail.push_str(&format!(
            " · {} archivo{} · {} carpeta{}",
            n.files,
            if n.files == 1 { "" } else { "s" },
            n.directories,
            if n.directories == 1 { "" } else { "s" }
        ));
    }
    let mut lines = vec![name, detail];
    if let (Some(b), true) = (n.bytes, total > 0) {
        let share = b as f64 * 100.0 / total as f64;
        lines.push(format!(
            "{} de esta carpeta{}",
            if share < 0.1 {
                "<0,1 %".to_string()
            } else {
                format!("{:.1} %", share).replace('.', ",")
            },
            if n.kind == NodeKind::Directory {
                " · Enter entra"
            } else {
                " · Enter abre"
            }
        ));
    }
    if let Some(error) = &n.error {
        lines.push(error.clone());
    }
    lines
}
/// Canvas tooltip next to the pointer (hover) or below/above the block
/// (keyboard), kept inside `bounds`. It has no hit target.
fn paint_tip(
    p: &mut dyn Painter,
    c: &files_core::theme::Palette,
    radius: i32,
    lines: &[String],
    tile: Rect,
    pointer: Option<(i32, i32)>,
    bounds: Rect,
) {
    let max = (bounds.w - 16).clamp(120, 420);
    let fitted: Vec<String> = lines
        .iter()
        .map(|l| {
            crate::properties_dialog::fit_middle(l, max - 24, &mut |t| p.text_width(t, 12, true))
        })
        .collect();
    let width = fitted
        .iter()
        .map(|l| p.text_width(l, 12, true).ceil() as i32)
        .max()
        .unwrap_or(0)
        .saturating_add(24)
        .min(max);
    let height = fitted.len() as i32 * 20 + 12;
    let (ax, ay, below) = match pointer.filter(|&(x, y)| tile.contains(x, y)) {
        Some((x, y)) => (x + 14, y + 20, y + 20 + height <= bounds.y + bounds.h),
        None => (
            tile.x + tile.w / 2 - width / 2,
            tile.y + tile.h + 6,
            tile.y + tile.h + 6 + height <= bounds.y + bounds.h,
        ),
    };
    let y = if below {
        ay
    } else if pointer.is_some_and(|(x, y)| tile.contains(x, y)) {
        ay - 40 - height
    } else {
        tile.y - 6 - height
    };
    let rect = Rect::new(
        ax.clamp(
            bounds.x + 8,
            (bounds.x + bounds.w - width - 8).max(bounds.x + 8),
        ),
        y.clamp(
            bounds.y + 8,
            (bounds.y + bounds.h - height - 8).max(bounds.y + 8),
        ),
        width,
        height,
    );
    p.rounded_rect(rect, 5 * radius, c.lighter_background);
    p.rounded_rect(
        Rect::new(rect.x + 1, rect.y + 1, rect.w - 2, rect.h - 2),
        5 * radius,
        c.darker_background,
    );
    for (i, line) in fitted.iter().enumerate() {
        p.text(
            Rect::new(rect.x + 12, rect.y + 6 + i as i32 * 20, rect.w - 24, 20),
            line,
            12,
            if i == 0 {
                c.foreground
            } else {
                c.dark_foreground
            },
            true,
            false,
        );
    }
}
#[derive(Clone, Copy, Default)]
struct Marks {
    selected: bool,
    hovered: bool,
    focused: bool,
}
#[derive(Clone, Copy)]
struct TileInfo<'a> {
    name: &'a [u8],
    kind: NodeKind,
    bytes: Option<u64>,
    files: u64,
}
/// Stable block colour of a name (the same folder keeps its colour in the
/// parent and while it zooms).
pub(crate) fn tile_fill(c: &files_core::theme::Palette, name: &[u8]) -> files_core::theme::Color {
    let colors = [c.blue, c.cyan, c.magenta, c.green, c.yellow];
    let hash = name
        .iter()
        .fold(0usize, |v, b| v.wrapping_mul(31).wrapping_add(*b as usize));
    colors[hash % colors.len()]
}
/// Plain tile of a retained layout (no marks), drawn during a zoom.
pub(crate) fn paint_tile(
    p: &mut dyn Painter,
    c: &files_core::theme::Palette,
    radius: i32,
    r: Rect,
    t: &crate::space_map_zoom::Tile,
) {
    draw_tile(
        p,
        c,
        radius,
        r,
        TileInfo {
            name: &t.name,
            kind: t.kind,
            bytes: t.bytes,
            files: t.files,
        },
        Marks::default(),
    );
}
/// Rows of a labelled tile: name at the top, size at the bottom.
const NAME_BOTTOM: i32 = 31;
const SIZE_TOP: i32 = 32;
/// Smallest and largest type icon drawn between the name and size rows.
pub(crate) const ICON_MIN: i32 = 18;
pub(crate) const ICON_MAX: i32 = 64;
/// The type icon's rectangle and font size inside tile `r`, or `None` when
/// the tile cannot hold it between its name and size rows (then a small
/// glyph prefixes the name instead, when the tile is labelled at all).
pub(crate) fn icon_layout(r: Rect) -> Option<(Rect, i32)> {
    if r.w < 65 || r.h < 45 {
        return None;
    }
    let top = r.y + NAME_BOTTOM;
    let bottom = (r.y + r.h - SIZE_TOP).max(r.y + 30);
    let height = bottom - top - 4;
    let width = r.w - 20;
    let size = ((height.min(width) as f64) * 0.8).round() as i32;
    (size >= ICON_MIN).then(|| {
        let size = size.min(ICON_MAX);
        (Rect::new(r.x + 10, top + 2, width, height), size)
    })
}
fn draw_tile(
    p: &mut dyn Painter,
    c: &files_core::theme::Palette,
    radius: i32,
    r: Rect,
    t: TileInfo<'_>,
    marks: Marks,
) {
    if r.w <= 0 || r.h <= 0 {
        return;
    }
    let fill = tile_fill(c, t.name);
    let fg = readable(fill, c);
    let tile = Rect::new(r.x + 1, r.y + 1, (r.w - 2).max(0), (r.h - 2).max(0));
    // Marks (and hover) are the light band; keyboard focus is the accent
    // ring drawn over the outer gap, so focus alone never looks marked.
    if marks.selected || marks.hovered {
        p.rounded_rect(tile, 7 * radius, fg);
    }
    let gap = if marks.focused {
        5
    } else if marks.selected || marks.hovered {
        3
    } else {
        1
    };
    let inner = Rect::new(
        r.x + gap,
        r.y + gap,
        (r.w - 2 * gap).max(0),
        (r.h - 2 * gap).max(0),
    );
    p.rounded_rect(inner, 6 * radius, fill);
    p.clip(inner);
    if r.w >= 65 && r.h >= 45 {
        let name = files_core::name_label::bytes_label(t.name);
        let directory = t.kind == NodeKind::Directory;
        let (glyph, _) = crate::grid_view::type_glyph(
            directory,
            std::path::Path::new(&*String::from_utf8_lossy(t.name)),
            c,
        );
        let icon = icon_layout(r);
        let prefix = crate::fonts::icon(glyph, "");
        let title = if icon.is_some() || prefix.is_empty() {
            name.into_owned()
        } else {
            format!("{prefix}  {name}")
        };
        text(
            p,
            Rect::new(r.x + 10, r.y + 7, r.w - 20, 24),
            &title,
            fg,
            13,
            true,
        );
        if let Some((rect, size)) = icon {
            // Same glyph as Grid, in the label colour so it keeps contrast
            // on every block colour.
            let glyph = crate::fonts::icon(glyph, if directory { "DIR" } else { "FILE" });
            let width = (p.text_width(glyph, size, true).ceil() as i32)
                .max(size)
                .min(rect.w);
            p.text(
                Rect::new(rect.x + (rect.w - width) / 2, rect.y, width, rect.h),
                glyph,
                size,
                fg,
                true,
                false,
            );
        }
        let size = if directory && r.w > 220 {
            format!(
                "{} · {} {}",
                format_size(t.bytes.unwrap_or(0)),
                t.files,
                if t.files == 1 { "archivo" } else { "archivos" }
            )
        } else {
            format_size(t.bytes.unwrap_or(0))
        };
        text(
            p,
            Rect::new(r.x + 10, (r.y + r.h - SIZE_TOP).max(r.y + 30), r.w - 20, 24),
            &size,
            fg,
            12,
            false,
        );
    }
    p.unclip();
}
fn luminance(c: files_core::theme::Color) -> f64 {
    let channel = |v: u8| {
        let n = v as f64 / 255.;
        if n <= 0.04045 {
            n / 12.92
        } else {
            ((n + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(c.0) + 0.7152 * channel(c.1) + 0.0722 * channel(c.2)
}
fn contrast(a: files_core::theme::Color, b: files_core::theme::Color) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
fn readable(
    fill: files_core::theme::Color,
    p: &files_core::theme::Palette,
) -> files_core::theme::Color {
    let best = if contrast(fill, p.darker_background) > contrast(fill, p.bright_foreground) {
        p.darker_background
    } else {
        p.bright_foreground
    };
    if contrast(fill, best) >= 4.5 {
        best
    } else {
        let black = files_core::theme::Color(0, 0, 0);
        let white = files_core::theme::Color(255, 255, 255);
        if contrast(fill, black) > contrast(fill, white) {
            black
        } else {
            white
        }
    }
}

pub fn inspector_details(m: &Model, index: usize) -> Option<String> {
    let crate::space_map_state::View::Ready(report) = m.space_map.view() else {
        return None;
    };
    if m.space_map.path() != Some(m.path.as_path()) {
        return None;
    }
    let entry = m.entries.get(index)?;
    if !files_core::visibility::accepts(&entry.name, m.hidden_visible()) {
        return None;
    }
    let n = report
        .nodes
        .iter()
        .find(|n| n.name == entry.name.as_encoded_bytes())?;
    Some(match (n.status, n.bytes) {
        (NodeStatus::Complete, Some(bytes)) => format!(
            "Aparente {}\n{} archivos · {} carpetas\n{}",
            format_size(bytes),
            n.files,
            n.directories,
            if matches!(n.kind, NodeKind::Symlink | NodeKind::Other) {
                "Enlace/especial excluido"
            } else {
                "Medición completa, no snapshot"
            }
        ),
        _ => format!(
            "{:?} · ≥{} observado\n{}",
            n.status,
            format_size(n.observed_bytes),
            n.error.as_deref().unwrap_or("Tamaño completo desconocido")
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use files_core::{
        listing::{Entry, Kind},
        space_map::SpaceNode,
    };
    fn fixture() -> (SpaceReport, Vec<Entry>) {
        let nodes = [("photos", 8), ("docs", 4), ("video", 2), ("empty", 0)]
            .into_iter()
            .map(|(n, b)| SpaceNode {
                name: n.as_bytes().into(),
                kind: NodeKind::File,
                bytes: Some(b),
                observed_bytes: b,
                files: 1,
                directories: 0,
                status: NodeStatus::Complete,
                error: None,
            })
            .collect::<Vec<_>>();
        let entries = nodes
            .iter()
            .rev()
            .map(|n| Entry {
                name: std::ffi::OsString::from(std::str::from_utf8(&n.name).unwrap()),
                kind: Kind::File,
                metadata: None,
                stat_attempted: false,
                link: None,
            })
            .collect();
        (
            SpaceReport {
                nodes,
                total_bytes: 14,
                observed_bytes: 14,
                visited: 4,
                complete: true,
                reason: None,
                omitted_children: Some(0),
            },
            entries,
        )
    }
    fn selection_model() -> Model {
        let (mut report, mut entries) = fixture();
        report.complete = false;
        report.omitted_children = Some(1);
        entries.push(Entry {
            name: "outside-report".into(),
            kind: Kind::File,
            metadata: None,
            stat_attempted: false,
            link: None,
        });
        let mut m = Model::new(std::env::temp_dir());
        m.view = crate::model::View::SpaceMap;
        m.entries = entries;
        m.order = (0..m.entries.len()).collect();
        m.loaded = true;
        m.query = "not-a-map-filter".into();
        m.space_map = crate::space_map_state::State::ready_for_test(m.path.clone(), report.clone());
        m.space_geometry.prepare(
            &report,
            &m.entries,
            m.generation,
            m.space_map.revision(),
            (356, 198),
            false,
        );
        m
    }
    #[test]
    fn map_bulk_marks_use_projection_and_preserve_ignored_query() {
        use crate::selection_tools::Command;
        let mut m = selection_model();
        m.selection_command(Command::All);
        assert_eq!(m.selection.len(), 4);
        assert!(!m.selection.contains(&4));
        assert_eq!(m.query, "not-a-map-filter");
        m.selection_command(Command::Invert);
        assert!(m.selection.is_empty());
        assert_eq!(m.selected, None);
        assert!(m.preserve_empty_selection);
        m.selection.insert(4);
        m.selection_command(Command::Invert);
        assert_eq!(m.selection.len(), 4);
        assert!(!m.selection.contains(&4));
        let marks = m.selection.clone();
        m.selection_command(Command::Filtered);
        assert_eq!(m.selection, marks);
        assert_eq!(m.query, "not-a-map-filter");
        assert!(m.error.contains("no aplica el filtro"));
    }
    #[test]
    fn map_stale_projection_blocks_marking_but_clear_survives_close() {
        use crate::selection_tools::Command;
        let mut m = selection_model();
        m.selection.clear();
        m.show_hidden = true;
        m.selection_command(Command::All);
        assert!(m.selection.is_empty());
        assert!(m.error.contains("proyección vigente"));
        m.show_hidden = false;
        m.generation += 1;
        m.mark_and_advance();
        assert!(m.selection.is_empty());
        m.selection.insert(0);
        m.space_map.close();
        m.space_geometry.clear();
        m.selection_command(Command::Clear);
        assert!(m.selection.is_empty());
        assert_eq!(m.selected, None);
    }
    #[test]
    fn map_menu_and_palette_reject_hidden_projection_changes() {
        use crate::selection_tools::Command;
        let mut m = selection_model();
        m.selection.clear();
        crate::context_menu::open_selection(&mut m, 440, 440);
        let menu = m.context_menu.as_ref().unwrap();
        assert!(
            menu.items
                .iter()
                .any(|i| i.label.contains("Filtro no aplicado") && i.action.is_none())
        );
        let all = menu
            .items
            .iter()
            .position(|i| matches!(i.action, Some(Action::Selection(Command::All))))
            .unwrap();
        m.show_hidden = true;
        crate::context_menu::activate(&mut m, all);
        assert!(m.selection.is_empty());
        assert!(m.error.contains("mapa cambió"));
        m.show_hidden = false;
        m.open_commands();
        let all = crate::command_palette::ITEMS
            .iter()
            .position(|s| s.id == "select-all")
            .unwrap();
        assert!(m.commands.as_ref().unwrap().reason(&m, all).is_none());
        m.show_hidden = true;
        assert!(
            m.commands
                .as_ref()
                .unwrap()
                .reason(&m, all)
                .unwrap()
                .contains("mapa cambió")
        );
        m.commands = None;
    }
    #[test]
    fn map_insert_advances_in_projection_order_and_stops_at_last() {
        let mut m = selection_model();
        let order = m.space_geometry.navigation_indices().to_vec();
        m.selection.clear();
        m.selected = Some(order[0]);
        m.mark_and_advance();
        assert!(m.selection.contains(&order[0]));
        assert_eq!(m.selected, Some(order[1]));
        m.selected = order.last().copied();
        m.mark_and_advance();
        assert_eq!(m.selected, order.last().copied());
        assert!(m.selection.contains(order.last().unwrap()));
    }
    #[test]
    fn projection_keeps_byte_ratios_entry_identity_and_zero_out_of_areas() {
        let (report, entries) = fixture();
        let mut geometry = Geometry::default();
        for (w, h) in [(800, 400), (320, 700)] {
            geometry.rebuild(&report, &entries, 4, 7, (w, h), false);
            assert_eq!(geometry.areas.len(), 3);
            assert_eq!(geometry.excluded, vec![3]);
            for a in &geometry.areas {
                let expected =
                    w as f64 * h as f64 * report.nodes[a.node].bytes.unwrap() as f64 / 14.;
                assert!((a.w * a.h - expected).abs() < 1e-6);
                assert_eq!(geometry.mapped[a.node], Some(3 - a.node));
            }
            assert_eq!(geometry.navigation_indices().len(), 4);
        }
        geometry.clear();
        assert!(
            geometry.areas.is_empty() && geometry.mapped.is_empty() && geometry.painted.is_none()
        );
    }
    #[test]
    fn refresh_generation_rebuilds_mapping_without_stale_entry_targets() {
        let (report, mut entries) = fixture();
        let mut geometry = Geometry::default();
        geometry.rebuild(&report, &entries, 1, 1, (800, 400), false);
        entries.reverse();
        geometry.rebuild(&report, &entries, 2, 1, (800, 400), false);
        assert_eq!(geometry.mapped, vec![Some(0), Some(1), Some(2), Some(3)]);
        entries.clear();
        geometry.rebuild(&report, &entries, 3, 1, (800, 400), false);
        assert!(geometry.navigation_indices().is_empty());
    }
    #[test]
    fn hidden_projection_has_no_targets_but_preserves_recursive_totals() {
        let (mut report, mut entries) = fixture();
        report.nodes[0].name = b".photos".to_vec();
        entries[3].name = ".photos".into();
        let mut geometry = Geometry::default();
        geometry.rebuild(&report, &entries, 1, 1, (600, 300), false);
        assert_eq!(geometry.areas.len(), 2);
        assert_eq!(geometry.mapped[0], None);
        assert!(!geometry.navigation_indices().contains(&3));
        assert_eq!(report.total_bytes, 14);
        for a in &geometry.areas {
            assert!(
                (a.w * a.h - 600. * 300. * report.nodes[a.node].bytes.unwrap() as f64 / 6.).abs()
                    < 1e-7
            );
        }
        geometry.rebuild(&report, &entries, 1, 1, (600, 300), true);
        assert_eq!(geometry.areas.len(), 3);
        assert!(geometry.navigation_indices().contains(&3));
    }
    #[test]
    fn label_contrast_remains_readable_on_both_light_and_dark_fills() {
        let palette = files_core::theme::Palette::default();
        for r in (0..=255).step_by(51) {
            for g in (0..=255).step_by(51) {
                for b in (0..=255).step_by(51) {
                    let fill = files_core::theme::Color(r, g, b);
                    assert!(contrast(fill, readable(fill, &palette)) >= 4.5);
                }
            }
        }
    }
    fn extreme_fixture() -> (SpaceReport, Vec<Entry>) {
        let (mut report, _) = fixture();
        report.nodes.clear();
        for i in 0..14 {
            let bytes = if i == 0 { 1u64 << 40 } else { 1 };
            report.nodes.push(SpaceNode {
                name: format!("entry-{i:02}").into_bytes(),
                kind: NodeKind::File,
                bytes: Some(bytes),
                observed_bytes: bytes,
                files: 1,
                directories: 0,
                status: NodeStatus::Complete,
                error: None,
            });
        }
        report.total_bytes = (1u64 << 40) + 13;
        report.observed_bytes = report.total_bytes;
        report.visited = 14;
        let entries = report
            .nodes
            .iter()
            .rev()
            .map(|n| Entry {
                name: std::ffi::OsString::from(std::str::from_utf8(&n.name).unwrap()),
                kind: Kind::File,
                metadata: None,
                stat_attempted: false,
                link: None,
            })
            .collect();
        (report, entries)
    }
    #[test]
    fn layout_failure_keeps_measured_entries_in_a_usable_fallback_list() {
        let (report, entries) = fixture();
        let mut geometry = Geometry::default();
        // The pure layout explicitly rejects dimensions beyond its numeric
        // contract. The view must retain every file instead of losing targets.
        assert!(geometry.prepare(&report, &entries, 1, 2, (20_000, 300), false));
        assert!(geometry.error.is_some());
        assert!(geometry.areas.is_empty());
        assert_eq!(geometry.excluded.len(), report.nodes.len());
        assert_eq!(geometry.navigation_indices().len(), entries.len());
        assert!(geometry.map_height <= 50);
        assert_eq!(report.total_bytes, 14);
        assert!(
            entries
                .iter()
                .enumerate()
                .all(|(i, _)| geometry.contains_entry(i))
        );
    }
    #[test]
    fn positive_only_tiny_blocks_get_rows_without_inflating_area_and_cache_navigation() {
        let (report, entries) = extreme_fixture();
        let mut g = Geometry::default();
        assert!(g.prepare(&report, &entries, 1, 2, (356, 198), false));
        assert_eq!(g.excluded.len(), 13);
        assert_eq!(g.navigation_indices().len(), 14);
        g.list_rect = Rect::new(68, 0, 356, 84);
        assert!(g.map_height < 198 - 26);
        assert_eq!(g.max_scroll(), 13 * 28 - 84);
        let sum = g.areas.iter().map(|a| a.w * a.h).sum::<f64>();
        assert!((sum - 356. * f64::from(g.map_height)).abs() < 1e-6);
        for area in &g.areas {
            let fraction = area.w * area.h / sum;
            assert!(
                (fraction
                    - report.nodes[area.node].bytes.unwrap() as f64 / report.total_bytes as f64)
                    .abs()
                    < 1e-12
            );
        }
        let cached = g.navigation_indices().as_ptr();
        assert!(!g.prepare(&report, &entries, 1, 2, (356, 198), false));
        assert_eq!(cached, g.navigation_indices().as_ptr());
        let last = g.excluded_entries().last().unwrap();
        let scroll = g.reveal_scroll(Some(last), 0);
        let rank = g
            .excluded
            .iter()
            .position(|&n| g.mapped[n] == Some(last))
            .unwrap() as i32;
        assert!(rank * 28 >= scroll && rank * 28 + 26 <= scroll + g.list_rect.h);
        assert_eq!(g.clamp_scroll(i32::MAX), g.max_scroll());
        assert_eq!(g.clamp_scroll(i32::MIN), 0);
        g.list_rect.h = 26;
        assert_eq!(g.reveal_scroll(Some(last), 0), rank * 28);
        g.list_rect.h = 25;
        assert_eq!(g.reveal_scroll(Some(last), i32::MAX), 0);
        // The MC strip leaves a 24px map: even the large tile needs a named row.
        assert!(g.prepare(&report, &entries, 1, 2, (356, 162), false));
        assert_eq!(g.map_height, 24);
        assert_eq!(g.excluded.len(), 14);
        g.list_rect.h = 82;
        assert_eq!(g.page_rows(), 3);
    }
    #[test]
    fn keyboard_range_focus_and_scroll_follow_map_order_with_shuffled_entry_indices() {
        fltk_omarchy::anim::set_enabled(false);
        use fltk::enums::Key;
        let (report, entries) = extreme_fixture();
        let mut m = Model::new(std::env::temp_dir());
        m.view = crate::model::View::SpaceMap;
        m.entries = entries;
        m.order = (0..m.entries.len()).collect();
        m.loaded = true;
        m.selected = None;
        m.anchor = None;
        m.selection.clear();
        m.space_geometry.prepare(
            &report,
            &m.entries,
            m.generation,
            m.space_map.revision(),
            (356, 198),
            false,
        );
        m.space_geometry.list_rect = Rect::new(68, 0, 356, 84);
        let order = m.space_geometry.navigation_indices().to_vec();
        assert_ne!(order, m.order);
        crate::keyboard_nav::key(&mut m, Key::End, false, false, 440, 440);
        assert_eq!(m.selected, order.last().copied());
        assert!(m.scroll > 0);
        crate::keyboard_nav::key(&mut m, Key::Home, false, false, 440, 440);
        assert_eq!(m.selection.len(), 1);
        crate::keyboard_nav::key(&mut m, Key::End, false, true, 440, 440);
        assert_eq!(m.selection, order.iter().copied().collect());
        let marks = m.selection.clone();
        crate::keyboard_nav::key(&mut m, Key::Home, true, false, 440, 440);
        assert_eq!(m.selected, Some(order[0]));
        assert_eq!(m.selection, marks);
        m.select_entry(order[3], false, true);
        assert_eq!(m.selection, order[..=3].iter().copied().collect());
        scroll(&mut m, i32::MAX);
        assert_eq!(m.scroll, m.space_geometry.max_scroll());
        scroll(&mut m, i32::MIN);
        assert_eq!(m.scroll, 0);
        m.space_geometry.clear();
        let before = m.selected;
        crate::keyboard_nav::key(&mut m, Key::End, false, false, 440, 440);
        assert_eq!(m.selected, before);
        // Cancel/clear may leave old paint hits with the same listing generation.
        // Neither double-click nor Enter may open an old, unmapped focus then.
        let i = before.unwrap();
        m.entries[i].kind = Kind::Directory;
        m.hits.push(Hit {
            rect: Rect::new(0, 0, 10, 10),
            action: Action::Entry(i),
            generation: m.generation,
        });
        let path = m.path.clone();
        let generation = m.generation;
        m.pointer_down(5, 5, false, false, true);
        m.open_selected();
        assert_eq!(m.path, path);
        assert_eq!(m.generation, generation);
    }
    mod zoom {
        use super::*;
        use crate::model::View;
        use fltk::enums::Key;
        use fltk_omarchy::anim;
        use std::{path::PathBuf, time::Duration};

        #[derive(Default)]
        struct Rec {
            texts: Vec<(Rect, String, i32)>,
            fills: usize,
        }
        impl Painter for Rec {
            fn rect(&mut self, _: Rect, _: files_core::theme::Color) {
                self.fills += 1;
            }
            fn rounded_rect(&mut self, _: Rect, _: i32, _: files_core::theme::Color) {
                self.fills += 1;
            }
            fn text(
                &mut self,
                r: Rect,
                s: &str,
                size: i32,
                _: files_core::theme::Color,
                _: bool,
                _: bool,
            ) {
                self.texts.push((r, s.into(), size));
            }
            fn image(&mut self, _: Rect, _: &crate::painter::Image<'_>) {
                panic!("Espacio never draws images");
            }
            fn clip(&mut self, _: Rect) {}
            fn unclip(&mut self) {}
        }
        fn body() -> Rect {
            Rect::new(0, 0, 800, 600)
        }
        fn node(name: &str, kind: NodeKind, bytes: u64) -> SpaceNode {
            SpaceNode {
                name: name.as_bytes().into(),
                kind,
                bytes: Some(bytes),
                observed_bytes: bytes,
                files: 1,
                directories: 0,
                status: NodeStatus::Complete,
                error: None,
            }
        }
        fn report(nodes: Vec<SpaceNode>) -> SpaceReport {
            let total = nodes.iter().filter_map(|n| n.bytes).sum();
            SpaceReport {
                visited: nodes.len() as u64,
                nodes,
                total_bytes: total,
                observed_bytes: total,
                complete: true,
                reason: None,
                omitted_children: Some(0),
            }
        }
        fn parent_report() -> SpaceReport {
            report(vec![
                node("photos", NodeKind::Directory, 8),
                node("notes.pdf", NodeKind::File, 4),
                node("clip.mp4", NodeKind::File, 2),
            ])
        }
        fn child_report() -> SpaceReport {
            report(vec![
                node("a.png", NodeKind::File, 5),
                node("b.png", NodeKind::File, 3),
                node("c.png", NodeKind::File, 2),
            ])
        }
        /// Lists `report` as the current folder, ready to paint.
        fn show(m: &mut Model, report: SpaceReport) {
            m.entries = report
                .nodes
                .iter()
                .map(|n| Entry {
                    name: std::str::from_utf8(&n.name).unwrap().into(),
                    kind: if n.kind == NodeKind::Directory {
                        Kind::Directory
                    } else {
                        Kind::File
                    },
                    metadata: None,
                    stat_attempted: true,
                    link: None,
                })
                .collect();
            m.order = (0..m.entries.len()).collect();
            m.loaded = true;
            m.space_requested = false;
            m.space_map = crate::space_map_state::State::ready_for_test(m.path.clone(), report);
        }
        fn model() -> (Model, PathBuf) {
            let root = crate::thumbnail_disk_cache::retained_test_directory("space-zoom");
            std::fs::create_dir(root.join("photos")).unwrap();
            let mut m = Model::new(root.clone());
            let (tx, _) = std::sync::mpsc::sync_channel(4);
            let (_, rx) = std::sync::mpsc::channel();
            m.aux = crate::aux_io::AuxIo { tx, rx };
            m.view = View::SpaceMap;
            m.auto_folder_sizes = false;
            show(&mut m, parent_report());
            (m, root)
        }
        fn frame(m: &mut Model) -> Rec {
            let mut p = Rec::default();
            paint(&mut p, m, body());
            p
        }
        /// Painted and final (hit) rectangle of every mapped tile.
        fn tiles(m: &Model) -> Vec<(Rect, Rect)> {
            m.space_geometry
                .drawn
                .iter()
                .filter_map(|&(entry, painted)| {
                    let i = entry?;
                    let hit = m
                        .hits
                        .iter()
                        .find(|h| matches!(h.action, Action::Entry(j) if j == i) && h.rect.w > 0)?;
                    Some((painted, hit.rect))
                })
                .collect()
        }
        fn index(m: &Model, name: &str) -> usize {
            m.entries.iter().position(|e| e.name == name).unwrap()
        }
        fn inside(r: Rect, outer: Rect) -> bool {
            r.x >= outer.x - 1
                && r.y >= outer.y - 1
                && r.x + r.w <= outer.x + outer.w + 1
                && r.y + r.h <= outer.y + outer.h + 1
        }
        fn area(r: Rect) -> i64 {
            i64::from(r.w) * i64::from(r.h)
        }
        fn block(m: &Model, name: &str) -> Rect {
            let i = index(m, name);
            m.hits
                .iter()
                .find(|h| matches!(h.action, Action::Entry(j) if j == i))
                .unwrap()
                .rect
        }
        fn settle(m: &mut Model) {
            frame(m);
            anim::clock::advance(Duration::from_millis(1000));
            frame(m);
            assert!(!m.space_zoom.animating());
        }

        #[test]
        fn zoom_in_updates_model_at_once_and_paints_geometry_midway_then_final() {
            anim::set_enabled(true);
            anim::clock::freeze();
            let (mut m, root) = model();
            frame(&mut m);
            assert!(!m.space_zoom.animating(), "an idle map requests nothing");
            let photos = block(&m, "photos");
            m.select_entry(index(&m, "photos"), false, false);
            m.open_selected();
            // The model is final at t=0: path, selection and focus.
            assert_eq!(m.path, root.join("photos"));
            assert!(m.selection.is_empty() && m.selected.is_none());
            show(&mut m, child_report());
            frame(&mut m);
            let start = tiles(&m);
            assert_eq!(start.len(), 3);
            assert!(m.space_zoom.animating());
            assert!(crate::motion::wants_frame(&m));
            for (painted, last) in &start {
                assert!(inside(*painted, photos), "{painted:?} inside {photos:?}");
                assert_ne!(painted, last);
            }
            anim::clock::advance(Duration::from_millis(110));
            // A key during the transition acts on the new state immediately.
            assert!(crate::keyboard_nav::key(
                &mut m,
                Key::End,
                false,
                false,
                1000,
                700
            ));
            let last = *m.space_geometry.navigation_indices().last().unwrap();
            assert_eq!(m.selected, Some(last));
            frame(&mut m);
            let middle = tiles(&m);
            for ((mid, last), (first, _)) in middle.iter().zip(&start) {
                assert_ne!(mid, last, "still moving at 110 ms");
                assert!(area(*mid) > area(*first) && area(*mid) < area(*last));
            }
            anim::clock::advance(Duration::from_millis(crate::space_map_zoom::ZOOM_MS as u64));
            frame(&mut m);
            for (painted, last) in tiles(&m) {
                assert_eq!(painted, last);
            }
            assert!(!m.space_zoom.animating());
            assert!(!crate::motion::wants_frame(&m), "no redraw once settled");
            anim::clock::resume();
            anim::set_enabled(None);
        }

        #[test]
        fn backspace_zooms_out_and_refocuses_the_origin_block() {
            anim::set_enabled(true);
            anim::clock::freeze();
            let (mut m, root) = model();
            frame(&mut m);
            let photos = block(&m, "photos");
            m.select_entry(index(&m, "photos"), false, false);
            m.open_selected();
            show(&mut m, child_report());
            settle(&mut m);
            let full = tiles(&m)
                .iter()
                .fold(None::<(i32, i32, i32, i32)>, |acc, (_, r)| {
                    let (x0, y0, x1, y1) = acc.unwrap_or((r.x, r.y, r.x + r.w, r.y + r.h));
                    Some((
                        x0.min(r.x),
                        y0.min(r.y),
                        x1.max(r.x + r.w),
                        y1.max(r.y + r.h),
                    ))
                })
                .unwrap();
            crate::keyboard_nav::backspace(&mut m);
            assert_eq!(m.path, root);
            show(&mut m, parent_report());
            // The parent listing alone is not enough in Espacio: focus waits
            // for the painted projection instead of being lost.
            assert!(!m.pump_parent_return_focus());
            frame(&mut m);
            let origin = index(&m, "photos");
            assert_eq!(m.selected, Some(origin), "back re-focuses the origin");
            assert!(m.space_zoom.animating());
            // At t=0 the parent is zoomed so the origin block fills the map.
            let (painted, last) = m
                .space_geometry
                .drawn
                .iter()
                .find(|(e, _)| *e == Some(origin))
                .map(|&(_, r)| (r, photos))
                .unwrap();
            assert_ne!(painted, last);
            assert!((painted.x - full.0).abs() <= 1 && (painted.y - full.1).abs() <= 1);
            assert!((painted.x + painted.w - full.2).abs() <= 1);
            assert!((painted.y + painted.h - full.3).abs() <= 1);
            anim::clock::advance(Duration::from_millis(crate::space_map_zoom::ZOOM_MS as u64));
            frame(&mut m);
            for (painted, last) in tiles(&m) {
                assert_eq!(painted, last);
            }
            assert!(!m.space_zoom.animating());
            anim::clock::resume();
            anim::set_enabled(None);
        }

        #[test]
        fn asynchronous_scan_zooms_the_outline_first_then_children_grow_in() {
            anim::set_enabled(true);
            anim::clock::freeze();
            let (mut m, _) = model();
            frame(&mut m);
            m.select_entry(index(&m, "photos"), false, false);
            m.open_selected();
            // Listing and scan pending: the zoom starts anyway.
            frame(&mut m);
            assert!(m.space_zoom.animating());
            assert!(m.space_geometry.drawn.is_empty());
            anim::clock::advance(Duration::from_millis(crate::space_map_zoom::ZOOM_MS as u64));
            frame(&mut m);
            assert!(!m.space_zoom.animating(), "waiting for the scan is idle");
            anim::clock::advance(Duration::from_millis(500));
            show(&mut m, child_report());
            frame(&mut m);
            assert!(m.space_zoom.animating());
            for (painted, last) in tiles(&m) {
                assert!(inside(painted, last) && area(painted) < area(last));
            }
            anim::clock::advance(Duration::from_millis(
                crate::space_map_zoom::REVEAL_MS as u64,
            ));
            frame(&mut m);
            for (painted, last) in tiles(&m) {
                assert_eq!(painted, last);
            }
            assert!(!m.space_zoom.animating());
            anim::clock::resume();
            anim::set_enabled(None);
        }

        #[test]
        fn disabled_animation_paints_the_final_layout_on_the_first_frame() {
            anim::set_enabled(false);
            anim::clock::freeze();
            let (mut m, _) = model();
            frame(&mut m);
            m.select_entry(index(&m, "photos"), false, false);
            m.open_selected();
            frame(&mut m);
            show(&mut m, child_report());
            frame(&mut m);
            let painted = tiles(&m);
            assert_eq!(painted.len(), 3);
            for (painted, last) in painted {
                assert_eq!(painted, last);
            }
            assert!(!m.space_zoom.animating());
            crate::keyboard_nav::backspace(&mut m);
            show(&mut m, parent_report());
            frame(&mut m);
            assert_eq!(m.selected, Some(index(&m, "photos")));
            for (painted, last) in tiles(&m) {
                assert_eq!(painted, last);
            }
            assert!(!m.space_zoom.animating());
            anim::clock::resume();
            anim::set_enabled(None);
        }

        #[test]
        fn type_icons_scale_with_blocks_and_skip_blocks_below_the_threshold() {
            anim::set_enabled(false);
            assert!(icon_layout(Rect::new(0, 0, 64, 300)).is_none());
            assert!(icon_layout(Rect::new(0, 0, 300, 44)).is_none());
            assert!(
                icon_layout(Rect::new(0, 0, 120, 70)).is_none(),
                "label only"
            );
            let (small, small_size) = icon_layout(Rect::new(0, 0, 120, 100)).unwrap();
            let (big, big_size) = icon_layout(Rect::new(0, 0, 600, 400)).unwrap();
            assert!(small_size >= ICON_MIN && big_size <= ICON_MAX && small_size < big_size);
            for (rect, tile) in [
                (small, Rect::new(0, 0, 120, 100)),
                (big, Rect::new(0, 0, 600, 400)),
            ] {
                // Between the name row and the size row, never over them.
                assert!(rect.y >= tile.y + 31 && rect.y + rect.h <= tile.y + tile.h - 32);
            }
            let (mut m, _) = model();
            let mut nodes = parent_report().nodes;
            for i in 0..6 {
                nodes.push(node(&format!("tiny-{i}.txt"), NodeKind::File, 0));
            }
            nodes[0].bytes = Some(1 << 30);
            nodes[0].observed_bytes = 1 << 30;
            // About 1250 px² of the map: painted, but too small for a label.
            nodes.push(node("speck.txt", NodeKind::File, 1 << 22));
            show(&mut m, report(nodes));
            let p = frame(&mut m);
            let icon = |name: &str, dir: bool| {
                let (glyph, _) =
                    crate::grid_view::type_glyph(dir, std::path::Path::new(name), &m.palette);
                crate::fonts::icon(glyph, if dir { "DIR" } else { "FILE" }).to_string()
            };
            let icons = p
                .texts
                .iter()
                .filter(|(_, _, size)| *size >= ICON_MIN)
                .collect::<Vec<_>>();
            let photos = block(&m, "photos");
            assert!(
                icons
                    .iter()
                    .any(|(r, s, _)| *s == icon("photos", true) && inside(*r, photos)),
                "the folder block shows the folder glyph"
            );
            let speck = block(&m, "speck.txt");
            assert!(speck.w < 65 || speck.h < 45);
            assert!(
                p.texts.iter().all(|(r, _, _)| !inside(*r, speck)),
                "no icon or label in a block below the threshold"
            );
            let expected = m
                .space_geometry
                .drawn
                .iter()
                .filter(|(_, r)| icon_layout(*r).is_some())
                .count();
            assert_eq!(icons.len(), expected);
            anim::set_enabled(None);
        }

        #[test]
        fn tooltips_explain_hovered_blocks_and_focused_blocks_too_small_to_read() {
            anim::set_enabled(false);
            let (mut m, _) = model();
            let mut nodes = parent_report().nodes;
            nodes[0].bytes = Some(1 << 30);
            nodes[0].observed_bytes = 1 << 30;
            nodes.push(node("speck.txt", NodeKind::File, 1 << 22));
            show(&mut m, report(nodes));
            frame(&mut m);
            let tip =
                |p: &Rec, name: &str| p.texts.iter().any(|(_, s, size)| *size == 12 && s == name);
            // A readable focused block needs no tooltip.
            m.selected = Some(index(&m, "photos"));
            m.hovered = None;
            assert!(!tip(&frame(&mut m), "photos"));
            // A focused block too small for its label gets one, with details.
            m.selected = Some(index(&m, "speck.txt"));
            let p = frame(&mut m);
            assert!(tip(&p, "speck.txt"), "keyboard focus explains a tiny block");
            assert!(
                p.texts
                    .iter()
                    .any(|(_, s, size)| *size == 12 && s.contains("de esta carpeta")),
                "share of the folder is shown"
            );
            // Pointer hover always explains its block, even a readable one.
            m.selected = None;
            m.hovered = Some(index(&m, "photos"));
            let photos = block(&m, "photos");
            m.pointer = Some((photos.x + 5, photos.y + 5));
            let p = frame(&mut m);
            assert!(tip(&p, "photos"));
            assert!(
                p.texts
                    .iter()
                    .any(|(_, s, size)| *size == 12 && s.starts_with("Carpeta · ")),
                "folders list kind, size and counts"
            );
            anim::set_enabled(None);
        }

        #[test]
        fn arrows_move_spatially_and_cross_between_map_and_rows() {
            anim::set_enabled(false);
            let (mut m, _) = model();
            let mut nodes = parent_report().nodes;
            nodes.push(node("empty.txt", NodeKind::File, 0));
            show(&mut m, report(nodes));
            frame(&mut m);
            let rect = |m: &Model, i: usize| {
                m.hits
                    .iter()
                    .find(|h| matches!(h.action, Action::Entry(j) if j == i))
                    .unwrap()
                    .rect
            };
            let tiles = m
                .space_geometry
                .drawn
                .iter()
                .filter_map(|(e, _)| *e)
                .collect::<Vec<_>>();
            assert_eq!(tiles.len(), 3);
            let mut moves = 0;
            for &from in &tiles {
                for (key, ok) in [
                    (
                        Key::Right,
                        (|a: Rect, b: Rect| b.x >= a.x + a.w - 1) as fn(Rect, Rect) -> bool,
                    ),
                    (Key::Left, |a, b| b.x + b.w <= a.x + 1),
                    (Key::Down, |a, b| b.y >= a.y + a.h - 1),
                    (Key::Up, |a, b| b.y + b.h <= a.y + 1),
                ] {
                    m.select_entry(from, false, false);
                    m.space_geometry.list_focus = None;
                    crate::keyboard_nav::key(&mut m, key, false, false, 1000, 700);
                    let to = m.selected.unwrap();
                    if to != from && tiles.contains(&to) {
                        moves += 1;
                        assert!(ok(rect(&m, from), rect(&m, to)), "{key:?} {from}->{to}");
                    }
                }
            }
            // Three blocks always have at least two facing pairs.
            assert!(moves >= 4, "{moves} spatial moves");
            // Down from the bottom of the map enters the rows; Up returns.
            let empty = index(&m, "empty.txt");
            let bottom = *tiles
                .iter()
                .max_by_key(|&&i| {
                    let r = rect(&m, i);
                    (r.y + r.h, -r.x)
                })
                .unwrap();
            m.select_entry(bottom, false, false);
            crate::keyboard_nav::key(&mut m, Key::Down, false, false, 1000, 700);
            assert_eq!(m.selected, Some(empty));
            crate::keyboard_nav::key(&mut m, Key::Down, false, false, 1000, 700);
            assert_eq!(m.selected, Some(empty), "last row stays");
            crate::keyboard_nav::key(&mut m, Key::Up, false, false, 1000, 700);
            assert_eq!(m.selected, Some(bottom));
            // Ctrl+arrow moves focus spatially and keeps the marks.
            m.select_entry(tiles[0], false, false);
            crate::keyboard_nav::key(&mut m, Key::Right, true, false, 1000, 700);
            let moved = m.selected.unwrap();
            if moved != tiles[0] {
                assert_eq!(m.selection.len(), 1, "Ctrl keeps the single mark");
                assert!(m.selection.contains(&tiles[0]));
            }
            anim::set_enabled(None);
        }
    }
}
