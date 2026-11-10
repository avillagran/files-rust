//! Short paint-only transitions for the file views.
//!
//! Rules (see docs/DESIGN.md, "Motion"): input never waits for motion, so
//! `selected`, marks and the logical scroll target change synchronously and
//! only painted values animate. Every transition is at most 160 ms, retargets
//! instead of queueing, uses the toolkit clock (`anim::now`) and jumps to its
//! final value when animations are disabled. Nothing here runs a timer: the
//! canvas asks `State::animating` after painting and requests one more frame
//! only while something is mid-flight, so an idle window schedules nothing.
use crate::painter::{EffectId, Image, Painter, Rect, TextureId, Uniforms};
use files_core::theme::Color;
use fltk_omarchy::anim::{self, Anim, Ease, Lerp, Tween};
use std::path::{Path, PathBuf};

/// Keyboard-focus ring glide between rows.
pub const RING_MS: u32 = 80;
/// Hover highlight fade-in.
pub const HOVER_MS: u32 = 60;
/// Items fading in when a folder first shows its listing.
pub const LISTING_MS: u32 = 120;
/// First opacity of a freshly listed item.
pub const LISTING_FROM: f64 = 0.4;
/// Thumbnail fade-in once its bitmap arrives.
pub const THUMB_MS: u32 = 120;

impl Lerp for Rect {
    fn lerp(a: Rect, b: Rect, t: f64) -> Rect {
        Rect::new(
            i32::lerp(a.x, b.x, t),
            i32::lerp(a.y, b.y, t),
            i32::lerp(a.w, b.w, t),
            i32::lerp(a.h, b.h, t),
        )
    }
}

/// `fg` over `bg` with opacity `a` (1.0 returns `fg` exactly).
pub fn mix(fg: Color, bg: Color, a: f64) -> Color {
    let a = a.clamp(0.0, 1.0);
    if a >= 1.0 {
        return fg;
    }
    let ch = |f: u8, b: u8| (f64::from(b) + (f64::from(f) - f64::from(b)) * a).round() as u8;
    Color(ch(fg.0, bg.0), ch(fg.1, bg.1), ch(fg.2, bg.2))
}

/// Draws everything it forwards as if it had opacity `alpha` over `bg`.
/// Text, fills and images fade toward `bg`; at alpha 1 it is transparent.
pub struct FadePainter<'a> {
    inner: &'a mut dyn Painter,
    bg: Color,
    alpha: f64,
}
impl<'a> FadePainter<'a> {
    pub fn new(inner: &'a mut dyn Painter, bg: Color, alpha: f64) -> Self {
        Self { inner, bg, alpha }
    }
    fn c(&self, c: Color) -> Color {
        mix(c, self.bg, self.alpha)
    }
    fn blend(&self, img: &Image<'_>) -> Option<Vec<u8>> {
        if self.alpha >= 1.0 {
            return None;
        }
        let a = self.alpha.clamp(0.0, 1.0);
        let bg = [self.bg.0, self.bg.1, self.bg.2];
        Some(
            img.rgb
                .iter()
                .enumerate()
                .map(|(i, &v)| {
                    (f64::from(bg[i % 3]) + (f64::from(v) - f64::from(bg[i % 3])) * a).round() as u8
                })
                .collect(),
        )
    }
}
impl Painter for FadePainter<'_> {
    fn rect(&mut self, r: Rect, c: Color) {
        let c = self.c(c);
        self.inner.rect(r, c);
    }
    fn rounded_rect(&mut self, r: Rect, radius: i32, c: Color) {
        let c = self.c(c);
        self.inner.rounded_rect(r, radius, c);
    }
    fn text(&mut self, r: Rect, text: &str, size: i32, c: Color, mono: bool, right: bool) {
        let c = self.c(c);
        self.inner.text(r, text, size, c, mono, right);
    }
    fn literal_text(&mut self, r: Rect, text: &str, size: i32, c: Color) {
        let c = self.c(c);
        self.inner.literal_text(r, text, size, c);
    }
    fn text_width(&mut self, text: &str, size: i32, mono: bool) -> f32 {
        self.inner.text_width(text, size, mono)
    }
    fn styled_text(&mut self, r: Rect, text: &str, size: i32, c: Color, bold: bool, italic: bool) {
        let c = self.c(c);
        self.inner.styled_text(r, text, size, c, bold, italic);
    }
    fn styled_width(&mut self, text: &str, size: i32, bold: bool, italic: bool) -> f32 {
        self.inner.styled_width(text, size, bold, italic)
    }
    fn image(&mut self, r: Rect, image: &Image<'_>) {
        match self.blend(image) {
            Some(rgb) => self.inner.image(
                r,
                &Image {
                    rgb: &rgb,
                    width: image.width,
                    height: image.height,
                },
            ),
            None => self.inner.image(r, image),
        }
    }
    fn image_scaled(&mut self, r: Rect, image: &Image<'_>) {
        match self.blend(image) {
            Some(rgb) => self.inner.image_scaled(
                r,
                &Image {
                    rgb: &rgb,
                    width: image.width,
                    height: image.height,
                },
            ),
            None => self.inner.image_scaled(r, image),
        }
    }
    fn clip(&mut self, r: Rect) {
        self.inner.clip(r);
    }
    fn unclip(&mut self) {
        self.inner.unclip();
    }
    fn effect(&mut self, region: Rect, id: EffectId, u: &Uniforms) -> bool {
        self.inner.effect(region, id, u)
    }
    fn capture(&mut self, region: Rect) -> Option<TextureId> {
        self.inner.capture(region)
    }
}

