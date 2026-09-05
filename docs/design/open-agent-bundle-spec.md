# Open Agent Bundle（OAB）规范 v1

> 状态：草案 v1
> 协议标识：`oab/v1`
> 本文档是 OAB 的**规范性定义**：目录约定、清单格式、能力单元格式、装配语义与一致性要求。
>
> **本文档与任何具体实现无关。** 它不描述任何宿主的内部机制、模块划分或代码结构，
> 也不引用任何宿主专有的 API 或标识符。任何组织、任何语言、任何框架都可以仅依据
> 本文档实现 OAB 的**生产方**（生成 bundle）或**消费方**（宿主）。

### 本规范中的要求级别

关键词「**必须**（MUST）」「**不得**（MUST NOT）」「**应当**（SHOULD）」
「**可以**（MAY）」按 RFC 2119 的语义解释。

---

## 1. 范围

**OAB（Open Agent Bundle）定义一种把「完整的 agent」作为文件包分发的开放格式。**

一个 bundle 是一个自包含目录（或该目录的 zip 归档），携带组成一个 agent 所需的全部
声明：人格与规则、技能、工具来源，以及身份、兼容性、权限等元信息。

| 维度 | Skill / MCP（既有行业方案） | OAB |
|---|---|---|
| 交付物 | 单个能力（一段提示词 / 一组工具） | **完整的 agent** |
| 形态 | 松散文件 / 远程服务 | **自包含目录包**，可 zip 分发、可版本化 |
| 装配 | 消费方自行拼装 | **消费方按约定目录装配**，行为一致 |

### 1.1 设计原则：只定义一个新约定

OAB 刻意**不重新发明**已有行业标准的东西。三个能力来源中，两个直接复用行业标准，
只有一个是 OAB 新增：

| 目录 | 性质 | 贡献 |
|---|---|---|
| `skills/…` | **行业标准**（SKILL 格式，原样兼容） | 提示词片段 |
| `mcps/…` | **行业标准**（MCP server 配置，原样透传） | 工具 |
| `prompts/…` | **OAB 新增** | 提示词片段，**直接追加进系统提示词** |

新增 `prompts/` 的理由：既有行业标准中**不存在「无条件追加并影响系统提示词」的文件约定**——
MCP 的 prompts 原语只是可被客户端拉取的模板，不会自动进入系统提示词；
SKILL 则是「按需取用的手册」语义。OAB 需要一个承载「人格 / 全局规则 / 常驻工作流」的位置。

### 1.2 不在本规范范围内

以下内容由**消费方自行决定**，本规范不作规定：

- 系统提示词的注入机制（拼接后的文本如何进入模型上下文）；
- MCP server 的启动方式、传输载体与进程隔离策略（stdio / HTTP / 进程内等）；
- bundle 的存储位置、发现方式与管理界面；
- 权限的强制执行机制（本规范只定义**声明格式**，见 §8）。

这些事项在不同消费方之间天然不同，把它们写进规范只会损害通用性。

---

## 2. 术语

- **Bundle**：一个可分发的 agent 包（目录或 zip 归档）。
- **能力单元（Capability Unit）**：bundle 内一个能力提供者，由**约定目录中的一个条目**承载。
- **生产方（Producer）**：生成符合本规范的 bundle 的一方。
- **消费方（Consumer / 宿主）**：加载并按本规范装配 bundle 的一方。
- **装配（Assembly）**：消费方扫描约定目录、解析条目、合并产出的过程。
- **系统提示词片段（Prompt Fragment）**：装配产出的、用于追加进系统提示词的文本块。

---

## 3. 一致性要求

### 3.1 Bundle（生产方）

符合本规范的 bundle **必须**满足：

1. 根目录存在 `manifest.yaml`（或 `.yml` / `.json`），且符合 §5；
2. 目录结构符合 §4，能力单元格式符合 §6；
3. `prompts/`、`skills/`、`mcps/` 三者**至少有一个非空**；
4. 所有资源引用**不得**指向 bundle 目录之外。

