#!/usr/bin/env bash
# Records a rig's clip headlessly and encodes it for the web: builds the fly-through's `clip`
# example (release, memory-capped), renders numbered PNG frames with it, and encodes them with
# ffmpeg to a looping GIF (two-pass palette) and/or an MP4 (H.264, yuv420p, faststart).
#
#   scripts/record-clip.sh [OPTIONS] [-- CLIP FLAGS]
#
# Options:
#   --out BASE        the outputs' path without extension [target/clip/<rig>-<clip>]
#   --format F        gif, mp4 or both [gif]
#   --width W         the encoded width in pixels, the height kept in proportion [640]
#   --fps N           frames a second, rendered and encoded [25]
#   --crop W:H:X:Y    crop the frames (in rendered pixels) before scaling
#   --colors N        the GIF palette's colours, 2 to 256 [192]
#   --crf N           the MP4's quality, lower is better [20]
#   --input PATH      encode PATH (a directory of frame-*.png, or a video) instead of rendering
#   --no-build        run the example as already built
#   -h, --help        print this and exit
#
# Everything after `--` goes to the example (`--rig`, `--clip`, `--orbit`, `--size`, `--loops`,
# `--seconds`, `--settle`, `--exposure`, `--adapter` ...; `cargo run --release -p flythrough
# --example clip -- --help` lists them). The example's `--fps` and `--out-dir` are this
# script's. The render runs under the repo's GPU lock (`.git/svet-perf.lock`) when flock is
# installed, and in a systemd scope capped at RECORD_MEMORY_MAX (6G) when systemd-run is.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'
}

out="" format="gif" width=640 fps=25 crop="" colors=192 crf=20 input="" build=1
clip_args=()
while (($#)); do
  case "$1" in
    --out) out="$2"; shift 2 ;;
    --format) format="$2"; shift 2 ;;
    --width) width="$2"; shift 2 ;;
    --fps) fps="$2"; shift 2 ;;
    --crop) crop="$2"; shift 2 ;;
    --colors) colors="$2"; shift 2 ;;
    --crf) crf="$2"; shift 2 ;;
    --input) input="$2"; shift 2 ;;
    --no-build) build=0; shift ;;
    -h|--help) usage; exit 0 ;;
    --) shift; clip_args=("$@"); break ;;
    *) echo "record-clip.sh: unknown option $1 (--help lists them)" >&2; exit 2 ;;
  esac
done
case "$format" in gif|mp4|both) ;; *) echo "--format takes gif, mp4 or both" >&2; exit 2 ;; esac
command -v ffmpeg >/dev/null || { echo "record-clip.sh: needs ffmpeg" >&2; exit 1; }

# The prefix that runs a command in a memory-capped systemd scope, when systemd-run exists.
cap=()
if command -v systemd-run >/dev/null; then
  cap=(systemd-run --user --scope --quiet -p "MemoryMax=${RECORD_MEMORY_MAX:-6G}"
    -p MemorySwapMax=1G --)
fi

# The value the example would take for flag $1 from the pass-through flags, or $2.
clip_flag() {
  local want="$1" fallback="$2" i
  for ((i = 0; i < ${#clip_args[@]}; i++)); do
    if [[ "${clip_args[i]}" == "$want" ]]; then
      echo "${clip_args[i + 1]:-$fallback}"
      return
    fi
  done
  echo "$fallback"
}

if [[ -z "$input" ]]; then
  rig="$(clip_flag --rig apps/flythrough/assets/entities/spider/spider.rig)"
  clip="$(clip_flag --clip "")"
  name="$(basename "$rig" .rig)${clip:+-$clip}"
  out="${out:-$root/target/clip/$name}"
  frames="$root/target/clip/frames/$name"
  if ((build)); then
    "${cap[@]}" cargo build --manifest-path "$root/Cargo.toml" --release -q -p flythrough \
      --example clip
  fi
  bin="${CARGO_TARGET_DIR:-$root/target}/release/examples/clip"
  run=("${cap[@]}" "$bin" "${clip_args[@]}" --fps "$fps" --out-dir "$frames")
  lock=""
  if git_dir="$(git -C "$root" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
  then
    lock="$git_dir/svet-perf.lock"
  fi
  start=$SECONDS
  # The example resolves its default rig against the workspace root.
  if [[ -n "$lock" ]] && command -v flock >/dev/null; then
    (cd "$root" && flock "$lock" "${run[@]}")
  else
    (cd "$root" && "${run[@]}")
  fi
  echo "rendered in $((SECONDS - start)) s"
  source=(-framerate "$fps" -i "$frames/frame-%05d.png")
else
  out="${out:-$root/target/clip/$(basename "${input%.*}")}"
  if [[ -d "$input" ]]; then
    source=(-framerate "$fps" -i "$input/frame-%05d.png")
  else
    source=(-i "$input")
  fi
fi

mkdir -p "$(dirname "$out")"
filters="fps=$fps"
[[ -n "$crop" ]] && filters="$filters,crop=$crop"
written=()
if [[ "$format" != mp4 ]]; then
  # Pass one builds a palette from the frames' changing pixels; pass two maps every frame to
  # it with an ordered dither, redrawing only the rectangle that changed.
  palette="$(mktemp --suffix=.png)"
  trap 'rm -f "$palette"' EXIT
  gif_filters="$filters,scale=$width:-1:flags=lanczos"
  ffmpeg -v error -y "${source[@]}" \
    -vf "$gif_filters,palettegen=max_colors=$colors:stats_mode=diff" "$palette"
  ffmpeg -v error -y "${source[@]}" -i "$palette" -lavfi \
    "$gif_filters[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" \
    -loop 0 "$out.gif"
  written+=("$out.gif")
fi
if [[ "$format" != gif ]]; then
  ffmpeg -v error -y "${source[@]}" \
    -vf "$filters,scale=$width:-2:flags=lanczos" -c:v libx264 -preset slow -crf "$crf" \
    -pix_fmt yuv420p -movflags +faststart -an "$out.mp4"
  written+=("$out.mp4")
fi
for f in "${written[@]}"; do
  echo "wrote $f ($(du -h "$f" | cut -f1))"
done