/// Asks for one more frame while any transition is mid-flight; an idle
/// window requests nothing.
pub fn schedule(m: &crate::model::Model, canvas: &fltk::frame::Frame) {
    if m.motion.animating() || m.wheel_scroll.running() || m.thumbnails.fading() {
        anim::request_redraw(canvas);
    }
}

/// Which list the ring belongs to: a different key jumps instead of gliding.
pub type RingKey = (u64, u64);

struct Pending {
    key: RingKey,
    /// Content-space rectangle (screen rectangle plus the scroll offsets).
    content: Rect,
    scroll: (i32, i32),
    radius: i32,
    color: Color,
}

/// All animated paint values of the browser: one ring rectangle, one hover
/// tween, one listing tween. No per-row state.
#[derive(Default)]
pub struct State {
    ring: Anim<Rect>,
    ring_key: Option<RingKey>,
    ring_valid: bool,
    pending: Option<Pending>,
    hover: Option<usize>,
    hover_tween: Option<Tween>,
    listing_path: Option<PathBuf>,
    listing_shown: bool,
    listing_tween: Option<Tween>,
    animating: bool,
    /// The ring was drawn this frame; a later `flush` is a no-op. A view
    /// whose rows sit under a pinned header flushes inside its row clip.
    flushed: bool,
    /// Keyboard focus is in another widget (the places rail): the list keeps
    /// its cursor position but does not draw a second focus ring.
    pub elsewhere: bool,
}

impl State {
    /// Call once at the start of every browser paint.
    pub fn begin(&mut self, hovered: Option<usize>, path: &Path, has_items: bool) {
        self.animating = false;
        self.pending = None;
        self.flushed = false;
        if hovered != self.hover {
            self.hover = hovered;
            self.hover_tween = hovered
                .filter(|_| anim::enabled())
                .map(|_| Tween::start(HOVER_MS, Ease::OutCubic));
        }
        if self.listing_path.as_deref() != Some(path) {
            self.listing_path = Some(path.to_path_buf());
            self.listing_shown = false;
            self.listing_tween = None;
        }
        // Only the first visible listing of a navigation fades; a refresh of
        // the same folder keeps `listing_path` and therefore never restarts it.
        if !self.listing_shown && has_items {
            self.listing_shown = true;
            self.listing_tween = anim::enabled().then(|| Tween::start(LISTING_MS, Ease::OutCubic));
        }
    }

    /// True when the last paint left a transition running: the canvas must
    /// request one more frame. False for an idle window.
    pub fn animating(&self) -> bool {
        self.animating
    }

    /// Opacity of the rows while the listing fades in (1.0 when settled).
    pub fn listing_alpha(&mut self) -> f64 {
        match self.listing_tween {
            Some(t) if !t.done() => {
                self.animating = true;
                LISTING_FROM + (1.0 - LISTING_FROM) * t.value()
            }
            _ => 1.0,
        }
    }

    /// Hover opacity of row `i` (1.0 unless it is the freshly hovered one).
    pub fn hover_mix(&mut self, i: usize) -> f64 {
        match self.hover_tween {
            Some(t) if self.hover == Some(i) && !t.done() => {
                self.animating = true;
                t.value()
            }
            _ => 1.0,
        }
    }

    /// Records the focused item's visible rectangle for this frame. The ring
    /// itself is drawn by `flush`, over the rows.
    pub fn focus(
        &mut self,
        key: RingKey,
        screen: Rect,
        scroll: (i32, i32),
        radius: i32,
        color: Color,
    ) {
        self.pending = Some(Pending {
            key,
            content: Rect::new(screen.x + scroll.0, screen.y + scroll.1, screen.w, screen.h),
            scroll,
            radius,
            color,
        });
    }

