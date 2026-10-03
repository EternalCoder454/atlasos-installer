#!/usr/bin/env bash
# Clicks in an atlasinst-* VM through QMP, at screen pixels of its display
# (1280x800 unless WIDTH/HEIGHT say otherwise). Works in any session,
# Wayland included, since it is an absolute pointer event at the device.
#
#   tests/vm/click.sh <vm> X Y        (vm as vm.py names it: "ui")
set -euo pipefail

vm=atlasinst-${1:?usage: click.sh <vm> X Y}
w=${WIDTH:-1280} h=${HEIGHT:-800}
x=$(($2 * 32767 / (w - 1))) y=$(($3 * 32767 / (h - 1)))
q() { virsh -c qemu:///system qemu-monitor-command "$vm" "$1" >/dev/null; }
q "{\"execute\":\"input-send-event\",\"arguments\":{\"events\":[{\"type\":\"abs\",\"data\":{\"axis\":\"x\",\"value\":$x}},{\"type\":\"abs\",\"data\":{\"axis\":\"y\",\"value\":$y}}]}}"
sleep 0.2
q '{"execute":"input-send-event","arguments":{"events":[{"type":"btn","data":{"down":true,"button":"left"}}]}}'
sleep 0.1
q '{"execute":"input-send-event","arguments":{"events":[{"type":"btn","data":{"down":false,"button":"left"}}]}}'
