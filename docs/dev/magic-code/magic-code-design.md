# magic Code 源码设计调研

本文调研的是仓库中的 magic-code 目录。它实际对应的是 magic Code 的源码实现。重点覆盖四条主线：斜杠命令、主 agent 循环、工具系统、提示词构造，并补充一些值得参考的工程设计。

## 总览

这个代码库的核心思路可以概括为三层：

1. 用户输入先进入命令/文本分流层，判断是普通提示词还是斜杠命令。
2. 普通提示词进入 QueryEngine 驱动的主循环，由模型决定是否调用工具、是否继续下一轮。
3. 工具系统负责权限、并发、结果渲染、日志和上下文回写，提示词系统负责把这些能力稳定地暴露给模型。

最值得注意的是，它不是把所有逻辑塞进一个“聊天回调”，而是把命令、提示词、工具、权限、prompt 拆成了多个独立层，并且通过缓存和动态加载降低启动成本与 prompt 抖动。

## 斜杠命令系统

### 命令注册与加载

命令总表在 [magic-code/src/commands.ts](../magic-code/src/commands.ts) 里统一汇总。这个文件不是简单的静态数组，而是一个带有多层来源和过滤规则的注册中心：

 - 内置命令直接 import 后加入总表。
 - 特性开关控制的命令使用条件 require，借助构建期裁剪掉不需要的代码。
 - 技能目录、插件命令、工作流命令、动态技能会在运行时追加进来。
 - availability 和 isEnabled() 会在每次 getCommands() 调用时重新评估，以便登录状态或环境变化立即生效。

命令类型定义在 [magic-code/src/types/command.ts](../magic-code/src/types/command.ts)。这里把命令分成三类：

 - prompt 命令：会展开成一段 prompt，交给模型继续执行。
 - local 命令：由本地代码直接执行，不走模型。
 - local-jsx 命令：返回 React JSX，通常用于交互式 UI。

### 主要命令类别

源码里的命令数量很多，但从设计上可以分成几组：

| 类别 | 典型命令 |
| --- | --- |
| 会话与导航 | help、clear、resume、status、session、exit、share、summary、rewind |
| 模型与执行模式 | model、fast、effort、permissions、privacy-settings、rate-limit-options、output-style |
| 代码工作流 | commit、diff、files、branch、rename、review、security-review、plan、tasks |
| 集成与环境 | mcp、ide、desktop、mobile、chrome、terminalSetup、login、logout、upgrade |
| 诊断与维护 | doctor、debug-tool-call、cost、usage、stats、heapdump、break-cache |
| 技能与扩展 | skills、plugin、reload-plugins、hooks、context、memory、onboarding |

这批命令并不只是“用户菜单项”，很多还会被模型间接调用，或者在特定模式下作为技能展开。

### 命令解析与执行

斜杠命令的入口在 [magic-code/src/utils/processUserInput/processSlashCommand.tsx](../magic-code/src/utils/processUserInput/processSlashCommand.tsx)。执行流程非常清晰：

1. 先用 parseSlashCommand() 解析输入。
2. 如果解析失败，就回一条提示，说明输入格式应该是“/command args”。
3. 再用命令名查找真实命令；如果找不到，会检查它像不像文件路径，避免把 /tmp/... 之类的路径误判成命令。
4. 命令根据类型分别执行：local、local-jsx、prompt。

这里最有意思的是 prompt 命令的两种执行方式：

 - inline：命令内容直接展开进当前对话，继续主 agent 上下文。
 - fork：命令会被当成一个子 agent 任务执行，主线程可以继续工作。

fork 这条路径里还专门处理了 KAIROS 模式下的后台执行：命令会启动一个异步 subagent，完成后把结果重新入队为隐藏的 meta prompt，再由主循环消费。这种设计把“长任务”从用户交互线程里剥离了出去。

### 斜杠命令和技能的关系

这里的命令系统不只是传统 CLI 命令，它还承担了技能层的入口职责。getSkillToolCommands() 和 getSlashCommandToolSkills() 会从命令总表中筛出可被模型调用的 prompt 型技能。也就是说，magic 看到的“可调用能力”不完全等于用户能直接输入的命令列表。

从源码看，技能系统和命令系统是互相叠加的：

 - 用户能直接输入的命令由命令总表决定。
 - 模型能调用的技能由 prompt 命令过滤后形成。
 - 动态技能、插件技能、内置插件技能会被插入到总表里。

## 主 Agent 循环

### 入口和状态

主循环的核心在 [magic-code/src/QueryEngine.ts](../magic-code/src/QueryEngine.ts)。QueryEngine 是一个会话级状态机，而不是一次性函数。它在对象内部持有：

 - 当前消息数组。
 - 文件读取缓存。
 - 权限拒绝记录。
 - 总体 usage。
 - turn 级别的技能发现状态。

submitMessage() 是单轮输入的入口。它会把 prompt、模型配置、工具集、MCP 客户端、是否允许工具调用等信息收集起来，然后进入查询循环。

