# 图片 base64 编码职责

**状态**：已完成

图片附件在协议层保存原始数据和 MIME 信息。tidev-llm 的各 provider 在构造
请求时分别编码为 OpenAI、Anthropic 或 Gemini 所需的格式。工具层只负责读取
文件和产生附件，不负责 provider 专属的数据 URL 编码。

该职责边界不再依赖旧 tidev-types 路径。
