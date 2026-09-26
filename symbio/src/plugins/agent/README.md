# Agent 插件

**智能体域的唯一所有者**。三件事在同一处：**托管**（把 `<homedir>/agent/<id>/` 挂成
与系统同构的 composite 插件树，注册经代理层并进系统树）、**门槛**（manifest 不合规范时
拒绝接入，绝不静默降级成「没有人格的通用助手」）、**智能体自身的 `AGENTS.md`**
（系统态与子智能体态两个作用域）。

## 职责

| 交出什么 | 机制 |
|---|---|
| agent 目录管理 | `impl VdfsProvider`（`<根>/agent`：浏览 / 整包导入 / 导出 / 删除），目录自管（`AgentDirStore`：工作区级 + 全局级双层） |
| 子树装配 | 会话选定智能体（`ctx[AGENT_ID]`）时构造该目录的插件树并转发能力收集；注册经 `SubAgentVisitor` 加来源前缀（`agent/<id>/…`），与系统树**并集**且不撞名 |
| 智能体自身的 `AGENTS.md` | `CapabilityVisitor::register_system_prompt`（注入）+ `<根>/agent/…`（编辑） |
| 工具贡献 | `traverse(agent/available_tools)` 把 `agent_run`（子智能体委托）加入会话工具集 |
| 选项贡献 | `traverse(agent/available_options)` 提供会话页的「智能体」选择项 |

### 为什么三件事必须在同一个插件里

「拥有智能体」包含同源的三件事：**智能体库**（agent 目录）、**智能体的装配**（插件树）、
**智能体自身的指令**（`AGENTS.md`）。指令文件就躺在被扫描、被装配、被整包浏览的那些
目录里——读写面与注入面落在同一个所有者上，`谁能读写它，谁负责注入它` 这条原则才完整。

> 读写面散到别处都会破这条原则：`session` 只能只读地注入全局指令（无地址、无容量）；
> 子树里的 `work` 实例会把智能体指令混进「工作区记忆」名下、还带着工作区的地址；
> `plugin_manager` 是**设置页的入口**（自有分区 + 各插件配置清单），不是任何内容文件的所有者。

### 子 Agent 的默认插件清单

子树挂**与父 Agent 同构的默认插件集**（见 `symbio_core::ASSEMBLY_SUB_AGENT_PLUGINS`）——与系统侧
是**同一份清单**：`home` 的 `SYSTEM_PLUGINS` 直接取这个常量，**不是第二份手抄**。清单内容不在此
复述（复述即重复，改一次要动两处），要看得去常量定义或 `docs/CURRENT.md` §1。`vdfs` 会随子树
构造出实例，但其注册经 `SubAgentVisitor` 丢弃（VDFS 根单槽归根）。

- `work` 在其中：子树 `WORKDIR` **继承父会话**，所以 `work` 注入的是工作区记忆，
  与系统侧读的是同一份语义、但落在子树自己的 `agent/<id>/work` 挂载点，无双重注入。
- `plugin_manager` 在其中：`SubAgentVisitor` 把它前缀到 `agent/<id>/plugin_manager`，子 Agent 页因此有了
  设置入口，与父 Agent 对齐。
- `agent` 在其中：子 Agent 也能在其目录内再挂子 Agent（`<id>/agent/<sub-id>` 递归）——分形。
- `model` 在其中：子智能体有自己的模型服务。子树会话收集能力时以**子容器**为 parent，
  子树 `model` 实例注册进该次收集自己的管理器——子会话用子智能体自己解析的模型；
  父会话收集期，这个注册才被 `SubAgentVisitor` 丢弃（单槽，防子树模型劫持父会话）。
- `vdfs` 不在其中：VDFS 根是系统级单槽，归系统 Agent 独占，子树经 `SubAgentVisitor`
  丢弃对应注册；列在子树里只会构造出无挂载点的空实例。

⚠️ 子树的 `WORKDIR` **不得**覆写成 Agent 目录：那会让 `work` 与系统侧注入同一份
`AGENTS.md`（双重注入），且 `work` 的作用域名实不符。

## 智能体自身的 `AGENTS.md`（两个作用域）

```text
作用域        物理落位                      可编辑地址
系统智能体    {homedir}/AGENTS.md          <根>/agent/AGENTS.md
子智能体      <agentdir>/AGENTS.md         <根>/agent/<agent id>/AGENTS.md
```

| | 系统态 | 子智能体态 |
|---|---|---|
| 归属 | 宿主应用级设置（用户可编辑） | agent 目录自带资产（随包分发） |
| 生效 | 所有会话 | 选中该智能体时 |
| 模块 | `host/instruction.rs` | `host/memory.rs` |
| 片段标题 | 【全局指令】 | 【智能体记忆】 |
| 片段里的地址 | 挂载根下那个文件 | 整包浏览里那个文件 |

两个作用域的读写、两道闸门、片段排版、节点形状**共用内核**（`symbio_core::memory`）；
本插件只提供「落位 + 标题 + 地址 + 空提示 + 闸门取值」。片段里的地址与闸门都是
**本插件自己会执行的**，因此印出来的数字是真的。

⚠️ 挂载根下的 `AGENTS.md` 是**保留名**（本应用自身的指令，不是名为它的 agent 目录）——
两者不可能相撞：agent id 首字符必须是小写字母或数字（§5.1），保留名以大写 `A` 开头。

⚠️ **`AgentDirStore` 不持有记忆的读写与闸门**：它只回答「记忆文件在哪」（`memory_path`）。
读 / 写 / 两道容量闸门一律走内核 `symbio_core::memory`，work / session / agent 三层共用
同一份实现——否则「超限是拒绝还是截断」「读不到算不算错误」会随「这条记忆属于哪一层」
而分叉。

**不可删除**：要清空就写入空内容（两个作用域同一约定）。

## 寻址（无自有路由，权威登记见 [ROUTES.md](../../../../docs/reference/ROUTES.md) §Agent 插件）

agent 目录的寻址是**挂载点语义**而非平铺三段（`route()` 直接返回 `NotFound` 并指引到 VDFS）：

- `agent/<id>` 是挂载点；钻进它即**委托给子 composite 的 `CompositeVfs`**（与系统根
  分形同构，而非把子智能体资源并集进系统树的三段地址）。
- 钻进后的清单由子 composite 的 `list("")` 返回自身子目录（session / model / mcp /
  skill / plugin_manager / …，经 `root_hidden` 过滤后的可见项），地址**递归**为
  `agent/<id>/<子目录>/<相对路径>`，没有固定的「子类别标签」三层。
- 两条链路要分清：**LLM 链路**的子智能体能力经 `SubAgentVisitor` 加 `agent/<id>/`
  前缀并集进系统树（前缀可见）；**系统 / 前端链路**走挂载点穿越（`sub_agent(id)`
  的 `get_vfs_provider()`），返回的相对地址用 `agent/<id>` 提回全局路径。

## 关联

- 会话编排与系统提示词的拼接 / 消费：`../session/README.md`
- 另外几层记忆：`../work/README.md`（工作区）、`../session/README.md`（会话）、
  `../plugin_manager/README.md`（设置入口，**不拥有任何记忆文件**）
- 记忆内核（各层共用）：`symbio_core::memory`
- Agent 目录规范：`docs/design/agent-directory-spec.md`
- VDFS 机制（`<根>/agent` 挂载点由本插件自持 `impl VdfsProvider`）：`docs/design/vdfs.md` §13.4
