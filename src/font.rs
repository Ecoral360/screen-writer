//! Font loading (via fontconfig's `fc-match`) and text-to-path conversion.

use ab_glyph::{Font as _, FontArc, OutlineCurve, ScaleFont as _};
use tiny_skia::{Path, PathBuilder};

pub struct Font {
    font: FontArc,
}

/// Laid-out text converted to a fillable path.
pub struct TextPath {
    pub path: Option<Path>,
    pub width: f32,
    pub height: f32,
    /// Position of the caret (after the last character), relative to the origin.
    pub caret: (f32, f32),
    pub line_height: f32,
}

impl Font {
    /// Finds a sans-serif bold font. `SCREEN_WRITER_FONT` overrides the lookup.
    pub fn load() -> Option<Font> {
        let mut candidates: Vec<String> = Vec::new();
        if let Ok(p) = std::env::var("SCREEN_WRITER_FONT") {
            candidates.push(p);
        }
        for pattern in ["sans-serif:bold", "sans-serif"] {
            if let Ok(out) = std::process::Command::new("fc-match").args(["-f", "%{file}", pattern]).output()
                && out.status.success()
            {
                candidates.push(String::from_utf8_lossy(&out.stdout).trim().to_string());
            }
        }
        candidates.extend(
            [
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/noto/NotoSans-Bold.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
            ]
            .map(String::from),
        );
        for path in candidates {
            if let Ok(data) = std::fs::read(&path)
                && let Ok(font) = FontArc::try_from_vec(data)
            {
                return Some(Font { font });
            }
        }
        eprintln!("screen-writer: no usable font found; text features disabled (set SCREEN_WRITER_FONT)");
        None
    }

    /// Lays out `text` (may contain newlines) with its top-left corner at `(x, y)`.
    pub fn text_path(&self, text: &str, x: f32, y: f32, px: f32) -> TextPath {
        let scaled = self.font.as_scaled(px);
        let ascent = scaled.ascent();
        let line_height = scaled.height() + scaled.line_gap();
        let s = scaled.h_scale_factor();
        let sy = scaled.v_scale_factor();
        let mut pb = PathBuilder::new();
        let mut max_w: f32 = 0.0;
        let mut caret = (0.0, 0.0);
        for (li, line) in text.split('\n').enumerate() {
            let baseline = y + ascent + li as f32 * line_height;
            let mut cx = 0.0f32;
            let mut prev = None;
            for ch in line.chars() {
                let id = self.font.glyph_id(ch);
                if let Some(p) = prev {
                    cx += scaled.kern(p, id);
                }
                if let Some(outline) = self.font.outline(id) {
                    let ox = x + cx;
                    let map = |p: ab_glyph::Point| (ox + p.x * s, baseline - p.y * sy);
                    let mut last: Option<ab_glyph::Point> = None;
                    for curve in &outline.curves {
                        let start = match curve {
                            OutlineCurve::Line(a, _) | OutlineCurve::Quad(a, _, _) | OutlineCurve::Cubic(a, _, _, _) => *a,
                        };
                        if last.is_none_or(|l| (l.x - start.x).abs() > 0.01 || (l.y - start.y).abs() > 0.01) {
                            if last.is_some() {
                                pb.close();
                            }
                            let (mx, my) = map(start);
                            pb.move_to(mx, my);
                        }
                        match curve {
                            OutlineCurve::Line(_, b) => {
                                let (bx, by) = map(*b);
                                pb.line_to(bx, by);
                                last = Some(*b);
                            }
                            OutlineCurve::Quad(_, c, b) => {
                                let ((cx_, cy_), (bx, by)) = (map(*c), map(*b));
                                pb.quad_to(cx_, cy_, bx, by);
                                last = Some(*b);
                            }
                            OutlineCurve::Cubic(_, c1, c2, b) => {
                                let ((c1x, c1y), (c2x, c2y), (bx, by)) = (map(*c1), map(*c2), map(*b));
                                pb.cubic_to(c1x, c1y, c2x, c2y, bx, by);
                                last = Some(*b);
                            }
                        }
                    }
                    if last.is_some() {
                        pb.close();
                    }
                }
                cx += scaled.h_advance(id);
                prev = Some(id);
            }
            max_w = max_w.max(cx);
            caret = (cx, li as f32 * line_height);
        }
        let lines = text.split('\n').count().max(1);
        TextPath {
            path: pb.finish(),
            width: max_w,
            height: lines as f32 * line_height,
            caret,
            line_height,
        }
    }
}
