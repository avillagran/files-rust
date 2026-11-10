//! Paint-only zoom between a folder block and the whole Espacio map.
//!
//! The model's path, selection and focus change synchronously exactly as
//! without motion; this module only remembers the last painted layout of a
//! few folders (bounded snapshots) and, after a navigation to a child or to
//! the parent, maps painted rectangles through one animated affine transform
//! (see docs/DESIGN.md, "Motion"). The painter cannot blend, so geometry is
//! what moves: zoom in grows the opened block to the full map while the
//! child layout appears inside it; zoom out shrinks the child map into the
//! block it occupies in the parent while the parent appears around it.
//!
//! The size scan is asynchronous: when the destination layout is not ready
//! the block outline still zooms, and the children grow from their centres
//! (a short second stage) once the report arrives. Animations disabled means
//! no transition is ever started, so every frame is final. Nothing here runs
//! a timer; `animating` is read by `motion::schedule` after each paint.
use crate::painter::{Painter, Rect};
use files_core::{
    space_map::{Area, NodeKind, SpaceReport},
    theme::Palette,
};
use fltk_omarchy::anim::{self, Ease, Tween};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

/// Block ↔ full-map zoom. Larger movement than a row transition.
pub const ZOOM_MS: u32 = 220;
/// Children growing in when the scan arrives after (or during) the zoom.
pub const REVEAL_MS: u32 = 140;
/// Folders whose last painted layout is retained for zoom out.
const SNAPSHOTS: usize = 8;
/// Largest tiles retained per snapshot (the layout lists larger areas first).
const SNAPSHOT_TILES: usize = 512;

/// One painted tile of a retained layout, relative to the map origin.
#[derive(Clone, Debug)]
pub struct Tile {
    pub rect: Rect,
    pub name: Vec<u8>,
    pub kind: NodeKind,
    pub bytes: Option<u64>,
    pub files: u64,
}

