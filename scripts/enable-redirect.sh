#!/usr/bin/env bash
# Re-enable the Warp → proxy redirect after a temporary disable (e.g. for
# re-login through Warp's auth flow). Assumes the proxy is already running.
# Run with: sudo ./scripts/enable-redirect.sh
set -euo pipefail

if grep -q "app.warp.dev.*warp-byok-proxy" /etc/hosts; then
  echo "redirect already in /etc/hosts, nothing to do"
else
  echo "127.0.0.1 app.warp.dev  # warp-byok-proxy" | sudo tee -a /etc/hosts >/dev/null
  echo "hosts entry added"
fi

echo "flushing DNS..."
sudo dscacheutil -flushcache
sudo killall -HUP mDNSResponder

echo "killing Warp so it drops cached connections..."
pkill -9 -f '/Applications/Warp.app' 2>/dev/null || true
sleep 2

echo "verifying proxy is up..."
if sudo /usr/sbin/lsof -nP -iTCP:443 -sTCP:LISTEN 2>/dev/null | grep -q warp-byok; then
  echo "✅ proxy listening on 127.0.0.1:443"
else
  echo "⚠️  proxy NOT running. Start it with:"
  echo "    sudo -E ~/Documents/workspace/warp-byok-proxy/target/release/warp-byok-proxy run"
  exit 1
fi

echo ""
echo "✅ redirect enabled. Launch Warp with: open -a Warp"
echo "   Then open a NEW agent tab (the tab you were using is cached dead)."
