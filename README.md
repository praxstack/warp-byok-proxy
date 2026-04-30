# warp-byok-proxy

> **Status: v0.0.1 (GA).** End-to-end streaming against real AWS Bedrock is
> working. Smoke-tested on 2026-04-30 with `anthropic.claude-opus-4-7`
> (1M context beta + adaptive max thinking) on `us-east-1`. 77 Rust tests
> green, one live-AWS smoke test (`#[ignore]`-gated, runs on demand).

Local proxy that routes Warp Terminal's AI calls to your own AWS Bedrock
account so you can pay-per-token for Claude Opus 4.7 instead of renting a
Warp "Lightspeed" seat. Phase 0 of the `warp-byok` experiment.

Runs as a loopback HTTPS server on `127.0.0.1:443`. Warp is pointed at it
via `/etc/hosts` redirect so `app.warp.dev` resolves to localhost. The
proxy translates Warp's protobuf Request over to AWS Bedrock Converse
Stream, drains the SDK event stream, and translates the result back into
Warp's protobuf ResponseEvent shape so the Warp UI renders normally.

## Scope

- **In:** `POST /ai/multi-agent` (the main chat/agent endpoint) — protobuf
  in, protobuf-framed SSE out. Claude Opus 4.7 via AWS Bedrock Converse
  Stream. Four auth modes (API key bearer, AWS profile, explicit
  credentials, default chain). Adaptive-mode thinking with configurable
  effort. Enabled-mode thinking with configurable budget tokens.
  Cache-point injection on the system block and last two user messages.
  1-million-token context via `:1m` model suffix → `anthropic_beta:
  ["context-1m-2025-08-07"]`.
- **Out of Phase 0:** Every other `/ai/*` endpoint (suggestions,
  transcription, query prediction, codebase indexing, etc.) — return 404.
  Tool-use + tool-result translation in RealBedrock is stubbed (text-only
  turns work; tool dispatch comes in Phase A fork).

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
sudo sh -c 'echo "127.0.0.1 app.warp.dev" >> /etc/hosts'

# 3. Configure AWS creds in ~/.config/warp-byok-proxy/config.toml (see below).

# 4. Run the proxy (requires sudo because port 443).
sudo -E warp-byok-proxy run
```

## Config file (`~/.config/warp-byok-proxy/config.toml`)

```toml
[bedrock]
# One of: "api-key" (AWS_BEARER_TOKEN_BEDROCK), "profile", "credentials",
# or "default-chain".
auth_mode = "api-key"
region    = "us-east-1"

# Bedrock model ID. Append ":1m" to enable 1M context (Opus 4.6+ only).
# CRI prefix (us./eu./apac./global.) is applied automatically based on
# `use_cross_region_inference` and `use_global_inference` below.
model = "anthropic.claude-opus-4-7:1m"

use_cross_region_inference = true
use_global_inference       = false
use_prompt_cache           = true
enable_1m_context          = true

[bedrock.thinking]
# "off" | "enabled" | "adaptive"
mode   = "adaptive"
# For "adaptive": "low" | "medium" | "high" | "max".
effort = "max"
# For "enabled": token budget (default 16000).
# budget_tokens = 32000
```

## Verified working end-to-end

Against real Bedrock as of 2026-04-30:

```
$ AWS_BEARER_TOKEN_BEDROCK=$AWS_BEARER_TOKEN_BEDROCK \
    cargo nextest run --test smoke_real_bedrock --run-ignored all

