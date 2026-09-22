#!/usr/bin/env bash
# Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
# Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
# scripts/relay-diag.sh — watch a transfer through the relay across every
# attached device, side by side.
#
# Usage:
#   scripts/relay-diag.sh              # snapshot all devices once
#   scripts/relay-diag.sh watch        # repeat every 2s until Ctrl-C
#   scripts/relay-diag.sh reset        # zero counters on all devices
#   scripts/relay-diag.sh watch 1      # custom interval (seconds)
#
# WHY: "audio isn't getting through" has three distinct causes that look
# identical from one device:
#
#   1. Nothing arriving      — wrong room, WS down, or the peer isn't sending.
#                              relay.received flat on the receiver.
#   2. Arriving, not opening — the peers hold DIFFERENT session keys. Wire
#                              packets climb while crypto_rx.ok stays 0 and
#                              crypto_rx.fail climbs. This is the one that
#                              used to be invisible: every visible counter
#                              looked healthy.
#   3. Arriving and opening  — transport is fine; the problem is downstream
#                              (playback, routing, jitter buffer).
#
# Reading both ends at once is what separates them, which is why this drives
# every adb device rather than one.
#
# NOTE ON TRANSPORTS: an emulator reaches the relay over the host's internet
# like any device, so emulator<->phone works here. WiFi multicast does NOT
# cross that boundary (the emulator is behind its own NAT and never joins the
# LAN multicast group), and emulator Bluetooth is a virtual controller that
# only reaches other emulators on the same host. Relay is the only transport
# that spans emulator and physical device — so if you are testing mixed, this
# is the plane to watch.
set -uo pipefail

PKG=com.sassyconsulting.sassytalkie
MODE="${1:-once}"
INTERVAL="${2:-2}"

devices() { adb devices | awk '/\tdevice$/ {print $1}'; }

# Read the most recent snapshot the app emitted to logcat.
#
# Deliberately NOT a broadcast the script triggers: that would require an
# exported receiver in the app, i.e. a permanent surface exposing the session
# room id to every other app on the device, in exchange for a debugging
# convenience. The app already emits this line every ~2s while diagnostics are
# enabled, so reading logcat costs nothing and adds no attack surface.
#
# Requires: Settings -> diagnostics overlay enabled (or a debug build).
snapshot_for() {
  local serial="$1"
  adb -s "$serial" logcat -d -s SassyDiag 2>/dev/null | tail -1 | sed 's/^.*SassyDiag: //'
}

fmt_one() {
  local serial="$1" json="$2"
  local model
  model=$(adb -s "$serial" shell getprop ro.product.model 2>/dev/null | tr -d '\r')
  echo "── $serial ($model)"
  if [ -z "$json" ]; then
    echo "   no snapshot — app not running, or build predates diagSnapshot"
    return
  fi
  python3 - "$json" <<'PY' 2>/dev/null || echo "   $json"
import json,sys
try:
    d=json.loads(sys.argv[1])
except Exception:
    print("   "+sys.argv[1]); raise SystemExit
r=d.get("relay",{}) or {}
c=(d.get("counters") or {})
rx=c.get("crypto_rx",{}) or {}
cap=c.get("capture",{}) or {}
cod=c.get("codec",{}) or {}
act=c.get("activity",{}) or {}
print(f"   transport={d.get('transport')} encrypted={d.get('encrypted')}")
print(f"   relay     state={r.get('state')} room={r.get('room')} sent={r.get('sent')} recv={r.get('received')} "
      f"qin={r.get('inbound_queue')} qout={r.get('outbound_queue')} drops={r.get('dropped_inbound_overflow')}/{r.get('dropped_outbound_overflow')}")
ok,fail,ns,rp = rx.get('ok',0),rx.get('fail',0),rx.get('no_session',0),rx.get('replay',0)
pct = rx.get('ok_pct',-1)
print(f"   crypto_rx ok={ok} fail={fail} no_session={ns} replay={rp} ok_pct={pct}")
print(f"   capture   frames={cap.get('frames')} dbfs={cap.get('last_dbfs')} | codec enc={cod.get('frames_encoded')} avg={cod.get('avg_frame_bytes')}B")
print(f"   activity  last_tx={act.get('last_tx_age_ms')}ms ago  last_rx_ok={act.get('last_rx_ok_age_ms')}ms ago")
# Verdict — the whole point of the tool.
recv=r.get('received') or 0
if recv==0 and ok==0:
    print("   VERDICT: nothing arriving -- check room match on BOTH ends, and that the peer is transmitting")
elif ok==0 and (fail or ns):
    print("   VERDICT: arriving but NOT decrypting -- peers are on different session keys (re-pair via QR)")
elif ok>0:
    stale = act.get('last_rx_ok_age_ms',-1)
    if isinstance(stale,int) and stale>10000:
        print(f"   VERDICT: decrypted fine but nothing recently ({stale}ms) -- peer stopped sending")
    else:
        print("   VERDICT: relay path healthy -- look downstream (playback/routing/jitter)")
PY
}

case "$MODE" in
  reset)
    # Clear the log buffer so the next sample is a fresh emission rather than
    # a stale line from a previous run. The native counters are cumulative by
    # design (a reset would race the 1 Hz emitter and lose data); compare two
    # samples instead when you want a rate.
    for s in $(devices); do
      adb -s "$s" logcat -c >/dev/null 2>&1
      echo "log buffer cleared: $s (next snapshot lands within ~2s)"
    done
    ;;
  watch)
    trap 'echo; echo "stopped."; exit 0' INT
    while true; do
      clear
      echo "relay-diag  $(date '+%H:%M:%S')   (Ctrl-C to stop)"
      echo
      for s in $(devices); do fmt_one "$s" "$(snapshot_for "$s")"; echo; done
      sleep "$INTERVAL"
    done
    ;;
  *)
    d=$(devices)
    [ -n "$d" ] || { echo "no adb devices attached" >&2; exit 1; }
    for s in $d; do fmt_one "$s" "$(snapshot_for "$s")"; echo; done
    ;;
esac
