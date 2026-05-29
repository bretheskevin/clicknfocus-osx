# clicknfocus-osx

Eager click-to-focus for macOS, written in Rust.

Clicking a background window normally takes two clicks: one to focus it,
one to act. ClicknFocus activates the window **before the click reaches the
app**, so a single click both focuses *and* hits the control under the cursor.

## Install

macOS 11+ (Intel or Apple Silicon).

### Download (no Rust needed)

1. From the [latest release](https://github.com/bretheskevin/clicknfocus-osx/releases/latest),
   download the zip for your Mac and unzip it:
   - **Apple Silicon** (M1/M2/M3…): `clicknfocus-macos-apple-silicon.zip`
   - **Intel**: `clicknfocus-macos-intel.zip`
2. Run the installer:
   ```sh
   ./install.sh
   ```

### Build from source

Requires [Rust](https://rustup.rs) (stable) and `git`.

```sh
git clone https://github.com/bretheskevin/clicknfocus-osx.git
cd clicknfocus-osx
./build-app.sh      # builds the universal app bundle into dist/
./dist/install.sh   # installs, enables auto-start, opens Accessibility settings
```

Either way, `install.sh` copies the app to `/Applications`, sets up a LaunchAgent
for auto-start at login, and opens Accessibility settings.

**You must enable ClicknFocus** in System Settings → Privacy & Security →
Accessibility — without it the event tap can't intercept clicks. The installer
restarts the agent for you once you've toggled it on.

It runs in the background with no Dock or menu bar icon. Check on it with:

```sh
launchctl list | grep clicknfocus
cat ~/Library/Logs/clicknfocus.log   # should show "Accessibility permission granted"
```

## Update / Uninstall

To update, grab the new zip from
[Releases](https://github.com/bretheskevin/clicknfocus-osx/releases/latest) and
re-run `./install.sh` — or from source: `git pull && ./build-app.sh && ./dist/install.sh`.

To uninstall, run `./uninstall.sh`.

If click-to-focus stops working after an update, toggle ClicknFocus off and
back on in the Accessibility list — ad-hoc signatures change between builds.

## How it works

An active `CGEventTap` intercepts each mouse-down. If the target window is in
the background, ClicknFocus **swallows** the original click, makes the app
frontmost, then immediately re-posts a synthetic click at the same spot.
Clicks on the frontmost app, the Dock, or the menu bar pass through untouched.

This defeats macOS's `acceptsFirstMouse:` behaviour, where the first click on
an inactive window is consumed just to raise it. A few apps that validate the
event source may still ignore the synthetic click.

## License

MIT
