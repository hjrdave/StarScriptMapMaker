#!/usr/bin/env bash
# Checks an installed .deb or an AppImage: .ssmap resolves to the app, and launching the
# app with a Map file opens it. Needs xvfb, xdotool and xdg-utils; run under xvfb-run.
#
#   smoke-linux-package.sh <map.ssmap> deb
#   smoke-linux-package.sh <map.ssmap> appimage <path/to.AppImage>
set -euo pipefail

map=$(realpath "$1")
kind=$2

case $kind in
  deb)
    mimetype=$(xdg-mime query filetype "$map")
    [[ $mimetype == application/x-starscript-map ]] || { echo "filetype: $mimetype"; exit 1; }
    handler=$(xdg-mime query default application/x-starscript-map)
    [[ $handler == starscript-map-maker.desktop ]] || { echo "handler: $handler"; exit 1; }
    test -f /usr/lib/starscript-map-maker/assets/README.md
    # What the file manager runs on a double-click: the .desktop Exec line with %F filled in.
    exec_line=$(sed -n 's/^Exec=//p' /usr/share/applications/starscript-map-maker.desktop)
    cmd=(${exec_line% %F} "$map")
    ;;
  appimage)
    export APPIMAGE_EXTRACT_AND_RUN=1
    cmd=("$(realpath "$3")" "$map")
    ;;
  *) echo "unknown kind: $kind"; exit 2 ;;
esac

"${cmd[@]}" &
pid=$!
trap 'kill $pid 2>/dev/null || true' EXIT
# The window title names the open Map file.
name=$(basename "$map")
for _ in $(seq 60); do
  if xdotool search --name "$name" >/dev/null 2>&1; then
    echo "$kind: opened $name"
    exit 0
  fi
  kill -0 $pid 2>/dev/null || { echo "$kind: the app exited"; exit 1; }
  sleep 1
done
echo "$kind: no window titled $name"
exit 1
