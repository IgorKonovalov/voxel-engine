#!/usr/bin/env bash
# Records the screen while you drive a windowed run, then encodes it like record-clip.sh: the
# fallback when a scene needs a hand on the controls. Counts down in desktop notifications
# that each expire after 1.5 s, the last of them --quiet seconds before the recording starts
# (so it has expired and faded by the first frame: closing one by D-Bus does not work on every
# daemon), dismisses any others (makoctl, when installed), records a monitor
# with gpu-screen-recorder for the given seconds, stops it with SIGINT, and hands the video to
# scripts/record-clip.sh --input for the GIF or MP4. It never closes, moves or focuses a window:
# start the fly-through yourself and put it where it should be filmed.
#
#   scripts/record-screen.sh [OPTIONS]
#
# Options:
#   --seconds N       how long to record [10]
#   --countdown N     seconds of countdown before recording, the quiet gap included [6]
#   --quiet N         seconds with no notification before recording, over the
#                     notifications' 1.5 s expiry [3]
#   --monitor NAME    the monitor to record [the first gpu-screen-recorder
#                     --list-capture-options names]
#   --capture-fps N   the recording's frame rate [30]
#   --out BASE        the outputs' path without extension [target/clip/screen-<time>]
#   --format, --width, --fps, --crop, --colors, --crf
#                     the encode's, as record-clip.sh takes them (--crop in screen pixels)
#   --keep-raw        keep the recorded BASE-raw.mp4 (removed after the encode otherwise)
#   --dry-run         print the countdown's schedule and the commands; notify, record and
#                     encode nothing
#   -h, --help        print this and exit
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'
}

seconds=10 countdown=6 quiet=3 monitor="" capture_fps=30 out="" keep_raw=0 dry_run=0
# A notification's expiry in ms; --quiet must outlast it.
expiry_ms=1500
encode=()
while (($#)); do
  case "$1" in
    --seconds) seconds="$2"; shift 2 ;;
    --countdown) countdown="$2"; shift 2 ;;
    --quiet) quiet="$2"; shift 2 ;;
    --monitor) monitor="$2"; shift 2 ;;
    --capture-fps) capture_fps="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --format|--width|--fps|--crop|--colors|--crf) encode+=("$1" "$2"); shift 2 ;;
    --keep-raw) keep_raw=1; shift ;;
    --dry-run) dry_run=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "record-screen.sh: unknown option $1 (--help lists them)" >&2; exit 2 ;;
  esac
done
[[ "$countdown" =~ ^[0-9]+$ && "$quiet" =~ ^[0-9]+$ ]] \
  || { echo "record-screen.sh: --countdown and --quiet take whole seconds" >&2; exit 2; }
((quiet * 1000 > expiry_ms)) \
  || { echo "record-screen.sh: --quiet must be over the notifications' ${expiry_ms} ms expiry" >&2; exit 2; }
((countdown >= quiet)) \
  || { echo "record-screen.sh: --countdown must be at least --quiet" >&2; exit 2; }
if ((!dry_run)); then
  command -v gpu-screen-recorder >/dev/null \
    || { echo "record-screen.sh: needs gpu-screen-recorder" >&2; exit 1; }
fi

if [[ -z "$monitor" ]] && ((dry_run)); then
  monitor="<monitor>"
elif [[ -z "$monitor" ]]; then
  # A monitor's line reads NAME|WxH; `region` and `portal` lines have no `|`.
  monitor="$(gpu-screen-recorder --list-capture-options 2>/dev/null | grep '|' | head -n 1 | cut -d'|' -f1 || true)"
  [[ -n "$monitor" ]] || { echo "record-screen.sh: no monitor found; give --monitor" >&2; exit 1; }
fi
out="${out:-$root/target/clip/screen-$(date +%Y%m%d-%H%M%S)}"
raw="$out-raw.mp4"
((dry_run)) || mkdir -p "$(dirname "$out")"

# notify <text>: shows or replaces the countdown's notification, when notify-send exists.
note_id=""
notify() {
  if ((dry_run)); then
    echo "  notify-send -t $expiry_ms: $1"
    return 0
  fi
  command -v notify-send >/dev/null || return 0
  if [[ -n "$note_id" ]]; then
    notify-send -r "$note_id" -t "$expiry_ms" "record-screen" "$1"
  else
    note_id="$(notify-send -p -t "$expiry_ms" "record-screen" "$1")"
  fi
}

# tick <seconds>: waits, or in a dry run only says so.
tick() {
  if ((dry_run)); then echo "  sleep $1"; else sleep "$1"; fi
}

# The notifications count down to T-quiet; the last one says the screen goes quiet. Only the
# terminal counts the quiet seconds, so no notification is on screen at T.
for ((i = countdown; i > quiet; i--)); do
  notify "recording $monitor for $seconds s in $i"
  echo "recording in $i"
  tick 1
done
notify "recording $monitor for $seconds s in $quiet (quiet now)"
for ((i = quiet; i > 0; i--)); do
  echo "recording in $i (no notification)"
  tick 1
done
if ((!dry_run)) && command -v makoctl >/dev/null; then
  makoctl dismiss --all || true
fi

record=(gpu-screen-recorder -w "$monitor" -f "$capture_fps" -c mp4 -cursor no -o "$raw")
if ((dry_run)); then
  echo "would record: ${record[*]} for $seconds s, then"
  echo "would encode: $root/scripts/record-clip.sh --input $raw --out $out ${encode[*]}"
  exit 0
fi
echo "recording $monitor for $seconds s into $raw"
"${record[@]}" &
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
