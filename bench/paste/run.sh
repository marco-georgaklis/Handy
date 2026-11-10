#!/bin/bash
# Paste correctness benchmark.
#
# For each round: put a sentinel on the clipboard, focus a text field in the
# target app, ask the running app to paste a unique string (--paste-text, the
# same code path a finished dictation takes), then check two things:
#   1. the target received exactly that string (not the sentinel, not nothing)
#   2. the clipboard is back to the sentinel afterwards
#
#   bench/paste/run.sh [rounds] [app ...]      # default: 20 rounds, TextEdit Notes
#
# Needs: the fork build running with Accessibility permission, and Automation
# permission for the terminal to drive each target app (macOS asks on first
# run). It types into a new TextEdit document / a new note; nothing else is
# touched. Targets without AppleScript text access (Slack, Chrome, VS Code)
# are checked by selecting all and copying, which is why they are opt-in.
set -u

HANDY="${HANDY_BIN:-/Applications/Handy Flow.app/Contents/MacOS/handy}"
ROUNDS="${1:-20}"
shift || true
APPS=("$@")
[ ${#APPS[@]} -eq 0 ] && APPS=(TextEdit Notes)
SETTLE="${SETTLE_SECONDS:-1.2}"

prepare() { # focus an empty text field in app $1
  case "$1" in
    TextEdit)
      osascript -e 'tell application "TextEdit" to activate' \
                -e 'tell application "TextEdit" to make new document' >/dev/null ;;
    Notes)
      osascript -e 'tell application "Notes" to activate' \
                -e 'tell application "System Events" to keystroke "n" using command down' >/dev/null ;;
    *)
      osascript -e "tell application \"$1\" to activate" >/dev/null
      echo "  focus an empty text field in $1 now (5s)"; sleep 5 ;;
  esac
  sleep 1
}

read_field() { # print what the focused field of app $1 contains
  case "$1" in
    TextEdit) osascript -e 'tell application "TextEdit" to get text of front document' ;;
    *)
      # Generic: select all + copy, read, then restore the sentinel.
      osascript -e 'tell application "System Events" to keystroke "a" using command down' \
                -e 'delay 0.15' \
                -e 'tell application "System Events" to keystroke "c" using command down' \
                -e 'delay 0.25' >/dev/null
      pbpaste ;;
  esac
}

clear_field() {
  osascript -e 'tell application "System Events" to keystroke "a" using command down' \
            -e 'delay 0.1' \
            -e 'tell application "System Events" to key code 51' >/dev/null
  sleep 0.2
}

total=0; ok_text=0; ok_clip=0
for app in "${APPS[@]}"; do
  prepare "$app"
  app_text=0; app_clip=0
  for i in $(seq 1 "$ROUNDS"); do
    sentinel="SENTINEL-$app-$i-$RANDOM"
    payload="dictation $i for $app: the quick brown fox, 1. one 2. two [$RANDOM]"
    clear_field
    printf '%s' "$sentinel" | pbcopy
    "$HANDY" --paste-text "$payload"
    sleep "$SETTLE"
    clip_after="$(pbpaste)"
    got="$(read_field "$app")"
    printf '%s' "$sentinel" | pbcopy
    total=$((total + 1))
    if [ "$got" = "$payload" ]; then app_text=$((app_text + 1)); ok_text=$((ok_text + 1));
    else echo "  [$app #$i] wrong text: got '${got:0:80}'"; fi
    if [ "$clip_after" = "$sentinel" ]; then app_clip=$((app_clip + 1)); ok_clip=$((ok_clip + 1));
    else echo "  [$app #$i] clipboard not restored: '${clip_after:0:60}'"; fi
  done
  echo "$app: text correct $app_text/$ROUNDS, clipboard restored $app_clip/$ROUNDS"
done
echo "TOTAL: text correct $ok_text/$total, clipboard restored $ok_clip/$total"
[ "$ok_text" -eq "$total" ] && [ "$ok_clip" -eq "$total" ]
