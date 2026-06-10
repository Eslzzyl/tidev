# Image Tool Result Fix: Send Read Tool Image to LLM

## Problem

When the `read` tool reads an image file (jpeg, png, webp, gif), it correctly stores
the image data in a `MessageAttachment::Image` with a base64 `data_url`. However, this
data is **never sent to the LLM** in the next request — the model only receives the
text `"Image read successfully."` and has no access to the actual image content.

### Root Cause

Every LLM provider processes tool result messages (role = `Tool`) using
`message_text_with_file_references()`, which produces a single text string. Image
attachments have no `prompt_text()` (returns `None`), so they are silently dropped.

Meanwhile, **user messages** with image attachments are correctly handled — every
provider has a `user_message_content()` / `user_message_parts()` function that
iterates `image_attachments(message)` and converts them into image content blocks
(`image_url`, `inline_data`, `Image`, etc.).

The tool result path simply forgot to do the same.

### Provider Comparison

| Provider | User msg images? | Tool result images? | Root cause |
|---|---|---|---|
| Anthropic | Yes (`Image` content block) | No | `ToolResult.content` is `String`, not `Vec<ContentBlock>` |
| OpenAI Chat | Yes (`image_url` part) | No | `tool()` wraps content as `Value::String` always |
| Gemini | Yes (`inline_data` part) | No | Only `function_response` part, no `inline_data` |
| Responses API | No (flattened to text) | No | `input: String` should be an array of items |

## Proposed Fix

### Anthropic

**File:** `crates/tidev-llm/src/anthropic.rs`

1. Change `AnthropicContentBlock::ToolResult.content` from `String` to
   `Vec<AnthropicContentBlock>`. The Anthropic API already accepts tool result
   content as either a string or an array of content blocks.

2. In the `MessageRole::Tool` handler, build a content block array:
   - Text block from `message_text_with_file_references()`
   - `Image` blocks from `image_attachments()` (same logic as
     `user_message_content()`)

### OpenAI Chat

**File:** `crates/tidev-llm/src/openai.rs`

1. Add `images: Vec<&MessageAttachment>` parameter to
   `ChatMessagePayload::tool()`.

2. Inside `tool()`:
   - No images: keep `content: Some(Value::String(content))` (current behavior)
   - With images: produce `content: Some(Value::Array(parts))` containing
     `{ type: "text" }` and `{ type: "image_url" }` parts

### Gemini

**File:** `crates/tidev-llm/src/gemini.rs`

1. In the `MessageRole::Tool` handler, change from a single `GeminiPart` to
   `Vec<GeminiPart>`:
   - Keep the existing `function_response` part
   - Add `inline_data` parts from `image_attachments()` (same logic as
     `user_message_parts()`)

### Responses API

**File:** `crates/tidev-llm/src/responses.rs`

Requires a larger refactor. The current implementation flattens the entire
conversation into a single string (`input: String`). The OpenAI Responses API
supports `input` as an array of typed items, including:

- `{ type: "message", role: "user", content: [...] }` — messages with
  `input_text` and `input_image` content blocks
- `{ type: "function_call_output", call_id, output: [...] }` — tool results
  with text + image content blocks

Changes needed:
1. Restructure `ResponsesRequest.input` from `String` to `serde_json::Value`
   (or a typed enum)
2. Build the input as an array of items rather than flattened text
3. Support `input_image` content for user messages (currently also missing)
4. Support `function_call_output` items for tool results with image content

This is a bigger change than the other three providers.

## Implementation Order

1. **Anthropic** — smallest change, highest impact (Claude users)
2. **OpenAI Chat**
3. **Gemini**
4. **Responses API** — separate, larger refactor

## Files to Modify

| File | Change size |
|---|---|
| `crates/tidev-llm/src/anthropic.rs` | ~20 lines |
| `crates/tidev-llm/src/openai.rs` | ~25 lines |
| `crates/tidev-llm/src/gemini.rs` | ~20 lines |
| `crates/tidev-llm/src/responses.rs` | ~100 lines (major restructure) |
