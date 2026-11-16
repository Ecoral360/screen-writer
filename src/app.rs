//! Application state, input handling and frame composition.

use crate::font::Font;
use crate::geom::{Damage, IRect, Pt};
use crate::shapes::{self, Kind, Rgba, Shape, paint, round_stroke};
use crate::toolbar::{Btn, PALETTE, Tool, Toolbar, View};
use crate::xwin::{XWin, keysym, keysym_char};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};
use tiny_skia::{FillRule, PathBuilder, Pixmap, PixmapMut, Rect, Transform};
use x11rb::protocol::Event;
use x11rb::protocol::xproto::KeyButMask;

#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Toggle,
    Activate,
    Deactivate,
    Clear,
    Visibility,
    Undo,
    Redo,
    Quit,
    Tool(Tool),
    Color(Rgba),
    Size(f32),
    Demo,
}

impl Cmd {
    pub fn parse(s: &str) -> Option<Cmd> {
        let mut it = s.split_whitespace();
        let head = it.next()?.trim_start_matches('-').to_ascii_lowercase();
        let arg = it.next();
        Some(match head.as_str() {
            "toggle" => Cmd::Toggle,
            "activate" | "on" | "start" => Cmd::Activate,
            "deactivate" | "off" => Cmd::Deactivate,
            "clear" => Cmd::Clear,
            "visibility" | "hide" | "show" => Cmd::Visibility,
            "undo" => Cmd::Undo,
            "redo" => Cmd::Redo,
            "quit" | "exit" => Cmd::Quit,
            "demo" => Cmd::Demo,
            "tool" => Cmd::Tool(Tool::from_name(arg?)?),
            "color" | "colour" => Cmd::Color(parse_color(arg?)?),
            "size" => Cmd::Size(arg?.parse::<f32>().ok()?.clamp(MIN_SIZE, MAX_SIZE)),
            _ => return None,
        })
    }
}

fn parse_color(s: &str) -> Option<Rgba> {
    let names = ["red", "orange", "yellow", "green", "blue", "purple", "white", "black"];
    if let Some(i) = names.iter().position(|n| n.eq_ignore_ascii_case(s)) {
        return Some(PALETTE[i]);
    }
    if let Ok(i) = s.parse::<usize>()
        && (1..=PALETTE.len()).contains(&i)
    {
        return Some(PALETTE[i - 1]);
    }
    let h = s.trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Rgba((v >> 16) as u8, (v >> 8) as u8, v as u8, 255))
}

pub enum Msg {
    X(Event),
    Cmd(Cmd),
    XClosed,
}

const MIN_SIZE: f32 = 1.0;
const MAX_SIZE: f32 = 40.0;
const LASER_LIFE: Duration = Duration::from_millis(650);
const BOARD_COLORS: [Rgba; 3] = [Rgba(0, 0, 0, 0), Rgba(250, 250, 248, 255), Rgba(30, 45, 38, 255)];

enum Drag {
    None,
    Draw,
    Erase { erased: bool },
    Laser,
    Toolbar { dx: i32, dy: i32 },
}

struct TextEdit {
    pos: Pt,
    text: String,
    px: f32,
    color: Rgba,
}

pub struct App {
    x: XWin,
    font: Option<Font>,
    base: Pixmap,
    shapes: Vec<Rc<Shape>>,
    undo: Vec<Vec<Rc<Shape>>>,
    redo: Vec<Vec<Rc<Shape>>>,

    active: bool,
    visible: bool,
    tool: Tool,
    color: Rgba,
    size: f32,
    fill: bool,
    board: u8,

    toolbar: Toolbar,
    monitors: Vec<IRect>,
    hover: Option<Btn>,
    drag: Drag,
    current: Option<Shape>,
    start: Pt,
    cursor: Option<Pt>,
    laser: Vec<(Pt, Instant)>,
    text: Option<TextEdit>,
    marker_n: u32,

    damage: Damage,
    last_transient: Vec<IRect>,
    last_spot: bool,
    kbd_focused: bool,
    quit: bool,
}

impl App {
    pub fn new(x: XWin) -> App {
        let base = Pixmap::new(x.width as u32, x.height as u32).expect("screen has zero size");
        let mut toolbar = Toolbar::new();
        toolbar.place(x.primary_monitor());
        let monitors = x.monitors();
        let mut app = App {
            x,
            font: Font::load(),
            base,
            shapes: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            active: false,
            visible: true,
            tool: Tool::Pen,
            color: PALETTE[0],
            size: 4.0,
            fill: false,
            board: 0,
            monitors,
            toolbar,
            hover: None,
            drag: Drag::None,
            current: None,
            start: Pt::default(),
            cursor: None,
            laser: Vec::new(),
            text: None,
            marker_n: 1,
            damage: Damage::default(),
            last_transient: Vec::new(),
            last_spot: false,
            kbd_focused: false,
            quit: false,
        };
        app.damage_all();
        app
    }

