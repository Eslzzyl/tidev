# 图片 base64 编码职责错位

**状态**: ✅ 已完成
**涉及文件**: `tidev-types/src/message.rs`、`tidev-llm/src/*.rs`

## 问题

当前 `MessageAttachment::Image` 存储预编码的 `data_url: String`（如 `data:image/png;base64,...`），由工具层在读取图片时直接生成。

base64 编码和 data URL 格式是 LLM API 的呈现细节，不应由工具层负责。工具层的职责是读取文件，不是为 API 格式化数据。

## 期望设计

`MessageAttachment::Image` 存储原始数据，tidev-llm 在构建 API 请求时按各 provider 格式编码：

```rust
MessageAttachment::Image {
    filename: String,
    mime: String,
    data_url: String,  // ← 改为存储原始字节或文件路径
    file_size: u64,
}
```

各 provider 编码方式：
- **OpenAI**: `"image_url": { "url": "data:mime;base64,..." }`
- **Anthropic**: `{ "type": "base64", "media_type": mime, "data": base64_bytes }`
- **Gemini**: `{ "inline_data": { "mime_type": mime, "data": base64_bytes }`

## 修复时机

在 tidev-llm 接口重构阶段一并处理。
