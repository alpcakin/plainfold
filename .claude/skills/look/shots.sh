#!/bin/zsh
# Usage: shots.sh <model.json> <out dir> [key=value …]
# Screenshots of the app's browser preview in the four looks: macOS and Windows, light and dark.
# Needs the preview running (`npm run dev` in app/, port 1420). Each value is URL-encoded here, so a story id is
# passed as written: story='action:/login#submit'. LOOKS="mac-dark windows-light" takes only those; W and H set the
# window size (1440×900).
set -u
repo=${0:A:h:h:h:h}
[[ $# -ge 2 ]] || { echo "usage: shots.sh <model.json> <out dir> [key=value …]"; exit 2; }
model=${1:A} out=${2:A}
shift 2
[[ -f $model ]] || { echo "no model at $model"; exit 1; }
[[ $model == $repo/* ]] || { echo "the preview serves only files under $repo: put the model in out/"; exit 1; }
curl -s -o /dev/null --max-time 3 http://localhost:1420/ || { echo "the preview is not running: start it with (cd app && npm run dev)"; exit 1; }

chrome="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
enc() { python3 -I -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1], safe="/@:,"))' "$1" }
query="preview=/@fs$(enc $model)"
for kv in "$@"; do query+="&${kv%%=*}=$(enc ${kv#*=})"; done

mkdir -p $out
for look in ${=${LOOKS:-mac-light mac-dark windows-light windows-dark}}; do
  platform=${look%-*} scheme=${look#*-}
  [[ $scheme == light ]] && pref=1 || pref=0
  png=$out/$look.png profile=${TMPDIR:-/tmp}/plainfold-look-$look
  rm -f $png
  # headless Chrome often stays open after writing the screenshot: a profile of its own per look, stopped once the
  # file is written (or after 45 s)
  "$chrome" --headless=new --disable-gpu --hide-scrollbars \
    --user-data-dir=$profile --window-size=${W:-1440},${H:-900} --virtual-time-budget=6000 \
    --blink-settings=preferredColorScheme=$pref --screenshot=$png \
    "http://localhost:1420/?$query&platform=$platform" >/dev/null 2>&1 &
  for i in {1..90}; do
    [[ -s $png ]] && { perl -e 'select(undef, undef, undef, 0.5)'; break; }
    perl -e 'select(undef, undef, undef, 0.5)'
  done
  pkill -f "plainfold-look-$look" 2>/dev/null
  wait 2>/dev/null
  [[ -s $png ]] && echo $png || echo "no screenshot for $look"
done