    fn screen(&self) -> IRect {
        IRect::xywh(0, 0, self.x.width as i32, self.x.height as i32)
    }

    fn damage_all(&mut self) {
        let s = self.screen();
        self.damage.add(s);
    }

    fn damage_toolbar(&mut self) {
        let r = self.toolbar.rect_with_tooltip();
        self.damage.add(r);
    }

    pub fn run(mut self, rx: Receiver<Msg>) {
        loop {
            let animating = !self.laser.is_empty();
            let first = if animating {
                match rx.recv_timeout(Duration::from_millis(16)) {
                    Ok(m) => Some(m),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            } else {
                match rx.recv() {
                    Ok(m) => Some(m),
                    Err(_) => break,
                }
            };
            if let Some(m) = first {
                self.handle(m);
            }
            while let Ok(m) = rx.try_recv() {
                self.handle(m);
            }
            if self.quit {
                break;
            }
            self.tick();
            self.render();
        }
    }

    fn handle(&mut self, m: Msg) {
        match m {
            Msg::XClosed => self.quit = true,
            Msg::Cmd(c) => self.command(c),
            Msg::X(e) => self.x_event(e),
        }
    }

    // ---------------------------------------------------------------- commands

    pub fn command(&mut self, c: Cmd) {
        match c {
            Cmd::Toggle => {
                if self.active && self.visible { self.deactivate() } else { self.activate() }
            }
            Cmd::Activate => self.activate(),
            Cmd::Deactivate => self.deactivate(),
            Cmd::Clear => self.clear(),
            Cmd::Visibility => {
                if self.visible {
                    self.deactivate();
                    self.visible = false;
                    let _ = self.x.set_mapped(false);
                } else {
                    self.visible = true;
                    let _ = self.x.set_mapped(true);
                    let _ = self.x.raise();
                    self.damage_all();
                }
            }
            Cmd::Undo => self.undo(),
            Cmd::Redo => self.redo(),
            Cmd::Quit => self.quit = true,
            Cmd::Tool(t) => self.set_tool(t),
            Cmd::Color(c) => self.set_color(c),
            Cmd::Size(s) => self.set_size(s),
            Cmd::Demo => self.demo(),
        }
    }

    /// Draws one sample of every annotation kind on the primary monitor.
    fn demo(&mut self) {
        let m = self.x.primary_monitor();
        let (ox, oy) = (m.x0 as f32 + 120.0, m.y0 as f32 + 160.0);
        let p = |x: f32, y: f32| Pt::new(ox + x, oy + y);
        let mk = |kind, color, width| Shape { kind, color, width };
        let squiggle: Vec<Pt> = (0..60).map(|i| p(i as f32 * 4.0, 40.0 + (i as f32 * 0.3).sin() * 30.0)).collect();
        let hl: Vec<Pt> = (0..50).map(|i| p(300.0 + i as f32 * 5.0, 40.0)).collect();
        let mut list = vec![
            mk(Kind::Free { points: squiggle, highlighter: false }, PALETTE[0], 5.0),
            mk(Kind::Free { points: hl, highlighter: true }, PALETTE[2], 6.0),
            mk(Kind::Line { a: p(600.0, 10.0), b: p(760.0, 80.0), arrow: false }, PALETTE[3], 4.0),
            mk(Kind::Line { a: p(0.0, 220.0), b: p(200.0, 140.0), arrow: true }, PALETTE[4], 5.0),
            mk(Kind::Rect { a: p(260.0, 130.0), b: p(420.0, 230.0), filled: false }, PALETTE[1], 4.0),
            mk(Kind::Rect { a: p(450.0, 130.0), b: p(560.0, 230.0), filled: true }, PALETTE[5], 4.0),
            mk(Kind::Ellipse { a: p(600.0, 130.0), b: p(780.0, 230.0), filled: false }, PALETTE[6], 4.0),
        ];
        if let Some(f) = &self.font {
            list.push(Shape::text(f, "Hello, screen!\nSecond line", p(0.0, 270.0), 34.0, PALETTE[6]));
        }
        for (i, x) in [420.0, 480.0, 540.0].into_iter().enumerate() {
            list.push(Shape::marker(self.font.as_ref(), p(x, 300.0), 4.0, i as u32 + 1, PALETTE[i]));
        }
        for s in list {
            self.commit(s);
        }
    }

    fn activate(&mut self) {
        if !self.visible {
            self.visible = true;
            let _ = self.x.set_mapped(true);
            self.damage_all();
        }
        if self.active {
            let _ = self.x.raise();
            return;
        }
        self.active = true;
        let _ = self.x.set_input(true);
        let _ = self.x.raise();
        self.x.take_keyboard();
        self.damage_toolbar();
    }

    fn deactivate(&mut self) {
        if !self.active {
            return;
        }
        self.commit_text();
        self.cancel_drag();
        self.laser.clear();
        self.active = false;
        self.hover = None;
        let _ = self.x.set_input(false);
        self.x.release_keyboard();
        self.damage_toolbar();
    }

    fn cancel_drag(&mut self) {
        if let Some(s) = self.current.take() {
            self.damage.add(s.bounds());
        }
        self.drag = Drag::None;
    }

    fn set_tool(&mut self, t: Tool) {
        if t != Tool::Text {
            self.commit_text();
        }
        self.tool = t;
        self.damage_toolbar();
    }

    fn set_color(&mut self, c: Rgba) {
        self.color = c;
        if let Some(te) = &mut self.text {
            te.color = c;
        }
        self.damage_toolbar();
    }

    fn set_size(&mut self, s: f32) {
        self.size = s.clamp(MIN_SIZE, MAX_SIZE);
        if let Some(te) = &mut self.text {
            te.px = text_px(self.size);
        }
        self.damage_toolbar();
    }

    // ---------------------------------------------------------------- history

    fn checkpoint(&mut self) {
        self.undo.push(self.shapes.clone());
        if self.undo.len() > 300 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.damage_toolbar();
    }

    /// Replace the shape list, damaging only shapes that changed.
    fn set_shapes(&mut self, new: Vec<Rc<Shape>>) {
        for s in self.shapes.iter().filter(|s| !new.iter().any(|n| Rc::ptr_eq(n, s))) {
            self.damage.add(s.bounds());
        }
        for s in new.iter().filter(|n| !self.shapes.iter().any(|s| Rc::ptr_eq(n, s))) {
            self.damage.add(s.bounds());
        }
        self.shapes = new;
        self.rebuild_base();
    }

    fn rebuild_base(&mut self) {
        self.base.fill(tiny_skia::Color::TRANSPARENT);
        let mut pm = self.base.as_mut();
        for s in &self.shapes {
            s.draw(&mut pm, Transform::identity());
        }
    }

    fn commit(&mut self, shape: Shape) {
        self.checkpoint();
        shape.draw(&mut self.base.as_mut(), Transform::identity());
        self.damage.add(shape.bounds());
        self.shapes.push(Rc::new(shape));
    }

    fn undo(&mut self) {
        self.commit_text();
        self.cancel_drag();
        if let Some(prev) = self.undo.pop() {
            self.redo.push(self.shapes.clone());
            self.set_shapes(prev);
            self.damage_toolbar();
        }
    }

    fn redo(&mut self) {
        self.cancel_drag();
        if let Some(next) = self.redo.pop() {
            self.undo.push(self.shapes.clone());
            self.set_shapes(next);
            self.damage_toolbar();
        }
    }

    fn clear(&mut self) {
        self.text = None;
        self.cancel_drag();
        self.laser.clear();
        self.marker_n = 1;
        if !self.shapes.is_empty() {
            self.checkpoint();
            self.set_shapes(Vec::new());
        }
    }

    fn commit_text(&mut self) {
        let Some(te) = self.text.take() else { return };
        if let Some(font) = &self.font {
            let preview = Shape::text(font, &te.text, te.pos, te.px, te.color);
            self.damage.add(preview.bounds().union(&self.text_box(&te)));
            if !te.text.trim().is_empty() {
                self.commit(preview);
            }
        }
    }

    // ---------------------------------------------------------------- input

    fn x_event(&mut self, e: Event) {
        match e {
            Event::Expose(ev) => {
                self.damage.add(IRect::xywh(ev.x as i32, ev.y as i32, ev.width as i32, ev.height as i32));
            }
            Event::ConfigureNotify(ev) if ev.window == self.x.root => {
                if ev.width != self.x.width || ev.height != self.x.height {
                    let _ = self.x.resize(ev.width, ev.height);
                    if let Some(p) = Pixmap::new(ev.width as u32, ev.height as u32) {
                        self.base = p;
                    }
                    self.rebuild_base();
                    self.toolbar.place(self.x.primary_monitor());
                    self.monitors = self.x.monitors();
                    let _ = self.x.set_input(self.active);
                    self.damage_all();
                }
            }
            Event::KeyPress(ev) => {
                let state = u16::from(ev.state);
                if self.x.is_hotkey(ev.detail) {
                    let cmd = if state & u16::from(KeyButMask::CONTROL) != 0 {
                        Cmd::Visibility
                    } else if state & u16::from(KeyButMask::SHIFT) != 0 {
                        Cmd::Clear
                    } else if state & u16::from(KeyButMask::MOD1) != 0 {
                        Cmd::Quit
                    } else {
                        Cmd::Toggle
                    };
                    self.command(cmd);
                    return;
                }
                if self.active {
                    let sym = self.x.keymap.lookup(ev.detail, state);
                    self.key(sym, state);
                }
            }
            Event::ButtonPress(ev) if self.active => {
                if !self.kbd_focused {
                    self.x.refocus_keyboard(ev.time);
                }
                let p = Pt::new(ev.event_x as f32, ev.event_y as f32);
                self.cursor = Some(p);
                self.press(ev.detail, p, u16::from(ev.state));
            }
            Event::ButtonRelease(ev) if self.active => {
                let p = Pt::new(ev.event_x as f32, ev.event_y as f32);
                self.release(ev.detail, p, u16::from(ev.state));
            }
            Event::MotionNotify(ev) if self.active => {
                let p = Pt::new(ev.event_x as f32, ev.event_y as f32);
                self.cursor = Some(p);
                self.motion(p, u16::from(ev.state));
            }
            Event::LeaveNotify(_) => {
                self.cursor = None;
                if self.hover.take().is_some() {
                    self.damage_toolbar();
                }
            }
            Event::FocusIn(ev) if ev.event == self.x.kbd => self.kbd_focused = true,
            Event::FocusOut(ev) if ev.event == self.x.kbd => self.kbd_focused = false,
            // Closing the keyboard window (e.g. Alt+F4) stops drawing.
            Event::ClientMessage(ev) if ev.window == self.x.kbd && ev.data.as_data32()[0] == self.x.wm_delete_window => {
                self.deactivate()
            }
            Event::Error(err) => eprintln!("screen-writer: X error: {err:?}"),
            _ => {}
        }
    }

    fn key(&mut self, sym: u32, state: u16) {
        let ctrl = state & u16::from(KeyButMask::CONTROL) != 0;
        let shift = state & u16::from(KeyButMask::SHIFT) != 0;

        if let Some(te) = &self.text {
            let before = self.text_box(te);
            let te = self.text.as_mut().unwrap();
            match sym {
                keysym::ESCAPE => {
                    self.commit_text();
                    return;
                }
                keysym::RETURN | keysym::KP_ENTER if ctrl => {
                    self.commit_text();
                    return;
                }
                keysym::RETURN | keysym::KP_ENTER => te.text.push('\n'),
                keysym::BACKSPACE => {
                    te.text.pop();
                }
                _ if ctrl => {}
                _ => match keysym_char(sym) {
                    Some(c) if !c.is_control() => te.text.push(c),
                    _ => return,
                },
            }
            let te = self.text.as_ref().unwrap();
            let after = self.text_box(te);
            self.damage.add(before.union(&after));
            return;
        }

        let ch = keysym_char(sym).map(|c| c.to_ascii_lowercase());
        if ctrl {
            match ch {
                Some('z') if shift => self.redo(),
                Some('z') => self.undo(),
                Some('y') => self.redo(),
                _ => {}
            }
            return;
        }
        match sym {
            keysym::ESCAPE => return self.deactivate(),
            keysym::SPACE | keysym::TAB | keysym::ISO_LEFT_TAB => {
                self.toolbar.visible = !self.toolbar.visible;
                self.hover = None;
                return self.damage_toolbar();
            }
            keysym::DELETE => return self.clear(),
            keysym::KP_ADD => return self.set_size(self.size + 1.0),
            keysym::KP_SUBTRACT => return self.set_size(self.size - 1.0),
            _ => {}
        }
        let Some(c) = ch else { return };
        if let Some(t) = Tool::ALL.iter().find(|t| t.key().to_ascii_lowercase() == c) {
            return self.set_tool(*t);
        }
        match c {
            '1'..='8' => self.set_color(PALETTE[c as usize - '1' as usize]),
            'f' => {
                self.fill = !self.fill;
                self.damage_toolbar();
            }
            'c' => self.clear(),
            'w' => self.set_board(if self.board == 1 { 0 } else { 1 }),
            'b' => self.set_board(if self.board == 2 { 0 } else { 2 }),
            '[' | '-' => self.set_size(self.size - 1.0),
            ']' | '=' | '+' => self.set_size(self.size + 1.0),
            _ => {}
        }
    }

    fn set_board(&mut self, b: u8) {
        self.board = b;
        self.damage_all();
    }

    fn on_toolbar(&self, p: Pt) -> Option<Btn> {
        if self.toolbar.visible { self.toolbar.btn_at(p.x, p.y) } else { None }
    }

    fn press(&mut self, button: u8, p: Pt, state: u16) {
        let shift = state & u16::from(KeyButMask::SHIFT) != 0;
        let ctrl = state & u16::from(KeyButMask::CONTROL) != 0;
        match button {
            4 | 5 => {
                let dir = if button == 4 { 1 } else { -1 };
                if ctrl {
                    let i = PALETTE.iter().position(|c| *c == self.color).unwrap_or(0) as i32;
                    let n = PALETTE.len() as i32;
                    self.set_color(PALETTE[((i - dir).rem_euclid(n)) as usize]);
                } else {
                    let step = if shift { 4.0 } else { 1.0 };
                    self.set_size(self.size + dir as f32 * step);
                }
                return;
            }
            6 | 7 => return,
            _ => {}
        }
        if !matches!(self.drag, Drag::None) {
            return;
        }
        if button == 1
            && let Some(b) = self.on_toolbar(p)
        {
            return self.toolbar_click(b, p);
        }
        if button == 3 {
            self.drag = Drag::Erase { erased: false };
            self.erase_at(p);
            return;
        }
        if button != 1 {
            return;
        }
        self.start = p;
        let w = self.size;
        let mk = |kind| Some(Shape { kind, color: self.color, width: w });
        match self.tool {
            Tool::Pen | Tool::Highlighter => {
                self.current = mk(Kind::Free { points: vec![p], highlighter: self.tool == Tool::Highlighter });
                self.damage.add(self.current.as_ref().unwrap().bounds());
                self.drag = Drag::Draw;
            }
            Tool::Line | Tool::Arrow => {
                self.current = mk(Kind::Line { a: p, b: p, arrow: self.tool == Tool::Arrow });
                self.drag = Drag::Draw;
            }
            Tool::Rect => {
                self.current = mk(Kind::Rect { a: p, b: p, filled: self.fill });
                self.drag = Drag::Draw;
            }
            Tool::Ellipse => {
                self.current = mk(Kind::Ellipse { a: p, b: p, filled: self.fill });
                self.drag = Drag::Draw;
            }
            Tool::Marker => {
                let m = Shape::marker(self.font.as_ref(), p, self.size, self.marker_n, self.color);
                self.marker_n += 1;
                self.commit(m);
            }
            Tool::Text => {
                self.commit_text();
                if self.font.is_some() {
                    let px = text_px(self.size);
                    self.text = Some(TextEdit {
                        pos: Pt::new(p.x, p.y - px * 0.6),
                        text: String::new(),
                        px,
                        color: self.color,
                    });
                }
            }
            Tool::Eraser => {
                self.drag = Drag::Erase { erased: false };
                self.erase_at(p);
            }
            Tool::Laser => {
                self.drag = Drag::Laser;
                self.laser.push((p, Instant::now()));
            }
            Tool::Spotlight => {}
        }
    }

    fn toolbar_click(&mut self, b: Btn, p: Pt) {
        match b {
            Btn::Grip => {
                self.drag = Drag::Toolbar { dx: p.x as i32 - self.toolbar.x, dy: p.y as i32 - self.toolbar.y };
            }
            Btn::Tool(t) => self.set_tool(t),
            Btn::Fill => {
                self.fill = !self.fill;
                self.damage_toolbar();
            }
            Btn::Color(i) => self.set_color(PALETTE[i]),
            Btn::SizeDown => self.set_size(self.size - 1.0),
            Btn::SizeUp => self.set_size(self.size + 1.0),
            Btn::SizeShow => {}
            Btn::Undo => self.undo(),
            Btn::Redo => self.redo(),
            Btn::Clear => self.clear(),
            Btn::Board => self.set_board((self.board + 1) % 3),
            Btn::Close => self.deactivate(),
        }
    }

    fn motion(&mut self, p: Pt, state: u16) {
        let shift = state & u16::from(KeyButMask::SHIFT) != 0;
        // Toolbar hover / tooltip.
        let hover = if matches!(self.drag, Drag::None) { self.on_toolbar(p) } else { None };
        if hover != self.hover {
            self.hover = hover;
            self.damage_toolbar();
        }
        match self.drag {
            Drag::None => {}
            Drag::Toolbar { dx, dy } => {
                self.damage_toolbar();
                let s = self.screen();
                self.toolbar.x = (p.x as i32 - dx).clamp(s.x0 - 40, s.x1 - 80);
                self.toolbar.y = (p.y as i32 - dy).clamp(s.y0, s.y1 - 40);
                self.damage_toolbar();
            }
            Drag::Erase { .. } => self.erase_at(p),
            Drag::Laser => self.laser.push((p, Instant::now())),
            Drag::Draw => {
                let start = self.start;
                let Some(cur) = &mut self.current else { return };
                match &mut cur.kind {
                    Kind::Free { points, highlighter } => {
                        if points.last().is_some_and(|l| l.dist(p) < 1.0) {
                            return;
                        }
                        points.push(p);
                        let n = points.len();
                        let tail = &points[n.saturating_sub(4)..];
                        let w = if *highlighter { shapes::highlighter_width(cur.width) } else { cur.width };
                        self.damage.add(IRect::from_points(tail, w / 2.0 + 3.0));
                    }
                    Kind::Line { b, .. } => *b = if shift { shapes::snap_angle(start, p) } else { p },
                    Kind::Rect { b, .. } | Kind::Ellipse { b, .. } => {
                        *b = if shift { shapes::snap_square(start, p) } else { p }
                    }
                    _ => {}
                }
            }
        }
    }

    fn release(&mut self, button: u8, _p: Pt, _state: u16) {
        match (&self.drag, button) {
            (Drag::Draw, 1) => {
                self.drag = Drag::None;
                if let Some(shape) = self.current.take() {
                    let degenerate = match &shape.kind {
                        Kind::Line { a, b, .. } => a.dist(*b) < 3.0,
                        Kind::Rect { a, b, .. } | Kind::Ellipse { a, b, .. } => {
                            (a.x - b.x).abs() < 2.0 && (a.y - b.y).abs() < 2.0
                        }
                        _ => false,
                    };
                    if degenerate {
                        self.damage.add(shape.bounds());
                    } else {
                        self.commit(shape);
                    }
                }
            }
            (Drag::Erase { .. }, 1 | 3) | (Drag::Laser, 1) | (Drag::Toolbar { .. }, 1) => self.drag = Drag::None,
            _ => {}
        }
    }

    fn eraser_radius(&self) -> f32 {
        6.0 + self.size * 2.5
    }

    fn erase_at(&mut self, p: Pt) {
        let r = self.eraser_radius();
        if !self.shapes.iter().any(|s| s.hit(p, r)) {
            return;
        }
        if let Drag::Erase { erased } = &mut self.drag
            && !*erased
        {
            *erased = true;
            self.checkpoint();
        }
        let keep: Vec<Rc<Shape>> = self.shapes.iter().filter(|s| !s.hit(p, r)).cloned().collect();
        self.set_shapes(keep);
    }

    fn tick(&mut self) {
        if self.laser.is_empty() {
            return;
        }
        let now = Instant::now();
        self.laser.retain(|(_, t)| now.duration_since(*t) < LASER_LIFE);
    }

    // ---------------------------------------------------------------- rendering

    fn text_box(&self, te: &TextEdit) -> IRect {
        let Some(font) = &self.font else { return IRect::EMPTY };
        let tp = font.text_path(&te.text, te.pos.x, te.pos.y, te.px);
        IRect::from_f(te.pos.x, te.pos.y, te.pos.x + tp.width + te.px, te.pos.y + tp.height, 10.0)
    }

    /// Bottom edge of the monitor the toolbar is on.
    fn toolbar_monitor_bottom(&self) -> i32 {
        let t = self.toolbar.rect();
        let (cx, cy) = ((t.x0 + t.x1) as f32 / 2.0, (t.y0 + t.y1) as f32 / 2.0);
        self.monitors.iter().find(|m| m.contains(cx, cy)).map_or(self.x.height as i32, |m| m.y1)
    }

    fn spotlight_on(&self) -> bool {
        self.active && self.visible && self.tool == Tool::Spotlight && self.cursor.is_some()
    }

    fn spot_radius(&self) -> f32 {
        70.0 + self.size * 10.0
    }

    fn cursor_over_toolbar(&self) -> bool {
        self.cursor.is_some_and(|c| self.on_toolbar(c).is_some())
    }

    /// Rectangles of overlays that move and must be repainted where they were and are.
    fn transient_rects(&self) -> Vec<IRect> {
        let mut v = Vec::new();
        if let Some(s) = &self.current
            && !matches!(s.kind, Kind::Free { .. })
        {
            v.push(s.bounds());
        }
        if let Some(te) = &self.text {
            v.push(self.text_box(te));
        }
        if !self.active {
            return v;
        }
        if !self.laser.is_empty() {
            let pts: Vec<Pt> = self.laser.iter().map(|(p, _)| *p).collect();
            v.push(IRect::from_points(&pts, self.size + 16.0));
        }
        if let Some(c) = self.cursor {
            let r = match self.tool {
                Tool::Spotlight => self.spot_radius() + 4.0,
                Tool::Eraser => self.eraser_radius() + 4.0,
                Tool::Marker => shapes::marker_radius(self.size) + 4.0,
                Tool::Highlighter => shapes::highlighter_width(self.size) / 2.0 + 4.0,
                Tool::Laser => 14.0,
                _ => self.size / 2.0 + 6.0,
            };
            v.push(IRect::around(c, r));
        }
        v
    }

    fn render(&mut self) {
        let spot = self.spotlight_on();
        if spot != self.last_spot {
            self.last_spot = spot;
            self.damage_all();
        }
        let now = self.transient_rects();
        let prev = std::mem::replace(&mut self.last_transient, now.clone());
        for r in prev.into_iter().chain(now) {
            self.damage.add(r);
        }
        if self.damage.is_empty() || !self.visible {
            let _ = self.damage.take(self.screen());
            return;
        }
        let rects = self.damage.take(self.screen());
        for r in rects {
            self.render_rect(r);
        }
        self.x.flush();
    }

    fn render_rect(&self, r: IRect) {
        let (w, h) = (r.w() as u32, r.h() as u32);
        let Some(mut pm) = Pixmap::new(w, h) else { return };
        let board = BOARD_COLORS[self.board as usize];
        if self.board != 0 {
            pm.fill(board.color());
        }
        composite_region(&mut pm, &self.base, r, self.board != 0);
        let tf = Transform::from_translate(-r.x0 as f32, -r.y0 as f32);
        {
            let mut m = pm.as_mut();
            if let Some(s) = &self.current {
                s.draw(&mut m, tf);
            }
            if let (Some(te), Some(font)) = (&self.text, &self.font) {
                Shape::text(font, &te.text, te.pos, te.px, te.color).draw(&mut m, tf);
                let tp = font.text_path(&te.text, te.pos.x, te.pos.y, te.px);
                let (cx, cy) = (te.pos.x + tp.caret.0 + 2.0, te.pos.y + tp.caret.1);
                let mut pb = PathBuilder::new();
                pb.move_to(cx, cy + 2.0);
                pb.line_to(cx, cy + tp.line_height - 2.0);
                if let Some(p) = pb.finish() {
                    m.stroke_path(&p, &paint(te.color.contrast()), &round_stroke(4.0), tf, None);
                    m.stroke_path(&p, &paint(te.color), &round_stroke(2.0), tf, None);
                }
            }
            if self.active {
                self.draw_laser(&mut m, tf);
                if self.last_spot
                    && let Some(c) = self.cursor
                {
                    let mut pb = PathBuilder::new();
                    pb.push_rect(Rect::from_xywh(r.x0 as f32 - 1.0, r.y0 as f32 - 1.0, w as f32 + 2.0, h as f32 + 2.0).unwrap());
                    pb.push_circle(c.x, c.y, self.spot_radius());
                    if let Some(p) = pb.finish() {
                        m.fill_path(&p, &paint(Rgba(0, 0, 0, 165)), FillRule::EvenOdd, tf, None);
                    }
                }
                if !self.cursor_over_toolbar() {
                    self.draw_cursor(&mut m, tf);
                }
                if self.toolbar.visible {
                    let view = View {
                        tool: self.tool,
                        color: self.color,
                        size: self.size,
                        fill: self.fill,
                        board: self.board,
                        can_undo: !self.undo.is_empty(),
                        can_redo: !self.redo.is_empty(),
                        hover: self.hover,
                        pressed: None,
                        font: self.font.as_ref(),
                        screen_bottom: self.toolbar_monitor_bottom(),
                    };
                    if self.toolbar.rect_with_tooltip().touches(&r, 0) {
                        self.toolbar.draw(&mut m, tf, &view);
                    }
                }
            }
        }
        let mut data = pm.take();
        if let Err(e) = self.x.put_rgba(r.x0, r.y0, w, h, &mut data) {
            eprintln!("screen-writer: upload failed: {e}");
        }
    }

    fn draw_laser(&self, m: &mut PixmapMut, tf: Transform) {
        if self.laser.is_empty() {
            return;
        }
        let now = Instant::now();
        let w = self.size + 4.0;
        for pair in self.laser.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let age = now.duration_since(b.1).as_secs_f32() / LASER_LIFE.as_secs_f32();
            let alpha = (1.0 - age).clamp(0.0, 1.0);
            let mut pb = PathBuilder::new();
            pb.move_to(a.0.x, a.0.y);
            pb.line_to(b.0.x, b.0.y);
            let Some(p) = pb.finish() else { continue };
            m.stroke_path(&p, &paint(Rgba(255, 30, 30, (70.0 * alpha) as u8)), &round_stroke(w * 2.6), tf, None);
            m.stroke_path(&p, &paint(Rgba(255, 70, 60, (255.0 * alpha) as u8)), &round_stroke(w), tf, None);
            m.stroke_path(&p, &paint(Rgba(255, 220, 210, (230.0 * alpha) as u8)), &round_stroke(w * 0.35), tf, None);
        }
    }

