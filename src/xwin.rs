//! X11 plumbing: a transparent, override-redirect, full-screen ARGB window,
//! input-shape based click-through, global hotkeys and image upload.

use crate::geom::IRect;
use std::sync::Arc;
use x11rb::connection::{Connection, RequestConnection as _};
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};

pub mod keysym {
    pub const F9: u32 = 0xffc6;
    pub const ESCAPE: u32 = 0xff1b;
    pub const RETURN: u32 = 0xff0d;
    pub const KP_ENTER: u32 = 0xff8d;
    pub const BACKSPACE: u32 = 0xff08;
    pub const TAB: u32 = 0xff09;
    pub const SPACE: u32 = 0x20;
    pub const ISO_LEFT_TAB: u32 = 0xfe20;
    pub const DELETE: u32 = 0xffff;
    pub const KP_ADD: u32 = 0xffab;
    pub const KP_SUBTRACT: u32 = 0xffad;
}

pub struct Keymap {
    min: u8,
    per: usize,
    syms: Vec<u32>,
}

impl Keymap {
    fn load(conn: &RustConnection) -> anyhow_lite::Result<Keymap> {
        let setup = conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let r = conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
        Ok(Keymap { min, per: r.keysyms_per_keycode as usize, syms: r.keysyms })
    }

    fn sym_at(&self, code: u8, idx: usize) -> u32 {
        if code < self.min || idx >= self.per {
            return 0;
        }
        *self.syms.get((code - self.min) as usize * self.per + idx).unwrap_or(&0)
    }

    /// Keysym for a key press with the given modifier state.
    pub fn lookup(&self, code: u8, state: u16) -> u32 {
        let shift = state & u16::from(KeyButMask::SHIFT) != 0;
        let level3 = state & u16::from(KeyButMask::MOD5) != 0;
        let base = if level3 && self.sym_at(code, 4) != 0 { 4 } else { 0 };
        let mut sym = self.sym_at(code, base + shift as usize);
        if sym == 0 {
            sym = self.sym_at(code, base);
        }
        // Single-case alphabetic keysym lists: derive the uppercase variant.
        if shift && sym == self.sym_at(code, base) && (0x61..=0x7a).contains(&sym) {
            sym -= 0x20;
        }
        let caps = state & u16::from(KeyButMask::LOCK) != 0;
        if caps && let Some(c) = keysym_char(sym) {
            let flipped: Vec<char> =
                if c.is_lowercase() { c.to_uppercase().collect() } else { c.to_lowercase().collect() };
            if flipped.len() == 1 && flipped[0] != c {
                return char_keysym(flipped[0]);
            }
        }
        sym
    }

    pub fn keycode_for(&self, sym: u32) -> Option<u8> {
        (0..self.syms.len() / self.per.max(1))
            .find(|&i| self.syms[i * self.per] == sym)
            .map(|i| self.min + i as u8)
    }
}

pub fn keysym_char(sym: u32) -> Option<char> {
    match sym {
        0x20..=0x7e | 0xa0..=0xff => char::from_u32(sym),
        s if s & 0xff00_0000 == 0x0100_0000 => char::from_u32(s & 0x00ff_ffff),
        _ => None,
    }
}

fn char_keysym(c: char) -> u32 {
    let u = c as u32;
    if (0x20..=0x7e).contains(&u) || (0xa0..=0xff).contains(&u) { u } else { 0x0100_0000 | u }
}

/// Minimal error type so we don't need extra crates.
pub mod anyhow_lite {
    pub type Error = Box<dyn std::error::Error + Send + Sync>;
    pub type Result<T> = std::result::Result<T, Error>;
}
use anyhow_lite::Result;

pub struct XWin {
    pub conn: Arc<RustConnection>,
    pub root: Window,
    pub win: Window,
    gc: Gcontext,
    pub width: u16,
    pub height: u16,
    pub keymap: Keymap,
    msb_first: bool,
    mapped: bool,
    hotkey_code: Option<u8>,
}