    /// Draws the focus ring. Glides in content space (so scrolling does not
    /// drag it) and jumps when the focused row was not on screen last frame
    /// or the list changed.
    pub fn flush(&mut self, p: &mut dyn Painter) {
        if std::mem::replace(&mut self.flushed, true) {
            return;
        }
        let Some(f) = self.pending.take() else {
            self.ring_valid = false;
            return;
        };
        let glide = self.ring_valid && self.ring_key == Some(f.key);
        let at = if glide {
            self.ring.value(f.content, RING_MS, Ease::OutCubic)
        } else {
            self.ring.jump(f.content)
        };
        self.ring_key = Some(f.key);
        self.ring_valid = true;
        if self.ring.running() {
            self.animating = true;
        }
        let screen = Rect::new(at.x - f.scroll.0, at.y - f.scroll.1, at.w, at.h);
        if !self.elsewhere {
            outline(p, screen, f.radius, 2, f.color);
        }
    }
}

/// Rounded outline of thickness `t` built from filled strips (the painter
/// has no stroke): four edges plus one scanline per corner row.
pub fn outline(p: &mut dyn Painter, r: Rect, radius: i32, t: i32, c: Color) {
    if r.w <= 2 * t || r.h <= 2 * t {
        return;
    }
    let rad = radius.clamp(0, (r.h / 2).min(r.w / 2));
    p.rect(Rect::new(r.x + rad, r.y, r.w - 2 * rad, t), c);
    p.rect(Rect::new(r.x + rad, r.y + r.h - t, r.w - 2 * rad, t), c);
    p.rect(Rect::new(r.x, r.y + rad, t, r.h - 2 * rad), c);
    p.rect(Rect::new(r.x + r.w - t, r.y + rad, t, r.h - 2 * rad), c);
    let (outer, inner) = (rad as f32, (rad - t).max(0) as f32);
    for k in 0..rad {
        let dy = (rad - k) as f32 - 0.5;
        let x0 = (outer - (outer * outer - dy * dy).max(0.0).sqrt()).round() as i32;
        let x1 = if dy < inner {
            (outer - (inner * inner - dy * dy).sqrt()).round() as i32
        } else {
            rad
        }
        .max(x0 + 1);
        let w = x1 - x0;
        for y in [r.y + k, r.y + r.h - 1 - k] {
            p.rect(Rect::new(r.x + x0, y, w, 1), c);
            p.rect(Rect::new(r.x + r.w - x1, y, w, 1), c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[derive(Default)]
    struct Rects(Vec<(Rect, Color)>);
    impl Painter for Rects {
        fn rect(&mut self, r: Rect, c: Color) {
            self.0.push((r, c));
        }
        fn rounded_rect(&mut self, r: Rect, _: i32, c: Color) {
            self.0.push((r, c));
        }
        fn text(&mut self, _: Rect, _: &str, _: i32, _: Color, _: bool, _: bool) {}
        fn image(&mut self, _: Rect, _: &Image<'_>) {}
        fn clip(&mut self, _: Rect) {}
        fn unclip(&mut self) {}
    }
    const RED: Color = Color(200, 0, 0);
    const BLACK: Color = Color(0, 0, 0);

    fn top(p: &Rects) -> i32 {
        p.0.iter().map(|(r, _)| r.y).min().unwrap()
    }
    fn frame(s: &mut State, y: i32, scroll: i32) -> Rects {
        let mut p = Rects::default();
        s.begin(None, Path::new("/a"), true);
        s.focus(
            (1, 1),
            Rect::new(10, y - scroll, 200, 38),
            (0, scroll),
            4,
            RED,
        );
        s.flush(&mut p);
        p
    }

    #[test]
    fn mix_is_exact_at_the_ends() {
        assert_eq!(mix(RED, BLACK, 1.0), RED);
        assert_eq!(mix(RED, BLACK, 0.0), BLACK);
        assert_eq!(mix(RED, BLACK, 0.5), Color(100, 0, 0));
    }

    #[test]
    fn ring_glides_in_content_space_and_settles() {
        anim::set_enabled(true);
        anim::clock::freeze();
        let mut s = State::default();
        assert_eq!(top(&frame(&mut s, 100, 0)), 100);
        // The focus moved one row: the ring is still at the old row at t=0.
        assert_eq!(top(&frame(&mut s, 138, 0)), 100);
        assert!(s.animating());
        anim::clock::advance(Duration::from_millis(40));
        let mid = top(&frame(&mut s, 138, 0));
        assert!(mid > 100 && mid < 138, "mid-glide {mid}");
        anim::clock::advance(Duration::from_millis(60));
        assert_eq!(top(&frame(&mut s, 138, 0)), 138);
        assert!(!s.animating());
        // A second flush in the same frame (a view already drew the ring
        // inside its clip) neither draws again nor invalidates the ring.
        let mut again = Rects::default();
        s.flush(&mut again);
        assert!(again.0.is_empty());
        assert_eq!(top(&frame(&mut s, 176, 0)), 138);
        assert!(s.animating(), "the next row glides from the kept ring");
        anim::clock::resume();
    }

    #[test]
    fn ring_follows_scroll_without_animating_and_jumps_when_hidden() {
        anim::set_enabled(true);
        anim::clock::freeze();
        let mut s = State::default();
        frame(&mut s, 100, 0);
        // Same content row while the painted scroll glides: no ring motion.
        assert_eq!(top(&frame(&mut s, 100, 30)), 70);
        assert!(!s.animating());
        // One frame without the focused row on screen invalidates the ring.
        s.begin(None, Path::new("/a"), true);
        s.flush(&mut Rects::default());
        frame(&mut s, 100, 0);
        assert_eq!(top(&frame(&mut s, 400, 0)), 100);
        // 400 was a glide; a hidden frame then makes the next target a jump.
        s.begin(None, Path::new("/a"), true);
        s.flush(&mut Rects::default());
        assert_eq!(top(&frame(&mut s, 138, 0)), 138);
        anim::clock::resume();
    }

    #[test]
    fn disabled_animation_is_final_immediately() {
        anim::set_enabled(false);
        let mut s = State::default();
        frame(&mut s, 100, 0);
        assert_eq!(top(&frame(&mut s, 400, 0)), 400);
        s.begin(Some(3), Path::new("/b"), true);
        assert_eq!(s.hover_mix(3), 1.0);
        assert_eq!(s.listing_alpha(), 1.0);
        assert!(!s.animating());
        anim::set_enabled(None);
    }

    #[test]
    fn listing_fades_once_per_navigation_and_not_on_refresh() {
        anim::set_enabled(true);
        anim::clock::freeze();
        let mut s = State::default();
        s.begin(None, Path::new("/a"), false);
        assert_eq!(s.listing_alpha(), 1.0);
        s.begin(None, Path::new("/a"), true);
        assert!((s.listing_alpha() - LISTING_FROM).abs() < 1e-9);
        assert!(s.animating());
        anim::clock::advance(Duration::from_millis(60));
        s.begin(None, Path::new("/a"), true);
        let mid = s.listing_alpha();
        assert!(mid > LISTING_FROM && mid < 1.0);
        anim::clock::advance(Duration::from_millis(100));
        s.begin(None, Path::new("/a"), true);
        assert_eq!(s.listing_alpha(), 1.0);
        assert!(!s.animating(), "idle windows schedule no frames");
        // A refresh clears and refills the same folder: no new fade.
        s.begin(None, Path::new("/a"), false);
        s.begin(None, Path::new("/a"), true);
        assert_eq!(s.listing_alpha(), 1.0);
        // Navigating elsewhere fades again.
        s.begin(None, Path::new("/b"), true);
        assert!(s.listing_alpha() < 1.0);
        anim::clock::resume();
        anim::set_enabled(None);
    }

    #[test]
    fn hover_fades_only_the_new_row_in_one_tween() {
        anim::set_enabled(true);
        anim::clock::freeze();
        let mut s = State::default();
        s.begin(Some(2), Path::new("/a"), false);
        assert_eq!(s.hover_mix(2), 0.0);
        assert_eq!(s.hover_mix(5), 1.0);
        anim::clock::advance(Duration::from_millis(HOVER_MS as u64));
        s.begin(Some(2), Path::new("/a"), false);
        assert_eq!(s.hover_mix(2), 1.0);
        assert!(!s.animating());
        anim::clock::resume();
        anim::set_enabled(None);
    }

    #[test]
    fn fade_painter_blends_and_is_identity_at_one() {
        let mut rec = Rects::default();
        FadePainter::new(&mut rec, BLACK, 0.5).rect(Rect::new(0, 0, 1, 1), RED);
        FadePainter::new(&mut rec, BLACK, 1.0).rect(Rect::new(0, 0, 1, 1), RED);
        assert_eq!(rec.0[0].1, Color(100, 0, 0));
        assert_eq!(rec.0[1].1, RED);
    }

    #[test]
    fn outline_never_leaves_its_rectangle() {
        let mut p = Rects::default();
        let r = Rect::new(10, 20, 60, 38);
        outline(&mut p, r, 9, 2, RED);
        assert!(!p.0.is_empty());
        for (q, _) in p.0 {
            assert!(q.x >= r.x && q.y >= r.y && q.x + q.w <= r.x + r.w && q.y + q.h <= r.y + r.h);
        }
    }
}
