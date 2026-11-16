//! On-screen floating toolbar: layout, icon drawing and hit-testing.

use crate::font::Font;
use crate::geom::IRect;
use crate::shapes::{Rgba, paint, round_stroke};
use tiny_skia::{FillRule, PathBuilder, PixmapMut, Rect, Transform};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Highlighter,
    Line,
    Arrow,
    Rect,
    Ellipse,
    Text,
    Marker,
    Eraser,
    Laser,
    Spotlight,
}

impl Tool {
    pub const ALL: [Tool; 11] = [
        Tool::Pen,
        Tool::Highlighter,
        Tool::Line,
        Tool::Arrow,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Text,
        Tool::Marker,
        Tool::Eraser,
        Tool::Laser,
        Tool::Spotlight,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Pen => "Pen",
            Tool::Highlighter => "Highlighter",
            Tool::Line => "Line",
            Tool::Arrow => "Arrow",
            Tool::Rect => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::Text => "Text",
            Tool::Marker => "Numbered marker",
            Tool::Eraser => "Eraser",
            Tool::Laser => "Laser pointer",
            Tool::Spotlight => "Spotlight",
        }
    }

    pub fn key(self) -> char {
        match self {
            Tool::Pen => 'P',
            Tool::Highlighter => 'H',
            Tool::Line => 'L',
            Tool::Arrow => 'A',
            Tool::Rect => 'R',
            Tool::Ellipse => 'O',
            Tool::Text => 'T',
            Tool::Marker => 'N',
            Tool::Eraser => 'E',
            Tool::Laser => 'K',
            Tool::Spotlight => 'S',
        }
    }

    pub fn from_name(s: &str) -> Option<Tool> {
        let s = s.to_ascii_lowercase();
        Tool::ALL.into_iter().find(|t| {
            let n = t.name().to_ascii_lowercase();
            n == s || n.split(' ').next() == Some(s.as_str()) || (s.len() == 1 && t.key().to_ascii_lowercase().to_string() == s)
        })
        .or(match s.as_str() {
            "rect" => Some(Tool::Rect),
            "circle" => Some(Tool::Ellipse),
            "laser" => Some(Tool::Laser),
            "marker" | "number" => Some(Tool::Marker),
            _ => None,
        })
    }
}

pub const PALETTE: [Rgba; 8] = [
    Rgba(255, 59, 48, 255),
    Rgba(255, 149, 0, 255),
    Rgba(255, 214, 10, 255),
    Rgba(52, 199, 89, 255),
    Rgba(10, 132, 255, 255),
    Rgba(191, 90, 242, 255),
    Rgba(255, 255, 255, 255),
    Rgba(20, 20, 20, 255),
];
const COLOR_NAMES: [&str; 8] = ["Red", "Orange", "Yellow", "Green", "Blue", "Purple", "White", "Black"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    Grip,
    Tool(Tool),
    Fill,
    Color(usize),
    SizeDown,
    SizeShow,
    SizeUp,
    Undo,
    Redo,
    Clear,
    Board,
    Close,
}

impl Btn {
    pub fn tooltip(self) -> String {
        match self {
            Btn::Grip => "Drag to move toolbar  (Space or Tab hides it)".into(),
            Btn::Tool(t) => format!("{}  ({})", t.name(), t.key()),
            Btn::Fill => "Fill shapes  (F)".into(),
            Btn::Color(i) => format!("{}  ({})", COLOR_NAMES[i], i + 1),
            Btn::SizeDown => "Thinner  ( [ or scroll down )".into(),
            Btn::SizeShow => "Current size".into(),
            Btn::SizeUp => "Thicker  ( ] or scroll up )".into(),
            Btn::Undo => "Undo  (Ctrl+Z)".into(),
            Btn::Redo => "Redo  (Ctrl+Y)".into(),
            Btn::Clear => "Clear all  (C / Shift+F9)".into(),
            Btn::Board => "Board: none / whiteboard / blackboard  (W / B)".into(),
            Btn::Close => "Stop drawing  (Esc / F9)".into(),
        }
    }
}

