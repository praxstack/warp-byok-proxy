# Upstream Warp client audit — 2026-05

Audit of this proxy's wire shape against the current `warpdotdev/warp`
and `zerx-lab/warp` (OpenWarp) client source code. Pulled `HEAD` of both
repos on 2026-05-04 and compared the relevant consumer paths.

## Method

```bash
git clone --depth 1 https://github.com/warpdotdev/warp.git /tmp/warp-upstream
git clone --depth 1 https://github.com/zerx-lab/warp.git /tmp/zerx-warp
# Relevant files:
#   crates/field_mask/src/lib.rs        — FieldMask apply_path()
#   app/src/ai/agent/task.rs            — append_to_message_content
#   app/src/ai/agent/conversation.rs    — ClientAction dispatch
```

zerx-lab/warp is a fork of warpdotdev/warp with i18n additions and an
OpenWarp BYOP compaction sidecar; it is NOT a divergent protocol. The
multi-agent wire contract below holds for both.

## 1. FieldMask path bug — CONFIRMED and FIXED

**Upstream location:** `crates/field_mask/src/lib.rs:92-110`

```rust
let field_desc = match target.descriptor().get_field_by_name(field_name) {
    Some(f) => f,
    None => {
        // Applying a field mask on unknown fields are a no-op.
        //
        // This implies the client's API version is outdated with respect
        // to the server response. Adding fields is backwards-compatible
        // in protobuf, where expected behavior is to no-op.
        return Ok(());
    }
};
```

**Our bug (pre-fix):** `AppendToMessageContent` carried
`mask.paths = ["message.agent_output.text"]`. The outer `api::Message`
proto has an `oneof message { ... }` — proto3 exposes oneof variants as
TOP-LEVEL fields on the descriptor; the oneof name itself is NOT a field.
Every one of our appends fell through the "unknown field → silent no-op"
branch above, leaving the merged `Message` empty and the UI blank.

**Fix** (commit `b57c0f0`): strip the `message.` prefix; paths rooted at
the variant (`agent_output.text`, `agent_reasoning.reasoning`,
`tool_call.server.payload`). Guarded by 4 new tests in
`tests/ui_adapter_tests.rs`, including a full descriptor-walk replay of
`FieldMaskOperation::apply` that fails loudly if the path ever regresses.

## 2. Event sequence — MATCHES upstream

**Upstream dispatch:** `app/src/ai/agent/conversation.rs:2016-2529`

```rust
match action {
    Action::CreateTask(CreateTask { task: Some(task) }) => { ... }
    Action::AddMessagesToTask(AddMessagesToTask { task_id, messages }) => { ... }
    Action::AppendToMessageContent(AppendToMessageContent {
        task_id, message: Some(message), mask: Some(mask)
    }) => {
        // Requires the message.id already exist (added via AddMessagesToTask).
        // Otherwise returns UpdateTaskError::MessageNotFound — task.rs:761.
    }
    ...
}
```

**Our adapter** (`src/ui_adapter.rs::translate`) emits the full prelude
before the first append:

1. `StreamInit { conversation_id, request_id, run_id }` — all non-empty
2. `CreateTask { task: Task { id, ... } }`
3. `AddMessagesToTask { task_id, messages: [Message{ id, task_id,
   message: AgentOutput{ text: "" } }] }` — registers the message id
4. `AppendToMessageContent { task_id, message, mask }` — per chunk
5. `StreamFinished { reason, token_usage }` — terminal

Verified the `MessageNotFound` guard at
`app/src/ai/agent/task.rs:746-763` requires a pre-existing message with
matching `id`, which is exactly what step 3 provides. Skipping step 3 is
what caused the 2026-04-30 "I couldn't complete that request" live-Warp
failure.

## 3. Mid-stream error — FIXED

**Upstream behavior:** Warp's UI renders whatever frames arrived + EOF
as a clean end-of-turn (no error indicator). Our proxy used to break out
of the pump loop on a Bedrock `Err`, producing a silent EOF — documented
as a Phase-1 TODO in `route_multi_agent.rs`.