### 系统提示和上下文准备

在每次提交消息前，QueryEngine 会先调用 [magic-code/src/utils/queryContext.ts](../magic-code/src/utils/queryContext.ts) 里的 fetchSystemPromptParts()，拿到三块东西：

 - defaultSystemPrompt。
 - userContext。
 - systemContext。

如果用户显式提供了 customSystemPrompt，就会跳过默认系统 prompt 和系统上下文的构建。这一点很重要，因为它保证了自定义 prompt 可以真正替换默认行为，而不是在默认行为之上叠加一层不可控内容。

### 一轮查询的执行顺序

QueryEngine 的一轮执行大致遵循这个顺序：

1. 构造系统 prompt 和上下文。
2. 组装 processUserInputContext，处理用户输入、附件、技能、权限、IDE 选择等。
3. 调用 query() 进入主循环。
4. 模型输出后，如果产生工具调用，就交给工具执行器。
5. 工具结果回写到消息流，再进入下一轮，直到 stop reason 结束。

这个循环不是单次“请求-响应”，而是带有状态演进的多轮 agent loop。它会把工具调用、错误恢复、token 预算、compact、stop hook 等都纳入同一条执行路径。

### 真实的循环控制点

真正的循环代码在 [magic-code/src/query.ts](../magic-code/src/query.ts)。这里能看到几个关键行为：

 - 维护每轮的 mutable state，例如消息、工具上下文、token 预算、compact 状态等。
 - 在 API 返回后，把 assistant message 里的 tool_use blocks 交给工具执行器。
 - 当遇到 max output tokens、compact、reactive compact 等情况时，支持恢复和续跑。
 - 通过 notifyCommandLifecycle() 跟踪命令生命周期。

从设计上看，这一层是“会话控制器”，而不是单纯的模型客户端。它负责把模型输出、工具结果、权限决策、上下文变化串成一个可恢复的状态机。

## 工具设计

### Tool 抽象

工具接口在 [magic-code/src/Tool.ts](../magic-code/src/Tool.ts) 里定义得很完整。它不是只有一个 call 方法，而是包含了：

 - schema 验证。
 - 权限检查。
 - 是否只读。
 - 是否并发安全。
 - 工具 prompt 文本。
 - 结果渲染。
 - 进度显示。
 - 自动分类器输入。

更重要的是，buildTool() 会给一组默认实现补齐安全默认值，比如：

 - 默认不并发安全。
 - 默认不是只读。
 - 默认权限是 allow，但会走统一权限体系。
 - 默认自动分类器输入为空，要求安全相关工具显式声明。

这说明作者把“工具定义”看成一个高风险边界，因此采用了 fail-closed 的默认策略。

### 工具注册与组合

所有可用工具都通过 [magic-code/src/tools.ts](../magic-code/src/tools.ts) 汇总。这个文件展示了一个很重要的模式：工具池不是静态常量，而是根据环境、功能开关、权限上下文和运行模式动态生成的。

值得注意的点包括：

 - REPL 模式会隐藏一部分原始工具，只保留适合的外壳工具。
 - 只有在功能开关开启时才加载某些重量级工具。
 - 通过 filterToolsByDenyRules() 在模型看见工具前就先做 deny 过滤。
 - assembleToolPool() 把内置工具和 MCP 工具合并，并去重，避免同名工具冲突。

### 工具执行与并发策略

工具执行的核心在 [magic-code/src/services/tools/toolOrchestration.ts](../magic-code/src/services/tools/toolOrchestration.ts) 和 [magic-code/src/services/tools/toolExecution.ts](../magic-code/src/services/tools/toolExecution.ts)。它们把“模型说要调用工具”拆成了两个阶段：

 - 先分组，判断哪些调用可以并发。
 - 再执行单个工具或并发批次。

并发划分的规则很实用：

 - 连续的只读、并发安全工具可以并行跑。
 - 一旦碰到非只读或不安全工具，就退回到串行。
 - 工具可以通过 contextModifier 回写上下文，串行和并发路径会分别处理这个修改。

这比“全部并发”或“全部串行”都更稳。只读搜索类工具可以快跑，写操作则保持顺序和一致性。

### 工具调用的权限和校验层

工具调用不是直接执行 call()，而是先经过多层检查：

 - input schema 校验。
 - 工具自身的 validateInput。
 - 统一权限系统中的 canUseTool。
 - pre-tool hooks。
 - 失败后还会生成可供模型继续推理的 tool_result 错误消息。

这层设计的好处是，工具失败不会只是“抛异常结束”，而是会被转化成模型可见的上下文，让模型有机会自我修正。

## 提示词构造

### 系统 prompt 的总体结构

系统 prompt 的构造主文件是 [magic-code/src/constants/prompts.ts](../magic-code/src/constants/prompts.ts)。这里的设计非常明显地服务于 prompt 缓存：

 - 静态、跨会话可缓存的内容放在边界前。
 - 会话相关、环境相关、动态变化的内容放在边界后。
 - 通过 SYSTEM_PROMPT_DYNAMIC_BOUNDARY 明确切分缓存层。

