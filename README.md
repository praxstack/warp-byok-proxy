# warp-byok-proxy

> **Status: pre-alpha (v0.0.1-alpha)**. The scaffold is complete but the
> real AWS Bedrock call path is not yet wired — `oz-local::converse_stream`
> still returns `todo!()`. Today this repo builds, tests, and runs the
> transport pipeline against scripted mock events; it does NOT stream
> real Opus 4.7 tokens end-to-end yet. See `docs/TODO-phase0-finish.md`
> for the concrete 4-step follow-up.

Local proxy that routes Warp Terminal's AI calls to your own AWS Bedrock
account so you can pay-per-token for Claude Opus 4.7 instead of renting a
Warp "Lightspeed" seat. Phase 0 of the `warp-byok` experiment.

Runs as a loopback HTTPS server on `127.0.0.1:443`. Warp is pointed at it
via `/etc/hosts` redirect so `app.warp.dev` resolves to localhost. The
proxy translates Warp's protobuf Request over to AWS Bedrock Converse
Stream, drains the SDK event stream, and translates the result back into
Warp's protobuf ResponseEvent shape so the Warp UI renders normally.

## Scope

- **In:** `/ai/multi-agent` (the main chat/agent endpoint) — protobuf in,
  protobuf-framed SSE out. Claude Opus 4.7 via AWS Bedrock Converse
  Stream. Four auth modes (SSO, API key, Identity Center, env bearer).
  Thinking / reasoning config. Cache-point injection on the last user
  message. 1-million-token context via `:1m` model suffix.
- **Out of Phase 0:** Every other `/ai/*` endpoint (suggestions,
  transcription, query prediction, codebase indexing, etc.). Those are
  captured as stubs in `docs/warp-client-behavior-audit-stub.md` for the
  Phase A fork to address.

## Build

```bash
rustup toolchain install stable
cargo build --release
```

## Use

```bash
# 1. Generate & trust the loopback cert (one-time, macOS Keychain).
sudo warp-byok-proxy cert --install

# 2. Add the /etc/hosts redirect (one-time).
sudo warp-byok-proxy login --hosts-add

# 3. Configure AWS creds in ~/.config/warp-byok/config.toml (see below).

# 4. Run the proxy.
sudo warp-byok-proxy run

# (Pre-alpha: actually hitting a prompt in Warp will fail because
# RealBedrock's converse_stream is still `todo!()`. Follow
# docs/TODO-phase0-finish.md to land the final piece.)
```

## Config file (`~/.config/warp-byok/config.toml`)

```toml
[bedrock]
# One of: "sso" (uses ~/.aws/config profile), "api_key" (long-lived
# Bedrock API key), "identity_center" (SSO via AWS Identity Center),
# or "env_bearer" (AWS_BEARER_TOKEN_BEDROCK env var).
auth_mode = "sso"
profile   = "default"
region    = "us-east-1"

[model]
# Base model ID. Proxy appends :1m at runtime for 1M context.
id     = "anthropic.claude-opus-4-7-20260101-v1:0"
# Optional CRI / global prefix. "" for none, "us.", "global.".
prefix = "us."

[thinking]
# "off" | "enabled" | "adaptive"
mode   = "adaptive"
# Only used for enabled/adaptive. "low" | "medium" | "max".
effort = "medium"
```

## Known limitations

- **RealBedrock is not finished.** The 4-step follow-up in
  `docs/TODO-phase0-finish.md` covers: (1) serde_json `messages` →
  `Vec<aws_sdk_bedrockruntime::types::Message>`, (2) serde_json `system`
  → `Vec<SystemContentBlock>`, (3) serde_json `tools` →
  `Option<ToolConfiguration>`, (4) serde_json `Value` →
  `aws_smithy_types::Document` for `additional_model_request_fields`.
  Without these the `/ai/multi-agent` route will panic `todo!()` on any
  real request.
- Phase 0 only covers `/ai/multi-agent`. Other `/ai/*` endpoints return
  501/404 — Warp UI will show degraded behavior for suggestions,
  transcription, etc.
- macOS only today. The cert-install + hosts-patch scripts assume macOS
  Keychain and `/etc/hosts`. Linux port is mechanical but untested.
- The proxy is a disposable shim. It is NOT hardened for production — no
  auth on the loopback listener, no rate limiting, assumes a single local
  Warp client.
- Tool-use + tool-result translation in RealBedrock is stubbed; text-only
  turns work first, tool dispatch comes in Phase A.

## Contributing

Bug reports welcome. Pull requests welcome for the 4-step RealBedrock
follow-up (`docs/TODO-phase0-finish.md`) — that's the critical-path work
to get Phase 0 demo-ready. Beyond Phase 0, the project will fork into
`praxstack/warp-byok` for Phase A and this proxy repo will be archived.

## License

AGPL-3.0-or-later. See `LICENSE`.
