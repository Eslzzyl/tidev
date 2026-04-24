# magic-code vs opencode: 上下文管理与工具设计对比

## 1. 概览

- `magic-code` 侧重于“一次对话会话状态机 + prompt cache 边界”的设计。
- `opencode` 侧重于“Effect 运行时 + 会话事件流 + 服务层工具注册”的架构。

两者都支持模型驱动的工具调用，但实现方式和关注点不同。

## 2. 上下文管理对比

### magic-code

- 核心由 `magic-code/src/QueryEngine.ts` 驱动。
- `QueryEngine` 是一个会话级状态机，持有：会话消息、文件读取缓存、权限拒绝记录、总使用量、技能发现状态等。
- 每次 `submitMessage()` 都会重新组装一次请求上下文，但会复用同一个 `QueryEngine` 的状态。
- 上下文构造通过 `fetchSystemPromptParts()` 拆分为三部分：`defaultSystemPrompt`、`userContext`、`systemContext`。
- 这份拆分显式支持 cache-safe prompt 构造：静态 prompt 与动态上下文分离，避免每次都破坏缓存。
- `customSystemPrompt` 可以完全替换默认 prompt，避免默认 prompt 与用户自定义 prompt 混合导致的不确定性。
- 还会在 loop 之前注入 coordinator userContext、memory-mechanics prompt、MCP 服务器指令等动态信息。
- 状态机风格使得工具调用、错误恢复、compact、stop hook、权限决策等都能在同一执行路径里处理。

### opencode

- 核心由 `opencode/packages/opencode/src/session/prompt.ts` 的 `SessionPrompt.loop` 驱动。
- 会话状态以持久化消息流和 `SessionRunState`、`SessionStatus`、`SessionProcessor` 等 Effect 服务存在。
- `loop()` 用事件驱动、Effect 管道式方式运行：读取消息、查找最后一条用户消息和 assistant 消息，判断是否继续。
- `SessionProcessor.create()` 负责将具体的模型调用、流事件、工具执行等封装到一个处理器中。
- 上下文构造由 `SystemPrompt.service`、`Instruction.service`、`MessageV2.toModelMessagesEffect()` 等组合：模型 prompt 由环境信息、agent skills、系统 instructions 和对话消息共同构成。
- `SystemPrompt.provider(model)` 根据模型 ID 选择不同的系统 prompt 文件，并把当前环境（工作目录、git 状态、平台、日期等）注入到 prompt 中。
- 会话压缩/overflow 由 `SessionCompaction` 管理；如果历史消息过大，会插入 compaction 任务并继续 loop。

### 主要区别

- `magic-code` 更强调“prompt 构造的缓存边界”和“QueryEngine 内部维护的 turn 级状态”。
- `opencode` 更强调“Effect 服务层次”和“消息流作为会话事实源”，把上下文管理放在多层服务协作里。
- `magic-code` 的系统 prompt 采用显式拆分、工具集合参与 prompt 生成；`opencode` 的系统 prompt 采用模型模板 + 环境注入 + 技能提示的组合形式。

## 3. 工具设计对比

### magic-code

- 工具定义在 `magic-code/src/Tool.ts`，是一个丰富的对象接口：
  - schema 验证
  - 权限检查
  - 是否只读
  - 是否并发安全
  - 工具 prompt 文本
  - 结果渲染
  - 进度显示
  - 自动分类器输入
- `magic-code/src/tools.ts` 不是静态常量，而是动态生成：
  - 根据环境、特性开关、权限上下文、运行模式决定可用工具
  - 支持 `REPL`、`KAIROS`、`COORDINATOR_MODE` 等条件加载
  - 先构建全部候选工具，再过滤掉 deny 规则
- 工具执行分成两层：
  - `toolOrchestration.ts` 负责工具调用分组、并发策略、串行/并发调度
  - `toolExecution.ts` 负责单个工具的执行、错误分类、权限 hooks、结果返回
- 并发策略更细粒度：只读工具可以并行，非只读工具则串行；工具可设置 `concurrency` 属性，并根据 `contextModifier` 决定是否回写上下文。
- 还集成了“模型可见工具池”和“斜杠命令工具技能”的概念，命令和工具之间存在交叉；prompt 型命令会转为可被模型调用的技能。
- 权限与工具调用深度绑定：工具调用前先做 schema 校验、validateInput、`canUseTool`、pre-tool hooks，失败会转成可供模型继续推理的 `tool_result`。
- 支持 `customSystemPrompt`、`appendSystemPrompt`、`mcpClients`、`agentDefinitions` 等高级 context 注入，工具执行与主循环紧密耦合但又层次分明。

### opencode

- 工具注册由 `opencode/packages/opencode/src/tool/registry.ts` 管理。
- `Tool.Def` 定义类似于：`id`、`description`、`parameters`、`execute(args, ctx)`，并且使用 `zod` 做参数校验。
- 注册逻辑通过 Effect 服务创建：
  - 内置工具在 service 初始化时构建
  - 插件工具会被动态发现并按规范转换成 `Tool.Def`
  - 还支持从配置目录加载自定义工具
- 工具执行被包装在 `Tool.wrap()` 里，统一处理参数验证、执行、结果截断、元数据输出。
- `SessionPrompt.loop` 通过 `resolveTools()` 结合 agent permission 来决定当前 turn 可见工具。
- `llm.ts` 负责将这些工具传入模型流：
  - 兼容 LiteLLM/Bedrock 等要求工具参数必须存在的情况
  - 为 DWS workflow 模型提供 `toolExecutor`
  - 支持 `toolChoice` 控制（例如 json schema 输出模式下强制工具调用）
- 工具权限在 `resolveTools()` 中做一次筛选；工具调用本身也借助 `Permission` 服务在会话级别控制。
- 与 `magic-code` 不同，`opencode` 的工具系统更明显地融合在 Effect 服务层次中，工具定义和运行时素材都通过 `Layer` 和 `Context.Service` 注入。

### 主要区别

- `magic-code` 的工具设计更注重“工具本身的行为与执行策略”，包括并发、安全、钩子、权限链路和 prompt 可见性。
- `opencode` 的工具设计更注重“工具注册与运行时可见性”，以 Effect 服务、插件动态加载、会话权限过滤为主。
- `magic-code` 通过工具池+prompt cache 让模型只看到“当前快照下需要的工具”；`opencode` 则更倾向于“先构建会话可用工具集，再在模型流中传入过滤后的工具”。
- `magic-code` 的工具执行路径有独立的 orchestration 层；`opencode` 的工具执行则更多体现在 `llm.ts` 与 `SessionProcessor` 的协作上。

## 4. 总结

- `magic-code` 是“状态机 + prompt cache + 细粒度工具调度”的实现。
- `opencode` 是“Effect 服务 + 会话事件循环 + 动态工具注册”的实现。

如果你要比较二者的关键差异，最核心的结论是：

1. `magic-code` 在上下文管理上更强调 prompt 缓存边界与 QueryEngine 内部状态；
2. `opencode` 在上下文管理上更强调消息流与服务组合；
3. `magic-code` 在工具设计上更强调工具执行策略与并发/权限管线；
4. `opencode` 在工具设计上更强调注册体系、插件扩展和 provider 兼容性。
