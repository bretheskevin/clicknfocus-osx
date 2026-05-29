#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
LABEL="com.bretheskevin.clicknfocus"
PLIST_NAME="${LABEL}.plist"
LAUNCH_AGENT_DIR="$HOME/Library/LaunchAgents"
LAUNCH_AGENT_PLIST="$LAUNCH_AGENT_DIR/$PLIST_NAME"
LOG_DIR="$HOME/Library/Logs"
LOG_FILE="$LOG_DIR/clicknfocus.log"

# ── Locate ClicknFocus.app ──────────────────────────────────────────
APP_SOURCE=""
for candidate in "$SCRIPT_DIR/ClicknFocus.app" "$PWD/ClicknFocus.app"; do
  if [ -d "$candidate" ]; then
    APP_SOURCE="$candidate"
    break
  fi
done

if [ -z "$APP_SOURCE" ]; then
  echo "Error: ClicknFocus.app not found next to this script or in the current directory."
  echo "Make sure ClicknFocus.app is in the same folder as install.sh."
  exit 1
fi

echo "==> Found app bundle at: $APP_SOURCE"

# ── Remove quarantine attribute ─────────────────────────────────────
echo "==> Removing quarantine attribute..."
xattr -dr com.apple.quarantine "$APP_SOURCE" 2>/dev/null || true

# ── Copy to Applications ────────────────────────────────────────────
INSTALL_DIR=""
if [ -w "/Applications" ]; then
  INSTALL_DIR="/Applications"
elif mkdir -p "$HOME/Applications" 2>/dev/null; then
  INSTALL_DIR="$HOME/Applications"
else
  echo "Error: Cannot write to /Applications or ~/Applications."
  exit 1
fi

echo "==> Installing to $INSTALL_DIR/ClicknFocus.app..."
rm -rf "$INSTALL_DIR/ClicknFocus.app"
cp -R "$APP_SOURCE" "$INSTALL_DIR/ClicknFocus.app"

INSTALLED_APP="$INSTALL_DIR/ClicknFocus.app"

# ── Generate and install LaunchAgent ─────────────────────────────────
echo "==> Setting up LaunchAgent for auto-start at login..."
mkdir -p "$LAUNCH_AGENT_DIR"
mkdir -p "$LOG_DIR"

# Keep this LaunchAgent in sync with the reference template at
# packaging/com.bretheskevin.clicknfocus.plist (that file is documentation only;
# the real plist is generated here with paths substituted).
cat > "$LAUNCH_AGENT_PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>${LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>${INSTALLED_APP}/Contents/MacOS/clicknfocus-osx</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<true/>
	<key>StandardOutPath</key>
	<string>${LOG_FILE}</string>
	<key>StandardErrorPath</key>
	<string>${LOG_FILE}</string>
	<key>ProcessType</key>
	<string>Interactive</string>
</dict>
</plist>
PLIST

# ── Load the LaunchAgent ────────────────────────────────────────────
echo "==> Loading LaunchAgent..."
GUI_DOMAIN="gui/$(id -u)"
# Modern launchctl (bootstrap/bootout); fall back to legacy load/unload on
# older macOS where the new subcommands are unavailable.
launchctl bootout "$GUI_DOMAIN/$LABEL" 2>/dev/null \
  || launchctl unload -w "$LAUNCH_AGENT_PLIST" 2>/dev/null || true
if ! launchctl bootstrap "$GUI_DOMAIN" "$LAUNCH_AGENT_PLIST" 2>/dev/null; then
  launchctl load -w "$LAUNCH_AGENT_PLIST"
fi

# ── Open Accessibility settings ─────────────────────────────────────
echo "==> Opening Accessibility settings..."
open "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"

echo ""
echo "============================================================"
echo "  IMPORTANT: Grant Accessibility permission!"
echo "============================================================"
echo ""
echo "  In the System Settings window that just opened:"
echo "  1. Find 'ClicknFocus' in the Accessibility list"
echo "  2. Toggle it ON"
echo ""
echo "  If ClicknFocus is not in the list, click '+' and add:"
echo "    $INSTALLED_APP"
echo ""
echo "  Once enabled, ClicknFocus will start automatically."
echo "  (launchd KeepAlive will restart it if it crashes.)"
echo ""
echo "============================================================"
echo "  Useful commands:"
echo "============================================================"
echo "  Check if running:  launchctl list | grep clicknfocus"
echo "  View logs:         cat $LOG_FILE"
echo "  Follow logs:       tail -f $LOG_FILE"
echo "============================================================"
