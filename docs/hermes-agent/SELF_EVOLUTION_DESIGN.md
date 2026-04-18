# Hermes Agent 自进化架构深度分析

> 分析对象：[NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent)
> 分析时间：2026-04-18

---

## 目录

1. [整体架构概览](#1-整体架构概览)
2. [自进化技能系统](#2-自进化技能系统)
3. [可插拔内存架构](#3-可插拔内存架构)
4. [Honcho 用户建模](#4-honcho-用户建模)
5. [上下文管理与压缩](#5-上下文管理与压缩)
6. [持久化层与会话管理](#6-持久化层与会话管理)
7. [关键设计决策](#7-关键设计决策)
8. [多代理支持](#8-多代理支持)
9. [多平台交付](#9-多平台交付)
10. [总结与启示](#10-总结与启示)

---

## 1. 整体架构概览

Hermes Agent 是一个以自进化为核心设计理念的 AI 编程助手。其核心架构由以下层次构成：

```
┌─────────────────────────────────────────────┐
│              Delivery Platform               │
│  CLI / Telegram / Discord / Slack / WhatsApp │
├─────────────────────────────────────────────┤
│              Agent Core (run_agent.py)        │
│  ┌─────────────┬──────────────┬───────────┐ │
│  │ Context Eng │ Memory Mgmt  │ Skill Mgt │ │
│  │ (Engine)    │ (Manager)    │ (Manager) │ │
│  └─────────────┴──────────────┴───────────┘ │
├─────────────────────────────────────────────┤
│              Tooling Layer                   │
│  Skills / Memory Tools / Subagent Delegation │
├─────────────────────────────────────────────┤
│          Persistence Layer (SQLite + FTS5)   │
└─────────────────────────────────────────────┘
```

核心入口为 `run_agent.py`，它构建了 Agent 的主循环，协调上下文引擎、内存管理器和技能系统，并通过工具层实现与外部世界的交互。

---

## 2. 自进化技能系统

### 2.1 设计理念

技能系统是 Hermes Agent 实现"自进化"的核心机制之一。Skill 类似于人脑中的"程序性记忆"(procedural memory)——它记录的是**如何做任务**的知识，而非"知道什么"的事实性知识。

**关键文件：**
- `tools/skill_manager_tool.py`
- `agent/skill_utils.py`

### 2.2 技能的生命周期

Agent 可以通过 `skill_manager_tool.py` 自主完成技能的四种操作：

| 操作 | 说明 |
|------|------|
| `create` | 从经验中创建新技能 |
| `edit` | 编辑已有技能的内容或元数据 |
| `patch` | 对技能进行增量修改（打补丁） |
| `delete` | 删除不再需要的技能 |

### 2.3 技能格式

技能以 `SKILL.md` 文件形式存储，包含 YAML frontmatter 和 Markdown 正文：

```yaml
---
name: skill-name
description: What the skill does
version: 1.0
author: hermes
triggers:
  - trigger keyword 1
  - trigger keyword 2
---
# Skill Description

## Steps
1. Step one
2. Step two
...
```

### 2.4 技能存储

所有技能存储在 `~/.hermes/skills/` 目录下，每个技能一个目录，包含：
- `SKILL.md` — 技能定义主文件
- `references/` — 参考资料（可选）
- `templates/` — 模板文件（可选）
- `scripts/` — 辅助脚本（可选）
- `assets/` — 资源文件（可选）

### 2.5 安全扫描

Agent 创建的技能必须通过 `skills_guard` 安全扫描，防止注入恶意内容。这是一个关键的安全设计，确保自进化过程不会被滥用。

### 2.6 自我进化的意义

这一设计让 Agent 能够：
1. **积累经验**：在多次完成同类任务后，将最佳实践固化为技能
2. **优化流程**：随着使用不断调整和完善技能
3. **个性化**：不同用户的 Agent 会各自形成独特的技能库

---

## 3. 可插拔内存架构

### 3.1 架构概览

Hermes Agent 采用了高度模块化的内存管理设计，分为：

- **内置内存**：本地 SQLite 存储，提供基础的会话和消息持久化
- **外部内存提供者**：通过可插拔接口集成的跨会话记忆系统

**关键文件：**
- `agent/memory_manager.py` — 内存编排器
- `agent/memory_provider.py` — 内存提供者基类

### 3.2 内存提供者接口

```python
class MemoryProvider(BaseModel):
    """Memory provider base class"""
    name: str
    description: str
    enabled: bool = False
    
    def write_to_memory(self, ...):
        """Write memory data"""
        ...
    
    def read_from_memory(self, ...):
        """Read memory data"""
        ...
    
    def on_memory_write(self, ...):
        """Hook: built-in memory write triggers external provider sync"""
        ...
```

### 3.3 支持的内存提供商

| 提供商 | 说明 |
|--------|------|
| **Honcho** | 用户建模与跨会话画像 |
| **Hindsight** | 会话回顾与总结 |
| **Mem0** | 通用 AI 记忆系统 |
| **Supermemory** | 结构化知识管理 |
| **RetainDB** | 持久化记忆存储 |
| **OpenViking** | 开源记忆系统 |
| **ByteRover** | 智能记忆管理 |
| **Holographic** | 全息记忆模式 |

### 3.4 内存同步机制

`on_memory_write` 钩子是关键设计：当内置内存被写入时，自动触发外部内存提供者的同步，确保：
- 内置内存保存所有原始数据
- 外部内存提供增强智能（如用户建模）
- 两者保持一致性

### 3.5 内存注入策略

内存内容通过 `<memory-context>` fence 注入到 prompt 中，这样：
- 不破坏已有的 prompt cache
- 只在压缩/重建时才改变核心 prompt
- 保持缓存命中率最大化

---

## 4. Honcho 用户建模

### 4.1 核心能力

Honcho 是 Hermes Agent 中最复杂的记忆插件，提供 AI 原生的跨会话用户建模。

**关键文件：**
- `plugins/memory/honcho/README.md`
- `plugins/memory/honcho/__init__.py`

### 4.2 多轮辩证推理 (Multi-pass Dialectic Reasoning)

Honcho 采用双向辩证的推理模式：
- 用户代表视角 ↔ AI 代表视角
- 多轮迭代收敛，形成更全面准确的用户画像
- 动态推理级别（根据场景复杂度调整）

### 4.3 双层上下文注入

```
┌──────────────────────────────────────┐
│       Base Context (Always Active)   │
│  - User profile basics               │
│  - Project knowledge                 │
├──────────────────────────────────────┤
│    Dialectic Supplement (Conditional) │
│  - Active context from dialogue      │
│  - Session-specific insights         │
└──────────────────────────────────────┘
```

### 4.4 会话策略

Honcho 支持多种会话组织策略：

| 策略 | 说明 |
|------|------|
| `per-directory` | 每目录独立用户模型 |
| `per-repo` | 每仓库统一用户模型 |
| `per-session` | 每会话独立临时模型 |
| `global` | 全局统一用户模型 |

### 4.5 双向对等工具

Honcho 提供双向的对等作用户和 AI 表征构建工具，使得 Agent 能够：
- 理解用户的偏好、习惯和工作风格
- 调整自己的交互方式以更好地适应用户

---

## 5. 上下文管理与压缩

### 5.1 为什么需要上下文压缩

LLM 的上下文窗口有限，长对话会导致：
- 早期信息被挤出去
- 处理成本增加
- 缓存命中率下降

### 5.2 可插拔上下文引擎

Hermes Agent 设计了抽象的上下文引擎层，允许切换不同的压缩策略。

**关键文件：**
- `agent/context_engine.py`
- `agent/context_compressor.py`
- `plugins/` (存放额外引擎)

### 5.3 内置压缩引擎 (ContextCompressor)

内置压缩引擎提供以下核心功能：

1. **结构化摘要**：压缩后的摘要保留已解决/待解决问题列表
2. **Token 预算尾部保护**：确保压缩后的摘要仍在 token 预算内
3. **迭代式摘要更新**：逐步压缩，不丢失重要信息
4. **工具输出预修剪**：在 LLM 总结之前清理冗余的工具输出

### 5.4 上下文管理原则

Hermes 团队的核心原则是 **保守保持 prompt cache**：
- 每次交互时，仅保留 system prompt + 必要的压缩摘要
- 只在压缩/重建时才修改核心 prompt
- 最大化 prompt cache 命中率，降低成本

---

## 6. 持久化层与会话管理

### 6.1 数据存储

Hermes Agent 使用 SQLite 作为持久化存储后端，位于 `~/.hermes/`(可通过 `HERMES_HOME` 环境变量自定义)。

**关键文件：**
- `hermes_state.py`

### 6.2 数据库特性

| 特性 | 说明 |
|------|------|
| **WAL 模式** | 支持高并发读写 |
| **FTS5 全文搜索** | 在所有会话消息上支持全文检索 |
| **成本追踪** | 记录每个会话的 token 使用和 LLM 成本 |
| **推理链保存** | 跨会话保留推理链，确保上下文不中断 |
| **会话谱系** | 通过 `parent_session_id` 记录会话的压缩延续关系 |

### 6.3 会话管理流

```
Session A (原始)
  │
  ├── 压缩 → Session B (父: A)
  │       │
  │       ├── 压缩 → Session C (父: B)
  │       │
  │       └── 对话继续...
```

每次压缩会生成新的会话记录，但保留父会话 ID，确保完整的上下文谱系可追溯。

---

## 7. 关键设计决策

### 7.1 性能与成本优化

1. **激进使用 prompt caching**：核心 prompt 结构只在压缩/重建时改变
2. **Token 预算控制**：所有上下文操作都有 token 预算限制
3. **异步处理**：非关键路径操作异步执行

### 7.2 可配置性与隔离

1. **`HERMES_HOME` 环境变量**：支持完全的配置隔离
2. **插件化架构**：内存、上下文引擎均可插拔
3. **Profile 隔离**：不同用户使用独立的配置和数据

### 7.3 安全性

1. **Skills Guard**：Agent 创建的技能必须通过安全扫描
2. **沙箱执行**：技能脚本在沙箱环境中运行
3. **权限控制**：工具执行需要用户确认

---

## 8. 多代理支持

Hermes Agent 支持多代理协作，通过 subagent 委托机制：

```
主 Agent (Coordinator)
  │
  ├── Subagent A: 处理工具调用
  ├── Subagent B: 处理代码分析  
  └── Subagent C: 处理文档生成
```

主代理负责任务分解和结果聚合，子代理专注特定类型的任务，实现分工协作。

---

## 9. 多平台交付

Hermes Agent 支持多种交付平台，核心 Agent 引擎可无缝对接：

| 平台 | 状态 |
|------|------|
| **CLI (终端)** | 原生支持 |
| **Telegram** | Bot 集成 |
| **Discord** | Bot 集成 |
| **Slack** | Bot 集成 |
| **WhatsApp** | Bot 集成 |
| **Signal** | Bot 集成 |
| **其他** | 可扩展 |

多平台架构的设计使得 Agent 可以在不同场景下被使用，同时共享同一套核心能力（技能、记忆、上下文管理）。

---

## 10. 总结与启示

### 10.1 自进化三大支柱

Hermes Agent 的自进化能力由三大支柱支撑：

| 支柱 | 机制 | 效果 |
|------|------|------|
| **技能系统** | 自主创建/编辑/删除技能 | 程序性记忆，知识固化 |
| **内存架构** | 可插拔提供者 + 辩证推理 | 跨会话记忆，用户理解 |
| **上下文管理** | 智能压缩 + 缓存优化 | 长期对话能力，成本可控 |

### 10.2 对 TiDev 的启示

1. **技能系统**：TiDev 也可以设计类似的技能机制，让 Agent 积累经验、优化工作流
2. **用户建模**：借鉴 Honcho 的辩证推理，实现更深入的用户理解
3. **上下文管理**：激进使用 prompt caching 的策略值得借鉴
4. **插件化架构**：将内存、引擎等模块化，方便扩展
5. **安全设计**：Agent 自主创造内容时，必须有安全检查机制

### 10.3 核心架构图

```
                    ┌─────────────────┐
                    │   User Interface │
                    │                 │
                    └────────┬────────┘
                             │
                    ┌────────▼────────┐
                    │  Agent Core      │
                    │                  │
       ┌───────────┤  Skills ─────────┼───────────┐
       │           │  Memory ─────────┼───────────┤
       │           │  Context ────────┼───────────┤
       │           │  Tools ──────────┼───────────┤
       │           └────────┬────────┘           │
       │                    │                     │
┌──────▼──────┐    ┌───────▼────────┐     ┌──────▼──────┐
│ External    │    │  SQLite Store  │     │  LLM API    │
│ Memory      │    │  (FTS5 + WAL)  │     │  (Provider) │
│ Providers   │    └────────────────┘     └─────────────┘
│             │
│ Honcho /    │
│ Mem0 / ...  │
└─────────────┘
```

---

*文档生成时间: 2026-04-18*
*分析基于 hermes-agent v1.x (NousResearch/hermes-agent)*
