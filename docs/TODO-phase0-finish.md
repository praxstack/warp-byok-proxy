# Phase 0 Finish — the 4-step RealBedrock follow-up

Task 17 landed as BLOCKED_PARTIAL: the skeleton smoke test + the `#[ignore]` gate
are wired, but `RealBedrock::converse_stream` still panics `todo!()`. Finishing
Phase 0 requires 4 concrete steps:

## Step 1 — Translate serde_json `messages` → `Vec<aws_sdk_bedrockruntime::types::Message>`

In `src/bedrock_client.rs::RealBedrock::converse_stream`, take the
`BedrockInput.messages: Vec<serde_json::Value>` and build typed SDK messages.

Each serde Value looks like:
  {"role":"user","content":[{"type":"text","text":"..."},{"cachePoint":{...}}]}

Translate to `aws_sdk_bedrockruntime::types::Message::builder()
  .role(ConversationRole::User)
  .content(ContentBlock::Text(...))
  .content(ContentBlock::CachePoint(...))
  .build()?`

Block variants to support: text, tool_use (ContentBlock::ToolUse), tool_result,
cache_point. Phase 0 can cover text + cache_point first; tool_use/result when
the stub prompt walker in the translator gets extended.

## Step 2 — Translate serde_json `system` → `Vec<SystemContentBlock>`

Same pattern for `BedrockInput.system`. Each entry is a `SystemContentBlock::Text`
or `SystemContentBlock::CachePoint`.

## Step 3 — Translate serde_json `tools` → `Option<ToolConfiguration>`

Build `ToolConfiguration::builder().tools(tool1).tools(tool2).build()?`. Each tool
is a `Tool::ToolSpec(ToolSpecification::builder()...)`. Phase 0 can defer this
(translator's `extract_tool_defs` returns None today).

## Step 4 — `serde_json::Value` → `aws_smithy_types::Document` for `additional_model_request_fields`

Walk the serde Value recursively, emitting `Document::Object`, `Document::Array`,
`Document::String`, `Document::Number`, `Document::Bool`, `Document::Null`.
Zero lossy surprises expected; a trivial 20-line conversion.

## Output-side translation (1:1, confirmed)

`ConverseStreamOutput` events map 1:1 to our `BedrockEvent` enum — already
confirmed by the SDK shape check in Task 17. Drain via
`output.stream.recv().await -> Result<Option<ConverseStreamOutput>, SdkError>`
and dispatch on the 6 variants.

## Verification

After the impl lands:

```bash
AWS_BEARER_TOKEN_BEDROCK=<your-key> \
  cargo nextest run --test smoke_real_bedrock --run-ignored all
```

Expected: PASS, with latency + token counts in tracing logs. Then update the
README's status banner to remove "pre-alpha" and tag `v0.0.1`.
