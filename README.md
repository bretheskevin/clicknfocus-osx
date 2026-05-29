# hoverfocus-osx

Focus-follows-mouse (hover-to-focus) for macOS, written in Rust.

When you hover the mouse over a window, that window becomes focused — no click
required. This restores the classic "sloppy focus" / focus-follows-mouse behavior
that macOS doesn't provide out of the box.

> Status: early scaffold. See [`docs/kb/specs/2026-05-29-focus-follows-mouse-design.md`](docs/kb/specs/2026-05-29-focus-follows-mouse-design.md)
> for the design.

## How it works

A `CGEventTap` listens for mouse-moved events. When the cursor settles on a new
window (after a short configurable delay), hoverfocus-osx uses the macOS Accessibility
API (`AXUIElementCopyElementAtPosition`) to find the window under the cursor and
makes its application frontmost — focusing it without necessarily raising it above
other windows.

## Requirements

- macOS (Apple Silicon or Intel)
- Rust (stable)
- **Accessibility permission**: System Settings → Privacy & Security → Accessibility →
  enable the `hoverfocus-osx` binary (or your terminal, when running from one).

## Build & run

```sh
cargo build --release
./target/release/hoverfocus-osx --verbose
```

### Planned flags

| Flag                  | Description                                              |
|-----------------------|----------------------------------------------------------|
| `--delay <ms>`        | Settle time before focusing the hovered window           |
| `--raise`             | Also raise the window to the front (AutoRaise behavior)  |
| `--ignore <bundle-id>`| Skip an app by bundle id (repeatable)                    |
| `--verbose`           | Verbose logging                                          |

## Prior art

Inspired by [sbmpost/AutoRaise](https://github.com/sbmpost/AutoRaise).

## License

MIT