#[derive(Clone)]
struct Snapshot {
    path: PathBuf,
    key: (u64, u64, Rect),
    map: Rect,
    tiles: Vec<Tile>,
}
impl Snapshot {
    fn tile(&self, name: &[u8]) -> Option<&Tile> {
        self.tiles.iter().find(|t| t.name == name)
    }
    fn absolute(&self, r: Rect) -> Rect {
        Rect::new(self.map.x + r.x, self.map.y + r.y, r.w, r.h)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Direction {
    In,
    Out,
}

/// Rectangle with fractional coordinates, so interpolation does not jitter.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RectF {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}
impl From<Rect> for RectF {
    fn from(r: Rect) -> Self {
        Self {
            x: r.x.into(),
            y: r.y.into(),
            w: r.w.into(),
            h: r.h.into(),
        }
    }
}
impl RectF {
    fn lerp(a: RectF, b: RectF, t: f64) -> RectF {
        let l = |a: f64, b: f64| a + (b - a) * t;
        RectF {
            x: l(a.x, b.x),
            y: l(a.y, b.y),
            w: l(a.w, b.w),
            h: l(a.h, b.h),
        }
    }
    fn round(self) -> Rect {
        let x0 = self.x.round() as i32;
        let y0 = self.y.round() as i32;
        let x1 = (self.x + self.w).round() as i32;
        let y1 = (self.y + self.h).round() as i32;
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }
}

/// `p' = p * s + t` per axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    sx: f64,
    sy: f64,
    tx: f64,
    ty: f64,
}
impl Affine {
    pub const IDENTITY: Affine = Affine {
        sx: 1.,
        sy: 1.,
        tx: 0.,
        ty: 0.,
    };
    /// Maps rectangle `from` exactly onto `to`.
    fn between(from: RectF, to: RectF) -> Affine {
        let sx = if from.w.abs() > f64::EPSILON {
            to.w / from.w
        } else {
            1.
        };
        let sy = if from.h.abs() > f64::EPSILON {
            to.h / from.h
        } else {
            1.
        };
        Affine {
            sx,
            sy,
            tx: to.x - from.x * sx,
            ty: to.y - from.y * sy,
        }
    }
    fn apply_f(&self, r: RectF) -> RectF {
        RectF {
            x: r.x * self.sx + self.tx,
            y: r.y * self.sy + self.ty,
            w: r.w * self.sx,
            h: r.h * self.sy,
        }
    }
    pub fn apply(&self, r: Rect) -> Rect {
        if *self == Self::IDENTITY {
            return r;
        }
        self.apply_f(r.into()).round()
    }
}

struct Transition {
    direction: Direction,
    /// Layout that was on screen when the navigation happened (the parent
    /// for zoom in, the child for zoom out), if it had been painted.
    from: Option<Snapshot>,
    /// Cached parent layout for zoom out, drawn around the shrinking child
    /// until the parent's own report is ready.
    parent: Option<Snapshot>,
    /// The folder's block inside the parent layout (absolute), or `None` to
    /// zoom from/to the map centre.
    block: Option<Rect>,
    /// The folder's own tile (name, kind, size) for its moving outline.
    tile: Tile,
    tween: Tween,
}

/// What a frame paints while a zoom or a reveal is in flight.
pub struct Plan {
    /// Applied to live tiles (absolute rectangles of the current layout).
    live: Affine,
    /// Children growing from their centres: 1.0 when settled.
    reveal: f64,
    /// Moving block rectangle (the folder's tile) and the transform for the
    /// layout drawn behind (zoom in: the old parent) or in front (zoom out:
    /// the old child map).
    frame: Option<Rect>,
    back: Option<Affine>,
    front: Option<Affine>,
    /// The zoom is still moving (not only a reveal).
    pub moving: bool,
    /// Zoom in reached full size before the scan: keep the outline at the
    /// map edge so the children visibly arrive inside it.
    pub outline: bool,
}
impl Plan {
    /// Painted rectangle of a live tile whose final rectangle is `r`.
    pub fn live(&self, r: Rect) -> Rect {
        let r = self.live.apply(r);
        if self.reveal >= 1. {
            return r;
        }
        let f = RectF::from(r);
        let (cx, cy) = (f.x + f.w / 2., f.y + f.h / 2.);
        RectF {
            x: cx - f.w * self.reveal / 2.,
            y: cy - f.h * self.reveal / 2.,
            w: f.w * self.reveal,
            h: f.h * self.reveal,
        }
        .round()
    }
    /// True when live tiles are not at their final rectangles this frame.
    pub fn transforms(&self) -> bool {
        self.live != Affine::IDENTITY || self.reveal < 1.
    }
}

#[derive(Default)]
pub struct Zoom {
    /// Path the map last painted; a different path at paint time is a
    /// navigation.
    shown: Option<PathBuf>,
    cache: VecDeque<Snapshot>,
    transition: Option<Transition>,
    /// Destination whose layout was not ready when the zoom started, and the
    /// reveal tween once it is.
    reveal: Option<(PathBuf, Option<Tween>)>,
    /// The pending reveal follows a zoom in (outline kept at the map edge).
    came_in: bool,
    animating: bool,
}

impl Zoom {
    /// True when the last paint left a zoom or reveal running.
    pub fn animating(&self) -> bool {
        self.animating
    }

    /// Another view (or the dual pane) painted: forget the shown path so a
    /// later return to Espacio is not mistaken for a navigation. Retained
    /// layouts stay for a later zoom out.
    pub fn leave(&mut self) {
        self.shown = None;
        self.transition = None;
        self.reveal = None;
        self.animating = false;
    }

