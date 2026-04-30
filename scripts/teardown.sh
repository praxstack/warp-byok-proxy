#!/usr/bin/env bash
# Tear down warp-byok-proxy: stop the daemon, remove hosts entry, untrust cert.
# Run with: sudo ./scripts/teardown.sh
set -euo pipefail

echo "==> stopping any running warp-byok-proxy"
sudo pkill -f 'warp-byok-proxy run' 2>/dev/null && echo "   proxy killed" || echo "   (no proxy process)"

echo "==> removing app.warp.dev from /etc/hosts"
sudo sed -i '' '/app\.warp\.dev.*warp-byok-proxy/d' /etc/hosts
echo "   /etc/hosts entries touching app.warp.dev:"
grep 'app.warp.dev' /etc/hosts || echo "   (none)"

echo "==> untrusting cert from System Keychain"
sudo security delete-certificate -c warp-byok-proxy /Library/Keychains/System.keychain 2>/dev/null \
  && echo "   cert removed" \
  || echo "   (cert not found, maybe already removed)"

echo "==> flushing DNS cache"
sudo dscacheutil -flushcache
sudo killall -HUP mDNSResponder
echo "   done"

echo ""
echo "Teardown complete. Key material at ~/Library/Application Support/warp-byok-proxy/"
echo "is left in place (cert.pem, key.pem, config.toml). Delete manually if desired:"
echo "   rm -rf ~/Library/Application\\ Support/warp-byok-proxy/"
