//! Visible sort criteria share existing model Actions, with no filesystem work.
use crate::{
    model::{Action, Hit},
    painter::{Painter, Rect},
};
use files_core::{listing::Sort, theme::Palette};
pub struct Header<'a> {
    pub sort: Sort,
    pub descending: bool,
    pub generation: u64,
    pub pane: Option<(u64, u64)>,
    pub query: &'a str,
    /// Row column layout; when every optional cell is visible the header
    /// cells line up with the row cells beneath them.
    pub columns: Option<&'a crate::list_columns::Layout>,
}
pub fn paint(
    p: &mut dyn Painter,
    area: Rect,
    palette: &Palette,
    hits: &mut Vec<Hit>,
    header: Header<'_>,
) {
    let compact = area.w < 300;
    let gap = 4;
    let side = if compact {
        if area.w >= 180 {
            36
        } else {
            28.min(((area.w - 2 * gap) / 3).max(0))
        }
    } else {
        80
    };
    let widths = [(area.w - 3 * side - 3 * gap).max(0), side, side, side];
    let mut spans = [(0, 0); 4];
    let mut x = area.x;
    for (span, width) in spans.iter_mut().zip(widths) {
        *span = (x, width);
        x += width + gap;
    }
    if let Some(l) = header.columns
        && let (Some(size), Some(date), Some(kind)) = (l.size, l.date, l.kind)
    {
        spans = [
            (area.x, l.name.x + l.name.w),
            (area.x + size.x, size.w),
            (area.x + date.x, date.w),
            (area.x + kind.x, kind.w),
        ];
    }
    let compact = compact && header.columns.is_none_or(|l| l.kind.is_none());
    for ((sort, icon, title), (x, width)) in [
        (Sort::Name, "", "NOMBRE"),
        (Sort::Size, "", "TAMAÑO"),
        (Sort::Modified, "", "FECHA"),
        (Sort::Type, "\u{f15b}", "TIPO"),
    ]
    .into_iter()
    .zip(spans)
    {
        if width <= 0 || area.h <= 0 {
            continue;
        }
        let rect = Rect::new(x, area.y, width, area.h);
        let active = header.sort == sort;
        let arrow = if active {
            crate::fonts::icon(
                if header.descending { "" } else { "" },
                if header.descending { "↓" } else { "↑" },
            )
        } else {
            ""
        };
        let fallback = match sort {
            Sort::Name => "N",
            Sort::Size => "T",
            Sort::Modified => "F",
            Sort::Type => "Ti",
        };
        let detailed = !compact || sort == Sort::Name && width >= 100;
        let mut label = if detailed {
            format!("{} {title} {arrow}", crate::fonts::icon(icon, fallback))
        } else {
            format!("{} {arrow}", crate::fonts::icon(icon, fallback))
        };
        if sort == Sort::Name && !header.query.is_empty() && !compact {
            label.push_str(" · ");
            label.push_str(header.query);
        }
        p.rect(
            rect,
            if active {
                palette.lighter_background
            } else {
                palette.darker_background
            },
        );
        p.text(
            Rect::new(rect.x + 4, rect.y, (rect.w - 8).max(0), rect.h),
            &label,
            10,
            if active {
                palette.accent
            } else {
                palette.dark_foreground
            },
            true,
            false,
        );
        let action = match header.pane {
            Some((id, revision)) => Action::PaneSort(id, revision, sort),
            None => Action::Sort(sort),
        };
        hits.push(Hit {
            rect,
            action,
            generation: header.generation,
        });
    }
}