### 3.2 消费方（宿主）

符合本规范的消费方 **必须**满足：

1. **必须**扫描 `prompts/`、`skills/`、`mcps/` 三个约定目录（§7.1）；
2. **必须**按 §7.2 解析各条目，并按 §7.4 以 `priority` 升序合并提示词片段；
3. **必须**把合并得到的提示词文本追加进系统提示词（**具体注入机制由消费方自定**，§1.2）；
4. **必须**把 `mcps/` 中声明的 MCP server 交给自己的 MCP 客户端启动，使其工具对模型可用；
   若消费方不具备 MCP 客户端能力，**应当**如实告警而非静默忽略（§7.5）；
5. **必须**执行 §9 的版本门槛校验，不匹配时拒绝加载；
6. **必须**实现 §10 的安全约束（路径穿越防护）。

消费方 **不得**要求 bundle 声明任何超出本规范的、消费方专有的字段才能工作。

---

## 4. Bundle 目录结构

```text
<bundle>/                       ← 目录名即 bundle 实例名（zip 解压后即此形态）
├── manifest.yaml               ← 必需。唯一配置文件（.yaml / .yml / .json）
├── prompts/                    ← 约定目录（OAB 原生）。系统提示词片段
│   └── persona.md
├── skills/                     ← 约定目录（行业 SKILL 标准）
│   └── playbook/SKILL.md
├── mcps/                       ← 约定目录（行业 MCP 标准）。工具来源
│   └── github.yaml
├── assets/                     ← 可选。bundle 级公共资源
│   └── icon.svg
├── README.md                   ← 推荐
└── LICENSE                     ← 推荐
```

### 4.1 约定优于配置

能力单元**不需要在任何清单中登记**——目录里「有对应条目就表示已安装」。
`manifest.yaml` 只承载**无法从目录推导的信息**：身份、兼容性门槛、权限声明、实例配置。

### 4.2 目录命名

三个约定目录**统一使用复数形式**（`prompts/` `skills/` `mcps/`），与行业习惯一致
（技能放 `skills/` 是既成事实标准）。

| 约定目录 | 条目 → 能力单元 | 条目 id |
|---|---|---|
| `prompts/<name>.md` | 一个 Markdown 文件一个提示词片段 | 文件名（去扩展名） |
| `skills/<name>/SKILL.md` | 一个子目录一个技能 | 子目录名 |
| `mcps/<name>…` | 一个文件或目录一组 MCP server 配置 | 文件/目录名 |

### 4.3 路径根规则

条目内引用的相对路径一律以 **bundle 根目录**为基准。

---

## 5. manifest 规格

```jsonc
{
  // ── 身份 ──
  "spec": "oab/v1",                          // 必需，固定 "oab/v1"
  "id": "com.acme.code-reviewer",            // 必需。全局唯一 id，建议反向域名风格
  "name": "代码评审专家",                     // 必需。展示名
  "version": "1.2.0",                        // 必需。semver
  "description": "以资深评审视角审查代码……",     // 推荐
  "authors": ["acme@example.com"],           // 可选
  "license": "MIT",                          // 推荐
  "icon": "assets/icon.svg",                 // 可选，bundle 内相对路径

  // ── 兼容性声明 ──
  "requires": {
    "spec": "^1"                             // 消费方必须支持的 OAB 主版本
  },

  // ── 权限声明（见 §8）──
  "permissions": {
    "network": { "domains": ["api.github.com"] },
    "fs": { "read": ["assets/**"] },
    "limits": { "max_prompt_bytes": 65536 }
  },

  // ── 包级配置：消费方可据此生成实例配置表单 ──
  "config": {
    "schema": { "type": "object", "properties": { "strictness": { "type": "string" } } },
    "defaults": { "strictness": "high" }
  }
}
```

### 5.1 字段规则