    /// Retains the current layout of `path` (bounded) for later zooms.
    pub fn remember(
        &mut self,
        path: &Path,
        key: (u64, u64),
        map: Rect,
        areas: &[Area],
        report: &SpaceReport,
    ) {
        let key = (key.0, key.1, map);
        if self
            .cache
            .front()
            .is_some_and(|s| s.path == path && s.key == key)
        {
            return;
        }
        self.cache.retain(|s| s.path != path);
        let tiles = areas
            .iter()
            .take(SNAPSHOT_TILES)
            .filter_map(|a| {
                let node = report.nodes.get(a.node)?;
                let x0 = a.x.round() as i32;
                let x1 = (a.x + a.w).round() as i32;
                let y0 = a.y.round() as i32;
                let y1 = (a.y + a.h).round() as i32;
                (x1 > x0 && y1 > y0).then(|| Tile {
                    rect: Rect::new(x0, y0, x1 - x0, y1 - y0),
                    name: node.name.clone(),
                    kind: node.kind,
                    bytes: node.bytes,
                    files: node.files,
                })
            })
            .collect();
        self.cache.push_front(Snapshot {
            path: path.to_path_buf(),
            key,
            map,
            tiles,
        });
        self.cache.truncate(SNAPSHOTS);
    }

    fn snapshot(&self, path: &Path) -> Option<&Snapshot> {
        self.cache.iter().find(|s| s.path == path)
    }

    /// Detects a navigation since the last paint and starts, retargets or
    /// cancels the zoom; then returns what this frame paints (None when
    /// everything is final). `full` is the map rectangle, `ready` whether the
    /// current folder's layout is painted this frame.
    pub fn plan(&mut self, path: &Path, full: Rect, ready: bool) -> Option<Plan> {
        self.animating = false;
        if self.shown.as_deref() != Some(path) {
            let previous = self.shown.replace(path.to_path_buf());
            self.transition = None;
            self.reveal = None;
            if let Some(previous) = previous
                && anim::enabled()
            {
                self.start(&previous, path, ready);
            }
        }
        let mut plan = Plan {
            live: Affine::IDENTITY,
            reveal: 1.,
            frame: None,
            back: None,
            front: None,
            moving: false,
            outline: false,
        };
        if let Some((target, tween)) = &mut self.reveal
            && target == path
            && ready
        {
            let t = tween.get_or_insert_with(|| Tween::start(REVEAL_MS, Ease::OutCubic));
            if t.done() {
                self.reveal = None;
            } else {
                plan.reveal = t.value().clamp(0., 1.);
                self.animating = true;
            }
        }
        if let Some(z) = &self.transition {
            if z.tween.done() {
                self.transition = None;
            } else {
                let v = z.tween.value().clamp(0., 1.);
                let full_f = RectF::from(full);
                let block = RectF::from(z.block.unwrap_or_else(|| centre(full)));
                match z.direction {
                    Direction::In => {
                        let frame = RectF::lerp(block, full_f, v);
                        plan.live = Affine::between(full_f, frame);
                        plan.back = Some(Affine::between(block, frame));
                        plan.frame = Some(frame.round());
                    }
                    Direction::Out => {
                        let frame = RectF::lerp(full_f, block, v);
                        let around = Affine::between(block, frame);
                        plan.live = around;
                        plan.back = Some(around);
                        plan.front = z
                            .from
                            .as_ref()
                            .map(|s| Affine::between(s.map.into(), frame));
                        plan.frame = Some(frame.round());
                    }
                }
                plan.moving = true;
                self.animating = true;
            }
        }
        plan.outline = !ready
            && self.transition.is_none()
            && self
                .reveal
                .as_ref()
                .is_some_and(|(target, _)| target == path)
            && self.came_in;
        (plan.moving || plan.outline || plan.reveal < 1.).then_some(plan)
    }

