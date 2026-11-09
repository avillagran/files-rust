//! Omi, the Omarchy mascot, shown while Files is busy.
//!
//! The body is the official Omarchy logo (omarchy.org/brand/omarchy-logo.svg,
//! 1200 units, even-odd). Every edge of that path lies on a 40-unit grid, so
//! it is drawn here as 13 rectangles on a 30-cell grid. The eyes are two
//! identical rectangles mirrored around the center, as in the Omarchy Task
//! Manager mascot: they blink now and then (both, left first, right first),
//! roll a full circle every fourth turn and lean toward the pointer.
//!
//! Painting is pure; [`Omi::tick`] only asks for frames while busy and while
//! something moves, so an idle window never repaints for the mascot.
use crate::painter::{Painter, Rect};
use files_core::theme::Color;
use std::time::{Duration, Instant};

/// Body rectangles `(x, y, w, h)` in 30-cell units.
const BODY: [(i32, i32, i32, i32); 13] = [
    (0, 0, 30, 2),
    (0, 2, 2, 28),
    (14, 2, 2, 4),
    (28, 2, 2, 28),
    (4, 4, 10, 2),
    (22, 4, 4, 2),
    (4, 6, 2, 20),
    (24, 6, 2, 20),
    (2, 14, 2, 2),
    (6, 24, 18, 2),
    (14, 26, 2, 4),
    (2, 28, 12, 2),
    (18, 28, 10, 2),
];
/// Eye geometry as fractions of the mascot: (400,450,73,161) and
/// (715,450,73,161) in logo units.
const EYE_W: f32 = 0.0608;
const EYE_TOP: f32 = 0.375;
const EYE_H: f32 = 0.1342;
const EYE_LX: f32 = 0.3384;
const EYE_RX: f32 = 0.6008;
const ROLL_R: f32 = 0.022;
/// Full lean at 240 px from the center, capped at 3% of the mascot size.
const LOOK_REACH: f32 = 240.0;
const LOOK_MAX: f32 = 0.030;
const FRAME: Duration = Duration::from_millis(33);
const BLINK: Duration = Duration::from_millis(350);
const LAG: Duration = Duration::from_millis(90);
const ROLL: Duration = Duration::from_millis(850);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Turn {
    Both,
    LeftFirst,
    RightFirst,
    Roll,
}

pub struct Omi {
    next_turn: Instant,
    step: u32,
    seed: u64,
    turn: Option<(Turn, Instant)>,
    last_frame: Option<Instant>,
    look: (f32, f32),
    target: (f32, f32),
    /// Center of the last painted mascot, window coordinates.
    center: Option<(f32, f32)>,
}

impl Default for Omi {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl Omi {
    pub fn new(now: Instant) -> Self {
        let mut omi = Self {
            next_turn: now,
            step: 0,
            seed: 0x9e37_79b9_7f4a_7c15,
            turn: None,
            last_frame: None,
            look: (0.0, 0.0),
            target: (0.0, 0.0),
            center: None,
        };
        omi.next_turn = now + omi.pause();
        omi
    }

    /// 3 to 7.5 s between turns, from a small deterministic generator.
    fn pause(&mut self) -> Duration {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        Duration::from_millis(3000 + self.seed % 4500)
    }

    /// Advances the blink schedule and follows `pointer`; true when the
    /// mascot needs a new frame.
    pub fn tick(&mut self, now: Instant, busy: bool, pointer: Option<(i32, i32)>) -> bool {
        if !busy {
            self.turn = None;
            self.last_frame = None;
            self.center = None;
            return false;
        }
        if let Some(center) = self.center {
            self.target = look_target(pointer, center);
        }
        if let Some((turn, at)) = self.turn
            && now.saturating_duration_since(at) > duration(turn)
        {
            self.turn = None;
        }
        if self.turn.is_none() && now >= self.next_turn {
            let turn = match self.step % 4 {
                0 => Turn::Both,
                1 => Turn::LeftFirst,
                2 => Turn::RightFirst,
                _ => Turn::Roll,
            };
            self.step = self.step.wrapping_add(1);
            self.turn = Some((turn, now));
            self.next_turn = now + duration(turn) + self.pause();
        }
        let moving = self.turn.is_some()
            || (self.look.0 - self.target.0).abs() > 0.0005
            || (self.look.1 - self.target.1).abs() > 0.0005;
        let due = self
            .last_frame
            .is_none_or(|at| now.saturating_duration_since(at) >= FRAME);
        moving && due
    }

