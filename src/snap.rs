//! Screen capture through the xdg-desktop-portal Screenshot interface.
//!
//! XWayland can only read back X11 windows, so on Wayland the real screen
//! contents have to come from the compositor. The portal writes a PNG of the
//! whole desktop, which is loaded and then deleted.

use std::collections::HashMap;
use tiny_skia::Pixmap;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const DEST: &str = "org.freedesktop.portal.Desktop";

/// Full-desktop screenshot as a premultiplied RGBA pixmap.
pub fn capture() -> Result<Pixmap, String> {
    let conn = Connection::session().map_err(|e| format!("D-Bus: {e}"))?;
    let unique = conn.unique_name().ok_or("D-Bus: no unique name")?;
    let sender = unique.trim_start_matches(':').replace('.', "_");
    let token = format!("screen_writer_{}", std::process::id());
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");

    // Subscribe before calling, so the response can't be missed.
    let request = Proxy::new(&conn, DEST, path.as_str(), "org.freedesktop.portal.Request")
        .map_err(|e| format!("portal: {e}"))?;
    let mut responses = request.receive_signal("Response").map_err(|e| format!("portal: {e}"))?;

    let mut opts: HashMap<&str, Value> = HashMap::new();
    opts.insert("handle_token", Value::from(token.as_str()));
    opts.insert("interactive", Value::from(false));
    let _: OwnedObjectPath = conn
        .call_method(Some(DEST), "/org/freedesktop/portal/desktop", Some("org.freedesktop.portal.Screenshot"), "Screenshot", &("", opts))
        .and_then(|m| m.body().deserialize())
        .map_err(|e| format!("screenshot portal: {e}"))?;

    let msg = responses.next().ok_or("screenshot portal: no response")?;
    let (code, results): (u32, HashMap<String, OwnedValue>) =
        msg.body().deserialize().map_err(|e| format!("screenshot portal: {e}"))?;
    if code != 0 {
        return Err("screenshot was cancelled or denied".into());
    }
    let uri: String = results
        .get("uri")
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| String::try_from(v).ok())
        .ok_or("screenshot portal: no file returned")?;
    let file = uri_to_path(&uri).ok_or_else(|| format!("unexpected screenshot location {uri}"))?;
    let shot = Pixmap::load_png(&file).map_err(|e| format!("reading {file}: {e}"));
    let _ = std::fs::remove_file(&file);
    shot
}

fn uri_to_path(uri: &str) -> Option<String> {
    let raw = uri.strip_prefix("file://")?.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%'
            && let Some(b) = raw.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok())
        {
            out.push(b);
            i += 3;
        } else {
            out.push(raw[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Copy the `(x, y, w, h)` region (in screen pixels) out of a screenshot that is
/// `scale` times the screen size.
pub fn crop(shot: &Pixmap, x: i32, y: i32, w: i32, h: i32, scale: f32) -> Option<Pixmap> {
    let sx0 = ((x as f32 * scale).round() as i32).clamp(0, shot.width() as i32);
    let sy0 = ((y as f32 * scale).round() as i32).clamp(0, shot.height() as i32);
    let sx1 = (((x + w) as f32 * scale).round() as i32).clamp(0, shot.width() as i32);
    let sy1 = (((y + h) as f32 * scale).round() as i32).clamp(0, shot.height() as i32);
    let (cw, ch) = ((sx1 - sx0) as usize, (sy1 - sy0) as usize);
    let mut out = Pixmap::new(cw as u32, ch as u32)?;
    let stride = shot.width() as usize * 4;
    let src = shot.data();
    let dst = out.data_mut();
    for row in 0..ch {
        let so = (sy0 as usize + row) * stride + sx0 as usize * 4;
        dst[row * cw * 4..(row + 1) * cw * 4].copy_from_slice(&src[so..so + cw * 4]);
    }
    Some(out)
}
