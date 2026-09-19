# Agent 插件

**智能体域的唯一所有者**。三件事在同一处：**托管**（把 `<homedir>/agent/<id>/` 挂成
与系统同构的 composite 插件树，注册经代理层并进系统树）、**门槛**（manifest 不合规范时
拒绝接入，绝不静默降级成「没有人格的通用助手」）、**智能体自身的 `AGENTS.md`**
（系统态与子智能体态两个作用域）。

## 职责

| 交出什么 | 机制 |
|---|---|
| Bundle 资产管理 | `impl VdfsProvider`（`<根>/agent`：浏览 / 整包导入 / 导出 / 删除），目录自管（`BundleStore`：工作区级 + 全局级双层） |
| 子树装配 | 会话选定智能体（`ctx[AGENT_ID]`）时构造该目录的插件树并转发能力收集；注册经 `SubAgentVisitor` 加来源前缀（`agent/<id>/…`），与系统树**并集**且不撞名 |
| 智能体自身的 `AGENTS.md` | `CapabilityVisitor::register_system_prompt`（注入）+ `<根>/agent/…`（编辑） |
| 工具贡献 | `traverse(agent/available_tools)` 把 `agent_run`（子智能体委托）加入会话工具集 |
| 选项贡献 | `traverse(agent/available_options)` 提供会话页的「智能体」选择项 |

### 为什么三件事必须在同一个插件里

「拥有智能体」包含同源的三件事：**智能体库**（bundle）、**智能体的装配**（插件树）、
**智能体自身的指令**（`AGENTS.md`）。指令文件就躺在被扫描、被装配、被整包浏览的那些
目录里——读写面与注入面落在同一个所有者上，`谁能读写它，谁负责注入它` 这条原则才完整。

> 这件事曾经散在别处：`{homedir}/AGENTS.md` 由 `session` 临时读一遍注入（叫「全局指令」，
> 只读、无地址、无容量），`<agentdir>/AGENTS.md` 则由子树里那个被宿主改指了作用域的
> `work` 实例混在「工作区记忆」名下注入（还带着工作区的地址）。也一度试图整体收进
> `setting`——但 `setting` 是**设置页的入口**（自有分区 + 各插件配置清单），
> 不是任何内容文件的所有者。

### 子 Agent 的必需插件清单

`mcp`（工具）+ `skill`（技能）——**只放解释能力资产的插件**。

- ⚠️ **`work` 不在其中**：`work` 的作用域是 `ctx[WORKDIR]`，即**工作区**记忆
  （`{workdir}/AGENTS.md`）。它的实例挂进子树只会与系统侧那个实例读同一份文件、
  注入同一段内容；v2 早期把子树 `WORKDIR` 覆写成 Agent 目录，那是**错的**——
  `work` 只负责工作区信息。
- ⚠️ **`setting` 也不在其中**：它在子树里既没有挂载点、也没有可声明的配置（注册会
  串味，见 `../setting/plugin.rs`），无事可做。设置入口是**系统层**的事。

宿主的 `archive_retired_work_tree` 会把旧装配留下的那个 `<agentdir>/work/PLUGIN.yml`
改名为 `.disabled`（= 卸载，幂等且可逆）。

## 智能体自身的 `AGENTS.md`（两个作用域）

```text
作用域        物理落位                      可编辑地址
系统智能体    {homedir}/AGENTS.md          <根>/agent/AGENTS.md
子智能体      <agentdir>/AGENTS.md         <根>/agent/<bundle id>/AGENTS.md
```

| | 系统态 | 子智能体态 |
|---|---|---|
| 归属 | 宿主应用级设置（用户可编辑） | bundle 自带资产（随包分发） |
| 生效 | 所有会话 | 选中该智能体时 |
| 模块 | `host/instruction.rs` | `host/memory.rs` |
| 片段标题 | 【全局指令】 | 【智能体记忆】 |
| 片段里的地址 | 挂载根下那个文件 | 整包浏览里那个文件 |

两个作用域的读写、两道闸门、片段排版、节点形状**共用内核**（`symbio_core::memory`）；
本插件只提供「落位 + 标题 + 地址 + 空提示 + 闸门取值」。片段里的地址与闸门都是
**本插件自己会执行的**，因此印出来的数字是真的。

⚠️ 挂载根下的 `AGENTS.md` 是**保留名**（本应用自身的指令，不是名为它的 bundle）——
两者不可能相撞：bundle id 首字符必须是小写字母或数字（§5.1），保留名以大写 `A` 开头。

⚠️ **`BundleStore` 不持有记忆的读写与闸门**：它只回答「记忆文件在哪」（`memory_path`）。
此前它自带 `read_memory` / `write_memory` 与自己的字节闸门，与 work / session 两层各写一份
口径——「超限是拒绝还是截断」「读不到算不算错误」一旦分叉，用户看到的行为就会随
「这条记忆属于哪一层」而变化。收口后各层共用同一份实现。

**不可删除**：要清空就写入空内容（两个作用域同一约定）。

## 路由

**agent 插件没有任何自有路由**：`route()` 直接返回 `NotFound` 并指引到 VDFS。
bundle 及其内部（提示词 / 技能 / MCP）一律经 `<根>/agent/<id>/<子类别标签>/<相对路径>` 寻址。

## 关联

- 会话编排与系统提示词的拼接 / 消费：`../session/README.md`
- 另外几层记忆：`../work/README.md`（工作区）、`../session/README.md`（会话）、
  `../setting/README.md`（设置入口，**不拥有任何记忆文件**）
- 记忆内核（各层共用）：`symbio_core::memory`
- Agent 目录规范：`docs/design/agent-directory-spec.md`
- VDFS 机制（`<根>/agent` 挂载点由本插件自持 `impl VdfsProvider`）：`docs/design/vdfs.md` §13.4