| 字段 | 要求 |
|---|---|
| `spec` | **必须**为 `"oab/v1"` |
| `id` | **必须**。以 `[a-z0-9]` 开头，仅含小写字母/数字/`.`/`-`/`_`，长度 1–128 |
| `name` | **必须**。展示名 |
| `version` | **必须**。semver 字符串 |
| `requires.spec` | **必须**。形如 `^N` 或 `N`，见 §9 |
| `icon` | bundle 内相对路径，**不得**指向 bundle 外 |
| `permissions.fs.read\|write` | glob 模式，相对 bundle 根，**不得**含 `..` |
| `permissions.network.domains` | 精确域名或 `*.example.com` 通配 |
| 未知字段 | **必须**允许存在并忽略（向前兼容） |

> manifest **不得**声明 `providers`、`engine`、`tools` 之类重复登记目录事实的字段——
> 能力单元由约定目录承载（§4.1）。

---

## 6. 能力单元规格

### 6.1 `prompts/<name>.md` —— 系统提示词片段（OAB 原生）

文件正文**无条件追加进系统提示词**，用于承载人格、全局规则、常驻工作流。

- 文件名即条目 id（`persona.md` → `persona`）；仅识别 `.md` / `.markdown`；
- 可选的 YAML frontmatter **可以**携带 `priority`，**必须**在装配时剥离、不进入正文；
- 正文**可以**使用 `$bundle.id` / `$bundle.name` / `$bundle.version` 模板变量，
  由消费方在装配期渲染。

```markdown
---
priority: 0
---

你是「$bundle.name」（bundle: $bundle.id v$bundle.version），一位资深……
```

`priority` 语义：升序装配，数值小的在前；`0` 保留给「身份锚定」；缺省为 `10`。

> `prompts/` 与 `skills/` 在装配期都归约为提示词片段，区别只在**默认优先级**与
> **作者意图**：prompt（默认 10）是常驻指令，skill（默认 50）是行业手册。

### 6.2 `skills/<name>/SKILL.md` —— 技能（行业标准）

与行业通行的 SKILL 格式**完全兼容**，无需改造即可放入 bundle：

```markdown
---
name: delivery-playbook
description: 交付流程手册……
---

# 正文（装配时作为提示词片段，priority 默认 50）
```

消费方**应当**沿用 SKILL 标准中已定义的 frontmatter 字段（如 `name`、`description`）。

### 6.3 `mcps/<name>…` —— MCP server（行业标准，工具来源）

- 文件形态：`mcps/<name>.yaml` | `.yml` | `.json`，内容为**行业标准的 MCP server 配置对象**；
- 目录形态：`mcps/<name>/` 内含 `config.yaml`（或 `server.yaml` / `mcp.json` 等）；
- 配置对象**原样**交给消费方的 MCP 客户端，本规范不重新定义其字段。

**工具来源的唯一途径是 MCP。** 本规范不定义任何内置工具执行器，`manifest` 与
约定目录中均**不得**出现用于指名消费方内置执行器的字段。

### 6.4 失败语义

单个条目解析失败是**软故障**：记一条诊断信息并跳过该条目，bundle 其余部分照常装配。
manifest 缺失或校验失败是**硬错误**：拒绝加载整个 bundle（§9）。

---

## 7. 装配语义

装配是**消费方的责任**；本规范只定义输入（目录约定）与必须遵守的装配规则。

### 7.1 发现

扫描 `prompts/`、`skills/`、`mcps/`，逐条目解析。目录不存在即视为该来源为空。

### 7.2 解析

| 条目 | 产出 |
|---|---|
| `prompts/<n>.md` | 剥离 frontmatter → 渲染 `$bundle.*` → 提示词片段（`priority` 缺省 10） |
| `skills/<n>/SKILL.md` | 正文作为提示词片段（`priority` 缺省 50） |
| `mcps/<n>…` | MCP server 配置（交给消费方 MCP 客户端） |

### 7.3 失败策略

