//! Install/uninstall GNOME custom keyboard shortcuts. Under Wayland, X11 key
//! grabs don't see keys pressed in native Wayland apps, so the desktop must
//! forward the hotkeys to us.

use std::process::Command;

const LIST_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
const ITEM_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const BASE: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";

const BINDINGS: [(&str, &str, &str); 4] = [
    ("toggle", "Screen Writer: toggle drawing", "F9"),
    ("clear", "Screen Writer: clear", "<Shift>F9"),
    ("visibility", "Screen Writer: show/hide", "<Control>F9"),
    ("quit", "Screen Writer: quit", "<Alt>F9"),
];

fn gsettings(args: &[&str]) -> Result<String, String> {
    let out = Command::new("gsettings").args(args).output().map_err(|e| format!("gsettings: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Parse a GVariant string array like `['a', 'b']` or `@as []`.
fn parse_list(s: &str) -> Vec<String> {
    s.split('\'').skip(1).step_by(2).map(String::from).collect()
}

fn format_list(v: &[String]) -> String {
    format!("[{}]", v.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(", "))
}

fn gvariant_str(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn normalize(binding: &str) -> String {
    binding.trim_matches('\'').to_ascii_lowercase().replace("<ctrl>", "<control>").replace("<primary>", "<control>")
}

pub fn install() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.to_string_lossy();
    let mut list = parse_list(&gsettings(&["get", LIST_SCHEMA, "custom-keybindings"])?);

    // Disable other custom shortcuts using the same keys (e.g. gromit-mpx).
    let ours: Vec<String> = BINDINGS.iter().map(|b| normalize(b.2)).collect();
    list.retain(|path| {
        if path.contains("screen-writer-") {
            return true;
        }
        let key = format!("{ITEM_SCHEMA}:{path}");
        let binding = gsettings(&["get", &key, "binding"]).unwrap_or_default();
        if ours.contains(&normalize(&binding)) {
            let cmd = gsettings(&["get", &key, "command"]).unwrap_or_default();
            println!("removing conflicting shortcut {binding} -> {cmd}");
            false
        } else {
            true
        }
    });

    for (cmd, name, binding) in BINDINGS {
        let path = format!("{BASE}screen-writer-{cmd}/");
        let key = format!("{ITEM_SCHEMA}:{path}");
        gsettings(&["set", &key, "name", &gvariant_str(name)])?;
        gsettings(&["set", &key, "command", &gvariant_str(&format!("\"{exe}\" {cmd}"))])?;
        gsettings(&["set", &key, "binding", &gvariant_str(binding)])?;
        if !list.contains(&path) {
            list.push(path);
        }
        println!("{binding:>12}  ->  screen-writer {cmd}");
    }
    gsettings(&["set", LIST_SCHEMA, "custom-keybindings", &format_list(&list)])?;
    println!("GNOME shortcuts installed (command: {exe})");
    Ok(())
}

pub fn uninstall() -> Result<(), String> {
    let mut list = parse_list(&gsettings(&["get", LIST_SCHEMA, "custom-keybindings"])?);
    let before = list.len();
    for path in list.iter().filter(|p| p.contains("screen-writer-")) {
        let _ = gsettings(&["reset-recursively", &format!("{ITEM_SCHEMA}:{path}")]);
    }
    list.retain(|p| !p.contains("screen-writer-"));
    gsettings(&["set", LIST_SCHEMA, "custom-keybindings", &format_list(&list)])?;
    println!("removed {} shortcut(s)", before - list.len());
    Ok(())
}
