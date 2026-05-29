# HoverFocus — Focus-Follows-Mouse for macOS (Design)

Date: 2026-05-29
Scope: Medium (single Rust crate, but touches event taps, Accessibility API, run loop, packaging, permissions)

## Goal

A lightweight Rust CLI/daemon for macOS that focuses the window under the mouse cursor
on hover, removing the need to click into a window before interacting with it
(sloppy-focus / focus-follows-mouse).

## Decisions (made by lead; user can redirect)

Because the interactive question tool was unavailable, these were chosen using the
skill's decision principles (cleanest solution, follow prior art, YAGNI):

1. **Focus vs. raise** — default to **focus only** (do not bring the window to the
   front / above others). This is true "focus follows mouse" and is the less
   intrusive behavior. A `--raise` flag enables auto-raise (AutoRaise behavior).
   Rationale: focus-only is the literal request ("becomes focused"); raising is opt-in.
2. **Trigger mechanism** — **CGEventTap on `mouseMoved`** (event-driven) rather than a
   polling loop. Cleaner, lower idle CPU. A configurable settle **delay** debounces
   so we only act once the cursor rests on a new window.
3. **Window resolution** — `AXUIElementCopyElementAtPosition` on the system-wide
   element to find the element under the cursor, walk up to its window/app.
4. **Activation** — set the app as frontmost via
   `AXUIElementSetAttributeValue(app, kAXFrontmostAttribute, true)` and set the
   window's `kAXMainAttribute`/focused, WITHOUT calling `AXRaise` unless `--raise`.
5. **Packaging** — plain `cargo build --release` binary runnable from terminal /
   launchd. No `.app` bundle, no notarization in v1 (YAGNI). Document the
   Accessibility permission requirement.
6. **Config** — CLI flags only in v1 (`--delay`, `--raise`, `--ignore <bundle-id>`,
   `--verbose`). No config file yet.

## Architecture

```
main
 ├── cli            (clap: --delay, --raise, --ignore, --verbose)
 ├── permissions    (check AXIsProcessTrustedWithOptions; prompt + exit if denied)
 ├── event_tap      (CGEventTap on mouseMoved -> CFRunLoop source)
 │      callback ── debounce(delay) ──> focus::handle(cursor_point)
 └── focus
        ├── element_at_position(point) -> AXUIElement   (system-wide AX)
        ├── window_and_app(element)     -> (window, app, pid, bundle_id)
        ├── skip if same as currently focused / in ignore list / is our own / is Dock/menubar
        └── activate(app, window, raise)
```

Data flow: OS mouse move → CGEventTap callback (must return fast, just records
point + arms timer) → CFRunLoop timer fires after `delay` of no movement →
resolve window under point → if it's a new, eligible window → activate it.

### Key crates (prior-art aligned)
- `core-graphics` — `CGEventTap`, `CGEventType::MouseMoved`, event location.
- `core-foundation` — `CFRunLoop`, run loop source, timers.
- `accessibility-sys` (+ `accessibility` safe wrapper if it fits) — `AXUIElementCopyElementAtPosition`, `AXUIElementSetAttributeValue`, `AXIsProcessTrustedWithOptions`, attribute constants.
- `clap` — CLI.
- `log` + `env_logger` — `--verbose` logging.

## Edge cases / rules
- Ignore moves where the window under cursor is already the focused window (no-op).
- Ignore the menu bar, Dock, Desktop, and our own process.
- `--ignore <bundle-id>` (repeatable) to exclude apps.
- Apps that don't expose AX (return invalid element): skip gracefully, log in verbose.
- Multi-display: cursor point is global; AX position API is display-agnostic — OK.
- Fullscreen / Spaces: only act on windows on the active Space (AX returns those).
- Permission denied at runtime → print clear instructions, exit non-zero.

## Non-goals (v1)
- No GUI, menu-bar icon, or preferences pane.
- No `.app` bundle / notarization / signing distribution.
- No per-window (sub-window) focus chasing inside an app.
- No mouse-warp / focus-on-click-through behaviors.
- No config file / hot reload.

## Testing
- Unit-test pure logic: debounce timing, ignore-list matching, "same window" dedupe,
  window/app resolution given a mock element accessor (trait-abstract the AX calls so
  the resolution + decision logic is testable without a live desktop).
- Manual smoke test documented in README: build, grant Accessibility, run with
  `--verbose`, hover across two windows, confirm focus switches.
- CI: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` on macos runner.

## Pattern References
This is greenfield; references are external canonical implementations to mirror:
- **sbmpost/AutoRaise** (C++, GPL-3.0) — canonical FFM algorithm: element-at-position,
  app frontmost, optional AXRaise, settle delay. Mirror the algorithm, not the code.
- **core-graphics crate `event.rs`** (docs.rs) — correct `CGEventTap` + CFRunLoop
  source wiring; mirror the tap setup and run-loop integration.
- **accessibility-sys docs** — exact extern signatures and attribute constants
  (`kAXFrontmostAttribute`, `kAXMainAttribute`, `AXUIElementCopyElementAtPosition`).

## Repo
- Local: /Users/k.brethes/Documents/projects/hoverfocus
- GitHub: bretheskevin/hoverfocus (created via gh, private by default unless told otherwise)