pub const BTN: i32 = 34;
const GAP: i32 = 3;
const SEP: i32 = 12;
const PAD: i32 = 6;

/// State the toolbar needs to render itself.
pub struct View<'a> {
    pub tool: Tool,
    pub color: Rgba,
    pub size: f32,
    pub fill: bool,
    pub board: u8,
    pub can_undo: bool,
    pub can_redo: bool,
    pub hover: Option<Btn>,
    pub pressed: Option<Btn>,
    pub font: Option<&'a Font>,
}

pub struct Toolbar {
    pub x: i32,
    pub y: i32,
    pub visible: bool,
    items: Vec<(Btn, IRect)>, // relative to (x, y)
    w: i32,
    h: i32,
}

impl Toolbar {
    pub fn new() -> Toolbar {
        let mut items = Vec::new();
        let mut cx = PAD;
        let groups: Vec<Vec<Btn>> = vec![
            vec![Btn::Grip],
            Tool::ALL.iter().map(|&t| Btn::Tool(t)).collect(),
            vec![Btn::Fill],
            (0..PALETTE.len()).map(Btn::Color).collect(),
            vec![Btn::SizeDown, Btn::SizeShow, Btn::SizeUp],
            vec![Btn::Undo, Btn::Redo, Btn::Clear],
            vec![Btn::Board, Btn::Close],
        ];
        for (gi, g) in groups.iter().enumerate() {
            if gi > 0 {
                cx += SEP - GAP;
            }
            for &b in g {
                let bw = if b == Btn::Grip { 16 } else if matches!(b, Btn::Color(_)) { 26 } else { BTN };
                items.push((b, IRect::xywh(cx, PAD, bw, BTN)));
                cx += bw + GAP;
            }
        }
        let w = cx - GAP + PAD;
        Toolbar { x: 0, y: 0, visible: true, items, w, h: BTN + 2 * PAD }
    }

    pub fn place(&mut self, monitor: IRect) {
        self.x = monitor.x0 + (monitor.w() - self.w) / 2;
        self.y = monitor.y0 + 44;
    }

    pub fn rect(&self) -> IRect {
        IRect::xywh(self.x, self.y, self.w, self.h)
    }

    /// Area including space reserved for a tooltip below the bar.
    pub fn rect_with_tooltip(&self) -> IRect {
        let r = self.rect();
        IRect::new(r.x0 - 200, r.y0, r.x1 + 200, r.y1 + 44)
    }

    pub fn btn_at(&self, x: f32, y: f32) -> Option<Btn> {
        if !self.rect().contains(x, y) {
            return None;
        }
        let (rx, ry) = (x - self.x as f32, y - self.y as f32);
        Some(self.items.iter().find(|(_, r)| r.contains(rx, ry)).map(|(b, _)| *b).unwrap_or(Btn::Grip))
    }

    fn btn_rect(&self, b: Btn) -> Option<IRect> {
        self.items.iter().find(|(x, _)| *x == b).map(|(_, r)| r.translate(self.x, self.y))
    }

