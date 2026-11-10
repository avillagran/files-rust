//! Static bounded shortcut help. No unimplemented action is advertised.
use crate::shortcuts::{self, Cmd, Profile};
use crate::{
    model::{Action, Hit, Model},
    painter::{Painter, Rect},
};
use Keys::{C, T};
use fltk::enums::Key;
use std::borrow::Cow;
const ROW: i32 = 30;
/// What a help row prints in its key column.
pub enum Keys {
    /// Fixed text: a context chord (search, rail, preview) or a gesture.
    T(&'static str),
    /// Chords of these commands in the active profile, from the table.
    C(&'static [Cmd]),
}
/// Shown when a command has no chord in the active profile.
pub fn palette_only(profile: Profile) -> String {
    format!("Paleta ({})", shortcuts::label(profile, Cmd::Commands))
}
impl Keys {
    pub fn text(&self, profile: Profile) -> Cow<'static, str> {
        match self {
            T(text) => (*text).into(),
            C(cmds) => {
                let labels: Vec<String> = cmds
                    .iter()
                    .map(|&c| shortcuts::label(profile, c))
                    .filter(|l| !l.is_empty())
                    .collect();
                if labels.is_empty() {
                    palette_only(profile).into()
                } else {
                    labels.join(" / ").into()
                }
            }
        }
    }
    fn is_header(&self) -> bool {
        matches!(self, T(""))
    }
}
pub const ITEMS: &[(Keys, &str)] = &[
    (T(""), "NAVEGACIÓN"),
    (
        C(&[Cmd::Back, Cmd::Forward]),
        "Atrás / Adelante en el historial",
    ),
    (
        C(&[Cmd::Open]),
        "Abrir elemento o entrar en carpeta (también doble clic)",
    ),
    (
        T("Enter con varias marcas"),
        "Abrir todas (máx. 20): carpetas en pestañas de fondo",
    ),
    (
        C(&[Cmd::OpenInTab, Cmd::OpenInBackground]),
        "Carpeta en pestaña nueva / de fondo; archivo: abrir",
    ),
    (C(&[Cmd::Parent]), "Subir a la carpeta superior"),
    (C(&[Cmd::Home]), "Ir a la carpeta personal"),
    (
        C(&[Cmd::TabNew, Cmd::TabClose]),
        "Nueva pestaña / cerrar activa",
    ),
    (C(&[Cmd::TabNext]), "Pestaña siguiente"),
    (C(&[Cmd::TabPrev]), "Pestaña anterior"),
    (T("Alt+1 … Alt+9"), "Ir a la pestaña N; Alt+9 la última"),
    (C(&[Cmd::TabReopen]), "Reabrir pestaña cerrada"),
    (
        T("Ctrl+Alt+N / Escape"),
        "Enfocar aviso de operación / cerrar aviso",
    ),
    (
        T("← → / Enter en aviso"),
        "Elegir / ejecutar una acción del aviso",
    ),
    (C(&[Cmd::TabMenu]), "Acciones de la pestaña activa"),
    (T("Arrastrar pestaña"), "Reordenar sin activarla"),
    (
        T("Ctrl+Alt+T / Escape"),
        "Foco en pestañas / volver al navegador",
    ),
    (T("← → en barra"), "Elegir pestaña"),
    (T("Alt+← → en barra"), "Mover cursor sin activar pestaña"),
    (T("Ctrl+Shift+← → en barra"), "Mover la pestaña del cursor"),
    (
        T("Menú / Shift+F10 en barra"),
        "Acciones de la pestaña del cursor",
    ),
    (
        C(&[Cmd::TabMoveLeft, Cmd::TabMoveRight]),
        "Mover la pestaña activa a izquierda/derecha",
    ),
    (
        T("Clic central en carpeta"),
        "Abrir en pestaña de fondo sin cambiar selección",
    ),
    (
        T("Clic central en pestaña"),
        "Cerrar esa pestaña sin activarla primero",
    ),
    (C(&[Cmd::Location]), "Editar ruta: selecciona todo el campo"),
    (
        T("Tab en ruta"),
        "Completar carpeta; repetir o ↑ ↓ recorre opciones, Enter acepta, Esc cierra",
    ),
    (
        T("/ o ~ en la lista"),
        "Editar ruta empezando con ese carácter",
    ),
    (
        C(&[Cmd::Breadcrumbs]),
        "Ancestros ocultos (… en ruta); aquí o en pestaña de fondo",
    ),
    (
        T(">comando / :lugar"),
        "En ruta + Enter: buscar en catálogos",
    ),
    (
        T("←/→, Home/End, Shift"),
        "Cursor y selección: ruta, filtro o buscador",
    ),
    (
        T("Ctrl+←/→ / doble clic"),
        "Recorrer / seleccionar componente en campo",
    ),
    (
        T("Ctrl+Z / Ctrl+Shift+Z"),
        "En campos: deshacer / rehacer texto",
    ),
    (
        C(&[Cmd::Filter]),
        "Filtrar aquí (o escribir); Enter vuelve a la lista",
    ),
    (
        T("F6 en Buscar"),
        "Alternar nombre/texto literal (UTF8 exacto)",
    ),
    (
        C(&[Cmd::BookmarkAdd, Cmd::Bookmarks]),
        "Guardar carpeta / abrir favoritos y lugares",
    ),
    (
        C(&[Cmd::GoRecent, Cmd::GoStarred]),
        "Recientes / Destacados: listas virtuales, sin acciones de archivo",
    ),
    (
        C(&[Cmd::StarToggle]),
        "Destacar o quitar el elemento enfocado",
    ),
    (
        C(&[Cmd::OpenContaining]),
        "En Recientes/Destacados: abrir la carpeta contenedora",
    ),
    (
        T("Supr en Recientes/Destacados"),
        "Quitar la entrada de la lista; el archivo no se toca",
    ),
    (
        T("F2 / Supr en lugares"),
        "Etiqueta / quitar favorito; carpeta intacta",
    ),
    (
        C(&[Cmd::Search]),
        "Buscar nombres y contenido en subcarpetas",
    ),
    (
        C(&[Cmd::Commands]),
        "Buscar comandos y herramientas; Enter ejecuta",
    ),
    (
        T("F4 / Tab / Shift+Tab"),
        "En buscador: filtros de tamaño/fecha UTC y campos",
    ),
    (
        T("Enter / Alt+Enter"),
        "En buscador: abrir / localizar resultado",
    ),
    (
        T("report 2026 !draft"),
        "Combinar palabras y excluir coincidencias",
    ),
    (
        T("*.pdf / ext:pdf,md"),
        "Comodines o extensiones (sin distinguir caso)",
    ),
    (
        T("type:dir / file / link"),
        "Filtrar carpetas, archivos o enlaces",
    ),
    (
        T("\"informe anual\""),
        "Buscar una frase literal en el nombre",
    ),
    (T("Esc"), "Cancelar edición, quitar filtro o limpiar marcas"),
    (T(""), "BARRA DE LUGARES"),
    (T("Tab / Shift+Tab"), "Un panel: lista → lugares → pestañas"),
    (
        C(&[Cmd::FocusRail]),
        "Foco en lugares (también en doble panel)",
    ),
    (T("↑↓ Home/End · Lugares"), "Mover el cursor del lugar"),
    (
        T("Enter / Ctrl+Enter · Lugares"),
        "Abrir aquí / en pestaña nueva",
    ),
    (T("Shift+Enter · Lugares"), "Abrir en pestaña de fondo"),
    (T("Menú / Shift+F10 · Lugares"), "Menú del lugar enfocado"),
    (T("Alt+↑ / Alt+↓ · Lugares"), "Mover el marcador enfocado"),
    (
        T("Supr · Lugares"),
        "Quitar marcador; la carpeta no se toca",
    ),
    (T("Esc / → · Lugares"), "Volver a la lista"),
    (C(&[Cmd::Refresh]), "Actualizar conservando la selección"),
    (C(&[Cmd::Hidden]), "Mostrar / ocultar ocultos (global)"),
    (T(""), "VISTAS"),
    (C(&[Cmd::ViewList]), "Lista"),
    (C(&[Cmd::ViewGrid]), "Grilla de iconos"),
    (C(&[Cmd::ViewGroups]), "Grupos"),
    (C(&[Cmd::ViewColumns]), "Columnas tipo Finder"),
    (C(&[Cmd::ViewSpace]), "Espacio de tamaños"),
    (
        C(&[Cmd::SortMenu]),
        "Menú de orden: nombre/tamaño/fecha/tipo o invertir",
    ),
    (
        T("Cabeceras del listado"),
        "Orden del panel pulsado; flecha muestra dirección",
    ),
    (
        T("Ctrl+Shift+P → terminal"),
        "Abrir terminal del sistema en la carpeta actual",
    ),
    (C(&[Cmd::DualPane]), "Doble panel; Tab cambia de panel"),
    (C(&[Cmd::RailToggle]), "Barra de lugares ancha / compacta"),
    (
        C(&[Cmd::FocusCycle]),
        "Foco: lista → lugares → pestañas; otro panel en doble",
    ),
    (
        C(&[Cmd::ZoomIn, Cmd::ZoomOut]),
        "Zoom en Grilla (también Ctrl+rueda)",
    ),
    (C(&[Cmd::ZoomReset]), "Restablecer zoom de la Grilla"),
    (T("Flechas"), "Elemento; ←/→ columna en Columnas"),
    (T("Shift+flechas"), "Extender o reducir un rango visible"),
    (T("Ctrl+flechas"), "Mover foco conservando las marcas"),
    (
        T("Ctrl+Shift+flechas"),
        "Añadir rango a las marcas existentes",
    ),
    (T("Home / End"), "Primero / último del listado visible"),
    (
        T("Shift+Home / Shift+End"),
        "Extender rango hasta un extremo",
    ),
    (T("PageUp / PageDown"), "Recorrer páginas de elementos"),
    (T(""), "SELECCIÓN Y OPERACIONES"),
    (
        C(&[Cmd::MarkAdvance]),
        "Marcar / desmarcar y avanzar (estilo MC)",
    ),
    (
        C(&[Cmd::MarkToggle]),
        "Marcar / desmarcar sin mover el foco",
    ),
    (T("Ctrl+clic"), "Añadir / quitar de la selección"),
    (T("Shift+clic"), "Seleccionar un rango"),
    (
        C(&[Cmd::SelectAll, Cmd::InvertSelection]),
        "Seleccionar todo / invertir selección",
    ),
    (C(&[Cmd::SelectFiltered]), "Marcar filtrados y mostrar todo"),
    (C(&[Cmd::ClearSelection]), "Limpiar marcas y foco operativo"),
    (T("Botón derecho"), "Menú de elemento, lote o fondo"),
    (C(&[Cmd::ContextMenu]), "Menú del elemento enfocado"),
    (
        T("↑↓ Home/End Enter"),
        "Elegir y ejecutar en un menú abierto",
    ),
    (C(&[Cmd::SelectPattern]), "Seleccionar por patrón"),
    (C(&[Cmd::OpenWith]), "Abrir con otra aplicación"),
    (
        C(&[Cmd::LinkCopied, Cmd::CreateLinks]),
        "Enlace a lo copiado / crear enlaces",
    ),
    (C(&[Cmd::CopyPath]), "Copiar la ruta del elemento"),
    (C(&[Cmd::Terminal]), "Terminal en la carpeta actual"),
    (C(&[Cmd::Inspector]), "Mostrar / ocultar inspector"),
    (C(&[Cmd::Rename]), "Renombrar con vista previa"),
    (
        C(&[Cmd::ClipboardCopy, Cmd::ClipboardCut]),
        "Copiar / cortar en files (esta ventana)",
    ),
    (
        C(&[Cmd::ClipboardPaste]),
        "Pegar en files; texto en campos editables",
    ),
    (
        C(&[Cmd::NativePaste]),
        "Pegar MIME del sistema (con foco real)",
    ),
    (
        C(&[Cmd::OperationBar]),
        "Mostrar / ocultar barra de operaciones MC",
    ),
    (
        C(&[Cmd::CopyFiles]),
        "Copiar la selección: revisar y confirmar; Espacio: recalcular",
    ),
    (
        T("↑↓ PgUp/PgDn · Informe"),
        "Leer líneas/páginas; Home/End inicio/fin",
    ),
    (
        T("Ctrl+R · Informe"),
        "Reintentar sólo pendientes de Move multipadre revisado",
    ),
    (
        C(&[Cmd::MoveFiles]),
        "Mover la selección: revisar y confirmar",
    ),
    (C(&[Cmd::Trash]), "Enviar a Papelera"),
    (
        C(&[Cmd::DeletePermanently]),
        "Eliminar definitivamente, con confirmación",
    ),
    (C(&[Cmd::Preview]), "Vista previa del elemento"),
    (C(&[Cmd::SizesManager]), "Gestor de tamaños por carpeta"),
    (
        T("Paleta → SHA-256"),
        "Calcular archivos elegidos, comparar y copiar informe",
    ),
    (
        T("Comandos → sftp"),
        "Montaje SFTP; F2 perfiles, F4 informe, CtrlEnter conectar",
    ),
    (
        T("Espacio / Insert · Buscar"),
        "Marcar; Insert avanza, Shift extiende rango",
    ),
    (
        T("Ctrl+A / Ctrl+D · Buscar"),
        "Marcar todos / vaciar marcas (en resultados)",
    ),
    (
        T("F5 / Shift+F6 · Buscar"),
        "Copiar / mover resultados: capturar y revisar",
    ),
    (
        T("Ctrl+Enter · Buscar"),
        "Ubicaciones marcadas en pestañas de fondo",
    ),
    (
        T("F2 / Ctrl+S · Buscar"),
        "Búsquedas guardadas / guardar criterios actuales",
    ),
    (
        T("Alt+↑/→/← · Buscar"),
        "Subir / carpeta del resultado / raíz anterior; Enter busca",
    ),
    (T("F7 · Buscar"), "Ordenar resultados: ruta, nombre o tipo"),
    (
        T("F9 · Buscar"),
        "Nuevo recorrido por páginas; Enter repite ese modo",
    ),
    (
        T("Ctrl+PgUp / PgDn · Buscar"),
        "Página anterior conservada / siguiente; marcas por página",
    ),
    (T("F8 · Buscar"), "Fechas locales o últimas1/24/168horas"),
    (
        T("O / Shift+O · Tamaños"),
        "Olvidar fila / revisar y olvidar medidas; no borra archivos",
    ),
    (T("Alt+Enter · Tamaños"), "Abrir carpeta seleccionada"),
    (
        T("F5 · Tamaños"),
        "Recalcular sólo la carpeta medida seleccionada",
    ),
    (
        T("I · Tamaños"),
        "Informe; F5 actualiza, Ctrl+C solicita copia",
    ),
    (
        C(&[Cmd::SizeFocused]),
        "Calcular / cancelar tamaño de carpeta",
    ),
    (
        C(&[Cmd::SizesListed]),
        "Calcular / parar carpetas del listado",
    ),
    (
        T("≈ / ≥ en Tamaño"),
        "Valor previo / avance parcial; Ctrl+R invalida",
    ),
    (C(&[Cmd::NewFile]), "Crear archivo"),
    (C(&[Cmd::NewWindow]), "Nueva ventana en la carpeta actual"),
    (C(&[Cmd::NewFolder]), "Crear carpeta"),
    (C(&[Cmd::Undo]), "Deshacer cuando esté disponible"),
    (C(&[Cmd::Redo]), "Rehacer la última operación deshecha"),
    (C(&[Cmd::Properties]), "Propiedades y permisos"),
    (T(""), "PREVIEW Y CONFIGURACIÓN"),
    (T("Space / F3 / Esc"), "Abrir / cerrar vista previa"),
    (
        T("Ctrl+A / Ctrl+C"),
        "Seleccionar / copiar texto en preview",
    ),
    (
        T("Alt+↑ / Alt+↓"),
        "Elemento anterior / siguiente en preview",
    ),
    (
        T("Ctrl+rueda / +/- / 0"),
        "Zoom / ajustar imagen, PDF, vídeo o GIF",
    ),
    (
        T("R / Shift+R"),
        "Girar sólo la vista a derecha / izquierda",
    ),
    (T("P"), "Reproducir / pausar audio, vídeo o GIF"),
    (T("M"), "Activar / silenciar audio de vídeo"),
    (T("T"), "Alternar página / texto extraído del PDF"),
    (T("Ctrl+PgUp / PgDn"), "Página PDF anterior / siguiente"),
    (C(&[Cmd::Settings]), "Preferencias"),
    (C(&[Cmd::Actions]), "Acciones sobre archivos"),
    (C(&[Cmd::Help]), "Abrir / cerrar esta ayuda (Esc cierra)"),
];
#[derive(Default)]
pub struct State {
    pub top: usize,
    pub page: usize,
}
impl State {
    pub fn scroll(&mut self, delta: i32) {
        self.top = (self.top as i64 + i64::from(delta))
            .clamp(0, ITEMS.len().saturating_sub(self.page.max(1)) as i64)
            as usize;
    }
}
pub fn key(m: &mut Model, key: Key) {
    if matches!(key, Key::Escape | Key::F1 | Key::Enter | Key::KPEnter) {
        m.action(Action::HelpClose);
        return;
    }
    if let Some(s) = &mut m.help {
        let d = if key == Key::Down {
            1
        } else if key == Key::Up {
            -1
        } else if key == Key::PageDown {
            s.page as i32
        } else if key == Key::PageUp {
            -(s.page as i32)
        } else if key == Key::Home {
            i32::MIN
        } else if key == Key::End {
            i32::MAX
        } else {
            0
        };
        s.scroll(d);
    }
}
pub fn panel(w: i32, h: i32) -> Rect {
    let width = (w - 32).clamp(1, 860);
    let height = (h - 40).clamp(1, 650);
    Rect::new((w - width) / 2, (h - height) / 2, width, height)
}
fn button(p: &mut dyn Painter, m: &mut Model, r: Rect, label: &str, action: Action) {
    p.rounded_rect(r, 5 * m.radius, m.palette.selection);
    p.text(r, label, 12, m.palette.foreground, true, false);
    m.hits.push(Hit {
        rect: r,
        action,
        generation: m.generation,
    });
}
pub fn paint(p: &mut dyn Painter, m: &mut Model, w: i32, h: i32) {
    if m.help.is_none() {
        return;
    }
    m.hits.clear();
    let r = panel(w, h);
    let c = m.palette.clone();
    p.rounded_rect(r, 10 * m.radius, c.muted);
    p.rounded_rect(
        Rect::new(r.x + 1, r.y + 1, r.w - 2, r.h - 2),
        10 * m.radius,
        c.darker_background,
    );
    p.clip(Rect::new(
        r.x + 1,
        r.y + 1,
        (r.w - 2).max(0),
        (r.h - 2).max(0),
    ));
    p.text(
        Rect::new(r.x + 20, r.y + 10, r.w - 160, 30),
        "  Ayuda · Atajos",
        17,
        c.foreground,
        true,
        false,
    );
    button(
        p,
        m,
        Rect::new(r.x + r.w - 120, r.y + 10, 100, 30),
        "Cerrar · Esc",
        Action::HelpClose,
    );
    p.text(
        Rect::new(r.x + 20, r.y + 45, r.w - 40, 24),
        &format!(
            "Atajos: {} · Flechas o rueda para desplazar · PageUp / PageDown",
            m.shortcuts.label()
        ),
        11,
        c.foreground,
        true,
        false,
    );
    let body = Rect::new(r.x + 16, r.y + 78, r.w - 32, (r.h - 136).max(0));
    let page = (body.h / ROW).max(1) as usize;
    let state = m.help.as_mut().unwrap();
    state.page = page;
    state.scroll(0);
    let top = state.top;
    p.clip(body);
    let profile = m.shortcuts;
    for (rank, (keys, description)) in ITEMS.iter().enumerate().skip(top).take(page) {
        let y = body.y + (rank - top) as i32 * ROW;
        if keys.is_header() {
            p.text(
                Rect::new(body.x + 4, y, body.w - 8, ROW),
                description,
                11,
                c.accent,
                true,
                false,
            );
        } else {
            p.text(
                Rect::new(body.x + 4, y, 182, ROW),
                &keys.text(profile),
                11,
                c.bright_foreground,
                true,
                false,
            );
            p.text(
                Rect::new(body.x + 196, y, body.w - 200, ROW),
                description,
                11,
                c.foreground,
                true,
                false,
            );
        }
    }
    p.unclip();
    p.text(
        Rect::new(r.x + 20, r.y + r.h - 45, 180, 28),
        &format!(
            "{}–{} / {}",
            top + 1,
            (top + page).min(ITEMS.len()),
            ITEMS.len()
        ),
        11,
        c.foreground,
        true,
        false,
    );
    button(
        p,
        m,
        Rect::new(r.x + r.w - 248, r.y + r.h - 45, 108, 28),
        "  Anterior",
        Action::HelpScroll(-(page as i32)),
    );
    button(
        p,
        m,
        Rect::new(r.x + r.w - 132, r.y + r.h - 45, 112, 28),
        "  Siguiente",
        Action::HelpScroll(page as i32),
    );
    p.unclip();
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn help_keys_follow_the_shortcut_table_of_each_profile() {
        for profile in [Profile::Files, Profile::Nautilus] {
            for (keys, description) in ITEMS {
                if let C(cmds) = keys {
                    let expected: Vec<String> = cmds
                        .iter()
                        .map(|&c| shortcuts::label(profile, c))
                        .filter(|l| !l.is_empty())
                        .collect();
                    let text = keys.text(profile);
                    if expected.is_empty() {
                        assert_eq!(text, palette_only(profile), "{description}");
                    } else {
                        assert_eq!(text, expected.join(" / "), "{description}");
                    }
                }
            }
        }
        let row = |profile, description: &str| {
            ITEMS
                .iter()
                .find(|(_, d)| *d == description)
                .map(|(k, _)| k.text(profile).into_owned())
                .unwrap()
        };
        assert_eq!(row(Profile::Files, "Lista"), "Ctrl+2");
        assert_eq!(row(Profile::Nautilus, "Lista"), "Ctrl+1");
        assert_eq!(row(Profile::Nautilus, "Grilla de iconos"), "Ctrl+2");
        assert_eq!(
            row(Profile::Nautilus, "Propiedades y permisos"),
            "Ctrl+I / Alt+Enter"
        );
        assert_eq!(
            row(Profile::Nautilus, "Crear archivo"),
            palette_only(Profile::Nautilus)
        );
        assert_eq!(
            row(Profile::Nautilus, "Nueva ventana en la carpeta actual"),
            "Ctrl+N"
        );
        // Every command with a chord in some profile has a help row.
        for cmd in shortcuts::commands() {
            let bound = [Profile::Files, Profile::Nautilus]
                .iter()
                .any(|&p| !shortcuts::chords(p, cmd).is_empty());
            let listed = ITEMS
                .iter()
                .any(|(k, _)| matches!(k, C(cmds) if cmds.contains(&cmd)));
            let fixed = matches!(
                cmd,
                Cmd::NoticeFocus | Cmd::FocusTabs | Cmd::Open | Cmd::OpenInTab
            );
            assert!(!bound || listed || fixed, "{cmd:?} has no help row");
        }
    }
    #[test]
    fn scrolling_is_bounded_and_resize_reclamps() {
        let mut s = State { top: 0, page: 10 };
        s.scroll(i32::MAX);
        assert_eq!(s.top, ITEMS.len() - 10);
        s.page = 20;
        s.scroll(0);
        assert_eq!(s.top, ITEMS.len() - 20);
        s.scroll(i32::MIN);
        assert_eq!(s.top, 0);
    }
    #[test]
    fn help_fits_supported_sizes_and_only_advertises_implemented_views() {
        for (w, h) in [(700, 440), (1000, 560), (1600, 1000)] {
            let r = panel(w, h);
            assert!(r.x >= 0 && r.y >= 0 && r.x + r.w <= w && r.y + r.h <= h);
            assert!((r.h - 136) / ROW >= 8);
        }
        assert!(
            ITEMS
                .iter()
                .any(|(k, _)| k.text(Profile::Files) == "Ctrl+5")
        );
        assert!(!ITEMS.iter().any(|(_, d)| d.contains("F6")));
        assert!(
            ITEMS
                .iter()
                .filter(|(_, d)| d.contains("SFTP"))
                .all(|(_, d)| d.starts_with("Montaje SFTP"))
        );
        assert!(
            !ITEMS
                .iter()
                .any(|(_, d)| d.contains("Conectar SFTP") || d.contains("Montar SFTP"))
        );
    }
}
