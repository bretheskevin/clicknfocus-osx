# clicknfocus-osx

Eager click-to-focus for macOS, written in Rust.

When you click on a window, clicknfocus-osx activates it **synchronously
before the click event reaches the target application**. This means
a single click both focuses the window _and_ acts on the control
under the cursor — instead of the macOS default where the first click
on a background window is consumed just to bring it to the front.

> **Note:** Whether the first click also reaches the target control
> depends on each app's `acceptsFirstMouse:` behavior. Some Cocoa
> apps always accept the first mouse event; others may still swallow it
> regardless of external activation.

## How it works

A `CGEventTap` (HID-level, head-insert, listen-only) intercepts
`leftMouseDown`, `rightMouseDown`, and `otherMouseDown` events. On each
mouse-down the callback:

1. Resolves the window under the click via the macOS Accessibility API
   (`AXUIElementCopyElementAtPosition`).
2. Skips activation if the window belongs to the Dock, menu bar, system
   menus, the tool's own process, or a user-ignored bundle ID.
3. Skips activation if the same window is already focused (dedup).
4. Makes the owning application frontmost via the Accessibility API
   (`AXUIElementSetAttributeValue` with `kAXFrontmostAttribute`).

Because the tap fires before the event propagates, the target app is
already frontmost by the time it receives the click. There is **no settle
delay** — activation is synchronous in the tap callback.

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

## Prior art

Inspired by [sbmpost/AutoRaise](https://github.com/sbmpost/AutoRaise).

## License

MIT
