//! Exact breadcrumb targets and bounded chip layout; no filesystem probes.
use std::path::{Path, PathBuf};

pub const OVERFLOW_WIDTH: i32 = 28;
pub const GAP: i32 = 14;
pub const MIN_CHIP_WIDTH: i32 = 24;
pub const MAX_CHIP_WIDTH: i32 = 180;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    /// Display-only, escaped label. Never reconstruct a path from this text.
    pub label: String,
    pub path: PathBuf,
    pub home: bool,
}

/// Full lexical ancestry in root-to-current order. The supplied path, including
/// raw filename bytes, is kept as the current target; nothing is canonicalized.
/// The home chip is abbreviated, but its parents remain available for overflow.
pub fn build(path: &Path, home: &Path) -> Vec<Crumb> {
    // CLI startup can precede Model's normal navigation admission. Reject the
    // request before allocating one owned prefix per component.
    if files_core::navigation_history::validate(path).is_err() {
        return Vec::new();
    }
    let mut crumbs: Vec<_> = path
        .ancestors()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| {
            let is_home = p == home;
            let label = if is_home {
                "Inicio".into()
            } else {
                let component = p.components().next_back().map(|c| c.as_os_str());
                files_core::name_label::label(component.unwrap_or(p.as_os_str())).into_owned()
            };
            Crumb {
                label,
                path: p.to_path_buf(),
                home: is_home,
            }
        })
        .collect();
    crumbs.reverse();
    crumbs
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    /// Exact ancestor prefix hidden by compression; never includes current.
    pub hidden: Vec<Crumb>,
    /// Visible suffix, paired with each chip's width (including text padding).
    /// A compact current chip can be narrower than the normal minimum.
    pub visible: Vec<(Crumb, i32)>,
    /// Zero if there is no overflow or there is no available pixel.
    pub overflow_width: i32,
    /// Horizontal gap between controls, never after the final control.
    pub gap: i32,
    pub used_width: i32,
}

fn width(label: &str, measure: &mut impl FnMut(&str) -> f32) -> i32 {
    let measured = measure(label);
    let text = if measured.is_finite() {
        measured.max(0.).ceil().min((MAX_CHIP_WIDTH - 20) as f32) as i32
    } else {
        MAX_CHIP_WIDTH - 20
    };
    (text + 20).clamp(MIN_CHIP_WIDTH, MAX_CHIP_WIDTH)
}

fn current_only(crumbs: &[Crumb], available: i32, current_width: i32) -> Layout {
    let last = crumbs.len() - 1;
    let width = current_width.min(available);
    Layout {
        hidden: crumbs[..last].to_vec(),
        visible: vec![(crumbs[last].clone(), width)],
        overflow_width: 0,
        gap: 0,
        used_width: width,
    }
}

