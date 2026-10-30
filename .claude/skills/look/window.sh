#!/bin/zsh
# Usage: window.sh [out.png]
# The real app window on macOS: waits until a plainfold-app window is on screen (WAIT seconds, 600 by default, since
# the first `tauri dev` build takes minutes) and prints its id; with a file, captures that window only. Capturing the
# whole screen is not allowed.
set -u
find_window() {
  osascript -l JavaScript -e 'ObjC.import("CoreGraphics");
    var l = ObjC.castRefToObject($.CGWindowListCopyWindowInfo($.kCGWindowListOptionOnScreenOnly, 0)), r = [];
    for (var i = 0; i < l.count; i++) { var w = l.objectAtIndex(i);
      if (w.objectForKey("kCGWindowOwnerName").js == "plainfold-app") r.push(w.objectForKey("kCGWindowNumber").js); }
    r.join(" ")' 2>/dev/null | awk '{print $1}'
}
id=""
for i in {1..${WAIT:-600}}; do
  id=$(find_window)
  [[ -n $id ]] && break
  perl -e 'select(undef, undef, undef, 1)'
done
[[ -n $id ]] || { echo "no plainfold-app window on screen"; exit 1; }
if [[ $# -ge 1 ]]; then
  screencapture -x -o -l $id ${1:A} && echo ${1:A}
else
  echo $id
fi
