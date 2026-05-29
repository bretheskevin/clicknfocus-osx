# clicknfocus-osx

Eager click-to-focus for macOS, written in Rust.

When you click on a window, clicknfocus-osx activates it **synchronously
before the click event reaches the target application**. This means
a single click both focuses the window _and_ acts on the control
under the cursor — instead of the macOS default where the first click
on a background window is consumed just to bring it to the front.

> **Note:** The tool uses an active event tap that swallows the original
> click and re-posts a synthetic one after activation. This defeats the
> macOS `acceptsFirstMouse:` behaviour for most apps. In rare cases, an
> app may still not respond to the synthetic event (e.g. apps that
> validate the event source or apply custom first-responder logic).

## How it works

A `CGEventTap` (HID-level, head-insert, **active**) intercepts
`leftMouseDown`, `rightMouseDown`, and `otherMouseDown` events. On each
mouse-down the callback:

1. Checks if the event is a **synthetic re-post** (tagged with a magic
   value in `EVENT_SOURCE_USER_DATA`) — if so, lets it pass through
   unchanged. This prevents infinite loops.
2. Resolves the window under the click via the macOS Accessibility API
   (`AXUIElementCopyElementAtPosition`).
3. Skips redirect if the window belongs to the Dock, menu bar, system
   menus, the tool's own process, or a user-ignored bundle ID.
4. Skips redirect if the same window is already focused (dedup).
5. Skips redirect if the target app is **already frontmost** — the click
   is a normal in-app click and flows through untouched.
6. **Redirects** the click (target app is in the background):
   - **Drops** (swallows) the original mouse-down event.
   - Makes the owning application frontmost via the Accessibility API
     (`AXUIElementSetAttributeValue` with `kAXFrontmostAttribute`).
   - **Immediately** synthesizes and re-posts a new mouse-down event at
     the same screen position, tagged with the magic constant so the tap
     recognises it on the next pass.

The active tap + swallow/re-post strategy defeats macOS's
`acceptsFirstMouse:` behaviour, where the first click on an inactive
window is consumed just to bring it to the front. By dropping the
original (consumed-by-activation) click and injecting a fresh one after
the app is already frontmost, the control under the cursor receives the
click. There is **no settle delay** — the synthetic click is posted
immediately after activation.

Mouse-up events are **not** intercepted — the hardware mouse-up
naturally pairs with the synthetic mouse-down.

## Requirements

- macOS (Apple Silicon or Intel)
- Rust (stable)
- **Accessibility permission**: System Settings → Privacy & Security → Accessibility →
  enable the `clicknfocus-osx` binary (or your terminal, when running from one).

## Build & run

```sh
cargo build --release
./target/release/clicknfocus-osx --verbose
```

### Flags

| Flag                   | Description                                             |
|------------------------|---------------------------------------------------------|
| `--raise`              | Also raise the window to the front (opt-in)             |
| `--ignore <bundle-id>` | Skip an app by bundle id (repeatable)                   |
| `--verbose`            | Verbose logging                                         |

## Distribution (for friends without Rust)

ClicknFocus can be packaged as a standalone `.app` bundle that auto-starts
at login via a launchd LaunchAgent. No paid Apple Developer account needed —
the app is **ad-hoc signed** (not notarized).

### Build the distributable (author only)

```sh
./build-app.sh
```

This produces:
- `dist/ClicknFocus.app` — the signed universal (Intel + Apple Silicon) app bundle
- `dist/clicknfocus-macos.zip` — zip file to share

Share `clicknfocus-macos.zip` — it contains the app, `install.sh`, and `uninstall.sh`.

### Install (friend)

1. Unzip `clicknfocus-macos.zip`.
2. In the unzipped folder, run `./install.sh`.

The install script will:
- Remove the quarantine attribute (Gatekeeper).
- Copy the app to `/Applications` (or `~/Applications` as fallback).
- Set up a LaunchAgent so ClicknFocus starts automatically at login.
- Open Accessibility settings — **you must enable ClicknFocus** in the list.

> **Note:** Because the app is ad-hoc signed and not notarized by Apple,
> macOS may block it on first run. The install script handles quarantine
> removal. If you install manually instead, right-click the app → Open
> to bypass Gatekeeper the first time.

### Grant Accessibility permission

**Required.** Without this, the event tap cannot intercept clicks.

System Settings → Privacy & Security → Accessibility → enable **ClicknFocus**.

### Check it's running

```sh
launchctl list | grep clicknfocus
cat ~/Library/Logs/clicknfocus.log
```

### Uninstall

```sh
./uninstall.sh
```

This unloads the LaunchAgent, removes the plist, and deletes the app from
`/Applications` (or `~/Applications`). You may also want to remove
ClicknFocus from the Accessibility list manually.

## Prior art

Inspired by [sbmpost/AutoRaise](https://github.com/sbmpost/AutoRaise).

## License

MIT
