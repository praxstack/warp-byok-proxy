# Phase 0 Finish — the 4-step RealBedrock follow-up (CLOSED 2026-04-30)

All four steps landed before the 2026-04-30 real-Bedrock smoke test went
green. Preserved as a historical record of what Phase 0 finish took.

## Step 1 — Translate serde_json `messages` → `Vec<aws_sdk_bedrockruntime::types::Message>` ✅

Landed in `src/sdk_translator.rs::messages_to_sdk`.

## Step 2 — Translate serde_json `system` → `Vec<SystemContentBlock>` ✅

Landed in `src/sdk_translator.rs::system_to_sdk`.

## Step 3 — Translate serde_json `tools` → `Option<ToolConfiguration>` ⏳ (Phase-A)

Deferred. `translator::extract_tool_defs` still returns `None`. Tool-use +
tool-result loop is Phase-A scope.

## Step 4 — `serde_json::Value` → `aws_smithy_types::Document` ✅

Landed in `src/sdk_translator.rs::json_to_document`.

## Output-side translation ✅

`translate_output_event` in `src/bedrock_client.rs` does the 6-way
`ConverseStreamOutput` → `BedrockEvent` mapping.

## Verification ✅

```bash
AWS_BEARER_TOKEN_BEDROCK=<your-key> \
  cargo nextest run --test smoke_real_bedrock --run-ignored all
```

Passed 2026-04-30. README promoted to v0.0.1 GA same day.

---

## Post-GA polish (2026-05-04)

Three follow-up slices landed after the live-Warp UI audit surfaced bugs
the Phase-0 smoke test had missed:

1. **FieldMask path fix** (commit `b57c0f0`) — the `message.` prefix
   caused every `AppendToMessageContent` to silently no-op in upstream
   `field_mask::apply_path`. Guarded by 4 descriptor-walk tests.
2. **Mid-stream error synthesized StreamFinished** (commit `80594ed`) —
   closes the Phase-1 TODO in `route_multi_agent.rs`.
3. **Input-variant walker extended** (commit `178d5a7`) — 4 additional
   text-bearing variants beyond the original `UserInputs → UserQuery`.

See `docs/upstream-warp-audit-2026-05.md` for the full audit against
`warpdotdev/warp` and `zerx-lab/warp` HEAD.
