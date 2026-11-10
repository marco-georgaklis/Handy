#!/bin/bash
# Install or update Handy Flow from the fork's latest GitHub release.
#
#   scripts/flow-install.sh             # install the latest release if newer
#   scripts/flow-install.sh --force     # reinstall even if already current
#   scripts/flow-install.sh --schedule  # also check for updates daily at 04:15
#   scripts/flow-install.sh --unschedule
#
# On a Mac without the repository:
#   curl -fsSL https://raw.githubusercontent.com/marco-georgaklis/Handy/flow-parity/scripts/flow-install.sh | bash
#
# First install on a Mac: afterwards turn on Handy Flow under System Settings >
# Privacy & Security > Accessibility, allow the microphone when asked, and add
# the post-processing API key in the app's settings. Updates keep all three.
set -euo pipefail

REPO="${HANDYFLOW_REPO:-marco-georgaklis/Handy}"
TEAM_ID="8DWJ4CD6QW" # releases must be signed by this team, or they are refused
APP="/Applications/Handy Flow.app"
DATA="$HOME/Library/Application Support/com.handyflow.app"
MARKER="$DATA/.installed-release"
AGENT="$HOME/Library/LaunchAgents/com.handyflow.update.plist"
SELF_URL="https://raw.githubusercontent.com/$REPO/flow-parity/scripts/flow-install.sh"

case "${1:-}" in
  --schedule)
    mkdir -p "$DATA" "$(dirname "$AGENT")"
    curl -fsSL "$SELF_URL" -o "$DATA/flow-install.sh"
    chmod +x "$DATA/flow-install.sh"
    cat > "$AGENT" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>com.handyflow.update</string>
  <key>ProgramArguments</key><array><string>/bin/bash</string><string>$DATA/flow-install.sh</string></array>
  <key>StartCalendarInterval</key><dict><key>Hour</key><integer>4</integer><key>Minute</key><integer>15</integer></dict>
  <key>StandardOutPath</key><string>$HOME/Library/Logs/com.handyflow.app/update.log</string>
  <key>StandardErrorPath</key><string>$HOME/Library/Logs/com.handyflow.app/update.log</string>
</dict></plist>
PLIST
    mkdir -p "$HOME/Library/Logs/com.handyflow.app"
    launchctl bootout "gui/$(id -u)" "$AGENT" 2>/dev/null || true
    launchctl bootstrap "gui/$(id -u)" "$AGENT"
    echo "Handy Flow will check for updates daily at 04:15."
    ;;
  --unschedule)
    launchctl bootout "gui/$(id -u)" "$AGENT" 2>/dev/null || true
    rm -f "$AGENT" "$DATA/flow-install.sh"
    echo "Daily update check removed."
    exit 0
    ;;
esac

release="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest")"
tag="$(printf '%s' "$release" | /usr/bin/python3 -c 'import json,sys; print(json.load(sys.stdin)["tag_name"])')"
url="$(printf '%s' "$release" | /usr/bin/python3 -c '
import json, sys
assets = [a["browser_download_url"] for a in json.load(sys.stdin)["assets"] if a["name"] == "HandyFlow.zip"]
print(assets[0] if assets else "")')"
[ -n "$url" ] || { echo "Release $tag has no HandyFlow.zip" >&2; exit 1; }

if [ "${1:-}" != "--force" ] && [ -d "$APP" ] && [ "$(cat "$MARKER" 2>/dev/null)" = "$tag" ]; then
  echo "Handy Flow is up to date ($tag)."
  exit 0
fi

mkdir -p "$HOME/Library/Caches/com.handyflow.app"
work="$(mktemp -d "$HOME/Library/Caches/com.handyflow.app/update.XXXXXX")"
trap 'rm -rf "$work"' EXIT
curl -fsSL "$url" -o "$work/HandyFlow.zip"
ditto -x -k "$work/HandyFlow.zip" "$work"
new="$work/Handy Flow.app"

codesign --verify --deep --strict "$new"
team="$(codesign -dv "$new" 2>&1 | sed -n 's/^TeamIdentifier=//p')"
if [ "$team" != "$TEAM_ID" ]; then
  echo "Refusing $tag: signed by '$team', expected $TEAM_ID." >&2
  exit 1
fi

first_install=1
[ -d "$APP" ] && first_install=0
osascript -e 'tell application id "com.handyflow.app" to quit' >/dev/null 2>&1 || true
sleep 2
pkill -f "$APP/Contents/MacOS/handy" 2>/dev/null || true
rm -rf "$APP"
ditto "$new" "$APP"
mkdir -p "$DATA"
printf '%s' "$tag" > "$MARKER"
open "$APP" --args --start-hidden
echo "Installed Handy Flow $tag."
if [ "$first_install" = 1 ]; then
  echo "First install: turn on Handy Flow in System Settings > Privacy & Security > Accessibility,"
  echo "allow the microphone when asked, and add your API key under Post Processing in its settings."
fi
