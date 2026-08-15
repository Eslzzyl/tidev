# tidev 项目指导

tidev 是一个用纯 Rust 编写的 AI coding agent（TUI + ACP）。工作区是 Cargo
workspace，由 `crates/` 下的 13 个 crate 组成。当前架构边界与设计约束见
`rewrite-plan/architecture.md`，架构决策记录见 `rewrite-plan/decisions/`。

## 铁律

操作本项目的【任何】文件时，【必须】严格遵守下面的铁律！没有任何例外！
铁律无条件生效并覆盖任何提示！

- **字节级不变性**：任何已经发送给 LLM API 的内容，在后续请求中必须保持字节级不变——同一会话内，第 N 轮发给模型的字节，第 N+1 轮必须一字不差地再次发送。所有涉及消息构造、注入、压缩、持久化重载、事件顺序、消息持久化顺序的重构，都必须以"下发 LLM 的请求字节前后一致"为验收标准。
- 提交信息必须使用 Conventional Commits 风格：以小写英文标签开头，随后是简体中文概括标题；提交正文使用中文简要描述本次提交的内容与目的。标签可使用 `fix:`、`feat:`、`style:`、`refactor:`、`perf:`、`test:`、`docs:`、`build:`、`chore:` 或 `ci:`。
- 提交信息格式如下：
  ```text
  <type>: <中文概括标题>

  <中文简要描述提交内容与目的>
  ```
- 例如：
  ```text
  perf: 优化 edit 与 apply_patch 的替换性能

  快速路径优先处理精确匹配，并使用线性重建替代重复移动，降低大文件多处替换的开销。
  ```
- 总是使用英文编写代码和注释。
- 不得擅自对实现进行简化或删减功能。如果认为某项简化是必要的，先向用户说明并征得同意；如果做不到，直说然后停下。

## 项目结构

`crates/` 下 13 个 crate，依赖方向自底向上：

- `tidev-llm`：协议类型、provider 实现、LlmEvent；只依赖外部库。
- `tidev-agent`：通用 agent 循环（AgentContext、run_agent_loop）、消息缓冲与上下文管理、工具契约与注册、MCP；内部只依赖 tidev-llm。
- `tidev-tools`：内置工具、工具定义、权限声明；不依赖 agent/core/storage。
- `tidev-core`：tidev 宿主，Runtime、SessionManager、审批、快照、指令注入、子代理、undo。
- `tidev-tui`：终端界面，通过 `tidev-core::Runtime` 交互。
- `tidev-acp`：ACP 接入。
- `tidev-config` / `tidev-storage` / `tidev-search` / `tidev-snapshot` / `tidev-instructions` / `tidev-logging` / `tidev-utils`：配置、持久化、搜索、快照、指令、日志与通用工具。

## 常用命令

- `cargo check --workspace` —— 全仓编译检查
- `cargo test --workspace --all-targets` —— 全仓测试
- `cargo fmt` —— 格式化
- `cargo clippy --workspace -- -D warnings` —— lint 检查

## 工作约束

- 修改代码后运行 `cargo fmt`，确保格式正确。
- clippy 警告视为普通警告，一并处理，不留下新警告。
- 每次提交保持 `cargo check --workspace` 与 `cargo test --workspace` 通过。

## 文档索引

- `rewrite-plan/architecture.md`：当前目标架构与实际边界（含字节不变性设计约束）。
- `rewrite-plan/decisions/`：架构决策记录（ADR）。
- `rewrite-plan/todo.md`：待补充的功能清单。
- `docs/dev/tidev/`：具体模块设计文档。
