# D-002: 工具类型系统分层

**日期**: 2026-07-02  
**状态**: 已采纳

## 背景

旧实现中工具相关的类型（`ToolDefinition`、`ToolPermission`、`ToolArgs` trait��和工具实现（file read/write/edit、bash、glob/grep、web 等）都在 `tidev-engine/src/tooling/` 下，没有 crate 边界。

## 决策

**拆分为三层：**

```
tidev-types/src/tools.rs   — 纯类型定义（ToolDefinition, ToolOrigin, ToolPermission,
                              PermissionConfig, ToolArgs trait + macros, Args structs,
                              canonical_tool_name, FileReadStamp）

tidev-tools/               — 工具实现（builtin/ 下的 read/write/edit/bash/glob/grep 等，
                              execute_tool_call() 路由，ToolContext，SkillCatalog）

tidev-core                 — 编排层（ToolRegistry：统一注册 builtin + MCP 工具，
                              权限检查，文件读取追踪）
```

## 理由

1. **依赖关系清晰**：tidev-mcp、tidev-llm 只需 tidev-types 获取 `ToolDefinition`，不必引入整个工具实现树
2. **编译分离**：工具实现依赖大量外部 crate（reqwest、diffy、base64 等），不影响类型层的编译
3. **职责单一**：types 定义"工具长什么样"，tools 实现"工具怎么执行"，core 协调"什么时候用什么工具"
