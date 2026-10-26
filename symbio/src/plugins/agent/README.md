---
kind: design
id: symbio/src/plugins/agent/README
title: Agent 插件
status: active
created: 2026-07-20
impl: done
---

# Agent 插件

> **目标形态**见 [DE-001 §12.3](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")
> 与 [DE-011](../../../../docs/003-design/011-Agent目录规范.md "DE-011-Agent目录规范")；
> 本文只写**当前实现**与落点，不复述目标裁决。

**外部智能体域的唯一所有者**。两件事在同一处：**托管**（把 `{homedir}/agent/<id>/` 挂成
与系统同构的 composite 插件树，注册经代理层并进系统树）、**门槛**（manifest 不合规范时
拒绝接入，绝不静默降级成「没有人格的通用助手」）。

**当前智能体自身的 `AGENTS.md` 不归本插件**：记忆的唯一所有者是
[memory](../memory/README.md "symbio/src/plugins/memory/README")（[D-040](../memory/README.md "symbio/src/plugins/memory/README")：智能体腿 +
工作区腿两个作用域同住 `<根>/memory`）。本插件只负责把子树里那个 `memory` 实例**装进来**
（分形），为什么这样切见 §记忆不在本插件。

## 职责

| 交出什么 | 机制 |
|---|---|
| agent 目录管理 | `impl VdfsProvider`（`<根>/agent`：浏览 / 整包导入 / 导出 / 删除），目录自管（`AgentDirStore`：唯一来源是本插件自己的目录 `{homedir}/agent`，与 `WORKDIR` 无关；zip-slip 防护、导入即校验） |
| 子树装配 | 会话选定智能体（`ctx[AGENT_ID]`）时构造该目录的插件树并转发能力收集；注册经 `SubAgentVisitor` 加来源前缀（`agent/<id>/…`），与系统树**并集**且不撞名 |
| 工具贡献 | `traverse(agent/available_tools)` 把 `agent_run`（子智能体委托）加入会话工具集 |
| 选项贡献 | `traverse(agent/available_options)` 提供会话页的「智能体」选择项 |

### 为什么托管与门槛必须在同一个插件里

「装进来的智能体」只有一条链：**扫目录 → 校验 manifest → 构造子树 → 收集能力**。
门槛能落的位置就是这条链本身——拆开就变成「一处解释目录、一处判断能不能接」，
两处各持一半判据，而「不合规就拒绝接入，绝不静默降级」这句承诺不再由结构保证，
只能靠人记得同步。三段现在各在一处、判据同源：规则本体 `host/manifest.rs::validate`、
接入判据的唯一出处 `host/store.rs::load_record`、装配期错配的报错
`host/plugin.rs`（`ctx[AGENT_ID]` 与子树不匹配时经 `capability_report_error` 出声）。

解释 agent 目录内部（`prompts/` `skills/` `mcps/` 各有布局、条目按 `priority` 排序、
MCP 配置按约定文件名探测）等于宿主替技能系统与 MCP 客户端**再写一遍解析**，两条链必然
长期不同步（规范 §3.2 第 2 条）。Agent 因此是**一棵插件树**：能力由目录里的插件实例
自己解释，本插件只做枚举 / 建目录 / 导入导出，挂载点内**没有任何保留名**。

### 记忆不在本插件：谁能读写它，谁负责注入它

智能体的指令文件就躺在被扫描、被装配、被整包浏览的那些目录里，「顺手一起管」最省事——
但那样读写面与注入面就分在两个插件手里：`agent` 持有地址，`memory` 注入正文，
`谁能读写它，谁负责注入它` 这条原则当场破掉。而智能体腿与工作区腿本是**同一种东西的
两个作用域**（同样的读写、同样的两道容量闸门、同样的片段排版），跨插件复述就会分叉成
「超限是拒绝还是截断」随这条记忆属于哪一层而变。

[D-040](../memory/README.md "symbio/src/plugins/memory/README") 的切法因此是按**归属**划：两份 `AGENTS.md`
的落位、可编辑地址、两道闸门与空文件约定都归 `memory` 自己写
（[Memory 插件](../memory/README.md "symbio/src/plugins/memory/README") 与
[006 规范附录 A.3](../../../../docs/003-design/011-Agent目录规范.md "DE-011-Agent目录规范")）。
本插件交出的是**分形**：子树里那个 `memory` 实例的父目录恰好就是 agent 目录，
落位因此天然正确，谁也不必认识「系统智能体」与「子智能体」这两个词。

### 子 Agent 的默认插件清单

子树挂**与父 Agent 同构的默认插件集**（见装配方的 `ASSEMBLY_SUB_AGENT_PLUGINS`）——与系统侧
是**同一份清单**：`home` 的 `SYSTEM_PLUGINS` 直接取这个常量，**不是第二份手抄**。清单内容不在此
复述（复述即重复，改一次要动两处），要看得去常量定义或 `docs/004-ref/006-当前事实表.md` §1。
下面只写**为什么在列**——每个名字的作用域从哪儿来：

- `memory` 在其中：子智能体自己的 `AGENTS.md` 就躺在它的目录里，由子树里那个实例读写与注入；
 系统树那份管 `{homedir}/AGENTS.md`，**同一个插件的两个实例**，不是两份实现。
- `setting` 在其中：智能体自身的信息设置同样分形，系统树与每棵子树各有一份。
- `plugin_manager` 在其中：`SubAgentVisitor` 把它前缀到 `agent/<id>/plugin_manager`，子 Agent 页因此有了
 设置入口，与父 Agent 对齐。
- `agent` 在其中：子 Agent 也能在其目录内再挂子 Agent（`<id>/agent/<sub-id>` 递归）——分形。
- `model` 在其中：子智能体有自己的模型服务。子树会话收集能力时以**子容器**为 parent，
 子树 `model` 实例注册进该次收集自己的管理器——子会话用子智能体自己解析的模型；
 父会话收集期，这个注册才被 `SubAgentVisitor` 丢弃（单槽，防子树模型劫持父会话）。
- `vdfs` 在其中，但它的**注册**经 `SubAgentVisitor` 在每一层丢弃：VDFS 根是系统级单槽，
 归系统 Agent 独占。子树照样**构造**出自己的 `vdfs` 实例（所以在清单里），只是那条注册不生效。

⚠️ 子树的 `WORKDIR` **不得**覆写成 Agent 目录：那会让 `memory` 的工作区腿与智能体腿
读到同一份文件（双重注入），且「工作」这个名字与它承载的作用域名实不符。
子树工作区腿继承父会话的 `WORKDIR`（规范附录 A.3 的同一条警告）。

## 寻址（无自有路由，权威登记见 [路由参考](../../../../docs/004-ref/003-ROUTES.md "RE-003-ROUTES") §Agent 插件）

agent 目录的寻址是**挂载点语义**而非平铺三段（`route` 直接返回 `NotFound` 并指引到 VDFS）：

- `agent/<id>` 是挂载点；钻进它即**委托给子 composite 的 `CompositeVfs`**（与系统根
 分形同构，而非把子智能体资源并集进系统树的三段地址）。
- 钻进后的清单由子 composite 的 `list("")` 返回自身子目录（session / model / mcp /
 skill / plugin_manager / …，经 `root_hidden` 过滤后的可见项），地址**递归**为
 `agent/<id>/<子目录>/<相对路径>`，没有固定的「子类别标签」三层。
- 两条链路要分清：**LLM 链路**的子智能体能力经 `SubAgentVisitor` 加 `agent/<id>/`
 前缀并集进系统树（前缀可见）；**系统 / 前端链路**走挂载点穿越（`sub_agent(id)`
 的 `get_vfs_provider`），返回的相对地址用 `agent/<id>` 提回全局路径。

agent id 的语法门槛（首字符必须是小写字母或数字）由规范 §5.1 定，准入判据落在
`host/manifest.rs::validate`——本插件因此**不可能**与名为 `AGENTS.md` 的 agent 目录相撞。

## 关联

- 会话编排与系统提示词的拼接 / 消费：`../session/README.md`
- 记忆的另一个所有者（智能体腿 + 工作区腿，两作用域同住一个挂载点）：`../memory/README.md`
- 设置入口（**不拥有任何记忆文件**）：`../plugin_manager/README.md`
- 记忆的共享实现（各层共用）：`providers/memory`
- Agent 目录规范：`docs/003-design/011-Agent目录规范.md`
- VDFS 机制（`<根>/agent` 挂载点由本插件自持 `impl VdfsProvider`）：`docs/003-design/009-VDFS规范.md` §13.4