**Fix** (commit `80594ed`): on mid-stream error, synthesize a
`BedrockEvent::MessageStop { stop_reason: "error" }` and pump it through
the accumulator + adapter BEFORE breaking. The adapter's existing switch
in `stream_finished_event` maps any unknown reason string to
`Reason::Other`, so the UI sees an explicit terminal `StreamFinished`
frame rather than indistinguishable silent EOF.

## 4. Input-variant walker — EXTENDED

**Proto:** `apis/multi_agent/v1/request.proto:49-71`

```protobuf
message Input {
  InputContext context = 1;
  oneof type {
    UserInputs user_inputs = 6;
    QueryWithCannedResponse query_with_canned_response = 4;
    AutoCodeDiffQuery auto_code_diff_query = 5;
    ResumeConversation resume_conversation = 7;
    InitProjectRules init_project_rules = 8;
    GeneratePassiveSuggestions generate_passive_suggestions = 9;
    CreateNewProject create_new_project = 10;
    CloneRepository clone_repository = 11;
    CodeReview code_review = 12;
    SummarizeConversation summarize_conversation = 13;
    CreateEnvironment create_environment = 14;
    FetchReviewComments fetch_review_comments = 15;
    StartFromAmbientRunPrompt start_from_ambient_run_prompt = 16;
    InvokeSkill invoke_skill = 17;

    UserQuery user_query = 2 [deprecated = true];
    ToolCallResult tool_call_result = 3 [deprecated = true];
  }
}
```

**Pre-fix coverage:** 1 of 11 live variants (UserInputs → UserQuery only).
Everything else fell through to the `[PHASE0 WALKER: no UserQuery found]`
diagnostic stub.

**Post-fix coverage** (commit `178d5a7`): 5 text-bearing variants now
walked (UserInputs UserQuery + CliAgentUserQuery; deprecated top-level
UserQuery; AutoCodeDiffQuery; QueryWithCannedResponse; CreateNewProject).
Metadata-only variants (ResumeConversation, InitProjectRules, etc.) are
intentionally left to the empty-messages path since they feed
`task_context` not a user prompt. Tool-result / agent-message branches
are deferred to the Phase-A task_context walker.

## 5. zerx-lab/warp delta

OpenWarp-specific additions (from `diff -rq /tmp/warp-upstream
/tmp/zerx-warp`):

- `crate::ai::byop_compaction::state::CompactionState` sidecar on
  `Conversation` — opt-in local compaction metadata keyed by message_id,
  empty by default, fully non-invasive. Does NOT change the protobuf
  wire contract.
- i18n additions (zh-CN translations, i18n.toml).
- Channel icon assets for OSS branding.
- No changes under `crates/field_mask/`, no changes to the 3-event
  prelude or FieldMask path conventions.

**Conclusion:** BYOK proxy changes that work against `warpdotdev/warp`
also work against `zerx-lab/warp` HEAD.

## 6. Remaining gaps (Phase-A scope)

- `extract_system_prompt` and `extract_tool_defs` in `translator.rs`
  still return `None`. System prompts and tool definitions ride in
  `task_context`; walker landing in Phase-A fork.
- Tool-use + tool-result loop — adapter only encodes the opaque
  `Server { payload }` variant today. Variant-specific decoding
  (RunShellCommand, ReadFiles, etc.) is Phase-A.
- Other `/ai/*` endpoints (suggestions, transcription, query prediction,
  codebase indexing) still return 404.
- See `docs/warp-client-behavior-audit-stub.md` for the behavior-level
  audit checklist (cancel path, system-prompt assembly, etc.).

## References

- `warpdotdev/warp@HEAD` — 2026-05-04 clone
- `zerx-lab/warp@7742171` — 2026-05-04 clone
- `warp-proto-apis` — pinned rev `aa2f9cde164a5b48ac01087d417d1188771f9b6d`
  (see Cargo.toml)
