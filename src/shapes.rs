//! Drawable annotation objects: geometry, rendering, bounds and hit-testing.

use crate::font::Font;
use crate::geom::{IRect, Pt, seg_dist};
use std::rc::Rc;
use tiny_skia::{
    Color, FillRule, FilterQuality, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, PixmapMut, PixmapPaint, Rect,
    Stroke, Transform,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub fn with_alpha(self, a: u8) -> Rgba {
        Rgba(self.0, self.1, self.2, a)
    }
    pub fn color(self) -> Color {
        Color::from_rgba8(self.0, self.1, self.2, self.3)
    }
    /// Black or white, whichever contrasts better with this color.
    pub fn contrast(self) -> Rgba {
        let l = 0.299 * self.0 as f32 + 0.587 * self.1 as f32 + 0.114 * self.2 as f32;
        if l > 150.0 { Rgba(20, 20, 20, 255) } else { Rgba(255, 255, 255, 255) }
    }
}

pub fn paint(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(c.color());
    p.anti_alias = true;
    p
}

pub fn round_stroke(width: f32) -> Stroke {
    Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() }
}

#[derive(Clone)]
pub enum Kind {
    /// Freehand pen or highlighter stroke.
    Free { points: Vec<Pt>, highlighter: bool },
    Line { a: Pt, b: Pt, arrow: bool },
    Rect { a: Pt, b: Pt, filled: bool },
    Ellipse { a: Pt, b: Pt, filled: bool },
    /// Pre-rendered text; `bounds` is in screen coordinates.
    Text { path: Option<Path>, bounds: IRect },
    /// Numbered circular marker.
    Marker { c: Pt, r: f32, label: Option<Path> },
    /// Pinned screenshot, shown at `pos` with size `w`x`h` (screen pixels).
    Image { img: Rc<Pixmap>, pos: Pt, w: f32, h: f32 },
}

#[derive(Clone)]
pub struct Shape {
    pub kind: Kind,
    pub color: Rgba,
    pub width: f32,
}

pub const HIGHLIGHTER_ALPHA: u8 = 100;

pub fn highlighter_width(size: f32) -> f32 {
    (size * 4.0).max(14.0)
}

/// Snap `b` relative to `a` to multiples of 15 degrees.
pub fn snap_angle(a: Pt, b: Pt) -> Pt {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt();
    let step = std::f32::consts::PI / 12.0;
    let ang = (dy.atan2(dx) / step).round() * step;
    Pt::new(a.x + len * ang.cos(), a.y + len * ang.sin())
}

/// Make the box spanned by `a`-`b` square.
pub fn snap_square(a: Pt, b: Pt) -> Pt {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let m = dx.abs().max(dy.abs());
    Pt::new(a.x + m * dx.signum(), a.y + m * dy.signum())
}

fn rect_of(a: Pt, b: Pt) -> Option<Rect> {
    let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
    let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
    Rect::from_ltrb(x0, y0, x1.max(x0 + 0.5), y1.max(y0 + 0.5))
}

/// Smoothed path through the freehand points (quadratic curves through midpoints).
fn free_path(points: &[Pt]) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let first = points.first()?;
    pb.move_to(first.x, first.y);
    if points.len() == 1 {
        pb.line_to(first.x + 0.01, first.y);
    } else if points.len() == 2 {
        pb.line_to(points[1].x, points[1].y);
    } else {
        for i in 1..points.len() - 1 {
            let (p, n) = (points[i], points[i + 1]);
            pb.quad_to(p.x, p.y, (p.x + n.x) / 2.0, (p.y + n.y) / 2.0);
        }
        let l = points[points.len() - 1];
        pb.line_to(l.x, l.y);
    }
    pb.finish()
}

/// Geometry of an arrow: shaft end point and head triangle.
fn arrow_geom(a: Pt, b: Pt, w: f32) -> (Pt, [Pt; 3]) {
    let len = a.dist(b).max(0.001);
    let (ux, uy) = ((b.x - a.x) / len, (b.y - a.y) / len);
    let head = (w * 3.5 + 8.0).min(len * 0.6).max(w * 1.5);
    let half = head * 0.55;
    let base = Pt::new(b.x - ux * head, b.y - uy * head);
    let shaft_end = Pt::new(b.x - ux * head * 0.7, b.y - uy * head * 0.7);
    (shaft_end, [b, Pt::new(base.x - uy * half, base.y + ux * half), Pt::new(base.x + uy * half, base.y - ux * half)])
}

pub fn marker_radius(size: f32) -> f32 {
    12.0 + size * 2.0
}

impl Shape {
    pub fn marker(font: Option<&Font>, c: Pt, size: f32, n: u32, color: Rgba) -> Shape {
        let r = marker_radius(size);
        let label = font.and_then(|f| {
            let s = n.to_string();
            let px = r * if s.len() > 1 { 1.0 } else { 1.25 };
            let probe = f.text_path(&s, 0.0, 0.0, px);
            f.text_path(&s, c.x - probe.width / 2.0, c.y - probe.height / 2.0, px).path
        });
        Shape { kind: Kind::Marker { c, r, label }, color, width: size }
    }

