#!/usr/bin/env bash
set -u
W=$1
H=$2
tag=$3
bin=${CHAT_DEMO_BIN:-target/debug/examples/chat_demo}
rt=$(mktemp -d /tmp/agent-wl.XXXX)
chmod 700 "$rt"
printf 'xwayland disable\noutput HEADLESS-1 resolution %sx%s\ndefault_border none\n' "$W" "$H" > "$rt/config"
E="env -i HOME=$HOME PATH=$PATH XDG_RUNTIME_DIR=$rt"
$E WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 sway -c "$rt/config" > "$rt/sway.log" 2>&1 &
sp=$!
sleep 2
$E WAYLAND_DISPLAY=wayland-1 WGPU_BACKEND=vulkan "$bin" > "$rt/app.log" 2>&1 &
ap=$!
start=$(date +%s)
for t in 6 12 24; do
  while [ $(( $(date +%s) - start )) -lt "$t" ]; do sleep 0.5; done
  $E WAYLAND_DISPLAY=wayland-1 grim "/tmp/mb1138-$tag-$t.png"
done
kill "$ap" "$sp"
cp "$rt/app.log" "/tmp/mb1138-$tag.log"
cp "$rt/sway.log" "/tmp/mb1138-$tag-sway.log"
$E WAYLAND_DISPLAY=wayland-1 swaymsg -t get_tree > "/tmp/mb1138-$tag-tree.json" 2>&1
rm -rf "$rt"