    /// Draws the mascot in the square `area`, leaning its eyes toward
    /// `pointer` (window coordinates) when known.
    pub fn paint(
        &mut self,
        p: &mut dyn Painter,
        area: Rect,
        tint: Color,
        pointer: Option<(i32, i32)>,
        now: Instant,
    ) {
        let size = area.w.min(area.h);
        if size <= 0 {
            return;
        }
        let x0 = area.x + (area.w - size) / 2;
        let y0 = area.y + (area.h - size) / 2;
        let s = size as f32;
        let cell = |v: i32| (v as f32 * s / 30.0).round() as i32;
        for (x, y, w, h) in BODY {
            p.rect(
                Rect::new(
                    x0 + cell(x),
                    y0 + cell(y),
                    cell(x + w) - cell(x),
                    cell(y + h) - cell(y),
                ),
                tint,
            );
        }
        let center = (x0 as f32 + s / 2.0, y0 as f32 + s / 2.0);
        self.center = Some(center);
        self.target = look_target(pointer, center);
        let dt = self.last_frame.map_or(1.0, |at| {
            now.saturating_duration_since(at).as_secs_f32() / 0.14
        });
        let k = dt.clamp(0.0, 1.0);
        self.look.0 += (self.target.0 - self.look.0) * k;
        self.look.1 += (self.target.1 - self.look.1) * k;
        self.last_frame = Some(now);
        let (open_l, open_r, angle) = self.pose(now);
        // The orbit starts at the resting place, so rest has no offset.
        let (rx, ry) = ((angle.cos() - 1.0) * ROLL_R, angle.sin() * ROLL_R);
        for (ex, open) in [(EYE_LX, open_l), (EYE_RX, open_r)] {
            let h = (EYE_H * s * open).max(1.2);
            let top = EYE_TOP + EYE_H * (1.0 - open) / 2.0 + ry + self.look.1;
            p.rect(
                Rect::new(
                    x0 + ((ex + rx + self.look.0) * s).round() as i32,
                    y0 + (top * s).round() as i32,
                    ((EYE_W * s).round() as i32).max(1),
                    (h.round() as i32).max(1),
                ),
                tint,
            );
        }
    }

