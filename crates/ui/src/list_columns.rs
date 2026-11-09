//! Prioriza el nombre; los metadatos opcionales sólo usan el espacio sobrante.
#[derive(Clone, Copy)]
pub enum Profile {
    Single,
    Dual,
}
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub x: i32,
    pub w: i32,
}
pub struct Layout {
    pub name: Cell,
    pub size: Option<Cell>,
    pub date: Option<Cell>,
    /// File type, the last optional column (single-pane List view only).
    pub kind: Option<Cell>,
}
impl Layout {
    pub fn new(width: i32, profile: Profile) -> Self {
        let (start, pad, min_name, size_width, date_width, kind_width) = match profile {
            Profile::Single => (48, 10, 160, 104, 86, 92),
            Profile::Dual => (30, 6, 128, 74, 0, 0),
        };
        let available = (width - start - pad).max(0);
        let size = available >= min_name + 8 + size_width;
        let date = date_width > 0 && available >= min_name + 16 + size_width + date_width;
        let kind = date
            && kind_width > 0
            && available >= min_name + 24 + size_width + date_width + kind_width;
        let name_width = available
            - if size { 8 + size_width } else { 0 }
            - if date { 8 + date_width } else { 0 }
            - if kind { 8 + kind_width } else { 0 };
        let name = Cell {
            x: start,
            w: name_width,
        };
        let size = size.then_some(Cell {
            x: start + name_width + 8,
            w: size_width,
        });
        let date = date.then_some(Cell {
            x: start + name_width + 16 + size_width,
            w: date_width,
        });
        let kind = kind.then_some(Cell {
            x: start + name_width + 24 + size_width + date_width,
            w: kind_width,
        });
        Self {
            name,
            size,
            date,
            kind,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_keep_remaining_width_and_optional_cells_never_overlap_or_escape() {
        for profile in [Profile::Single, Profile::Dual] {
            for width in 0..=1200 {
                let l = Layout::new(width, profile);
                let mut end = l.name.x + l.name.w;
                assert!(l.name.w >= 0);
                if l.name.w > 0 {
                    assert!(end <= width);
                }
                for c in [l.size, l.date, l.kind].into_iter().flatten() {
                    assert!(c.x >= end + 8 && c.w > 0 && c.x + c.w <= width);
                    assert!(
                        l.name.w
                            >= match profile {
                                Profile::Single => 160,
                                Profile::Dual => 128,
                            }
                    );
                    end = c.x + c.w;
                }
            }
        }
        let dual = Layout::new(117, Profile::Dual);
        assert!(dual.name.w >= 80 && dual.size.is_none());
        let single = Layout::new(230, Profile::Single);
        assert!(single.name.w >= 170 && single.size.is_none() && single.date.is_none());
        let wide = Layout::new(900, Profile::Single);
        assert!(wide.kind.is_some() && wide.date.is_some());
        assert!(Layout::new(900, Profile::Dual).kind.is_none());
    }
}
