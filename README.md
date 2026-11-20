# screen-writer

Draw on your screen. Annotate a presentation, point things out during a screen
share, or sketch over anything. Inspired by
[gromit-mpx](https://github.com/bk138/gromit-mpx).

- Press **F9** to start drawing and **F9** again to stop. Your drawings stay on
  screen, and clicks pass through to the apps underneath.
- **Shift+F9** clears everything.

## Features

| Tool | Key | Notes |
|---|---|---|
| Select and move | `V` | Drag anything you placed (strokes, arrows, text, markers, snapshots) to move it |
| Pen | `P` | Smoothed freehand strokes |
| Highlighter | `H` | Wide, translucent |
| Line | `L` | `Shift` snaps to 15° |
| Arrow | `A` | `Shift` snaps to 15° |
| Rectangle | `R` | `Shift` makes a square, `F` toggles fill |
| Ellipse | `O` | `Shift` makes a circle, `F` toggles fill |
| Text | `T` | Click, type, then `Esc` or `Ctrl+Enter` to finish |
| Numbered marker | `N` | Click to drop 1, 2, 3… `Ctrl+R` restarts at 1 (so does clear) |
| Eraser | `E` | Removes whole objects. Right-drag erases with any tool |
| Laser pointer | `K` | Fading trail, nothing is saved |
| Spotlight | `S` | Darkens the screen except around the cursor |
| Snapshot | `G` | Drag a region to capture it, then click to pin the copy anywhere. Drag a pinned snapshot to move it (or use Select) |

You also get:

- 8 colors
- adjustable size
- unlimited undo/redo
- a whiteboard/blackboard background
- a floating toolbar you can drag

All of these are on the toolbar. Hover a button to see its shortcut.

## Install

```sh
cargo install --path .
screen-writer install-gnome-shortcuts   # GNOME only, see below
```

### Nix

```sh
nix run .                 # run without installing
nix build .               # result/bin/screen-writer
nix develop               # shell with cargo, clippy, rust-analyzer
```

To install it system-wide from your NixOS flake:

```nix
# inputs
screen-writer.url = "github:<you>/screen-writer";   # or "path:/path/to/screen-writer"

# configuration
environment.systemPackages = [ inputs.screen-writer.packages.${pkgs.stdenv.hostPlatform.system}.default ];
```

The package adds `fc-match` and `gsettings` to the program's `PATH`.

Requirements:

- X11, or Wayland with XWayland
- a running compositor (for transparency)
- `xdg-desktop-portal` (with the GNOME or another backend), used by the snapshot
  tool to capture the screen
- `fc-match` (fontconfig), used to find a font for the text tools. To pick a
  font yourself, set `SCREEN_WRITER_FONT=/path/to/font.ttf`.

## Global hotkeys

| Key | Command | Action |
|---|---|---|
| `F9` | `screen-writer toggle` | Start/stop drawing (starts the daemon if needed) |
| `Shift+F9` | `screen-writer clear` | Erase everything |
| `Ctrl+F9` | `screen-writer visibility` | Hide/show the drawings |
| `Alt+F9` | `screen-writer quit` | Exit |

On an **X11 session**, screen-writer grabs these keys itself.

On **Wayland**, apps can't grab global keys, so the desktop has to run the
commands. On GNOME, set this up with:

```sh
screen-writer install-gnome-shortcuts     # also removes other custom shortcuts on the same keys
screen-writer uninstall-gnome-shortcuts
```

On other desktops, bind the keys to the commands above yourself.

## While drawing

| Input | Action |
|---|---|
| Left drag | Draw with the current tool |
| Right drag | Erase objects |
| Scroll | Change size (`Shift`: in steps of 4) |
| `Ctrl` + scroll | Cycle colors |
| `1`–`8` | Pick a color |
| `[` `]` or `-` `+` | Change size |
| `F` | Toggle fill for shapes |
| `W` / `B` | Toggle whiteboard / blackboard |
| `C` or `Delete` | Clear |
| `Ctrl+Z` | Undo |
| `Ctrl+Y` or `Ctrl+Shift+Z` | Redo |
| `Space` | Hide/show the toolbar |
| `Tab` | Hide/show the toolbar *and* the cursor preview (marker ghost, size ring, select outline), for a clean screen in screenshots and shares. The laser dot stays |
| `Esc` | Stop drawing (drawings stay on screen). While placing a new snapshot, discards it instead |

## Command line

```
screen-writer                  start the daemon (idle)
screen-writer toggle | activate | deactivate
screen-writer clear | undo | redo | visibility | quit
screen-writer tool <select|pen|highlighter|line|arrow|rectangle|ellipse|text|marker|eraser|laser|spotlight|snapshot>
screen-writer color <red|orange|yellow|green|blue|purple|white|black|1-8|#rrggbb>
screen-writer size <1-40>
screen-writer demo             draw one sample of each annotation type
```

gromit-mpx-style flags also work, for example `screen-writer --toggle`.

Commands reach the running instance through a Unix socket in
`$XDG_RUNTIME_DIR`.

## How it works

screen-writer is a single X11 window:

- It has a 32-bit ARGB visual, so it can be transparent.
- It covers the whole screen and is override-redirect, so it sits above other
  windows.
- When you're not drawing, its *input shape* is set to empty, so clicks pass
  through it.

Drawing is done with [tiny-skia](https://github.com/RazrFalcon/tiny-skia). Only
the parts of the screen that changed are redrawn and sent to the X server.

On Wayland, screen-writer runs through XWayland. GNOME doesn't support the
`wlr-layer-shell` protocol, which a native Wayland overlay would need.

## Known limitations

- **Keyboard focus while drawing:** an always-on-top overlay window can't receive
  keyboard focus on Wayland. So while you're drawing, screen-writer also opens a
  tiny invisible window named "screen-writer (drawing)", and GNOME gives it focus
  like any other app. It may show up in the Activities overview. Alt+F4 on it
  stops drawing. When drawing stops, the window closes and focus goes back to the
  app you were using. If you switch apps mid-drawing, clicking the overlay brings
  keyboard focus back.
- **No pressure sensitivity:** tablet pressure isn't used yet.
- **Snapshots:** the screenshot portal saves a full-screen PNG (usually in
  `~/Pictures`). screen-writer crops it and deletes the file right away. The
  first capture may show a permission prompt.
- **HiDPI:** with fractional scaling, XWayland windows may look blurry.

## Project layout

```
src/main.rs      CLI and daemon startup
src/app.rs       state, input handling, frame composition
src/shapes.rs    annotation objects: rendering, bounds, hit-testing
src/toolbar.rs   on-screen toolbar and icons
src/xwin.rs      X11 window, input shape, hotkeys, image upload
src/font.rs      font lookup and text-to-path conversion
src/geom.rs      geometry and damage tracking
src/ipc.rs       control socket
src/gnome.rs     GNOME shortcut installer
src/snap.rs      screen capture through the screenshot portal
```
