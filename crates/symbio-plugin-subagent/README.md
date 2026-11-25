---
kind: design
id: crates/symbio-plugin-subagent/README
title: 子智能体插件
status: active
created: 2026-07-20
impl: done
---

# 子智能体插件

> **目标形态**见 [DE-001 §12.3](../../docs/003-design/001-核心架构.md "DE-001-核心架构")
> 与 [DE-011](../../docs/003-design/011-Agent目录规范.md "DE-011-Agent目录规范")；
> 本文只写**当前实现**与落点，不复述目标裁决。

**子智能体域的唯一所有者**。两件事在同一处：**托管**（把 `{homedir}/subagent/<id>/` 挂成
与系统同构的 composite 插件树，注册经代理层并进系统树）、**门槛**（manifest 不合规范时
拒绝接入，绝不静默降级成「没有人格的通用助手」）。

本插件只负责把子树实例**装进来**（分形）；指令、记忆等能力由各自端口提供，本插件不持有
它们的实现细节。

## 职责

| 职责 | 机制 |
|---|---|
| agent 目录管理 | `impl VdfsProvider`（`<根>/subagent`：浏览 / 整包导入 / 导出 / 删除），目录自管（`AgentDirStore`：唯一来源是本插件自己的目录 `{homedir}/subagent`，与 `WORKDIR` 无关；zip-slip 防护、导入即校验） |
| 子树装配 | 会话选定子智能体（`ctx[AGENT_ID]`）时构造该目录的插件树并转发能力收集；注册经 `SubAgentVisitor` 加来源前缀（`agent/<id>/…`），与系统树**并集**且不撞名 |
| 选项贡献 | `traverse(agent/available_options)` 提供会话页的「子智能体」选择项 |

委托工具是会话级原语，本插件不持有它的注册或执行。本插件只提供地址面：经 VDFS 视图列
`subagent` 挂载点得到 `agent_id` 候选、执行期以 `Stat subagent/<id>` 校验存在性。

### 为什么托管与门槛必须在同一个插件里

「装进来的子智能体」只有一条链：**扫目录 → 校验 manifest → 构造子树 → 收集能力**。
门槛能落的位置就是这条链本身——拆开就变成「一处解释目录、一处判断能不能接」，
两处各持一半判据，而「不合规就拒绝接入，绝不静默降级」这句承诺不再由结构保证，
只能靠人记得同步。三段现在各在一处、判据同源：规则本体 `host/manifest.rs::validate`、
接入判据的唯一出处 `host/store.rs::load_record`、装配期错配的报错
`host/plugin.rs`（`ctx[AGENT_ID]` 与子树不匹配时经 `capability_report_error` 出声）。

解释 agent 目录内部（`prompts/` `skills/` `mcps/` 各有布局、条目按 `priority` 排序、
MCP 配置按约定文件名探测）等于宿主替技能系统与 MCP 客户端**再写一遍解析**，两条链必然
长期不同步（规范 §3.2 第 2 条）。Agent 因此是**一棵插件树**：能力由目录里的插件实例
自己解释，本插件只做枚举 / 建目录 / 导入导出，挂载点内**没有任何保留名**。

### 子 Agent 的默认插件清单

子树挂**与父 Agent 同构的默认插件集**（见装配方的 `ASSEMBLY_SUB_AGENT_PLUGINS`）——与系统侧
是**同一份清单**：`home` 的 `SYSTEM_PLUGINS` 直接取这个常量，**不是第二份手抄**。清单内容不在此
复述（复述即重复，改一次要动两处），要看得去常量定义或 `docs/004-ref/006-当前事实表.md` §1。
下面只写**为什么在列**——每个名字的作用域从哪儿来：

- `memory` 在其中：子智能体自己的 `MEMORY.md` 就躺在它的目录里，由子树里那个实例读写与
 注入；系统树那份管 `{homedir}/MEMORY.md`，**同一个插件的两个实例**，不是两份实现。
- `setting` 在其中：智能体自身的信息设置与指令 / 人格（`AGENTS.md` 只读注入）同样分形，
 系统树与每棵子树各有一份。
- `plugin_manager` 在其中：`SubAgentVisitor` 把它前缀到 `agent/<id>/plugin_manager`，子 Agent 页因此有了
 设置入口，与父 Agent 对齐。
- `subagent` 在其中：子 Agent 也能在其目录内再挂子 Agent（`<id>/subagent/<sub-id>` 递归）——分形。
- `model` 在其中：子智能体有自己的模型服务。子树会话收集能力时以**子容器**为 parent，
 子树 `model` 实例注册进该次收集自己的管理器——子会话用子智能体自己解析的模型；
 父会话收集期，这个注册才被 `SubAgentVisitor` 丢弃（单槽，防子树模型劫持父会话）。
- `vdfs` 在其中，但它的**注册**经 `SubAgentVisitor` 在每一层丢弃：VDFS 根是系统级单槽，
 归系统 Agent 独占。子树照样**构造**出自己的 `vdfs` 实例（所以在清单里），只是那条注册不生效。

⚠️ 子树的 `WORKDIR` **不得**覆写成 Agent 目录：那会让 `memory` 的工作区记忆腿与智能体腿
读到同一份文件（双重注入），且「工作」这个名字与它承载的作用域名实不符。`setting` 的
工作区指令腿同样认 `{workdir}`，覆写也会双重注入。子树继承父会话的 `WORKDIR`
（规范附录 A.3 的同一条警告）。

## 寻址（无自有路由，权威登记见 [路由参考](../../docs/004-ref/003-ROUTES.md "RE-003-ROUTES") §子智能体插件）

agent 目录的寻址是**挂载点语义**而非平铺三段（`route` 直接返回 `NotFound` 并指引到 VDFS）：

- `subagent/<id>` 是挂载点；钻进它即**委托给子 composite 的 `CompositeVfs`**（与系统根
  分形同构，而非把子智能体资源并集进系统树的三段地址）。
- 钻进后的清单由子 composite 的 `list("")` 返回自身子目录（session / model / mcp /
 skill / plugin_manager / …，经 `root_hidden` 过滤后的可见项），地址**递归**为
 `subagent/<id>/<子目录>/<相对路径>`，没有固定的「子类别标签」三层。
- 两条链路要分清：**LLM 链路**的子智能体能力经 `SubAgentVisitor` 加 `agent/<id>/`
 前缀并集进系统树（前缀可见）；**系统 / 前端链路**走挂载点穿越（`sub_agent(id)`
 的 `get_vfs_provider`），返回的相对地址用 `subagent/<id>` 提回全局路径。

agent id 的语法门槛（首字符必须是小写字母或数字）由规范 §5.1 定，准入判据落在
`host/manifest.rs::validate`——本插件因此**不可能**与名为 `AGENTS.md` 的 agent 目录相撞。

## 本插件的边界

本插件只经端口与地址面提供子智能体目录的枚举、装配与校验，不描述、不假设其它能力的实现。
规范落点：

- Agent 目录规范：`docs/003-design/011-Agent目录规范.md`
- VDFS 机制（`<根>/subagent` 挂载点由本插件自持 `impl VdfsProvider`）：`docs/003-design/009-VDFS规范.md` §13.4