见 §6.4：条目级软故障，bundle 级硬错误。软故障**不得**中断会话。

### 7.4 合并

提示词片段按 `(priority, 来源标识)` **升序**拼接，段间以空行分隔；
数值相同时按来源标识字典序，以保证**跨消费方结果确定**。

拼接结果**必须**追加进系统提示词。当总字节数超过 `permissions.limits.max_prompt_bytes`
时，从**最低优先级**一端整段丢弃。

### 7.5 MCP server 处理

消费方**必须**把 §7.2 收集到的 MCP server 配置交给自己的 MCP 客户端，使其工具对模型可用。
若消费方不具备 MCP 客户端能力，**应当**记录明确告警（每条 server 至少一条），
**不得**静默丢弃声明。

---

## 8. 权限声明模型

OAB 采用**声明式权限**：bundle 声明其能力所需的权限上限，作为消费方侧的策略输入与审计依据。

| 维度 | 字段 | 含义 |
|---|---|---|
| 网络 | `permissions.network.domains` | 允许出网的域名白名单；空 = 不得出网 |
| 文件系统 | `permissions.fs.read\|write` | 相对 bundle 根的 glob；**不得**含 `..` |
| 限额 | `permissions.limits` | `max_prompt_bytes` 等资源上限 |

**执行机制不在本规范范围内**（§1.2）：真实沙箱（进程隔离、网络强制、写保护）
由消费方按其自身安全模型实施。本规范只规定声明格式，避免在不同消费方之间强加
不一致的运行时。

---

## 9. 版本化与接入判定

### 9.1 版本号体系

| 层 | 标识 | 说明 |
|---|---|---|
| 协议版本 | `spec: "oab/vN"` | **匹配判定的基准** |
| bundle 版本 | `version`（semver） | 仅信息性，不参与匹配 |

### 9.2 版本门槛（硬性）

1. bundle 在 `requires.spec` 声明其依赖的协议主版本（如 `"^1"`）；
2. 消费方在**加载期**做严格匹配：`^N` 与消费方支持的主版本**相等**方可接入；
3. **不匹配必须拒绝接入**，且**不得**静默降级为「无人格的通用助手」；
   错误信息**必须**写明双侧版本；
4. v1 仅支持 `^N` 与精确 `N` 两种形式。同一主版本内新增字段**必须**全部可选（向后兼容），
   因此 minor / patch 不参与匹配。

---

## 10. 安全考虑

1. **路径穿越**：bundle 内所有路径引用**必须**被解析并限制在 bundle 目录内。
   消费方在解包 zip 时**必须**逐条目校验规范化路径落在目标目录内。
2. **提示注入**：`prompts/` 与 `skills/` 的内容会进入系统提示词。消费方**应当**向用户
   明示 bundle 来源，并对来自不可信第三方的 bundle 保持警惕。
3. **工具风险**：工具实际由 MCP server 提供，其隔离与审计由消费方的 MCP 客户端策略决定。
4. **资源限额**：消费方**应当**依据 `permissions.limits` 约束提示词规模等资源占用。

---

## 11. 打包与分发

- bundle 目录整体 zip 后即一个可分发的包（扩展名 `.oab`）；
- zip 顶层为 `manifest`，**或**唯一一级目录下含 `manifest`——两种布局消费方**均须接受**；
- 解包后即 §4 的目录形态。

---

## 附录 A：最小可用 bundle

```text
com.acme.helper/
├── manifest.yaml
└── prompts/
    └── persona.md
```

`manifest.yaml`：

```yaml
spec: "oab/v1"
id: "com.acme.helper"
name: "小助手"
version: "1.0.0"
requires:
  spec: "^1"
```

`prompts/persona.md`：

```markdown
---
priority: 0
---

你是「$bundle.name」，一个简洁、直接的助手。
```

该 bundle 只含一个 `prompts/` 条目，满足 §3.1 的「至少一个来源非空」，
是一个符合本规范的最小 bundle。