    pub fn draw(&self, pm: &mut PixmapMut, tf: Transform, v: &View) {
        let bar = self.rect();
        let bg = rounded(bar, 12.0);
        if let Some(p) = &bg {
            pm.fill_path(p, &paint(Rgba(28, 28, 34, 235)), FillRule::Winding, tf, None);
            pm.stroke_path(p, &paint(Rgba(255, 255, 255, 40)), &round_stroke(1.0), tf, None);
        }
        // Group separators.
        let mut prev: Option<IRect> = None;
        for (_, r) in &self.items {
            let r = r.translate(self.x, self.y);
            if let Some(p) = prev
                && r.x0 - p.x1 > GAP + 2
            {
                let sx = (p.x1 + r.x0) as f32 / 2.0;
                let mut pb = PathBuilder::new();
                pb.move_to(sx, (r.y0 + 6) as f32);
                pb.line_to(sx, (r.y1 - 6) as f32);
                if let Some(path) = pb.finish() {
                    pm.stroke_path(&path, &paint(Rgba(255, 255, 255, 50)), &round_stroke(1.0), tf, None);
                }
            }
            prev = Some(r);
        }

        for &(b, r) in &self.items {
            let r = r.translate(self.x, self.y);
            let selected = match b {
                Btn::Tool(t) => t == v.tool,
                Btn::Fill => v.fill,
                Btn::Board => v.board != 0,
                _ => false,
            };
            let enabled = match b {
                Btn::Undo => v.can_undo,
                Btn::Redo => v.can_redo,
                _ => true,
            };
            let interactive = !matches!(b, Btn::Grip | Btn::SizeShow | Btn::Color(_));
            if interactive {
                let fill = if v.pressed == Some(b) && enabled {
                    Some(Rgba(70, 110, 230, 255))
                } else if selected {
                    Some(Rgba(64, 100, 220, 255))
                } else if v.hover == Some(b) && enabled {
                    Some(Rgba(255, 255, 255, 36))
                } else {
                    None
                };
                if let (Some(c), Some(p)) = (fill, rounded(r, 7.0)) {
                    pm.fill_path(&p, &paint(c), FillRule::Winding, tf, None);
                }
            }
            let fg = if enabled { Rgba(240, 240, 245, 255) } else { Rgba(240, 240, 245, 70) };
            draw_icon(pm, tf, b, r, fg, v);
        }

        // Tooltip below the hovered button.
        if let (Some(b), Some(font)) = (v.hover, v.font)
            && let Some(r) = self.btn_rect(b)
        {
            let text = b.tooltip();
            let px = 14.0;
            let probe = font.text_path(&text, 0.0, 0.0, px);
            let (tw, th) = (probe.width + 16.0, probe.height + 8.0);
            let tx = ((r.x0 + r.x1) as f32 / 2.0 - tw / 2.0).max((bar.x0 - 190) as f32);
            let ty = (bar.y1 + 6) as f32;
            if let Some(p) = rounded(IRect::from_f(tx, ty, tx + tw, ty + th, 0.0), 6.0) {
                pm.fill_path(&p, &paint(Rgba(15, 15, 20, 240)), FillRule::Winding, tf, None);
            }
            if let Some(p) = font.text_path(&text, tx + 8.0, ty + 4.0, px).path {
                pm.fill_path(&p, &paint(Rgba(240, 240, 245, 255)), FillRule::Winding, tf, None);
            }
        }
    }
}

fn rounded(r: IRect, rad: f32) -> Option<tiny_skia::Path> {
    let (x0, y0, x1, y1) = (r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32);
    let k = 0.4477; // 1 - kappa
    let mut pb = PathBuilder::new();
    pb.move_to(x0 + rad, y0);
    pb.line_to(x1 - rad, y0);
    pb.cubic_to(x1 - rad * k, y0, x1, y0 + rad * k, x1, y0 + rad);
    pb.line_to(x1, y1 - rad);
    pb.cubic_to(x1, y1 - rad * k, x1 - rad * k, y1, x1 - rad, y1);
    pb.line_to(x0 + rad, y1);
    pb.cubic_to(x0 + rad * k, y1, x0, y1 - rad * k, x0, y1 - rad);
    pb.line_to(x0, y0 + rad);
    pb.cubic_to(x0, y0 + rad * k, x0 + rad * k, y0, x0 + rad, y0);
    pb.close();
    pb.finish()
}

