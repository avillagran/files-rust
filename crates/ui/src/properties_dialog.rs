//! Properties browsing; UI holds at most one pinned object snapshot. All FS I/O
//! stays in the existing mutation worker, including current-path resolution.
use crate::{
    model::{Action, Hit, Model},
    painter::{Painter, Rect},
};
use files_core::{
    file_properties::{Kind, Properties, Snapshot, Timestamp},
    format_size,
    properties_details::Details,
};
use std::path::PathBuf;

pub struct Dialog {
    pub id: u64,
    pub path: PathBuf,
    pub objects: Vec<PathBuf>,
    pub object_index: usize,
    pub snapshot: Option<Box<Snapshot>>,
    pub current_path: Option<PathBuf>,
    pub observed: Option<Properties>,
    pub requested: u32,
    pub focus: usize,
    pub scroll: i32,
    pub body_height: i32,
    pub body: Rect,
    pub pending: bool,
    pub applying: bool,
    pub close_after: bool,
    pub stale: bool,
    pub message: String,
    pub warnings: Vec<String>,
    /// Owner/group names, volume, MIME, link state; gathered by the worker.
    pub details: Option<Box<Details>>,
    /// Whole-selection contents census (cancelled when the dialog is dropped).
    pub census: Option<crate::properties_census::Job>,
    pub totals: Option<crate::properties_census::Totals>,
    pub census_error: Option<String>,
    pub census_items: u64,
    layout: Option<Layout>,
    reveal_pending: bool,
}
pub const BITS: [u32; 9] = [
    0o400, 0o200, 0o100, 0o040, 0o020, 0o010, 0o004, 0o002, 0o001,
];
const PATH_LINE: i32 = 18;
struct Layout {
    width: i32,
    source: Vec<String>,
    lines: Vec<String>,
    exec: i32,
    path: PathBuf,
    current: Option<PathBuf>,
    target: Option<std::ffi::OsString>,
    requested: Vec<String>,
    verified: Vec<String>,
    link: Vec<String>,
    info_top: i32,
    verified_top: i32,
    link_top: i32,
    permission_label: i32,
    permissions: i32,
    note: i32,
    height: i32,
}
fn wrap(text: &str, width: i32, measure: &mut impl FnMut(&str) -> f32) -> Vec<String> {
    let bounds: Vec<_> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    let mut lines = Vec::new();
    let mut start = 0;
    while start + 1 < bounds.len() {
        let (mut low, mut high) = (start + 1, bounds.len() - 1);
        let mut end = low;
        while low <= high {
            let mid = low + (high - low) / 2;
            if measure(&text[bounds[start]..bounds[mid]]) <= width.max(1) as f32 {
                end = mid;
                low = mid + 1;
            } else {
                high = mid - 1;
            }
        }
        lines.push(text[bounds[start]..bounds[end]].to_owned());
        start = end;
    }
    lines
}
fn path_lines(
    p: &mut dyn Painter,
    body: Rect,
    origin: i32,
    lines: &[String],
    color: files_core::theme::Color,
) {
    let first = ((body.y - origin).max(0) / PATH_LINE) as usize;
    for (index, line) in lines
        .iter()
        .enumerate()
        .skip(first)
        .take((body.h.max(0) / PATH_LINE + 2) as usize)
    {
        p.literal_text(
            Rect::new(body.x, origin + index as i32 * PATH_LINE, body.w, PATH_LINE),
            line,
            10,
            color,
        );
    }
}
const CONTENT_HEIGHT: i32 = 406;
impl Dialog {
    pub fn new(id: u64, path: PathBuf) -> Self {
        Self {
            id,
            objects: vec![path.clone()],
            object_index: 0,
            path,
            snapshot: None,
            current_path: None,
            observed: None,
            requested: 0,
            focus: 0,
            scroll: 0,
            body_height: 0,
            body: Rect::new(0, 0, 0, 0),
            pending: true,
            applying: false,
            close_after: false,
            stale: false,
            message: "Leyendo propiedades…".into(),
            warnings: Vec::new(),
            details: None,
            census: None,
            totals: None,
            census_error: None,
            census_items: 0,
            layout: None,
            reveal_pending: false,
        }
    }
    fn contents_line(&self) -> Option<String> {
        let selection = self.objects.len() > 1;
        let folder = self
            .observed
            .is_some_and(|p| p.identity.kind == Kind::Directory);
        if !selection && !folder {
            return None;
        }
        let summary = match (&self.census_error, &self.totals) {
            (Some(error), _) => format!("no se pudo calcular: {error}"),
            (None, totals) => crate::properties_census::summary(totals.as_ref(), self.census_items),
        };
        Some(if selection {
            format!("Selección de {} objetos: {summary}", self.objects.len())
        } else {
            format!("Contenido: {summary}")
        })
    }
    /// Unfitted information rows (12 px, 24 px pitch); empty while unobserved.
    fn info_lines(&self) -> Vec<String> {
        let Some(info) = self.observed else {
            return Vec::new();
        };
        let symlink = info.identity.kind == Kind::Symlink;
        let kind = match info.identity.kind {
            Kind::File => format!("Archivo · aparente {}", format_size(info.apparent_bytes)),
            Kind::Directory => "Carpeta".to_owned(),
            Kind::Symlink => "Enlace simbólico (no se sigue)".to_owned(),
            Kind::Other => format!(
                "Objeto especial · aparente {}",
                format_size(info.apparent_bytes)
            ),
        };
        let mut lines = vec![kind];
        lines.extend(self.contents_line());
        lines.push(format!(
            "Asignado {} · enlaces duros {}",
            format_size(info.allocated_bytes),
            info.hardlinks
        ));
        let (owner, group) = self.details.as_ref().map_or_else(
            || (info.uid.to_string(), info.gid.to_string()),
            |d| (d.owner.clone(), d.group.clone()),
        );
        lines.push(format!("Propietario: {owner} · Grupo: {group}"));
        lines.push(if symlink {
            format!(
                "Permisos del enlace (lstat, no los del destino): {} · modo {:04o}",
                symbolic_mode(info.mode, 'l'),
                info.mode
            )
        } else {
            format!(
                "Permisos: {} · modo octal {:04o}",
                symbolic_mode(
                    info.mode,
                    if info.identity.kind == Kind::Directory {
                        'd'
                    } else {
                        '-'
                    }
                ),
                info.mode
            )
        });
        if let Some(details) = &self.details {
            if let Some(resolves) = details.link_resolves {
                lines.push(
                    if resolves {
                        "Destino del enlace: existe"
                    } else {
                        "Destino del enlace: no existe (enlace roto)"
                    }
                    .into(),
                );
            }
            if let Some(mime) = &details.mime {
                lines.push(format!(
                    "Tipo MIME: {mime} · Abrir con: {}",
                    details.default_app.as_deref().unwrap_or("(sin aplicación)")
                ));
                lines.push("Cambiar la aplicación: Ctrl+Shift+O en la lista".into());
            }
            if let Some(volume) = &details.volume {
                lines.push(format!(
                    "Volumen: {} · libre {} de {}",
                    volume
                        .filesystem
                        .as_deref()
                        .unwrap_or("sistema de archivos desconocido"),
                    format_size(volume.free_bytes),
                    format_size(volume.total_bytes)
                ));
            } else {
                lines.push("Volumen: no disponible".into());
            }
        }
        lines.push(format!(
            "Dispositivo {} · inode {}",
            info.identity.device, info.identity.inode
        ));
        lines.push(format!("Modificado: {}", date(info.modified)));
        lines.push(format!("Accedido: {}", date(info.accessed)));
        lines.push(match self.details.as_ref().and_then(|d| d.birth) {
            Some(birth) => format!("Creado: {}", date(birth)),
            None => "Creado: no disponible en este sistema de archivos".into(),
        });
        lines.push(format!("Cambio de inode: {}", date(info.changed)));
        lines
    }
    fn ensure_layout(&mut self, p: &mut dyn Painter, width: i32) {
        let target = self
            .snapshot
            .as_ref()
            .and_then(|s| s.symlink_target.as_ref());
        let source = self.info_lines();
        if self.layout.as_ref().is_some_and(|l| {
            l.width == width
                && l.source == source
                && l.path == self.path
                && l.current == self.current_path
                && l.target.as_ref() == target
        }) {
            return;
        }
        let mut measure_at = |text: &str, size: i32| p.text_width(text, size, true);
        let mut measure = |text: &str| measure_at(text, 10);
        let requested = wrap(
            &format!(
                "Ruta solicitada: {}",
                files_core::name_label::label(self.path.as_os_str())
            ),
            width,
            &mut measure,
        );
        let current = self
            .current_path
            .as_ref()
            .map(|path| {
                format!(
                    "Ruta verificada: {}",
                    files_core::name_label::label(path.as_os_str())
                )
            })
            .unwrap_or_else(|| {
                "Ruta actual no disponible; no se inventa la ruta de rescate".into()
            });
        let verified = wrap(&current, width, &mut measure);
        let link = target
            .map(|t| {
                wrap(
                    &format!(
                        "Destino raw (no abierto): {}",
                        files_core::name_label::label(t)
                    ),
                    width,
                    &mut measure,
                )
            })
            .unwrap_or_default();
        let lines = source
            .iter()
            .map(|line| fit_middle(line, width, &mut |t| measure_at(t, 12)))
            .collect::<Vec<_>>();
        let info_top = requested.len() as i32 * PATH_LINE + 8;
        let verified_top = info_top + lines.len() as i32 * 24 + 6;
        let link_top = verified_top + verified.len() as i32 * PATH_LINE + 8;
        let permission_label = link_top
            + if link.is_empty() {
                0
            } else {
                link.len() as i32 * PATH_LINE + 8
            };
        let exec = permission_label + 28;
        let permissions = exec + 36;
        let note = permissions + 104;
        let height = note + 36;
        self.layout = Some(Layout {
            width,
            source,
            lines,
            exec,
            path: self.path.clone(),
            current: self.current_path.clone(),
            target: target.cloned(),
            requested,
            verified,
            link,
            info_top,
            verified_top,
            link_top,
            permission_label,
            permissions,
            note,
            height,
        });
    }
    fn content_height(&self) -> i32 {
        self.layout.as_ref().map_or(CONTENT_HEIGHT, |l| {
            if self.observed.is_some() {
                l.height
            } else {
                l.info_top + 40
            }
        }) + self.warnings.len() as i32 * 28
    }
    pub fn editable(&self) -> bool {
        !self.pending
            && !self.stale
            && self.snapshot.is_some()
            && self.observed.is_some_and(|p| {
                matches!(p.identity.kind, Kind::File | Kind::Directory)
                    && p.mode & 0o7000 == 0
                    && own_uid() == Some(p.uid)
            })
    }
    pub fn changed(&self) -> bool {
        self.editable() && self.has_edits()
    }
    pub fn has_edits(&self) -> bool {
        self.observed
            .is_some_and(|p| p.mode & 0o777 != self.requested)
    }
    pub fn step_index(&self, delta: i32) -> Option<usize> {
        if self.pending || self.applying || self.has_edits() || delta == 0 {
            return None;
        }
        let index = self.object_index.checked_add_signed(delta as isize)?;
        (index < self.objects.len()).then_some(index)
    }
    pub fn toggle(&mut self, bit: usize) {
        if self.editable()
            && let Some(mask) = BITS.get(bit)
        {
            self.requested ^= mask;
            self.focus = bit;
        }
    }
    pub fn scroll_by(&mut self, delta: i32, body_height: i32) {
        if delta != 0 {
            self.reveal_pending = false;
        }
        self.scroll = self
            .scroll
            .saturating_add(delta)
            .clamp(0, (self.content_height() - body_height).max(0));
    }
    pub fn focus_next(&mut self, backwards: bool, body_height: i32) {
        self.focus = (self.focus + if backwards { 8 } else { 1 }) % 9;
        self.reveal_pending = true;
        self.reveal_focus(body_height);
    }
    fn reveal_focus(&mut self, body_height: i32) {
        let top =
            self.layout.as_ref().map_or(258, |l| l.permissions) + (self.focus / 3) as i32 * 32;
        if top < self.scroll {
            self.scroll = top;
        }
        if top + 28 > self.scroll + body_height {
            self.scroll = (top + 28 - body_height).max(0);
        }
    }
}
#[cfg(target_os = "linux")]
fn own_uid() -> Option<u32> {
    Some(unsafe { libc::geteuid() })
}
#[cfg(not(target_os = "linux"))]
fn own_uid() -> Option<u32> {
    None
}
/// Local time (libc localtime_r), falling back to the raw Unix seconds.
fn date(t: Timestamp) -> String {
    #[cfg(unix)]
    {
        // time_t is 64-bit on every supported target.
        let seconds: libc::time_t = t.seconds;
        let mut tm = std::mem::MaybeUninit::<libc::tm>::zeroed();
        if !unsafe { libc::localtime_r(&seconds, tm.as_mut_ptr()) }.is_null() {
            let tm = unsafe { tm.assume_init() };
            return format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                i64::from(tm.tm_year) + 1900,
                tm.tm_mon + 1,
                tm.tm_mday,
                tm.tm_hour,
                tm.tm_min,
                tm.tm_sec
            );
        }
    }
    format!("{} s Unix", t.seconds)
}
/// `ls`-style 10-character mode string.
fn symbolic_mode(mode: u32, kind: char) -> String {
    let mut out = String::with_capacity(10);
    out.push(kind);
    for (i, (bit, ch)) in [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ]
    .into_iter()
    .enumerate()
    {
        let special = match i {
            2 => mode & 0o4000 != 0,
            5 => mode & 0o2000 != 0,
            8 => mode & 0o1000 != 0,
            _ => false,
        };
        out.push(match (mode & bit != 0, special) {
            (true, false) => ch,
            (false, false) => '-',
            (exec, true) if i == 8 => {
                if exec {
                    't'
                } else {
                    'T'
                }
            }
            (exec, true) => {
                if exec {
                    's'
                } else {
                    'S'
                }
            }
        });
    }
    out
}
/// Shorten in the middle (head…tail) until the measured text fits `width`.
pub(crate) fn fit_middle(text: &str, width: i32, measure: &mut impl FnMut(&str) -> f32) -> String {
    let width = width.max(1) as f32;
    if measure(text) <= width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let build = |keep: usize| {
        let head: String = chars[..keep.div_ceil(2)].iter().collect();
        let tail: String = chars[chars.len() - keep / 2..].iter().collect();
        format!("{head}…{tail}")
    };
    let (mut low, mut high) = (0usize, chars.len().saturating_sub(1));
    while low < high {
        let mid = (low + high).div_ceil(2);
        if measure(&build(mid)) <= width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    build(low)
}
fn button(
    p: &mut dyn Painter,
    m: &mut Model,
    r: Rect,
    text: &str,
    action: Action,
    enabled: bool,
    on: bool,
) {
    p.rounded_rect(
        r,
        6 * m.radius,
        if on {
            m.palette.selection
        } else {
            m.palette.lighter_background
        },
    );
    p.text(
        Rect::new(r.x + 8, r.y, r.w - 16, r.h),
        text,
        12,
        if enabled {
            m.palette.accent
        } else {
            m.palette.dark_foreground
        },
        true,
        false,
    );
    if enabled {
        m.hits.push(Hit {
            generation: m.generation,
            rect: r,
            action,
        });
    }
}
pub fn paint(p: &mut dyn Painter, m: &mut Model, d: &mut Dialog, w: i32, h: i32) {
    m.hits.clear();
    let width = (w - 36).min(780);
    let height = (h - 68).min(640);
    let x = (w - width) / 2;
    let y = (h - 32 - height) / 2;
    let c = m.palette.clone();
    p.rounded_rect(Rect::new(x, y, width, height), 10 * m.radius, c.muted);
    p.rounded_rect(
        Rect::new(x + 1, y + 1, width - 2, height - 2),
        10 * m.radius,
        c.darker_background,
    );
    p.text(
        Rect::new(x + 20, y + 8, width - 40, 30),
        "  Propiedades y permisos",
        17,
        c.bright_foreground,
        true,
        false,
    );
    if d.objects.len() > 1 {
        button(
            p,
            m,
            Rect::new(x + 20, y + 40, 88, 24),
            "Anterior",
            Action::PropertyStep(-1),
            d.step_index(-1).is_some(),
            false,
        );
        p.text(
            Rect::new(x + 116, y + 40, width - 232, 24),
            &format!("Objeto {} de {}", d.object_index + 1, d.objects.len()),
            10,
            c.dark_foreground,
            true,
            false,
        );
        button(
            p,
            m,
            Rect::new(x + width - 108, y + 40, 88, 24),
            "Siguiente",
            Action::PropertyStep(1),
            d.step_index(1).is_some(),
            false,
        );
    } else {
        p.text(
            Rect::new(x + 20, y + 40, width - 40, 24),
            "Rutas completas en detalles · Inicio/Fin · Tab permisos",
            10,
            c.dark_foreground,
            true,
            false,
        );
    }
    let body = Rect::new(x + 20, y + 74, width - 40, (height - 176).max(0));
    d.ensure_layout(p, body.w);
    d.body_height = body.h;
    d.body = body;
    if d.reveal_pending && d.editable() {
        d.reveal_focus(body.h);
        d.reveal_pending = false;
    }
    d.scroll_by(0, body.h);
    p.clip(body);
    let origin = body.y - d.scroll;
    let layout = d.layout.as_ref().expect("properties layout prepared");
    path_lines(p, body, origin, &layout.requested, c.dark_foreground);
    let top = origin + layout.info_top;
    if let Some(info) = d.observed {
        for (i, line) in layout.lines.iter().enumerate() {
            p.text(
                Rect::new(body.x, top + i as i32 * 24, body.w, 24),
                line,
                12,
                c.foreground,
                true,
                false,
            );
        }
        path_lines(
            p,
            body,
            origin + layout.verified_top,
            &layout.verified,
            c.dark_foreground,
        );
        path_lines(p, body, origin + layout.link_top, &layout.link, c.accent);
        p.text(
            Rect::new(body.x, origin + layout.permission_label, body.w, 24),
            &if info.identity.kind == Kind::Symlink {
                format!(
                    "Permisos del enlace {:04o} · sólo lectura, no cambia los del destino",
                    info.mode
                )
            } else {
                format!(
                    "Permisos {:04o} → {:04o} · todos los enlaces al inode",
                    info.mode, d.requested
                )
            },
            12,
            c.bright_foreground,
            true,
            false,
        );
        {
            // Same user-execute bit as the grid's first row, third column.
            let r = Rect::new(body.x, origin + layout.exec, body.w, 28);
            let on = d.requested & 0o100 != 0;
            let whole = r.y >= body.y && r.y + r.h <= body.y + body.h;
            button(
                p,
                m,
                r,
                &format!(
                    "{} Permitir ejecutar como programa",
                    if on { "[x]" } else { "[ ]" }
                ),
                Action::PropertyBit(2),
                d.editable() && whole,
                on,
            );
        }
        for row in 0..3 {
            let by = origin + layout.permissions + row * 32;
            p.text(
                Rect::new(body.x, by, if body.w < 500 { 96 } else { 128 }, 28),
                ["Propietario", "Grupo", "Otros"][row as usize],
                12,
                c.foreground,
                true,
                false,
            );
            for col in 0..3 {
                let index = (row * 3 + col) as usize;
                let labels = if body.w < 500 { 96 } else { 128 };
                let cell = (body.w - labels - 8 - 16) / 3;
                let r = Rect::new(body.x + labels + 8 + col * (cell + 8), by, cell, 28);
                let on = d.requested & BITS[index] != 0;
                let label = format!(
                    "{} {}",
                    ["r", "w", "x"][col as usize],
                    if on { "" } else { "—" }
                );
                let whole = r.w > 0
                    && r.x >= body.x
                    && r.x + r.w <= body.x + body.w
                    && r.y >= body.y
                    && r.y + r.h <= body.y + body.h;
                button(
                    p,
                    m,
                    r,
                    &label,
                    Action::PropertyBit(index),
                    d.editable() && whole,
                    on,
                );
                if index == d.focus && whole && d.editable() {
                    for edge in [
                        Rect::new(r.x, r.y, r.w, 1),
                        Rect::new(r.x, r.y + r.h - 1, r.w, 1),
                        Rect::new(r.x, r.y, 1, r.h),
                        Rect::new(r.x + r.w - 1, r.y, 1, r.h),
                    ] {
                        p.rect(edge, c.accent);
                    }
                }
            }
        }
        for (i, warning) in d.warnings.iter().enumerate() {
            p.text(
                Rect::new(body.x, origin + layout.height + i as i32 * 28, body.w, 28),
                warning,
                11,
                c.red,
                true,
                false,
            );
        }
        let note = if d.snapshot.is_none() && d.observed.is_some() && !d.stale {
            "Directorio sólo lectura; no se habilitan cambios de permisos."
        } else if d.editable() {
            "Sólo este objeto; sin recursión, ACL ni cambio de propietario."
        } else {
            "Sólo lectura o snapshot obsoleto: Recargar para volver a comprobar."
        };
        p.text(
            Rect::new(body.x, origin + layout.note, body.w, 28),
            note,
            11,
            c.dark_foreground,
            true,
            false,
        );
    } else {
        p.text(
            Rect::new(body.x, top, body.w, 40),
            if d.pending {
                "Leyendo fuera de la UI…"
            } else {
                "No hay propiedades válidas; puedes recargar."
            },
            13,
            c.foreground,
            true,
            false,
        );
    }
    p.unclip();
    let summary = &d.message;
    // Messages from the worker are bounded and flattened for a one-line status.
    p.text(
        Rect::new(x + 20, y + height - 94, width - 40, 24),
        summary,
        11,
        if d.stale { c.red } else { c.accent },
        true,
        false,
    );
    p.text(
        Rect::new(x + 20, y + height - 70, width - 40, 20),
        if d.objects.len() > 1 {
            "Alt+←/→ objeto · rueda/↑/↓ · Tab permisos"
        } else {
            "Sin Deshacer · rueda/↑/↓ · Inicio/Fin · Tab permisos"
        },
        10,
        c.dark_foreground,
        true,
        false,
    );
    button(
        p,
        m,
        Rect::new(x + 20, y + height - 42, (width - 56) / 3, 30),
        "Recargar",
        Action::PropertyReload,
        !d.pending,
        false,
    );
    button(
        p,
        m,
        Rect::new(
            x + 28 + (width - 56) / 3,
            y + height - 42,
            (width - 56) / 3,
            30,
        ),
        if d.pending {
            if width < 520 {
                "Cancelar"
            } else {
                "Cancelar tarea"
            }
        } else {
            "Cerrar"
        },
        Action::PropertyClose,
        true,
        false,
    );
    button(
        p,
        m,
        Rect::new(
            x + 36 + 2 * ((width - 56) / 3),
            y + height - 42,
            (width - 56) / 3,
            30,
        ),
        if width < 520 {
            "Aplicar"
        } else {
            "Aplicar permisos"
        },
        Action::PropertyApply,
        d.changed(),
        false,
    );
}
pub fn fixture(m: &Model) -> serde_json::Value {
    m.properties_dialog.as_ref().map_or(serde_json::Value::Null,|d|serde_json::json!({"id":d.id,"object_index":d.object_index,"object_count":d.objects.len(),"objects_raw":d.objects.iter().map(|p|p.as_os_str().as_encoded_bytes()).collect::<Vec<_>>(),"pending":d.pending,"applying":d.applying,"changed":d.changed(),"editable":d.editable(),"stale":d.stale,"requested":d.requested,"observed":d.observed.map(|p|p.mode),"path_raw":d.path.as_os_str().as_encoded_bytes(),"current_raw":d.current_path.as_ref().map(|p|p.as_os_str().as_encoded_bytes()),"scroll":d.scroll,"focus":d.focus,"body":[d.body.x,d.body.y,d.body.w,d.body.h],"content_height":d.content_height(),"paths":d.layout.as_ref().map(|l|serde_json::json!({"requested":l.requested,"verified":l.verified,"link":l.link,"permissions_top":l.permissions,"verified_top":l.verified_top,"link_top":l.link_top})),"buttons":m.hits.iter().filter_map(|h|{let (command,index)=match &h.action {Action::PropertyBit(i)=>("Bit",Some(*i)),Action::PropertyApply=>("Apply",None),Action::PropertyReload=>("Reload",None),Action::PropertyClose=>("Close",None),Action::PropertyStep(delta)=>(if *delta<0 {"Previous"} else {"Next"},None),_=>return None};Some(serde_json::json!({"command":command,"index":index,"rect":[h.rect.x,h.rect.y,h.rect.w,h.rect.h]}))}).collect::<Vec<_>>()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Recorder {
        text: Vec<String>,
        measured: usize,
    }
    impl Painter for Recorder {
        fn rect(&mut self, _: Rect, _: files_core::theme::Color) {}
        fn rounded_rect(&mut self, _: Rect, _: i32, _: files_core::theme::Color) {}
        fn text(
            &mut self,
            _: Rect,
            text: &str,
            _: i32,
            _: files_core::theme::Color,
            _: bool,
            _: bool,
        ) {
            self.text.push(text.into());
        }
        fn text_width(&mut self, text: &str, size: i32, _: bool) -> f32 {
            self.measured += 1;
            text.chars().count() as f32 * size as f32 * 0.6
        }
        fn image(&mut self, _: Rect, _: &crate::painter::Image<'_>) {}
        fn clip(&mut self, _: Rect) {}
        fn unclip(&mut self) {}
    }
    #[test]
    fn wrapping_keeps_all_utf8_scalars_and_each_measured_line_fits() {
        let text = "Ruta: 日本語 👩‍💻 / literal\\xFF / fin";
        let mut measure = |s: &str| {
            s.chars()
                .map(|c| if c.is_ascii() { 6.0 } else { 12.0 })
                .sum()
        };
        let lines = wrap(text, 60, &mut measure);
        assert!(lines.len() > 1);
        assert_eq!(lines.concat(), text);
        assert!(lines.iter().all(|line| measure(line) <= 60.0));
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn raw_paths_are_complete_cached_and_minimum_permission_targets_do_not_overlap() {
        use std::os::unix::{ffi::OsStringExt, fs::PermissionsExt};
        let root = crate::test_support::private_dir("properties-layout");
        std::fs::create_dir_all(&root).unwrap();
        let mut raw = b"long-".to_vec();
        raw.extend(vec![b'a'; 180]);
        raw.extend(b"\xff\n.txt");
        let path = root.join(std::ffi::OsString::from_vec(raw));
        std::fs::write(&path, b"unchanged").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let snapshot = files_core::file_properties::capture(&path).unwrap();
        let mut d = Dialog::new(1, path.clone());
        d.pending = false;
        d.observed = Some(snapshot.properties);
        d.requested = 0o640;
        d.current_path = Some(snapshot.current_path().unwrap());
        d.snapshot = Some(Box::new(snapshot));
        let mut m = Model::new(root.clone());
        let mut p = Recorder::default();
        for (w, h) in [(440, 440), (800, 440), (1000, 650)] {
            d.focus = 0;
            d.scroll = 0;
            paint(&mut p, &mut m, &mut d, w, h);
            let layout = d.layout.as_ref().unwrap();
            assert_eq!(
                layout.requested.concat(),
                format!(
                    "Ruta solicitada: {}",
                    files_core::name_label::label(path.as_os_str())
                )
            );
            let measured = p.measured;
            paint(&mut p, &mut m, &mut d, w, h);
            assert_eq!(p.measured, measured);
            d.focus_next(true, d.body_height);
            paint(&mut p, &mut m, &mut d, w, h);
            let bits: Vec<_> = m
                .hits
                .iter()
                .filter(|h| matches!(h.action, Action::PropertyBit(_)))
                .collect();
            // Nine grid cells plus, when fully visible, the full-width
            // "Permitir ejecutar como programa" toggle (same user-execute bit).
            assert_eq!(bits.iter().filter(|h| h.rect.w < d.body.w).count(), 9);
            assert!(bits.len() <= 10);
            for hit in bits {
                assert!(
                    hit.rect.x >= d.body.x
                        && hit.rect.x + hit.rect.w <= d.body.x + d.body.w
                        && hit.rect.y >= d.body.y
                        && hit.rect.y + hit.rect.h <= d.body.y + d.body.h
                );
            }
            let buttons: Vec<_> = m
                .hits
                .iter()
                .filter(|h| {
                    matches!(
                        h.action,
                        Action::PropertyApply | Action::PropertyClose | Action::PropertyReload
                    )
                })
                .collect();
            assert_eq!(buttons.len(), 3);
            for (i, a) in buttons.iter().enumerate() {
                assert!(a.rect.y + a.rect.h <= h - 32);
                for b in &buttons[i + 1..] {
                    assert!(a.rect.x + a.rect.w <= b.rect.x || b.rect.x + b.rect.w <= a.rect.x);
                }
            }
        }
        // Moving the leaf correctly invalidates a properties snapshot. Moving
        // the pinned parent retains its child mapping and changes its verified
        // absolute path, exercising cache invalidation without inventing a path.
        let moved = root.with_extension("moved");
        std::fs::rename(&root, &moved).unwrap();
        let renamed = moved.join(path.file_name().unwrap());
        d.current_path = Some(d.snapshot.as_ref().unwrap().current_path().unwrap());
        let measured = p.measured;
        paint(&mut p, &mut m, &mut d, 1000, 650);
        assert!(p.measured > measured);
        assert_eq!(
            d.layout.as_ref().unwrap().verified.concat(),
            format!(
                "Ruta verificada: {}",
                files_core::name_label::label(renamed.as_os_str())
            )
        );
        assert_eq!(d.path, path);
        assert_eq!(std::fs::read(&renamed).unwrap(), b"unchanged");
        assert_eq!(
            std::fs::metadata(&renamed).unwrap().permissions().mode() & 0o777,
            0o600
        );
        drop(d);
        drop(m);
    }
    #[test]
    fn loading_cannot_apply_and_tab_reveals_last_permission_at_small_height() {
        let mut d = Dialog::new(1, "/fixture".into());
        assert!(!d.editable() && !d.changed());
        d.toggle(0);
        assert_eq!(d.requested, 0);
        d.focus_next(true, 228);
        assert_eq!(d.focus, 8);
        assert!(d.scroll > 0);
        d.scroll_by(i32::MAX, 228);
        assert_eq!(d.scroll, CONTENT_HEIGHT - 228);
        d.scroll_by(i32::MIN, 228);
        assert_eq!(d.scroll, 0);
    }
    #[test]
    fn batch_navigation_is_bounded_and_keeps_stale_unapplied_edits() {
        use files_core::file_properties::Identity;
        let mut d = Dialog::new(1, "/fixture/a".into());
        d.objects.push("/fixture/b".into());
        assert_eq!(d.step_index(1), None, "reading cannot switch objects");
        d.pending = false;
        assert_eq!(d.step_index(-1), None);
        assert_eq!(d.step_index(1), Some(1));
        assert_eq!(d.step_index(i32::MAX), None);
        d.object_index = 1;
        assert_eq!(d.step_index(1), None);
        assert_eq!(d.step_index(-1), Some(0));
        d.applying = true;
        assert_eq!(d.step_index(-1), None);
        d.applying = false;
        let time = Timestamp {
            seconds: 1,
            nanoseconds: 0,
        };
        d.observed = Some(Properties {
            identity: Identity {
                device: 1,
                inode: 2,
                kind: Kind::File,
            },
            apparent_bytes: 1,
            allocated_bytes: 512,
            hardlinks: 1,
            uid: 0,
            gid: 0,
            mode: 0o600,
            accessed: time,
            modified: time,
            changed: time,
        });
        d.requested = 0o640;
        d.stale = true;
        assert!(!d.changed(), "stale snapshots cannot Apply");
        assert!(d.has_edits());
        assert_eq!(
            d.step_index(-1),
            None,
            "failed edits are not discarded by navigation"
        );
        assert_eq!(d.requested, 0o640);
    }
    #[test]
    fn batch_navigation_header_fits_minimum_width_without_changing_permission_focus() {
        let mut m = Model::new("/fixture/properties-header".into());
        m.worker.suspend();
        let mut d = Dialog::new(1, "/fixture/a".into());
        d.objects.push("/fixture/b".into());
        d.pending = false;
        for (index, expected) in [(0, 1), (1, -1)] {
            d.object_index = index;
            d.path = d.objects[index].clone();
            let mut p = Recorder::default();
            paint(&mut p, &mut m, &mut d, 440, 440);
            let buttons: Vec<_> = m
                .hits
                .iter()
                .filter(|hit| matches!(hit.action, Action::PropertyStep(_)))
                .collect();
            assert_eq!(buttons.len(), 1);
            let hit = buttons[0];
            assert!(matches!(hit.action,Action::PropertyStep(delta) if delta==expected));
            assert!(hit.rect.x >= 18 && hit.rect.x + hit.rect.w <= 422);
            assert!(hit.rect.y + hit.rect.h <= d.body.y);
            assert!(
                p.text
                    .iter()
                    .any(|text| text == &format!("Objeto {} de 2", index + 1))
            );
            d.focus = 0;
            d.focus_next(true, d.body_height);
            assert_eq!(d.focus, 8, "existing permission Tab cycle remains intact");
        }
        d.pending = true;
        paint(&mut Recorder::default(), &mut m, &mut d, 440, 440);
        assert!(
            !m.hits
                .iter()
                .any(|hit| matches!(hit.action, Action::PropertyStep(_)))
        );
    }
    #[test]
    fn middle_truncation_keeps_both_ends_and_fits() {
        let mut measure = |t: &str| t.chars().count() as f32 * 6.0;
        let text = "Volumen: btrfs · libre 120 GiB de 500 GiB y bastante más texto";
        let fitted = fit_middle(text, 150, &mut measure);
        assert!(measure(&fitted) <= 150.0 && fitted.contains('…'));
        assert!(fitted.starts_with("Volumen") && fitted.ends_with("texto"));
        assert_eq!(fit_middle("corto", 150, &mut measure), "corto");
        assert_eq!(symbolic_mode(0o4755, '-'), "-rwsr-xr-x");
        assert_eq!(symbolic_mode(0o1777, 'd'), "drwxrwxrwt");
        assert_eq!(symbolic_mode(0o777, 'l'), "lrwxrwxrwx");
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn link_rows_use_lstat_show_target_volume_names_and_exec_toggle() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("files-props-rows-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("destino.sh");
        std::fs::write(&target, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = root.join("enlace");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let mut m = Model::new(root.clone());
        let mut p = Recorder::default();
        let load = |path: &std::path::Path, census: bool| {
            let snapshot = files_core::file_properties::capture(path).unwrap();
            let mut d = Dialog::new(1, path.to_path_buf());
            d.pending = false;
            d.observed = Some(snapshot.properties);
            d.requested = snapshot.properties.mode & 0o777;
            d.current_path = Some(snapshot.current_path().unwrap());
            d.details = Some(Box::new(files_core::properties_details::gather(
                d.current_path.as_ref().unwrap(),
                &snapshot.properties,
            )));
            d.snapshot = Some(Box::new(snapshot));
            if census {
                d.totals = Some(Default::default());
            }
            d
        };
        // Symlink: lstat mode (0777), raw target, resolution, no chmod on it.
        let mut d = load(&link, false);
        for (w, h) in [(700, 500), (440, 440)] {
            p.text.clear();
            paint(&mut p, &mut m, &mut d, w, h);
            let joined = p.text.join("\n");
            // Narrow windows shorten rows in the middle but keep their heads.
            assert!(joined.contains("Permisos del enlace"));
            assert!(joined.contains("Volumen:") && joined.contains("Propietario:"));
            if w == 700 {
                assert!(joined.contains("(lstat, no los del destino)"));
                assert!(joined.contains("lrwxrwxrwx"), "{joined}");
                assert!(joined.contains("Destino del enlace: existe"));
                assert!(joined.contains("libre"));
            }
            assert!(!d.editable());
            assert!(!joined.contains("Tipo MIME"), "the link itself has no MIME");
        }
        assert_eq!(
            d.layout.as_ref().unwrap().link.concat(),
            format!(
                "Destino raw (no abierto): {}",
                files_core::name_label::label(target.as_os_str())
            )
        );
        // Regular file: octal mode, MIME + default app row, executable toggle.
        let mut d = load(&target, false);
        p.text.clear();
        paint(&mut p, &mut m, &mut d, 700, 500);
        let joined = p.text.join("\n");
        assert!(joined.contains("modo octal 0600"), "{joined}");
        assert!(joined.contains("Tipo MIME:") && joined.contains("Ctrl+Shift+O"));
        assert!(joined.contains("[ ] Permitir ejecutar como programa"));
        assert!(
            !joined.contains("Contenido:"),
            "plain files have no census row"
        );
        d.toggle(2);
        paint(&mut p, &mut m, &mut d, 700, 500);
        assert_eq!(d.requested, 0o700, "user execute bit only");
        assert!(
            p.text
                .iter()
                .any(|t| t == "[x] Permitir ejecutar como programa")
        );
        // Folder: census row honest while running and partial afterwards.
        let mut d = load(&root, false);
        d.census_items = 42;
        p.text.clear();
        paint(&mut p, &mut m, &mut d, 700, 500);
        assert!(
            p.text
                .iter()
                .any(|t| t == "Contenido: calculando… 42 elementos")
        );
        d.totals = Some(crate::properties_census::Totals {
            census: files_core::folder_usage::Census {
                files: 3,
                unreadable: 1,
                ..Default::default()
            },
            ..Default::default()
        });
        p.text.clear();
        paint(&mut p, &mut m, &mut d, 700, 500);
        assert!(
            p.text
                .iter()
                .any(|t| t.contains("PARCIAL") && t.contains("1 sin permiso de lectura"))
        );
    }
}