    fn start(&mut self, previous: &Path, path: &Path, ready: bool) {
        let (direction, child) = if path.parent() == Some(previous) {
            (Direction::In, path)
        } else if previous.parent() == Some(path) {
            (Direction::Out, previous)
        } else {
            return;
        };
        let Some(name) = child.file_name() else {
            return;
        };
        let name = name.as_encoded_bytes().to_vec();
        let parent_path = child.parent().expect("child has a parent");
        let parent = self.snapshot(parent_path).cloned();
        let from = match direction {
            Direction::In => parent.clone(),
            Direction::Out => self.snapshot(child).cloned(),
        };
        if direction == Direction::Out && from.is_none() && parent.is_none() {
            // Nothing painted on either side: nothing to zoom.
            return;
        }
        let found = parent
            .as_ref()
            .and_then(|s| s.tile(&name).map(|t| (s.absolute(t.rect), t.clone())));
        let (block, tile) = match found {
            Some((rect, tile)) => (Some(rect), tile),
            None => (
                None,
                Tile {
                    rect: Rect::new(0, 0, 0, 0),
                    name,
                    kind: NodeKind::Directory,
                    bytes: None,
                    files: 0,
                },
            ),
        };
        // Zoom in draws the parent behind; zoom out draws the child in
        // front and the cached parent behind.
        self.came_in = direction == Direction::In;
        self.transition = Some(Transition {
            direction,
            from,
            parent: (direction == Direction::Out).then_some(parent).flatten(),
            block,
            tile,
            tween: Tween::start(ZOOM_MS, Ease::OutCubic),
        });
        if !ready {
            self.reveal = Some((path.to_path_buf(), None));
        }
    }

    /// Layout behind the live tiles: the old parent around a zoom in, or the
    /// cached parent around a zoom out until the parent's report is ready.
    /// Then the moving block itself (its fill, name and icon).
    pub fn paint_back(
        &self,
        p: &mut dyn Painter,
        c: &Palette,
        radius: i32,
        plan: &Plan,
        ready: bool,
    ) {
        let Some(z) = &self.transition else {
            return;
        };
        let source = match z.direction {
            Direction::In => z.from.as_ref(),
            Direction::Out => z.parent.as_ref().filter(|_| !ready),
        };
        if let (Some(s), Some(a)) = (source, plan.back) {
            for t in s.tiles.iter().filter(|t| t.name != z.tile.name) {
                crate::space_map_view::paint_tile(p, c, radius, a.apply(s.absolute(t.rect)), t);
            }
        }
        if let Some(frame) = plan.frame
            && (z.direction == Direction::Out || !ready)
        {
            crate::space_map_view::paint_tile(p, c, radius, frame, &z.tile);
        }
    }

    /// Layout in front of the live tiles: the old child map shrinking into
    /// its block during a zoom out.
    pub fn paint_front(&self, p: &mut dyn Painter, c: &Palette, radius: i32, plan: &Plan) {
        let (Some(z), Some(a)) = (&self.transition, plan.front) else {
            return;
        };
        if let Some(s) = &z.from {
            for t in &s.tiles {
                crate::space_map_view::paint_tile(p, c, radius, a.apply(s.absolute(t.rect)), t);
            }
        }
    }

    /// Outline of the opened folder at the map edge while its scan runs.
    pub fn paint_outline(&self, p: &mut dyn Painter, c: &Palette, radius: i32, full: Rect) {
        let fill = crate::space_map_view::tile_fill(c, self.outline_name());
        crate::motion::outline(p, full, 6 * radius, 2, fill);
    }

    fn outline_name(&self) -> &[u8] {
        self.reveal
            .as_ref()
            .and_then(|(path, _)| path.file_name())
            .map_or(&[], |n| n.as_encoded_bytes())
    }
}

/// A quarter-size rectangle at the centre of `full`, the zoom anchor when
/// the folder's block is not known.
fn centre(full: Rect) -> Rect {
    Rect::new(
        full.x + full.w * 3 / 8,
        full.y + full.h * 3 / 8,
        (full.w / 4).max(1),
        (full.h / 4).max(1),
    )
}