/// Show the full lexical route when it fits, then the longest fitting suffix.
/// Only ancestors enter overflow; the current directory always retains its own
/// display slot. If a separate overflow button cannot fit, painting attaches
/// the ancestor menu to that current-directory control instead.
///
/// `measure` must use the same font/size as painting. All returned widths and
/// `used_width` are nonnegative and bounded by `available.max(0)`. At zero width
/// the current slot has zero width and cannot produce a hit rectangle.
pub fn layout(crumbs: &[Crumb], available: i32, mut measure: impl FnMut(&str) -> f32) -> Layout {
    if crumbs.is_empty() {
        return Layout::default();
    }
    let available = available.max(0);
    let widths: Vec<_> = crumbs
        .iter()
        .map(|c| width(&c.label, &mut measure))
        .collect();
    let last = crumbs.len() - 1;
    if available < MIN_CHIP_WIDTH {
        return current_only(crumbs, available, widths[last]);
    }
    for start in 0..=last {
        let overflow_width = if start == 0 { 0 } else { OVERFLOW_WIDTH };
        let leading = if start == 0 { 0 } else { OVERFLOW_WIDTH + GAP };
        let budget = available.saturating_sub(leading);
        if budget < MIN_CHIP_WIDTH {
            return current_only(crumbs, available, widths[last]);
        }
        let count = crumbs.len() - start;
        let natural = widths[start..]
            .iter()
            .fold(0i32, |total, w| total.saturating_add(*w))
            .saturating_add(GAP.saturating_mul((count - 1).min(i32::MAX as usize) as i32));
        if natural > budget && start != last {
            continue;
        }
        let visible = crumbs[start..]
            .iter()
            .cloned()
            .zip(widths[start..].iter().copied())
            .map(|(c, w)| (c, w.min(budget)))
            .collect();
        return Layout {
            hidden: crumbs[..start].to_vec(),
            visible,
            overflow_width,
            gap: GAP,
            used_width: leading + natural.min(budget),
        };
    }
    unreachable!("the current chip or compact current control always fits")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(s: &str) -> f32 {
        s.chars().count() as f32 * 7.
    }

    fn targets(layout: &Layout) -> Vec<&Path> {
        layout
            .hidden
            .iter()
            .map(|c| c.path.as_path())
            .chain(layout.visible.iter().map(|(c, _)| c.path.as_path()))
            .collect()
    }

    fn assert_bounds(result: &Layout, available: i32) {
        assert!(result.used_width >= 0 && result.used_width <= available.max(0));
        let mut x = result.overflow_width;
        if result.overflow_width > 0 && !result.visible.is_empty() {
            x += result.gap;
        }
        for (i, (_, width)) in result.visible.iter().enumerate() {
            assert!(*width >= 0 && *width <= MAX_CHIP_WIDTH);
            if result.visible.len() > 1 || result.overflow_width > 0 {
                assert!(*width >= MIN_CHIP_WIDTH);
            }
            if i > 0 {
                x += result.gap;
            }
            x += width;
            assert!(
                x <= available.max(0),
                "controls exceed {available}px: {result:?}"
            );
        }
        assert_eq!(x, result.used_width);
    }

    #[test]
    fn invalid_startup_paths_are_rejected_before_prefix_construction() {
        let home = Path::new("/home/fixture");
        for bad in [
            Path::new(""),
            Path::new("relative/child"),
            Path::new("/nul\0/child"),
        ] {
            assert!(build(bad, home).is_empty());
        }
        let at_limit = format!(
            "/{}",
            "a".repeat(files_core::navigation_history::MAX_PATH_BYTES - 1)
        );
        let accepted = build(Path::new(&at_limit), home);
        assert_eq!(accepted.last().unwrap().path, Path::new(&at_limit));
        let too_long = format!("{at_limit}/child");
        assert!(build(Path::new(&too_long), home).is_empty());
        let deep = format!(
            "/{}",
            "a/".repeat(files_core::navigation_history::MAX_PATH_BYTES)
        );
        assert!(build(Path::new(&deep), home).is_empty());
    }

    #[test]
    fn home_abbreviation_keeps_every_parent_reachable() {
        let crumbs = build(
            Path::new("/home/fixture/Projects/Files"),
            Path::new("/home/fixture"),
        );
        assert_eq!(
            crumbs.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
            ["/", "home", "Inicio", "Projects", "Files"]
        );
        let result = layout(&crumbs, 800, measure);
        assert!(
            result.hidden.is_empty(),
            "a fitting route needs no overflow"
        );
        assert_eq!(result.visible[0].0.label, "/");
        assert!(
            result
                .visible
                .iter()
                .any(|(c, _)| c.home && c.label == "Inicio")
        );
        assert_eq!(
            targets(&result),
            crumbs.iter().map(|c| c.path.as_path()).collect::<Vec<_>>()
        );
        assert_bounds(&result, 800);
    }

    #[test]
    fn suffix_and_current_survive_compression_without_dropping_targets() {
        let crumbs = build(
            Path::new("/one/two/three/four/five"),
            Path::new("/home/fixture"),
        );
        for available in 0..=600 {
            let result = layout(&crumbs, available, measure);
            assert_bounds(&result, available);
            assert_eq!(
                targets(&result),
                crumbs.iter().map(|c| c.path.as_path()).collect::<Vec<_>>()
            );
            if let Some((current, _)) = result.visible.last() {
                assert_eq!(current.path, Path::new("/one/two/three/four/five"));
                let first = crumbs
                    .iter()
                    .position(|c| c.path == result.visible[0].0.path)
                    .unwrap();
                assert_eq!(
                    result
                        .visible
                        .iter()
                        .map(|(c, _)| &c.path)
                        .collect::<Vec<_>>(),
                    crumbs[first..].iter().map(|c| &c.path).collect::<Vec<_>>()
                );
            }
            assert!(
                result
                    .hidden
                    .iter()
                    .all(|c| c.path != Path::new("/one/two/three/four/five"))
            );
        }
    }

    #[test]
    fn compact_viewports_keep_current_separate_from_hidden_ancestors() {
        let crumbs = build(
            Path::new("/home/fixture/long-current-name"),
            Path::new("/home/fixture"),
        );
        for available in [-20, 0, 1, 10, 23, 24, 42, 65] {
            let result = layout(&crumbs, available, measure);
            assert_eq!(result.visible.len(), 1);
            assert_eq!(
                result.visible[0].0.path,
                Path::new("/home/fixture/long-current-name")
            );
            assert_eq!(result.visible[0].1, available.max(0));
            assert_eq!(result.hidden, crumbs[..crumbs.len() - 1]);
            assert_eq!(result.overflow_width, 0);
            assert_bounds(&result, available);
        }
        let result = layout(&crumbs, 66, measure);
        assert_eq!(result.visible.len(), 1);
        assert_eq!(result.visible[0].1, MIN_CHIP_WIDTH);
        assert_eq!(result.overflow_width, OVERFLOW_WIDTH);
        assert_bounds(&result, 66);
    }

    #[test]
    fn root_and_non_home_prefixes_are_not_mistaken_for_home() {
        let root = build(Path::new("/"), Path::new("/home/fixture"));
        assert_eq!(root.len(), 1);
        assert_eq!(root[0].path, Path::new("/"));
        let result = layout(&root, 100, measure);
        assert!(result.hidden.is_empty());
        assert_eq!(result.visible.len(), 1);
        assert_bounds(&result, 100);
        let other = build(
            Path::new("/home/fixture-other/child"),
            Path::new("/home/fixture"),
        );
        assert!(!other.iter().any(|c| c.home));
        let result = layout(&other, 1000, measure);
        assert!(result.hidden.is_empty());
        assert_eq!(result.visible[0].0.label, "/");
        assert_bounds(&result, 1000);
    }

    #[test]
    fn measurement_drives_widths_and_long_current_is_clipped_to_budget() {
        let crumbs = build(Path::new("/café/日本語👩‍💻"), Path::new("/unrelated"));
        let thin = layout(&crumbs, 240, |_| 1.);
        let wide = layout(&crumbs, 240, |_| 150.);
        assert!(thin.hidden.len() < wide.hidden.len());
        assert_bounds(&thin, 240);
        assert_bounds(&wide, 240);
        let single = build(Path::new("/"), Path::new("/unrelated"));
        for measured in [-1., f32::NAN, f32::INFINITY, f32::MAX] {
            let result = layout(&single, 60, |_| measured);
            assert_bounds(&result, 60);
        }
    }

    #[cfg(unix)]
    #[test]
    fn invalid_bytes_and_literal_escapes_have_distinct_labels_and_exact_targets() {
        use std::os::unix::ffi::OsStrExt;
        let raw = Path::new(std::ffi::OsStr::from_bytes(b"/raw-\xff/child\n/leaf"));
        let literal = Path::new("/raw-\\xFF/child\\n/leaf");
        let crumbs = build(raw, Path::new("/unrelated"));
        let other = build(literal, Path::new("/unrelated"));
        assert_eq!(
            crumbs.last().unwrap().path.as_os_str().as_bytes(),
            raw.as_os_str().as_bytes()
        );
        assert_ne!(crumbs[1].label, other[1].label);
        assert_ne!(crumbs[2].label, other[2].label);
        assert!(
            crumbs
                .iter()
                .all(|c| !c.label.chars().any(char::is_control))
        );
        let result = layout(&crumbs, 80, measure);
        let recovered: Vec<_> = targets(&result)
            .into_iter()
            .map(|p| p.as_os_str().as_bytes())
            .collect();
        assert_eq!(
            recovered,
            crumbs
                .iter()
                .map(|c| c.path.as_os_str().as_bytes())
                .collect::<Vec<_>>()
        );
        assert_bounds(&result, 80);
    }
}
