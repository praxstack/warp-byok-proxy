# Warp client behavior audit (Phase 0 findings)

Skeleton for the expanded audit required by the fork's Pre-Phase-A work.
Fill each section below as you run the proxy and observe Warp's behavior.

## 1. Cancel path
- What happens when user presses Esc mid-text-stream?
- What happens when user presses Esc mid-tool-use?
- Does Warp's next request include a synthetic tool_result?

## 2. System prompt assembly
- Does Request.task_context contain the system prompt?
- Or does Oz inject one server-side?

## 3. Context windowing
- How does the client decide what history to include?
- Token-counting on the client, or Oz summarizes server-side?

## 4. Auto-resume-after-error
- When does the client auto-retry?
- What metadata flows on retry?

## 5. Tool dispatch policy
- Any server-side filtering of tool_use names or inputs?

## 6. RAG signals
- Are codebase-indexing or Drive results injected client-side or server-side?

## 7. Wire shape per endpoint
- /ai/multi-agent: protobuf Request / ResponseEvent (confirmed)
- /ai/generate_am_query_suggestions: JSON (confirmed), but exact schema?
- /ai/passive-suggestions: protobuf (confirmed)
- /ai/predict_am_queries, /ai/generate_input_suggestions, /ai/relevant_files,
  /ai/generate_code_review_content, /ai/transcribe: UNCONFIRMED
