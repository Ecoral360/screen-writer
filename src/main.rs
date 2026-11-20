//! screen-writer: draw on your screen. Inspired by gromit-mpx.

mod app;
mod font;
mod geom;
mod gnome;
mod ipc;
mod shapes;
mod snap;
mod toolbar;
mod xwin;

use app::{App, Cmd, Msg};
use std::sync::mpsc;

const HELP: &str = "\
screen-writer — draw on your screen (gromit-mpx inspired)

USAGE:
    screen-writer                 start the daemon (idle, drawings click-through)
    screen-writer <command>       control the running daemon
    screen-writer install-gnome-shortcuts    bind F9 / Shift+F9 / Ctrl+F9 / Alt+F9 in GNOME
    screen-writer uninstall-gnome-shortcuts

COMMANDS (also accepted as --toggle, --clear, ... like gromit-mpx):
    toggle        start/stop drawing (starts the daemon if needed)    [F9]
    clear         erase everything                                    [Shift+F9]
    visibility    hide/show the drawings                              [Ctrl+F9]
    quit          exit                                                [Alt+F9]
    activate | deactivate | undo | redo
    tool <select|pen|highlighter|line|arrow|rectangle|ellipse|text|marker|eraser|laser|spotlight|snapshot>
    color <red|orange|yellow|green|blue|purple|white|black|1-8|#rrggbb>
    size <1-40>
    demo          draw one sample of each annotation type

WHILE DRAWING:
    Left drag     draw with current tool       Right drag   erase objects
    Scroll        size (Shift: x4)             Ctrl+Scroll  cycle colors
    Shift         snap lines to 15°, squares/circles
    V P H L A R O T N E K S G
                            select/move, pen, highlighter, line, arrow, rect,
                            ellipse, text, numbered marker, eraser, laser,
                            spotlight, snapshot
    1-8 colors   F fill   [ ] size   W/B white/blackboard   C clear
    Ctrl+Z undo   Ctrl+Y redo   Esc stop drawing
    Space  hide toolbar      Tab  hide toolbar and cursor preview (clean screen)
    Text tool: click, type, Enter = newline, Esc/Ctrl+Enter = finish
    Marker tool: Ctrl+R restarts numbering at 1
    Snapshot tool: drag a region, move, click to pin (Esc/right-click discards it);
                   drag a pinned snapshot to move it
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let line = args.join(" ");
    let first = args.first().map(|s| s.trim_start_matches('-').to_string());

    match first.as_deref() {
        None => {}
        Some("h" | "help") => {
            print!("{HELP}");
            return;
        }
        Some("install-gnome-shortcuts") => return exit_on_err(gnome::install()),
        Some("uninstall-gnome-shortcuts") => return exit_on_err(gnome::uninstall()),
        Some(_) => {
            let Some(cmd) = Cmd::parse(&line) else {
                eprintln!("screen-writer: unknown command '{line}' (see --help)");
                std::process::exit(2);
            };
            if ipc::send(&line) {
                return;
            }
            // No daemon running: commands that start drawing launch one.
            if !matches!(cmd, Cmd::Toggle | Cmd::Activate) {
                eprintln!("screen-writer: not running");
                std::process::exit(1);
            }
            return run_daemon(Some(Cmd::Activate));
        }
    }
    run_daemon(None);
}

fn exit_on_err(r: Result<(), String>) {
    if let Err(e) = r {
        eprintln!("screen-writer: {e}");
        std::process::exit(1);
    }
}

fn run_daemon(initial: Option<Cmd>) {
    let listener = match ipc::bind() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("screen-writer: {e}");
            std::process::exit(1);
        }
    };
    let x = match xwin::XWin::new() {
        Ok(x) => x,
        Err(e) => {
            ipc::cleanup();
            eprintln!("screen-writer: X11 setup failed: {e}");
            std::process::exit(1);
        }
    };

    let (tx, rx) = mpsc::channel();
    ipc::serve(listener, tx.clone());

    // X event reader thread.
    let conn = x.conn.clone();
    let app_tx = tx.clone();
    std::thread::spawn(move || {
        while let Ok(e) = conn.wait_for_event() {
            if tx.send(Msg::X(e)).is_err() {
                return;
            }
        }
        let _ = tx.send(Msg::XClosed);
    });

    let mut app = App::new(x, app_tx);
    if let Some(c) = initial {
        app.command(c);
    }
    eprintln!("screen-writer: running (F9 toggles drawing; `screen-writer --help` for more)");
    app.run(rx);
    ipc::cleanup();
}

use x11rb::connection::Connection as _;