    pub fn text(font: &Font, text: &str, pos: Pt, px: f32, color: Rgba) -> Shape {
        let tp = font.text_path(text, pos.x, pos.y, px);
        let bounds = IRect::from_f(pos.x, pos.y, pos.x + tp.width, pos.y + tp.height, 2.0);
        Shape { kind: Kind::Text { path: tp.path, bounds }, color, width: px }
    }

    /// Move the shape by `(dx, dy)` pixels.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        let mv = |p: &mut Pt| *p = Pt::new(p.x + dx, p.y + dy);
        let mv_path = |p: &mut Option<Path>| *p = p.take().and_then(|p| p.transform(Transform::from_translate(dx, dy)));
        match &mut self.kind {
            Kind::Free { points, .. } => points.iter_mut().for_each(mv),
            Kind::Line { a, b, .. } | Kind::Rect { a, b, .. } | Kind::Ellipse { a, b, .. } => {
                mv(a);
                mv(b);
            }
            Kind::Text { path, bounds } => {
                mv_path(path);
                *bounds = bounds.translate(dx.round() as i32, dy.round() as i32);
            }
            Kind::Marker { c, label, .. } => {
                mv(c);
                mv_path(label);
            }
            Kind::Image { pos, .. } => mv(pos),
        }
    }

    pub fn draw(&self, pm: &mut PixmapMut, tf: Transform) {
        let w = self.width;
        match &self.kind {
            Kind::Free { points, highlighter } => {
                let (color, width) = if *highlighter {
                    (self.color.with_alpha(HIGHLIGHTER_ALPHA), highlighter_width(w))
                } else {
                    (self.color, w)
                };
                if let Some(path) = free_path(points) {
                    pm.stroke_path(&path, &paint(color), &round_stroke(width), tf, None);
                }
            }
            Kind::Line { a, b, arrow } => {
                let mut pb = PathBuilder::new();
                pb.move_to(a.x, a.y);
                if *arrow {
                    let (end, tri) = arrow_geom(*a, *b, w);
                    pb.line_to(end.x, end.y);
                    if let Some(p) = pb.finish() {
                        pm.stroke_path(&p, &paint(self.color), &round_stroke(w), tf, None);
                    }
                    let mut hb = PathBuilder::new();
                    hb.move_to(tri[0].x, tri[0].y);
                    hb.line_to(tri[1].x, tri[1].y);
                    hb.line_to(tri[2].x, tri[2].y);
                    hb.close();
                    if let Some(p) = hb.finish() {
                        pm.fill_path(&p, &paint(self.color), FillRule::Winding, tf, None);
                        // Slightly rounded head corners.
                        pm.stroke_path(&p, &paint(self.color), &round_stroke((w * 0.5).min(4.0)), tf, None);
                    }
                } else {
                    pb.line_to(b.x, b.y);
                    if let Some(p) = pb.finish() {
                        pm.stroke_path(&p, &paint(self.color), &round_stroke(w), tf, None);
                    }
                }
            }
            Kind::Rect { a, b, filled } | Kind::Ellipse { a, b, filled } => {
                let Some(r) = rect_of(*a, *b) else { return };
                let is_rect = matches!(self.kind, Kind::Rect { .. });
                let path = if is_rect { Some(PathBuilder::from_rect(r)) } else { PathBuilder::from_oval(r) };
                let Some(path) = path else { return };
                if *filled {
                    pm.fill_path(&path, &paint(self.color), FillRule::Winding, tf, None);
                } else {
                    let mut s = round_stroke(w);
                    if is_rect {
                        s.line_join = LineJoin::Miter;
                    }
                    pm.stroke_path(&path, &paint(self.color), &s, tf, None);
                }
            }
            Kind::Text { path, .. } => {
                if let Some(p) = path {
                    // Thin contrasting outline keeps text readable on any background.
                    let outline = self.color.contrast().with_alpha(140);
                    pm.stroke_path(p, &paint(outline), &round_stroke((self.width / 14.0).max(1.5)), tf, None);
                    pm.fill_path(p, &paint(self.color), FillRule::Winding, tf, None);
                }
            }
            Kind::Image { img, pos, w, h } => {
                let r = Rect::from_xywh(pos.x, pos.y, *w, *h);
                let shadow = Rect::from_xywh(pos.x + 3.0, pos.y + 5.0, *w, *h).map(PathBuilder::from_rect);
                if let Some(s) = shadow {
                    pm.fill_path(&s, &paint(Rgba(0, 0, 0, 70)), FillRule::Winding, tf, None);
                }
                let (sx, sy) = (*w / img.width() as f32, *h / img.height() as f32);
                let quality = if (sx - 1.0).abs() < 0.01 && (sy - 1.0).abs() < 0.01 {
                    FilterQuality::Nearest
                } else {
                    FilterQuality::Bilinear
                };
                let pp = PixmapPaint { quality, ..Default::default() };
                pm.draw_pixmap(0, 0, img.as_ref().as_ref(), &pp, tf.pre_translate(pos.x, pos.y).pre_scale(sx, sy), None);
                if let Some(r) = r {
                    let border = PathBuilder::from_rect(r);
                    let mut s = round_stroke(self.width.clamp(2.0, 6.0));
                    s.line_join = LineJoin::Miter;
                    pm.stroke_path(&border, &paint(self.color), &s, tf, None);
                }
            }
            Kind::Marker { c, r, label } => {
                if let Some(circle) = PathBuilder::from_circle(c.x, c.y, *r) {
                    pm.fill_path(&circle, &paint(self.color), FillRule::Winding, tf, None);
                    pm.stroke_path(&circle, &paint(self.color.contrast().with_alpha(200)), &round_stroke(2.0), tf, None);
                }
                if let Some(l) = label {
                    pm.fill_path(l, &paint(self.color.contrast()), FillRule::Winding, tf, None);
                }
            }
        }
    }

    /// Screen-space bounding box of everything `draw` may touch.
    pub fn bounds(&self) -> IRect {
        let w = self.width;
        match &self.kind {
            Kind::Free { points, highlighter } => {
                let width = if *highlighter { highlighter_width(w) } else { w };
                IRect::from_points(points, width / 2.0 + 2.0)
            }
            Kind::Line { a, b, arrow } => {
                let pad = if *arrow { w * 4.0 + 12.0 } else { w / 2.0 + 2.0 };
                IRect::from_f(a.x, a.y, b.x, b.y, pad)
            }
            Kind::Rect { a, b, .. } | Kind::Ellipse { a, b, .. } => IRect::from_f(a.x, a.y, b.x, b.y, w + 2.0),
            Kind::Text { bounds, .. } => IRect::new(bounds.x0 - 4, bounds.y0 - 4, bounds.x1 + 4, bounds.y1 + 4),
            Kind::Marker { c, r, .. } => IRect::around(*c, r + 3.0),
            Kind::Image { pos, w: iw, h: ih, .. } => {
                let pad = w.clamp(2.0, 6.0) / 2.0 + 2.0;
                IRect::from_f(pos.x - pad, pos.y - pad, pos.x + iw + 5.0 + pad, pos.y + ih + 7.0 + pad, 0.0)
            }
        }
    }

    /// Does a circle of radius `r` around `p` touch this shape?
    pub fn hit(&self, p: Pt, r: f32) -> bool {
        let w = self.width;
        let segs_hit = |pts: &[Pt], tol: f32| -> bool {
            if pts.len() == 1 {
                return p.dist(pts[0]) <= tol;
            }
            pts.windows(2).any(|s| seg_dist(p, s[0], s[1]) <= tol)
        };
        match &self.kind {
            Kind::Free { points, highlighter } => {
                let width = if *highlighter { highlighter_width(w) } else { w };
                segs_hit(points, r + width / 2.0)
            }
            Kind::Line { a, b, arrow } => {
                seg_dist(p, *a, *b) <= r + w / 2.0 + if *arrow { w } else { 0.0 }
            }
            Kind::Rect { a, b, filled } => {
                let (x0, x1, y0, y1) = (a.x.min(b.x), a.x.max(b.x), a.y.min(b.y), a.y.max(b.y));
                if *filled {
                    return p.x >= x0 - r && p.x <= x1 + r && p.y >= y0 - r && p.y <= y1 + r;
                }
                let c = [Pt::new(x0, y0), Pt::new(x1, y0), Pt::new(x1, y1), Pt::new(x0, y1), Pt::new(x0, y0)];
                segs_hit(&c, r + w / 2.0)
            }
            Kind::Ellipse { a, b, filled } => {
                let (cx, cy) = ((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
                let (rx, ry) = (((a.x - b.x) / 2.0).abs(), ((a.y - b.y) / 2.0).abs());
                if *filled {
                    let (ex, ey) = (rx + r, ry + r);
                    return ((p.x - cx) / ex).powi(2) + ((p.y - cy) / ey).powi(2) <= 1.0;
                }
                let pts: Vec<Pt> = (0..=72)
                    .map(|i| {
                        let t = i as f32 / 72.0 * std::f32::consts::TAU;
                        Pt::new(cx + rx * t.cos(), cy + ry * t.sin())
                    })
                    .collect();
                segs_hit(&pts, r + w / 2.0)
            }
            Kind::Text { bounds, .. } => {
                let b = IRect::new(bounds.x0 - r as i32, bounds.y0 - r as i32, bounds.x1 + r as i32, bounds.y1 + r as i32);
                b.contains(p.x, p.y)
            }
            Kind::Marker { c, r: mr, .. } => p.dist(*c) <= mr + r,
            Kind::Image { pos, w, h, .. } => {
                p.x >= pos.x - r && p.x <= pos.x + w + r && p.y >= pos.y - r && p.y <= pos.y + h + r
            }
        }
    }
}