/// Draws a 24x24-unit icon centered in `r`.
fn draw_icon(pm: &mut PixmapMut, tf: Transform, b: Btn, r: IRect, fg: Rgba, v: &View) {
    let (cx, cy) = ((r.x0 + r.x1) as f32 / 2.0, (r.y0 + r.y1) as f32 / 2.0);
    // Icon-local coordinates: (0,0) is the top-left of a 24x24 box centered in r.
    let t = tf.pre_translate(cx - 12.0, cy - 12.0);
    let s = |w: f32| round_stroke(w);
    let stroke = |pm: &mut PixmapMut, pb: PathBuilder, w: f32, c: Rgba| {
        if let Some(p) = pb.finish() {
            pm.stroke_path(&p, &paint(c), &s(w), t, None);
        }
    };
    let fill = |pm: &mut PixmapMut, pb: PathBuilder, c: Rgba| {
        if let Some(p) = pb.finish() {
            pm.fill_path(&p, &paint(c), FillRule::Winding, t, None);
        }
    };
    let text = |pm: &mut PixmapMut, s: &str, px: f32, c: Rgba| {
        if let Some(f) = v.font {
            let probe = f.text_path(s, 0.0, 0.0, px);
            if let Some(p) = f.text_path(s, 12.0 - probe.width / 2.0, 12.0 - probe.height / 2.0, px).path {
                pm.fill_path(&p, &paint(c), FillRule::Winding, t, None);
            }
        }
    };
    let accent = v.color;
    match b {
        Btn::Grip => {
            for row in 0..3 {
                for col in 0..2 {
                    let mut pb = PathBuilder::new();
                    pb.push_circle(9.0 + col as f32 * 6.0, 6.0 + row as f32 * 6.0, 1.5);
                    fill(pm, pb, fg.with_alpha(120));
                }
            }
        }
        Btn::Tool(tool) => match tool {
            Tool::Pen => {
                let mut pb = PathBuilder::new();
                pb.move_to(4.0, 20.0);
                pb.line_to(6.0, 14.0);
                pb.line_to(16.0, 4.0);
                pb.line_to(20.0, 8.0);
                pb.line_to(10.0, 18.0);
                pb.close();
                stroke(pm, pb, 1.8, fg);
                let mut pb = PathBuilder::new();
                pb.move_to(4.0, 20.0);
                pb.line_to(7.5, 19.0);
                pb.line_to(5.0, 16.5);
                pb.close();
                fill(pm, pb, accent);
            }
            Tool::Highlighter => {
                let mut pb = PathBuilder::new();
                pb.move_to(3.0, 19.0);
                pb.line_to(21.0, 19.0);
                stroke(pm, pb, 5.0, accent.with_alpha(150));
                let mut pb = PathBuilder::new();
                pb.move_to(8.0, 15.0);
                pb.line_to(10.0, 11.0);
                pb.line_to(17.0, 3.0);
                pb.line_to(21.0, 7.0);
                pb.line_to(13.0, 14.0);
                pb.close();
                stroke(pm, pb, 1.8, fg);
            }
            Tool::Line => {
                let mut pb = PathBuilder::new();
                pb.move_to(4.0, 20.0);
                pb.line_to(20.0, 4.0);
                stroke(pm, pb, 2.2, fg);
            }
            Tool::Arrow => {
                let mut pb = PathBuilder::new();
                pb.move_to(4.0, 20.0);
                pb.line_to(18.0, 6.0);
                stroke(pm, pb, 2.2, fg);
                let mut pb = PathBuilder::new();
                pb.move_to(20.0, 4.0);
                pb.line_to(19.0, 12.0);
                pb.line_to(12.0, 5.0);
                pb.close();
                fill(pm, pb, fg);
            }
            Tool::Rect => {
                let mut pb = PathBuilder::new();
                pb.push_rect(Rect::from_xywh(4.0, 6.0, 16.0, 12.0).unwrap());
                if v.fill {
                    fill(pm, pb, fg);
                } else {
                    stroke(pm, pb, 2.0, fg);
                }
            }
            Tool::Ellipse => {
                let mut pb = PathBuilder::new();
                pb.push_oval(Rect::from_xywh(3.0, 5.0, 18.0, 14.0).unwrap());
                if v.fill {
                    fill(pm, pb, fg);
                } else {
                    stroke(pm, pb, 2.0, fg);
                }
            }
            Tool::Text => {
                if v.font.is_some() {
                    text(pm, "T", 20.0, fg);
                } else {
                    let mut pb = PathBuilder::new();
                    pb.move_to(5.0, 5.0);
                    pb.line_to(19.0, 5.0);
                    pb.move_to(12.0, 5.0);
                    pb.line_to(12.0, 20.0);
                    stroke(pm, pb, 2.5, fg);
                }
            }
            Tool::Marker => {
                let mut pb = PathBuilder::new();
                pb.push_circle(12.0, 12.0, 9.0);
                fill(pm, pb, accent);
                text(pm, "1", 12.0, accent.contrast());
            }
            Tool::Eraser => {
                let mut pb = PathBuilder::new();
                pb.move_to(9.0, 20.0);
                pb.line_to(3.0, 14.0);
                pb.line_to(13.0, 4.0);
                pb.line_to(21.0, 12.0);
                pb.line_to(13.0, 20.0);
                pb.close();
                stroke(pm, pb, 1.8, fg);
                let mut pb = PathBuilder::new();
                pb.move_to(8.0, 9.0);
                pb.line_to(16.0, 17.0);
                stroke(pm, pb, 1.8, fg);
                let mut pb = PathBuilder::new();
                pb.move_to(9.0, 20.0);
                pb.line_to(21.0, 20.0);
                stroke(pm, pb, 1.8, fg);
            }
            Tool::Laser => {
                let mut pb = PathBuilder::new();
                pb.push_circle(12.0, 12.0, 7.0);
                fill(pm, pb, Rgba(255, 40, 40, 90));
                let mut pb = PathBuilder::new();
                pb.push_circle(12.0, 12.0, 3.5);
                fill(pm, pb, Rgba(255, 60, 60, 255));
                let mut pb = PathBuilder::new();
                pb.move_to(2.0, 22.0);
                pb.quad_to(4.0, 14.0, 8.0, 15.0);
                stroke(pm, pb, 1.6, Rgba(255, 80, 80, 180));
            }
            Tool::Spotlight => {
                let mut pb = PathBuilder::new();
                pb.push_rect(Rect::from_xywh(2.0, 4.0, 20.0, 16.0).unwrap());
                pb.push_circle(13.0, 11.0, 5.0);
                if let Some(p) = pb.finish() {
                    pm.fill_path(&p, &paint(fg.with_alpha(110)), FillRule::EvenOdd, t, None);
                }
                let mut pb = PathBuilder::new();
                pb.push_circle(13.0, 11.0, 5.0);
                stroke(pm, pb, 1.5, fg);
            }
        },
        Btn::Fill => {
            let mut pb = PathBuilder::new();
            pb.move_to(12.0, 3.0);
            pb.line_to(20.0, 11.0);
            pb.line_to(12.0, 19.0);
            pb.line_to(4.0, 11.0);
            pb.close();
            if v.fill {
                let mut f = PathBuilder::new();
                f.move_to(12.0, 3.0);
                f.line_to(20.0, 11.0);
                f.line_to(12.0, 19.0);
                f.line_to(4.0, 11.0);
                f.close();
                fill(pm, f, accent);
            }
            stroke(pm, pb, 1.8, fg);
            let mut pb = PathBuilder::new();
            pb.push_circle(20.5, 18.5, 2.2);
            fill(pm, pb, accent);
        }
        Btn::Color(i) => {
            let c = PALETTE[i];
            let mut pb = PathBuilder::new();
            pb.push_circle(12.0, 12.0, 9.0);
            fill(pm, pb, c);
            let mut pb = PathBuilder::new();
            pb.push_circle(12.0, 12.0, 9.0);
            stroke(pm, pb, 1.0, Rgba(255, 255, 255, 90));
            if c == v.color {
                let mut pb = PathBuilder::new();
                pb.push_circle(12.0, 12.0, 11.5);
                stroke(pm, pb, 2.0, Rgba(255, 255, 255, 255));
            } else if v.hover == Some(b) {
                let mut pb = PathBuilder::new();
                pb.push_circle(12.0, 12.0, 11.5);
                stroke(pm, pb, 1.5, Rgba(255, 255, 255, 120));
            }
        }
        Btn::SizeDown | Btn::SizeUp => {
            let mut pb = PathBuilder::new();
            pb.move_to(7.0, 12.0);
            pb.line_to(17.0, 12.0);
            if b == Btn::SizeUp {
                pb.move_to(12.0, 7.0);
                pb.line_to(12.0, 17.0);
            }
            stroke(pm, pb, 2.2, fg);
        }
        Btn::SizeShow => {
            let shown = match v.tool {
                Tool::Highlighter => crate::shapes::highlighter_width(v.size),
                _ => v.size,
            };
            let rad = (shown / 2.0).clamp(1.0, 11.0);
            let mut pb = PathBuilder::new();
            pb.push_circle(12.0, 12.0, rad);
            fill(pm, pb, accent);
            if v.size * 0.5 > 11.0 || v.font.is_some() {
                let label = format!("{}", v.size as i32);
                if let Some(f) = v.font {
                    let probe = f.text_path(&label, 0.0, 0.0, 9.0);
                    if let Some(p) = f.text_path(&label, 24.0 - probe.width, 22.0 - probe.height + 2.0, 9.0).path {
                        pm.fill_path(&p, &paint(fg.with_alpha(200)), FillRule::Winding, t, None);
                    }
                }
            }
        }
        Btn::Undo | Btn::Redo => {
            let flip = b == Btn::Redo;
            let fx = |x: f32| if flip { 24.0 - x } else { x };
            let mut pb = PathBuilder::new();
            pb.move_to(fx(6.0), 10.0);
            pb.line_to(fx(14.0), 10.0);
            pb.cubic_to(fx(22.0), 10.0, fx(22.0), 20.0, fx(14.0), 20.0);
            pb.line_to(fx(9.0), 20.0);
            stroke(pm, pb, 2.0, fg);
            let mut pb = PathBuilder::new();
            pb.move_to(fx(10.0), 5.0);
            pb.line_to(fx(5.0), 10.0);
            pb.line_to(fx(10.0), 15.0);
            stroke(pm, pb, 2.0, fg);
        }
        Btn::Clear => {
            let mut pb = PathBuilder::new();
            pb.move_to(4.0, 6.0);
            pb.line_to(20.0, 6.0);
            pb.move_to(9.0, 6.0);
            pb.line_to(9.0, 3.5);
            pb.line_to(15.0, 3.5);
            pb.line_to(15.0, 6.0);
            pb.move_to(6.0, 6.0);
            pb.line_to(7.0, 21.0);
            pb.line_to(17.0, 21.0);
            pb.line_to(18.0, 6.0);
            pb.move_to(10.0, 10.0);
            pb.line_to(10.0, 17.0);
            pb.move_to(14.0, 10.0);
            pb.line_to(14.0, 17.0);
            stroke(pm, pb, 1.8, fg);
        }
        Btn::Board => {
            let mut pb = PathBuilder::new();
            pb.push_rect(Rect::from_xywh(3.0, 4.0, 18.0, 13.0).unwrap());
            let board_fill = match v.board {
                1 => Some(Rgba(250, 250, 250, 255)),
                2 => Some(Rgba(30, 45, 38, 255)),
                _ => None,
            };
            if let Some(c) = board_fill {
                let mut f = PathBuilder::new();
                f.push_rect(Rect::from_xywh(3.0, 4.0, 18.0, 13.0).unwrap());
                fill(pm, f, c);
            }
            stroke(pm, pb, 1.8, fg);
            let mut pb = PathBuilder::new();
            pb.move_to(8.0, 17.0);
            pb.line_to(6.0, 22.0);
            pb.move_to(16.0, 17.0);
            pb.line_to(18.0, 22.0);
            stroke(pm, pb, 1.8, fg);
        }
        Btn::Close => {
            let mut pb = PathBuilder::new();
            pb.move_to(7.0, 7.0);
            pb.line_to(17.0, 17.0);
            pb.move_to(17.0, 7.0);
            pb.line_to(7.0, 17.0);
            stroke(pm, pb, 2.2, fg);
        }
    }
}