    /// Eye openness (left, right) and roll angle in radians at `now`.
    fn pose(&self, now: Instant) -> (f32, f32, f32) {
        let Some((turn, at)) = self.turn else {
            return (1.0, 1.0, 0.0);
        };
        let t = now.saturating_duration_since(at);
        match turn {
            Turn::Both => (blink(t), blink(t), 0.0),
            Turn::LeftFirst => (blink(t), blink(t.saturating_sub(LAG)), 0.0),
            Turn::RightFirst => (blink(t.saturating_sub(LAG)), blink(t), 0.0),
            Turn::Roll => {
                let f = (t.as_secs_f32() / ROLL.as_secs_f32()).clamp(0.0, 1.0);
                let eased = if f < 0.5 {
                    2.0 * f * f
                } else {
                    1.0 - (-2.0 * f + 2.0).powi(2) / 2.0
                };
                let angle = eased * std::f32::consts::TAU;
                (1.0, 1.0, if f >= 1.0 { 0.0 } else { angle })
            }
        }
    }
}

fn duration(turn: Turn) -> Duration {
    match turn {
        Turn::Both => BLINK,
        Turn::LeftFirst | Turn::RightFirst => BLINK + LAG,
        Turn::Roll => ROLL,
    }
}

/// Openness during one blink: close 110 ms, hold 90 ms, open 150 ms.
fn blink(t: Duration) -> f32 {
    let ms = t.as_secs_f32() * 1000.0;
    if ms < 110.0 {
        let f = ms / 110.0;
        1.0 - f * f
    } else if ms < 200.0 {
        0.0
    } else if ms < 350.0 {
        let f = (ms - 200.0) / 150.0;
        1.0 - (1.0 - f) * (1.0 - f)
    } else {
        1.0
    }
}

fn look_target(pointer: Option<(i32, i32)>, center: (f32, f32)) -> (f32, f32) {
    let Some((px, py)) = pointer else {
        return (0.0, 0.0);
    };
    let (dx, dy) = (px as f32 - center.0, py as f32 - center.1);
    let d = (dx * dx + dy * dy).sqrt();
    if d < 1.0 {
        return (0.0, 0.0);
    }
    let f = (d / LOOK_REACH).min(1.0) * LOOK_MAX;
    (dx / d * f, dy / d * f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::painter::Image;

    #[derive(Default)]
    struct Rects(Vec<Rect>);
    impl Painter for Rects {
        fn rect(&mut self, r: Rect, _: Color) {
            self.0.push(r);
        }
        fn rounded_rect(&mut self, r: Rect, _: i32, _: Color) {
            self.0.push(r);
        }
        fn text(&mut self, _: Rect, _: &str, _: i32, _: Color, _: bool, _: bool) {}
        fn image(&mut self, _: Rect, _: &Image<'_>) {}
        fn clip(&mut self, _: Rect) {}
        fn unclip(&mut self) {}
    }

    #[test]
    fn body_fills_the_logo_grid_without_overlap() {
        let mut cells = [[false; 30]; 30];
        for (x, y, w, h) in BODY {
            for row in cells.iter_mut().skip(y as usize).take(h as usize) {
                for cell in row.iter_mut().skip(x as usize).take(w as usize) {
                    assert!(!*cell, "overlap at {x},{y}");
                    *cell = true;
                }
            }
        }
        // Outer frame rows and the gap in the bottom edge of the logo.
        assert!(cells[0].iter().all(|&c| c));
        assert!(!cells[29][16] && !cells[29][17] && cells[29][15] && cells[29][18]);
        let filled: usize = cells.iter().flatten().filter(|&&c| c).count();
        assert_eq!(
            filled,
            60 + 56 + 8 + 56 + 20 + 8 + 40 + 40 + 4 + 36 + 8 + 24 + 20
        );
    }

    #[test]
    fn paints_inside_its_square_and_symmetric_eyes_at_rest() {
        let now = Instant::now();
        let mut omi = Omi::new(now);
        let mut p = Rects::default();
        omi.paint(&mut p, Rect::new(10, 20, 60, 60), Color(1, 2, 3), None, now);
        assert_eq!(p.0.len(), 15);
        for r in &p.0 {
            assert!(
                r.x >= 10 && r.y >= 20 && r.x + r.w <= 70 && r.y + r.h <= 80,
                "{r:?}"
            );
        }
        let (l, rr) = (p.0[13], p.0[14]);
        assert_eq!(l.y, rr.y);
        assert_eq!(l.w, rr.w);
        // Mirrored around the center of the square (+/- 1 px of rounding).
        let mid2 = 2 * 10 + 60;
        assert!(((l.x + l.x + l.w) + (rr.x + rr.x + rr.w) - 2 * mid2).abs() <= 2);
    }

    #[test]
    fn eyes_lean_toward_the_pointer_within_the_cap() {
        let (x, y) = look_target(Some((1000, 40)), (40.0, 40.0));
        assert!((x - LOOK_MAX).abs() < 1e-6 && y.abs() < 1e-6);
        let (x, _) = look_target(Some((40 - 120, 40)), (40.0, 40.0));
        assert!((x + LOOK_MAX / 2.0).abs() < 1e-6);
        assert_eq!(look_target(None, (0.0, 0.0)), (0.0, 0.0));
    }

    #[test]
    fn frames_only_while_busy_and_moving() {
        let now = Instant::now();
        let mut omi = Omi::new(now);
        assert!(!omi.tick(now, false, None));
        // At rest and before the first turn nothing moves.
        assert!(!omi.tick(now, true, None));
        let later = omi.next_turn;
        assert!(omi.tick(later, true, None));
        assert!(omi.turn.is_some());
        // Idle stops everything at once.
        assert!(!omi.tick(later, false, None));
        assert!(omi.turn.is_none());
        assert_eq!(blink(Duration::ZERO), 1.0);
        assert_eq!(blink(Duration::from_millis(150)), 0.0);
        assert_eq!(blink(Duration::from_millis(400)), 1.0);
    }
}
