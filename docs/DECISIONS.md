# Symbio 架构决策记录 (ADRs)

> **文档类型：阐述** — 只记录**为什么这样设计**。
>
> - **只写现行决策**。已被取代 / 已回退 / 未落地的决策在下方索引里留一行状态，
>   全文见 `git log -p docs/DECISIONS.md`——本文件不维护变更史。
> - **格式**：状态 / 决策 / 理由 / 被否决的方案 / 后果与不变量。逐文件改动清单、
>   行数、测试基线、复核更正、实施批次过程记录一律**不进本文件**（那是 `git log` 的事）。
> - 「现在是什么」看 [CURRENT.md](./CURRENT.md)，模块机制看各模块 `README.md`。
>   其它文档**只引用 ADR 编号**，不复述结论。

## 0. 索引

| ADR | 决策 | 状态 |
|---|---|---|
| [001](./decisions/core.md#adr-001-分形插件架构) | 分形插件架构：容器与叶子同接口 | 现行 |
| [002](./decisions/core.md#adr-002-路径即路由) | 路径即路由 | 现行 |
| [003](./decisions/core.md#adr-003-session-作为编排入口) | Session 作为编排唯一入口 | 现行 |
| [004](./decisions/model.md#adr-004-多协议-llm-适配) | 多协议 LLM 适配（4 套适配器） | 现行 |
| [005](./decisions/agent.md#adr-005-agent-目录宿主实现) | Agent 目录宿主（agent-dir/v2） | 现行 |
| [006](./decisions/core.md#adr-006-薄宿主层设计) | 薄宿主层：Tauri 仅 3 个 command | 现行 |
| [007](./decisions/core.md#adr-007-静态注册-inventory) | 静态注册（inventory） | 现行 |
| 008 | 多存储后端：会话存储可选 `file` / `sqlite` / `memory` | **已回退**——三后端与 `store_kind` 已删除，会话存储收为单一 `SessionStore`（持久 = 磁盘布局 / 临时 = 进程内驻留）。与本条无关的资源存储见 ADR-011 |
| 009 | 机制化认知 v9：CU + `prop` 驱动的认知层 | **未落地**——CU 与认知层已从代码移除（分类残余亦已清理） |
| 010 | 统一实体管理：`entities/*` 契约 + `EntityCapabilities` | **已被取代**——`entities/*`、能力开关、`EntityProvider` trait + `VdfsAdapter`、`StorageService` 已逐层删除；资源访问统一经 VDFS（ADR-011） |
| [011](./decisions/vdfs.md#adr-011-资源存储--vdfsprovider-的集中实现) | 资源存储 = `VdfsProvider` 的三个集中实现 | 现行 |
| [012](./decisions/vdfs.md#adr-012-读侧成本是设计约束现在是什么必须有一张可核对的表) | 读侧成本是设计约束 | 现行 |
| [013](./decisions/model.md#adr-013-tls-后端--平台原生栈native-tls不做纯-rust-密码学) | TLS = 平台原生栈 | 现行 |
| 014 | 本地嵌入 = `tract-onnx` 纯 Rust 推理（弃 `fastembed` / ORT） | **已被 [ADR-016](./decisions/model.md#adr-016-本地嵌入改用-ortonnx-runtime推翻-adr-014-的性能前提并接受它当初拒绝的代价) 推翻**——`tract` 已完全退出依赖树 |
| [015](./decisions/session.md#adr-015-前端显示由节点状态驱动不由事件顺序驱动) | 前端显示由节点状态驱动 | 现行 |
| [016](./decisions/model.md#adr-016-本地嵌入改用-ortonnx-runtime推翻-adr-014-的性能前提并接受它当初拒绝的代价) | 本地嵌入改用 `ort`（ONNX Runtime） | 现行 |
| 017 | `session` 规模债暂不拆分（带触发条件） | **已被 [ADR-039](./decisions/session.md#adr-039-session-按域重组一域一目录编排层与领域层分离) 取代**——三条触发条件全部命中 |
| [018](./decisions/session.md#adr-018-压缩失败不得裁剪历史失败必须是可见可重试可持久化的状态) | 压缩失败不得裁剪历史 | 现行 |
| [019](./decisions/core.md#adr-019-跨栈契约手工镜像--审计守卫不引入代码生成g3前端面板去语义否决) | 跨栈契约手工镜像 + 审计守卫；G3 否决 | 现行 |
| [020](./decisions/core.md#adr-020-执行期与传输层分离eventsink出-abortsignal入取代-pluginchannel-的双职责) | `EventSink` + `AbortSignal` 取代 `PluginChannel` | 现行（决策 7 被 ADR-021 推翻） |
| [021](./decisions/core.md#adr-021-两个执行接口同形execenv-具名化拆信封收口到一处) | 两个执行接口同形（`ExecEnv`） | 现行 |
| [022](./decisions/model.md#adr-022-sse-增量解析契约在-core字段名在协议层) | SSE 增量解析：契约在 core，字段名在协议层 | 现行（**位置条款被 [ADR-034](./decisions/model.md#adr-034-sse-行解析契约随流循环迁入-model-插件) 取代**，形状不变） |
| [023](./decisions/core.md#adr-023-symbio_core-的准入规则--依赖方数量不是够不够底层) | `symbio_core` 准入规则 = 依赖方数量 | 现行 |
| [024](./decisions/session.md#adr-024-会话选项并入详情方言选项行是配置表单的字段不是独立协议) | 会话选项并入详情方言 | 现行 |
| [025](./decisions/session.md#adr-025-顺序是节点属性delta-是-updated-的传输形态) | 顺序是节点属性；变更信封 = `{path, data?}` | 现行 |
| [026](./decisions/session.md#adr-026-子智能体的完整会话在-vdfs-空间内自驱动用户消息入-inbox-集合进程内唤醒消费不走-route) | 子智能体会话在 VDFS 空间内自驱动（`inbox`） | 现行 |
| 027 | 可新建的东西至多一种（`new_type` 清单退役） | **部分现行**——「至多一个」仍成立；「类型内可选导入入口」与前端「选入口」已被 [ADR-029](./decisions/vdfs.md#adr-029-导入是详情页动作不是类型入口vdfsnewtype-收成纯呈现定义) 取代，`path` 归属已被 [ADR-030](./decisions/vdfs.md#adr-030-地址属于条目不属于节点vdfsitem-拆出写回执只给名字) 取代 |
| [028](./decisions/vdfs.md#adr-028-vdfsrequestmove-不收移动是外层组合不是核心原语) | `VdfsRequest::Move` 不收 | 现行 |
| [029](./decisions/vdfs.md#adr-029-导入是详情页动作不是类型入口vdfsnewtype-收成纯呈现定义) | 导入是详情页动作 | 现行 |
| [030](./decisions/vdfs.md#adr-030-地址属于条目不属于节点vdfsitem-拆出写回执只给名字) | 地址属于条目，不属于节点 | 现行 |
| [031](./decisions/session.md#adr-031-会话的输入是地址上的写入--动作路由不承担输入chatsend-与-chatabort-退役) | 会话输入 = 地址上的写入 / 动作 | 现行（含未完成项） |
| [032](./decisions/core.md#adr-032-插件身份归-pluginymlpluginmeta-从元信息降为出厂自述) | 插件身份归 `PLUGIN.yml` | 现行（含未完成项） |
| [033](./decisions/core.md#adr-033-生命周期钩子--start-同步stop-异步停用与卸载各给理由) | 生命周期钩子：`start` 同步、`stop` 异步 | 现行（含未完成项） |
| [034](./decisions/model.md#adr-034-sse-行解析契约随流循环迁入-model-插件) | SSE 行解析契约随流循环迁入 `model` 插件 | 现行（**取代 ADR-022 的位置条款**） |
| [035](./decisions/core.md#adr-035-provider-化的判据--三个条件与构造契约) | provider 化的三个条件与构造契约 | 现行 |
| [036](./decisions/core.md#adr-036-对象创建机制独立成域--它是系统级反射机制不是插件专属) | 对象创建机制独立成 `creator` 域 | 现行 |
| [037](./decisions/core.md#adr-037-实现可以离开-core--记忆整体迁往-providers) | **实现**可以离开 core：记忆整块迁 `providers` | 现行 |
| [038](./decisions/core.md#adr-038-帧与消息构造家族按依赖方数量下沉) | 帧与消息构造家族按依赖方数量下沉插件 | 现行（**取代 [ADR-034](./decisions/model.md#adr-034-sse-行解析契约随流循环迁入-model-插件) 决策 2 的位置条款**） |
| [039](./decisions/session.md#adr-039-session-按域重组一域一目录编排层与领域层分离) | `session` 按域重组：五个判据 | 现行（**取代 ADR-017**） |
| [040](./decisions/agent.md#adr-040-work-并入-memory记忆的两个作用域同属一个所有者) | `work` 并入 `memory`：记忆的两个作用域同属一个所有者 | 现行 |
| [041](./decisions/session.md#adr-041-会话的响应性由编排层调度--无副作用对话服务承担) | 会话的响应性由编排层调度 + 无副作用对话服务承担 | 现行 |
| [042](./decisions/core.md#adr-042-跨插件调用一律经容器-route--路径常量不持有对方类型) | 跨插件调用一律经容器 `route` + 路径常量 | 现行 |
| [043](./decisions/core.md#adr-043-v2-事件地基落地契约居中于中性层store--projection-按冻结形状进-symbio_core) | v2 事件地基：契约居中，`store` / `projection` 按冻结形状进 core | 现行 |
| [044](./decisions/core.md#adr-044-实测与判据同源成本时延与兜底率是事件网格的一等数据不用旁路遥测) | 实测与判据同源：成本/时延/兜底率是事件网格一等数据 | 现行 |

---

## 0.1 正文分册（域分配规则）

正文按**这条决策约束谁**归域——同一域的多条决策放同一册，跨册引用写成相对路径：

| 册 | 域 | 收什么 |
|---|---|---|
| [`decisions/core.md`](./decisions/core.md) | 平台基座 | 插件架构 / 路由 / 注册 / 执行接口 / `symbio_core` 准入 / 身份 / 生命周期 / 对象创建 |
| [`decisions/session.md`](./decisions/session.md) | 会话与执行 | 编排 / 上下文 / 转写节点与实时面 / 输入 / 响应性 |
| [`decisions/vdfs.md`](./decisions/vdfs.md) | 资源与地址 | `VdfsProvider` / 读侧成本 / 变更信封 / 新建与导入 / 地址归属 |
| [`decisions/model.md`](./decisions/model.md) | 模型与协议 | 多协议适配 / TLS / 本地嵌入 / SSE 解析 |
| [`decisions/agent.md`](./decisions/agent.md) | 智能体与记忆 | agent 目录宿主 / 记忆的两个作用域 |

> **为什么按域拆而不是按编号区间拆**：区间是**时间**的产物，读者找的是**职责**。
> 一条决策改属（例如会话响应性从"实现"升为"架构"）只该动它自己那一册，
> 而区间拆法会让每次新增都撞上"该塞进哪一段"。

---

> **维护原则**：新增架构决策在 [§0.1](#01-正文分册域分配规则) 对应的**那一册**追加一条 ADR，
> 并在上方索引表补一行（含指向该册的锚点链接）；一条 ADR 只含状态 / 决策 / 理由 /
> 被否决的方案 / 后果与不变量。
> 决策被推翻时**改写该条的状态并指向取代者**，不要另行复述过程；被彻底废止的决策降为索引一行，
> 全文交给 `git log`。其它文档只引用 ADR 编号。