也就是说，prompt 不是随手拼字符串，而是按照缓存语义组织的。

### 系统 prompt 的分段化

源码把 prompt 拆成多个 section：

 - intro 和 system 基础说明。
 - doing tasks，强调做代码工作时的约束。
 - actions，强调高风险动作前要确认。
 - using your tools，告诉模型优先用专用工具而不是 Bash。
 - tone and style，限制语气和引用格式。
 - output efficiency，约束输出简洁度。
 - session-specific guidance，按当前会话能力动态生成。

其中 section 的缓存由 [magic-code/src/constants/systemPromptSections.ts](../magic-code/src/constants/systemPromptSections.ts) 管理。它提供两种 section：

 - systemPromptSection：可缓存 section。
 - DANGEROUS_uncachedSystemPromptSection：每轮重算，明确会破坏缓存。

这套设计把“是否会破坏 prompt cache”变成了显式 API，而不是隐含副作用。

### 动态内容来源

动态内容主要来自以下几类：

 - 当前工具集：会被转成工具说明并写入系统提示。
 - MCP 服务器指令：每个已连接服务器如果带 instructions，会被注入到 prompt。
 - 输出风格：由 /output-style 等设置驱动。
 - 语言偏好、会话模式、scratchpad、memory、brief/proactive 之类的功能开关。

fetchSystemPromptParts() 的任务就是把这些内容按缓存边界拆好，再交给 QueryEngine 组合成最终的系统 prompt。

### 工具 prompt 的构造方式

工具本身在 Tool 接口里提供 prompt 一类的能力，最终会被转成 Anthropic 可消费的工具描述。配合 shouldDefer 和 alwaysLoad，系统还能决定哪些工具要先延迟加载，哪些工具必须在第一轮就可见。

这也是 magic Code 里一个非常重要的模式：模型并不总是一次性看到所有工具定义，而是按需发现、按需加载，从而降低 prompt 体积和工具污染。

## 值得参考的设计

### 1. 通过 memoization 和 lazy import 控制加载成本

commands.ts、tools.ts、prompt sections 都大量使用 memoize、dynamic import、conditional require。这样做的直接收益是：

 - 大模块不会在启动时一次性加载。
 - 特性关闭时，相关代码更容易被裁剪掉。
 - 复杂功能不会拖慢普通命令路径。

### 2. 先分类，再执行

无论是命令、工具还是 prompt section，这个项目都倾向于先分类再执行：

 - 命令分成 local、prompt、local-jsx。
 - 工具分成只读、安全、可并发、会写、可中断。
 - prompt 分成静态、动态、会话相关、会破坏缓存。

这种分类让代码更容易维护，也让很多边界条件在设计上就被消化掉了。

### 3. 把“模型可见内容”和“用户可见内容”严格分开

斜杠命令、tool_result、system message、meta message、transcript UI 之间有很清楚的区分。很多地方会显式标注是否 isMeta、是否要进入 transcript、是否只给模型看。这能避免把内部控制流意外暴露给用户，也避免把 UI 噪音污染到模型上下文。

### 4. 权限、hooks、hooks 决策和自动化检查分层

工具调用会经过权限规则、hook、自动分类器、后置 hook 等多层判断。它不是“用户点一下就执行”，而是一个可插拔的决策管线。这种设计虽然复杂，但非常适合一个需要同时兼顾自动化和安全性的 CLI agent。

### 5. 允许模型自我修正，而不是一次失败就结束

工具失败会被翻译成带有 tool_result 的错误消息返回给模型。这样模型可以根据错误信息继续修正输入，而不是只能在外层报错退出。对于 agent 系统来说，这比“抛错结束”更符合任务导向。

## 结论

magic-code 的核心设计可以概括成一句话：它把 magic Code 做成了一个“带缓存意识的分层 agent runtime”。

最值得借鉴的地方有三个：

 - 命令、工具、prompt、权限各自分层，边界清楚。
 - 对 prompt cache、并发安全、只读/写入、上下文污染这些问题，都是显式建模而不是靠约定。
 - 主循环不是单轮问答，而是一个可恢复、可并发、可插拔的会话状态机。

如果后续要继续深入，建议优先再看这几组文件：

 - [magic-code/src/utils/processUserInput/processSlashCommand.tsx](../magic-code/src/utils/processUserInput/processSlashCommand.tsx)
 - [magic-code/src/QueryEngine.ts](../magic-code/src/QueryEngine.ts)
 - [magic-code/src/services/tools/toolExecution.ts](../magic-code/src/services/tools/toolExecution.ts)
 - [magic-code/src/constants/prompts.ts](../magic-code/src/constants/prompts.ts)
 - [magic-code/src/Tool.ts](../magic-code/src/Tool.ts)
