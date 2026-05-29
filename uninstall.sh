#!/usr/bin/env bash
set -euo pipefail

LABEL="com.bretheskevin.clicknfocus"
PLIST_NAME="${LABEL}.plist"
LAUNCH_AGENT_PLIST="$HOME/Library/LaunchAgents/$PLIST_NAME"

# ── Unload LaunchAgent ──────────────────────────────────────────────
echo "==> Unloading LaunchAgent..."
if [ -f "$LAUNCH_AGENT_PLIST" ]; then
  # Modern launchctl (bootout); fall back to legacy unload on older macOS.
  launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null \
    || launchctl unload -w "$LAUNCH_AGENT_PLIST" 2>/dev/null || true
  rm -f "$LAUNCH_AGENT_PLIST"
  echo "    Removed $LAUNCH_AGENT_PLIST"
else
  echo "    LaunchAgent plist not found (already removed?)."
fi

# ── Remove installed app ────────────────────────────────────────────
REMOVED=false
for app_dir in "/Applications/ClicknFocus.app" "$HOME/Applications/ClicknFocus.app"; do
  if [ -d "$app_dir" ]; then
    echo "==> Removing $app_dir..."
    rm -rf "$app_dir"
    REMOVED=true
  fi
done

if [ "$REMOVED" = false ]; then
  echo "==> ClicknFocus.app not found in /Applications or ~/Applications."
fi

echo ""
echo "============================================================"
echo "  ClicknFocus has been uninstalled."
echo "============================================================"
echo ""
echo "  You may also want to:"
echo "  - Remove ClicknFocus from the Accessibility list:"
echo "    System Settings > Privacy & Security > Accessibility"
echo "  - Delete logs: rm ~/Library/Logs/clicknfocus.log"
echo "============================================================"
