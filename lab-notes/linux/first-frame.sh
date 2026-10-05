#!/bin/bash
bin="$1"; runs="${2:-10}"; label="${3:-run}"
pid_gs=$(pgrep -u "$USER" -x gnome-shell | head -1)
while IFS= read -r -d '' kv; do
  case "$kv" in XDG_RUNTIME_DIR=*|WAYLAND_DISPLAY=*|DISPLAY=*|DBUS_SESSION_BUS_ADDRESS=*|XDG_SESSION_TYPE=*) export "$kv";; esac
done < /proc/$pid_gs/environ
out=$(mktemp -d)
for i in $(seq 1 "$runs"); do
  stats="$out/frames-$i.jsonl"
  start=$(date +%s%N)
  GPUI_FRAME_STATS="$stats" "$bin" >/dev/null 2>&1 &
  pid=$!
  until [ -s "$stats" ] || ! kill -0 $pid 2>/dev/null; do sleep 0.002; done
  first=$(date +%s%N)
  sleep 1
  rss=$(awk '/^VmRSS/{print $2}' /proc/$pid/status 2>/dev/null)
  hwm=$(awk '/^VmHWM/{print $2}' /proc/$pid/status 2>/dev/null)
  lvp=$(grep -c -E 'lvp|lavapipe|LLVM' /proc/$pid/maps 2>/dev/null)
  kill $pid 2>/dev/null; wait $pid 2>/dev/null
  echo "$label $i first_frame_ms=$(( (first - start) / 1000000 )) rss_kb=$rss hwm_kb=$hwm lvp_maps=$lvp"
done
rm -rf "$out"