impl XWin {
    pub fn new() -> Result<XWin> {
        let (conn, screen_num) = x11rb::connect(None)?;
        let conn = Arc::new(conn);
        let screen = conn.setup().roots[screen_num].clone();
        let root = screen.root;

        // Find a 32-bit TrueColor (ARGB) visual.
        let visual = screen
            .allowed_depths
            .iter()
            .filter(|d| d.depth == 32)
            .flat_map(|d| d.visuals.iter())
            .find(|v| v.class == VisualClass::TRUE_COLOR)
            .ok_or("no 32-bit ARGB visual available (is a compositor running?)")?
            .visual_id;

        let colormap = conn.generate_id()?;
        conn.create_colormap(ColormapAlloc::NONE, colormap, root, visual)?;

        let (width, height) = (screen.width_in_pixels, screen.height_in_pixels);
        let win = conn.generate_id()?;
        conn.create_window(
            32,
            win,
            root,
            0,
            0,
            width,
            height,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &CreateWindowAux::new()
                .background_pixel(0)
                .border_pixel(0)
                .colormap(colormap)
                .override_redirect(1)
                .event_mask(
                    EventMask::EXPOSURE
                        | EventMask::BUTTON_PRESS
                        | EventMask::BUTTON_RELEASE
                        | EventMask::POINTER_MOTION
                        | EventMask::KEY_PRESS
                        | EventMask::LEAVE_WINDOW
                        | EventMask::FOCUS_CHANGE,
                ),
        )?;
        // Track screen size changes.
        conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY))?;

        let title = b"screen-writer";
        conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_NAME, AtomEnum::STRING, title)?;
        conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_CLASS, AtomEnum::STRING, b"screen-writer\0screen-writer\0")?;
        let bypass = conn.intern_atom(false, b"_NET_WM_BYPASS_COMPOSITOR")?.reply()?.atom;
        conn.change_property32(PropMode::REPLACE, win, bypass, AtomEnum::CARDINAL, &[2])?;

        // Crosshair cursor from the core cursor font.
        let font = conn.generate_id()?;
        conn.open_font(font, b"cursor")?;
        let cursor = conn.generate_id()?;
        conn.create_glyph_cursor(cursor, font, font, 34, 35, 0, 0, 0, 0xffff, 0xffff, 0xffff)?;
        conn.change_window_attributes(win, &ChangeWindowAttributesAux::new().cursor(cursor))?;
        conn.close_font(font)?;

        let gc = conn.generate_id()?;
        conn.create_gc(gc, win, &CreateGCAux::new())?;

        // Version negotiation for the extensions we use.
        conn.shape_query_version()?.reply()?;
        let _ = conn.randr_query_version(1, 5).map(|c| c.reply());

        let keymap = Keymap::load(&conn)?;
        let msb_first = conn.setup().image_byte_order == ImageOrder::MSB_FIRST;

        let mut x = XWin { conn, root, win, gc, width, height, keymap, msb_first, mapped: false, hotkey_code: None };
        x.set_input(false)?;
        x.grab_hotkeys();
        x.set_mapped(true)?;
        x.conn.flush()?;
        Ok(x)
    }

    /// Grab F9 (plain, Shift, Ctrl, Alt) on the root window. On X11 sessions this is
    /// the global hotkey; on Wayland it only works while an X11 window has focus,
    /// so desktop shortcuts calling `screen-writer toggle` etc. are used instead.
    fn grab_hotkeys(&mut self) {
        let Some(code) = self.keymap.keycode_for(keysym::F9) else { return };
        self.hotkey_code = Some(code);
        let base = [0u16, u16::from(ModMask::SHIFT), u16::from(ModMask::CONTROL), u16::from(ModMask::M1)];
        let ignored = [0u16, u16::from(ModMask::LOCK), u16::from(ModMask::M2), u16::from(ModMask::LOCK | ModMask::M2)];
        let mut failed = false;
        for b in base {
            for i in ignored {
                let res = self
                    .conn
                    .grab_key(true, self.root, ModMask::from(b | i), code, GrabMode::ASYNC, GrabMode::ASYNC)
                    .map(|c| c.check());
                if !matches!(res, Ok(Ok(()))) {
                    failed = true;
                }
            }
        }
        if failed {
            eprintln!("screen-writer: could not grab F9 globally (another program owns it); use desktop shortcuts");
        }
    }

    pub fn is_hotkey(&self, code: u8) -> bool {
        self.hotkey_code == Some(code)
    }

    /// Make the window capture input (`true`) or be click-through (`false`).
    pub fn set_input(&self, capture: bool) -> Result<()> {
        let rects: &[Rectangle] =
            if capture { &[Rectangle { x: 0, y: 0, width: self.width, height: self.height }] } else { &[] };
        self.conn.shape_rectangles(shape::SO::SET, shape::SK::INPUT, ClipOrdering::UNSORTED, self.win, 0, 0, rects)?;
        Ok(())
    }

    pub fn set_mapped(&mut self, mapped: bool) -> Result<()> {
        if mapped && !self.mapped {
            self.conn.map_window(self.win)?;
        } else if !mapped && self.mapped {
            self.conn.unmap_window(self.win)?;
        }
        self.mapped = mapped;
        Ok(())
    }

    pub fn raise(&self) -> Result<()> {
        self.conn.configure_window(self.win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;
        Ok(())
    }

    /// Try to obtain keyboard input (works on X11; best effort under XWayland).
    pub fn grab_keyboard(&self, time: u32) {
        let _ = self.conn.set_input_focus(InputFocus::PARENT, self.win, time);
        if let Ok(c) = self.conn.grab_keyboard(true, self.win, time, GrabMode::ASYNC, GrabMode::ASYNC) {
            // Don't block on the reply; we don't need to know.
            drop(c);
        }
    }

    pub fn ungrab_keyboard(&self) {
        let _ = self.conn.ungrab_keyboard(CURRENT_TIME);
        let _ = self.conn.set_input_focus(InputFocus::POINTER_ROOT, NONE, CURRENT_TIME);
    }

    pub fn resize(&mut self, w: u16, h: u16) -> Result<()> {
        self.width = w;
        self.height = h;
        self.conn.configure_window(self.win, &ConfigureWindowAux::new().x(0).y(0).width(w as u32).height(h as u32))?;
        Ok(())
    }

    /// Geometry of the primary monitor (falls back to the whole screen).
    pub fn primary_monitor(&self) -> IRect {
        let full = IRect::xywh(0, 0, self.width as i32, self.height as i32);
        let Ok(Ok(r)) = self.conn.randr_get_monitors(self.root, true).map(|c| c.reply()) else { return full };
        let pick = r.monitors.iter().find(|m| m.primary).or(r.monitors.first());
        pick.map(|m| IRect::xywh(m.x as i32, m.y as i32, m.width as i32, m.height as i32)).unwrap_or(full)
    }

    /// Upload premultiplied RGBA pixels (`w*h*4` bytes) to the window at `(x, y)`.
    pub fn put_rgba(&self, x: i32, y: i32, w: u32, h: u32, rgba: &mut [u8]) -> Result<()> {
        // Convert to the server's 32-bit ARGB pixel layout.
        for px in rgba.chunks_exact_mut(4) {
            let (r, g, b, a) = (px[0], px[1], px[2], px[3]);
            if self.msb_first {
                px.copy_from_slice(&[a, r, g, b]);
            } else {
                px.copy_from_slice(&[b, g, r, a]);
            }
        }
        let stride = w as usize * 4;
        let max = self.conn.maximum_request_bytes().saturating_sub(64);
        let rows_per = (max / stride).clamp(1, h as usize);
        let mut row = 0usize;
        while row < h as usize {
            let n = rows_per.min(h as usize - row);
            let data = &rgba[row * stride..(row + n) * stride];
            self.conn.put_image(ImageFormat::Z_PIXMAP, self.win, self.gc, w as u16, n as u16, x as i16, (y + row as i32) as i16, 0, 32, data)?;
            row += n;
        }
        Ok(())
    }

    pub fn flush(&self) {
        let _ = self.conn.flush();
    }
}