smoke: MessageStop reason=end_turn
smoke: usage input=24 output=13 cache_read=0 cache_write=0
smoke: drained 7 events
test opus_4_7_1m_max_thinking_streams_tokens ... ok
```

The smoke test parses the production-shaped TOML, resolves bearer auth,
builds a real `aws-sdk-bedrockruntime` client, dispatches a `ConverseStream`
call with the `anthropic_beta: ["context-1m-2025-08-07"]` + adaptive max
thinking payload, drains the event stream, and asserts at least one
`ContentBlockDelta` + one `MessageStop` arrived. Token counts and stop
reason are printed for human inspection.

## Bedrock GA shape notes (verified 2026-04-30)

These differ from what the plan's pseudocode assumed and are worth
capturing for anyone reading the code:

- **Opus 4.7 model ID** is `anthropic.claude-opus-4-7` (no `-v1:0` suffix).
  The older 4.5/4.1 models use dated suffixes; 4.6+ dropped them.
- **On-demand throughput is NOT supported** for Opus 4.7 — you MUST use an
  inference profile. CRI prefixes (`us.`, `global.`) resolve to the
  system-defined profiles. The proxy's `model_id.rs` applies the prefix
  based on config flags + region.
- **Thinking control** ships as two top-level keys in
  `additional_model_request_fields`: `thinking` + `output_config`.
  Adaptive mode = `{"thinking":{"type":"adaptive"},"output_config":{"effort":"max"}}`.
  The plan's single `reasoningConfig` blob is wrong for GA.
- **1M context** rides as `anthropic_beta: ["context-1m-2025-08-07"]`, NOT
  as part of the model ID. The `:1m` suffix in config is our own marker
  that `model_id.rs` strips before sending to the SDK.

## Known limitations

- macOS only today. The cert-install + hosts-patch scripts assume macOS
  Keychain and `/etc/hosts`. Linux port is mechanical but untested.
- The proxy is a disposable shim. It is NOT hardened for production — no
  auth on the loopback listener, no rate limiting, assumes a single local
  Warp client.
- Tool-use + tool-result translation is stubbed. Text-only turns work end
  to end; shell / file tools will `tracing::warn!` and skip. Phase A fork
  lands the full tool loop.
- `extract_system_prompt` and `extract_tool_defs` in `translator.rs`
  return `None` — the task_context walker for system prompts, RAG
  context, tool definitions, and prior tool results is Phase A scope.
- Only one `Request.input` variant is walked today (`UserInputs →
  UserQuery`). The 8 other variants (ResumeConversation, CodeReview,
  InvokeSkill, SummarizeConversation, etc.) fall through to a diagnostic
  stub. Warp client behavior audit in `docs/warp-client-behavior-audit-stub.md`.
- Bedrock mid-stream errors produce a silent EOF on the client side
  (no synthesized `StreamFinished` error frame). Flagged as a Phase-1
  TODO in `src/route_multi_agent.rs`.

## Running tests

```bash
# Unit + integration (fast, offline, ~1s). 77 tests.
cargo nextest run

# Real-AWS smoke (~5s, costs ~$0.001 per run, requires AWS creds).
AWS_BEARER_TOKEN_BEDROCK=<your-key> \
  cargo nextest run --test smoke_real_bedrock --run-ignored all

# Zero-egress verification (requires sudo, tcpdump, running proxy).
./scripts/verify_zero_egress.sh
```

## Architecture

```
Warp binary (hits app.warp.dev → 127.0.0.1:443 via /etc/hosts)
    │ POST /ai/multi-agent (protobuf over TLS)
    ▼
server.rs (hyper 1.x + rustls)
    │ handle_with_context
    ▼
route_multi_agent.rs (pipeline orchestrator)
    │   1. decode protobuf Request
    │   2. translator.rs → BedrockInput
    │          ├─ model_id.rs (:1m strip + CRI prefix)
    │          ├─ betas.rs    (anthropic_beta flags)
    │          ├─ thinking.rs (thinking + output_config fields)
    │          └─ cache.rs    (cachePoint injection)
    │   3. bedrock_client.rs::RealBedrock::converse_stream
    │          └─ sdk_translator.rs (serde_json → typed SDK values)
    ▼
aws-sdk-bedrockruntime::Client::converse_stream
    │ ConverseStream events
    ▼
stream_accumulator.rs (BedrockEvent → OzResponseFrame state machine)
    │ OzResponseFrame
    ▼
ui_adapter.rs (OzResponseFrame → ResponseEvent protobuf)
    │ SSE-encoded ResponseEvent
    ▼
back to Warp binary
```

## Contributing

Bug reports and patches welcome. Phase 0 is stable enough to use daily;
active development moves to `praxstack/warp-byok` (Phase A fork) once that
repo exists. Issue #1 tracks the retro + GO/NO-GO decision on the Phase A
fork.

## License

AGPL-3.0-or-later. See `LICENSE`.
