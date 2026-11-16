#!/usr/bin/env bash
# Records the screen while you drive a windowed run, then encodes it like record-clip.sh: the
# fallback when a scene needs a hand on the controls. Counts down in a desktop notification,
# dismisses the notifications (makoctl, when installed) so none is captured, records a monitor
# with gpu-screen-recorder for the given seconds, stops it with SIGINT, and hands the video to
# scripts/record-clip.sh --input for the GIF or MP4. It never closes, moves or focuses a window:
# start the fly-through yourself and put it where it should be filmed.
#
#   scripts/record-screen.sh [OPTIONS]
#
# Options:
#   --seconds N       how long to record [10]
#   --countdown N     seconds of countdown before recording [5]
#   --monitor NAME    the monitor to record [the first gpu-screen-recorder
#                     --list-capture-options names]
#   --capture-fps N   the recording's frame rate [30]
#   --out BASE        the outputs' path without extension [target/clip/screen-<time>]
#   --format, --width, --fps, --crop, --colors, --crf
#                     the encode's, as record-clip.sh takes them (--crop in screen pixels)
#   --keep-raw        keep the recorded BASE-raw.mp4 (removed after the encode otherwise)
#   -h, --help        print this and exit
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'
}

seconds=10 countdown=5 monitor="" capture_fps=30 out="" keep_raw=0
encode=()
while (($#)); do
  case "$1" in
    --seconds) seconds="$2"; shift 2 ;;
    --countdown) countdown="$2"; shift 2 ;;
    --monitor) monitor="$2"; shift 2 ;;
    --capture-fps) capture_fps="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --format|--width|--fps|--crop|--colors|--crf) encode+=("$1" "$2"); shift 2 ;;
    --keep-raw) keep_raw=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "record-screen.sh: unknown option $1 (--help lists them)" >&2; exit 2 ;;
  esac
done
command -v gpu-screen-recorder >/dev/null \
  || { echo "record-screen.sh: needs gpu-screen-recorder" >&2; exit 1; }

if [[ -z "$monitor" ]]; then
  # A monitor's line reads NAME|WxH; `region` and `portal` lines have no `|`.
  monitor="$(gpu-screen-recorder --list-capture-options 2>/dev/null | grep '|' | head -n 1 | cut -d'|' -f1 || true)"
  [[ -n "$monitor" ]] || { echo "record-screen.sh: no monitor found; give --monitor" >&2; exit 1; }
fi
out="${out:-$root/target/clip/screen-$(date +%Y%m%d-%H%M%S)}"
raw="$out-raw.mp4"
mkdir -p "$(dirname "$out")"

# notify <text>: shows or replaces the countdown's notification, when notify-send exists.
note_id=""
notify() {
  command -v notify-send >/dev/null || return 0
  if [[ -n "$note_id" ]]; then
    notify-send -r "$note_id" -t 1500 "record-screen" "$1"
  else
    note_id="$(notify-send -p -t 1500 "record-screen" "$1")"
  fi
}

for ((i = countdown; i > 0; i--)); do
  notify "recording $monitor for $seconds s in $i"
  echo "recording in $i"
  sleep 1
done
if command -v makoctl >/dev/null; then
  makoctl dismiss --all || true
fi
# The last notification must be gone before the first frame.
sleep 0.3

echo "recording $monitor for $seconds s into $raw"
gpu-screen-recorder -w "$monitor" -f "$capture_fps" -c mp4 -cursor no -o "$raw" &
recorder=$!
trap 'kill -INT "$recorder" 2>/dev/null || true' EXIT
sleep "$seconds"
kill -INT "$recorder"
wait "$recorder" || true
trap - EXIT
notify "recorded $seconds s"

"$root/scripts/record-clip.sh" --input "$raw" --out "$out" "${encode[@]}"
if ((keep_raw)); then
  echo "kept $raw ($(du -h "$raw" | cut -f1))"
else
  rm -f "$raw"
fi