    fn draw_cursor(&self, m: &mut PixmapMut, tf: Transform) {
        let Some(c) = self.cursor else { return };
        let ring = |m: &mut PixmapMut, r: f32, col: Rgba| {
            if let Some(p) = PathBuilder::from_circle(c.x, c.y, r.max(1.0)) {
                m.stroke_path(&p, &paint(Rgba(0, 0, 0, 160)), &round_stroke(3.0), tf, None);
                m.stroke_path(&p, &paint(col), &round_stroke(1.5), tf, None);
            }
        };
        let dot = |m: &mut PixmapMut, r: f32, col: Rgba| {
            if let Some(p) = PathBuilder::from_circle(c.x, c.y, r.max(1.0)) {
                m.fill_path(&p, &paint(col), FillRule::Winding, tf, None);
            }
        };
        match self.tool {
            Tool::Eraser => ring(m, self.eraser_radius(), Rgba(255, 255, 255, 230)),
            Tool::Highlighter => dot(m, shapes::highlighter_width(self.size) / 2.0, self.color.with_alpha(90)),
            Tool::Marker => {
                let mut ghost = Shape::marker(self.font.as_ref(), c, self.size, self.marker_n, self.color);
                ghost.color = ghost.color.with_alpha(110);
                ghost.draw(m, tf);
            }
            Tool::Laser => {
                dot(m, 11.0, Rgba(255, 30, 30, 70));
                dot(m, 5.0, Rgba(255, 60, 50, 255));
                dot(m, 2.0, Rgba(255, 230, 220, 255));
            }
            Tool::Spotlight => ring(m, self.spot_radius(), Rgba(255, 255, 255, 120)),
            Tool::Text => {}
            _ => {
                if self.size > 3.0 {
                    ring(m, self.size / 2.0, self.color);
                } else {
                    dot(m, 2.0, self.color);
                }
            }
        }
    }
}

fn text_px(size: f32) -> f32 {
    14.0 + size * 4.0
}

/// Composite (source-over) the `r` region of `src` onto `dst` (both premultiplied RGBA).
fn composite_region(dst: &mut Pixmap, src: &Pixmap, r: IRect, blend: bool) {
    let sw = src.width() as usize;
    let w = r.w() as usize;
    let s = src.data();
    let d = dst.data_mut();
    for row in 0..r.h() as usize {
        let so = ((r.y0 as usize + row) * sw + r.x0 as usize) * 4;
        let src_row = &s[so..so + w * 4];
        let dst_row = &mut d[row * w * 4..(row + 1) * w * 4];
        if !blend {
            dst_row.copy_from_slice(src_row);
            continue;
        }
        for (sp, dp) in src_row.chunks_exact(4).zip(dst_row.chunks_exact_mut(4)) {
            let a = sp[3] as u32;
            if a == 0 {
                continue;
            }
            if a == 255 {
                dp.copy_from_slice(sp);
                continue;
            }
            let inv = 255 - a;
            for i in 0..4 {
                dp[i] = (sp[i] as u32 + (dp[i] as u32 * inv + 127) / 255).min(255) as u8;
            }
        }
    }
}
