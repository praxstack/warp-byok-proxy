#!/usr/bin/env bash
set -euo pipefail

# Starts pcap on en0, runs the proxy, runs one prompt via Warp,
# stops pcap, and greps for app.warp.dev in the capture.
# Requires: sudo, tcpdump, a running warp-byok-proxy on :443,
# and a Warp binary configured to route app.warp.dev → 127.0.0.1.

PCAP=/tmp/warp-byok-egress.pcap
echo "Starting pcap capture → $PCAP"
sudo tcpdump -i any -w "$PCAP" 'host app.warp.dev' &
TCPDUMP_PID=$!
trap 'sudo kill "$TCPDUMP_PID" 2>/dev/null || true' EXIT

echo "Press ENTER after you've run a prompt in Warp..."
read -r _

sudo kill "$TCPDUMP_PID" 2>/dev/null || true
wait "$TCPDUMP_PID" 2>/dev/null || true

BYTES=$(sudo tcpdump -r "$PCAP" 2>/dev/null | wc -l | tr -d ' ')
echo "Captured $BYTES packets to/from app.warp.dev"
if [ "$BYTES" -gt 0 ]; then
    echo "FAIL: traffic to app.warp.dev was observed"
    exit 1
fi
echo "PASS: zero egress to app.warp.dev"
