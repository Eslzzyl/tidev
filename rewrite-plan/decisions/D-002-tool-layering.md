# D-002: 工具类型系统分层

**日期**：2026-07-02
**更新**：2026-08-04
**状态**：已采纳，当前边界已落地

## 当前决策

工具分为三个职责层：

    tidev-llm
      仅保留发送给 provider 的精简 ToolDefinition

    tidev-tools
      ToolDefinition、ToolPermission、ToolArgs 和 builtin 工具实现
      execute_tool_call、SkillCatalog、TodoPersistence、ShellOutput

    tidev-core
      ToolRegistry 宿主策略、审批、边界检查、敏感文件检查和 MCP 集成

    tidev-agent
      通用 Tool、ToolContext、ToolRegistry 和 MCP client

tidev-tools 不依赖 tidev-agent 或 tidev-core。core 通过 adapter 将 builtin
工具接入 tidev-agent 的 Tool 契约；MCP 的通用 client 和 registry 由
tidev-agent 提供，core 只负责产品配置和权限映射。

## 约束

工具定义、工具执行、权限策略和持久化必须保持可区分。agent 层不得引入
tidev 的审批、session 或 storage 类型。
