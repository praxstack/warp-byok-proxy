# Zero-egress verification

Phase 0 requires that no traffic reaches `app.warp.dev` when the proxy is active.

## Setup

1. Add to `/etc/hosts` (requires sudo):
   ```
   127.0.0.1    app.warp.dev
   ```
2. Generate + trust the cert:
   ```
   warp-byok-proxy cert --install
   ```
3. Run the proxy:
   ```
   sudo warp-byok-proxy run
   ```
4. In another terminal, run `scripts/verify_zero_egress.sh`.
5. Open Warp, run one prompt, then press ENTER in the verification script.
6. Expected output: `PASS: zero egress to app.warp.dev`.

## If it fails

- Check `/etc/hosts` is actually being used — some Warp binaries bypass system DNS. Fall back: block `app.warp.dev` at `pfctl` instead.
- Check the self-signed cert is trusted system-wide (`security find-certificate -c warp-byok-proxy /Library/Keychains/System.keychain`).
- If Warp still hits app.warp.dev directly via hardcoded IP, Phase 0 wedge does not work — pivot to forking the Warp binary to rewrite the URL constant (small scope; noted in TODOs).
