/**
 * VDFS 协议（前端侧类型契约）
 *
 * 域类型与常量对齐后端纯接口：symbio/src/symbio_core/vdfs_provider.rs
 * 操作路径/信封对齐后端线路层：symbio/src/plugins/vdfs/protocol.rs
 * 规范：docs/design/vdfs.md
 *
 * 前端只消费协议形状；**资源类型、能力、标签、路径模板一律来自后端**，
 * 前端仅持有 ext → 渲染器、资源类别 → 图标这类纯 UI 映射（见 registry/vdfsTypes.ts）。
 *
 * `ext = form` 节点的 `schema` 携带宿主方言（呈现描述），其类型见
 * `schemas/vdfs-form.ts`，并由本文件**统一出口**再导出一份
 * （消费方一律 `import ... from '@/schemas/vdfs'`）。
 */

import {
  MESSAGE_STATUS_ABORTED,
  MESSAGE_STATUS_COMPLETED,
  MESSAGE_STATUS_FAILED,
  MESSAGE_STATUS_PENDING,
  MESSAGE_STATUS_REMOVED,
  MESSAGE_STATUS_STREAMING,
  MESSAGE_STATUS_WAITING_USER_ACTION,
} from './chat_message'

export * from './vdfs-form'

// 操作名与地址常量**必须**定义在本文件：`scripts/mechanism-audit.mjs` 的 M-007
// 禁止在别处再定义一份（导入使用是允许的）。理由是它们是 VDFS **契约**的一部分，
// 要与下面的响应类型、变更词汇、地址代数同处一个文件——拆开就是把契约拆成两半。
// 控制面路由（`home/*` / `work/*` / `event_bus/*` / `gateway/*`）不在此列，
// 它们在 `constants/pluginPaths.ts`。
export const VDFS_LIST = 'vdfs/list'
export const VDFS_STAT = 'vdfs/stat'
export const VDFS_READ = 'vdfs/read'
export const VDFS_WRITE = 'vdfs/write'
export const VDFS_DELETE = 'vdfs/delete'
export const VDFS_WATCH = 'vdfs/watch'
export const VDFS_UNWATCH = 'vdfs/unwatch'
/** 执行节点动作（provider 自持的动词，如「测试连接」） */
export const VDFS_ACTION = 'vdfs/action'
/**
 * **进入地址空间**：列出虚拟根——**不给地址**（后端 `plugins/vdfs/protocol.rs::VDFS_ROOT`）。
 *
 * 根叫什么归后端 vdfs 插件，前端不该知道它：回包里的 `path` 即根地址，
 * 由 `services/vdfs.ensureVdfsRoot` 在启动期取回并登记进锚点
 * （`schemas/vdfsRoot`），此后一律当**运行期数据**从它往下拼。
 */
export const VDFS_ROOT_OP = 'vdfs/root'

/**
 * 节点状态词。
 *
 * **消息类状态（`pending` / `streaming` / `waiting_user_action` / `completed` /
 * `failed` / `aborted`）的权威定义在 `schemas/chat_message.ts`**——它们本就是消息
 * 状态词，只是经节点 `status` 承载（后端 `message_status()` 直接把 `MessageStatus`
 * 的序列化名写成节点 `status`）。此处**只做别名导出**，让 VDFS 侧消费方继续用
 * `VDFS_STATUS_*` 命名，同时保证值只有一个来源：改一个词的字面量、或新增一个
 * 消息状态词，**都不需要动本文件**。
 *
 * 本文件自己持有的是**会话类**状态（`working` / `active`）：它们不属于消息，
 * 是长驻会话容器的运行态。
 */
/** 会话状态：进行中（其余状态词由 provider 自定，前端只做呈现映射） */
export const VDFS_STATUS_WORKING = 'working'
/** 会话状态：空闲 / 正常 */
export const VDFS_STATUS_ACTIVE = 'active'

/** 节点状态：**以错误结束**（消息与会话共用这个词）。
 *
 * 会话用它取代「`active` + `last_failed` 布尔」这种「状态 + 平行标志位」写法：
 * 「上一轮失败了吗」= `status == VDFS_STATUS_FAILED`，一处判定，不会漏读。
 * 见 `symbio/src/plugins/session/docs/node-state-streaming.md` §2.3.1。 */
export const VDFS_STATUS_FAILED = MESSAGE_STATUS_FAILED
/** 节点状态：未开始（消息；会话没有这个态——它是长驻容器） */
export const VDFS_STATUS_PENDING = MESSAGE_STATUS_PENDING
/** 节点状态：正在产生内容（消息的「运行中」） */
export const VDFS_STATUS_STREAMING = MESSAGE_STATUS_STREAMING
/** 节点状态：等待用户响应（审批 / 提问） */
export const VDFS_STATUS_WAITING_USER_ACTION = MESSAGE_STATUS_WAITING_USER_ACTION
/** 节点状态：已结束（消息的终态）。
 *
 * **不与 `VDFS_STATUS_ACTIVE` 合并**：`active` 是「无特殊状态」，`completed`
 * 是「终态」，二者曾被后端映射成同一个字符串，导致消费端必须把 `active`
 * **猜回** `completed`（一次信息丢失 + 一次猜测还原）。现在状态原样透传。 */
export const VDFS_STATUS_COMPLETED = MESSAGE_STATUS_COMPLETED
/** 节点状态：**用户主动终止**（消息的终态，根级 Turn 用）。
 *
 * 与 `VDFS_STATUS_COMPLETED` / `VDFS_STATUS_FAILED` 并列的**第三个终态**：
 * 没有跑完（不是 `completed`），也没有出错（不是 `failed`）。它的语义是
 * **可重试**——前端的重试入口正是挂在这个终态上。
 *
 * 后端 `MessageStatus::as_str()` 早已产出这个词；前端曾漏在状态词表里，导致
 * 消费端把整条变更的状态**静默丢弃**（当时消费端的 `messageStatusOf` 未做透传），
 * 中止后重试入口不出现。现在它是 `MESSAGE_STATUS_ABORTED` 的别名——**漏改这件事
 * 已不可能发生**（词表只有一份），而未知词仍由该映射**显式告警**兜底。 */
export const VDFS_STATUS_ABORTED = MESSAGE_STATUS_ABORTED
/** 节点状态：**已被删除**（消息；会话没有这个态——它是长驻容器）。
 *
 * 删除在协议里是一次状态迁移（没有 `remove` 操作），与出现 / 增长 / 完成同走一帧。
 * 落到这个状态的帧在接收端**就地移除**该节点；VDFS 节点投影是 `MessageStatus`
 * 的**全量**映射（`nodes.rs::message_status` 直接取 `as_str()`），故本词同样是
 * 该全集的成员——不给它别名就等于让「消息状态词表」与「VDFS 可达词」分叉，
 * 正是下面 `vdfs.spec.ts` 那条集合相等断言要拦住的。 */
export const VDFS_STATUS_REMOVED = MESSAGE_STATUS_REMOVED

/** 「运行中」的唯一判据：节点 `status == working`。
 *
 * 「会话忙不忙」不是会话的私有布尔，它就是节点状态的一个取值——
 * 因此任何地方都不要另设 `is_working` 字段，一律由此函数派生，
 * 免得同一份真相出现两种写法（且新增 error / disabled 时不必再加布尔）。 */
export function isWorkingStatus(status?: string | null): boolean {
  return status === VDFS_STATUS_WORKING
}

/** 「上一轮失败」的唯一判据：节点 `status == failed`。 */
export function isFailedStatus(status?: string | null): boolean {
  return status === VDFS_STATUS_FAILED
}

/** 约定呈现扩展名（宿主可自行扩展） */
export const VDFS_EXT_FORM = 'form'
export const VDFS_EXT_SESSION = 'session'
/** 单条对话消息（**列表项**：正文在内容里，结构在 `attributes` 里） */
export const VDFS_EXT_MESSAGE = 'message'
export const VDFS_EXT_TEXT = 'text'
export const VDFS_EXT_JSON = 'json'
export const VDFS_EXT_MARKDOWN = 'md'
export const VDFS_EXT_DIR = 'dir'

/** 变更事件的总线 kind（后端 host::VDFS_EVENT_KIND） */
export const VDFS_EVENT_KIND = 'vdfs'

/** 访问位解析结果 */
export interface VdfsAccess {
  /** r：读取内容 */
  read: boolean
  /** w：写入内容 */
  write: boolean
  /** l：列出直接子节点（目录） */
  list: boolean
  /** t：树状递归遍历（目录） */
  traverse: boolean
}

/**
 * 目录可接受的**新建元素类型**（「新建」入口的类型）——**至多一个**。
 *
 * 对齐后端 `symbio_core/vdfs_provider.rs` 的 `VdfsNewType`。
 *
 * 一个目录接受的是**一类**东西（`session` 目录只收会话、`model` 目录只收模型），
 * 所以节点上声明的是 `new_type?: VdfsNewType` 而不是一张清单。
 *
 * ## 它只说「落成后长什么样」
 *
 * 本结构是**纯呈现定义**：`ext` / `title` / `icon` / `node_ext` / `schema`。
 * 「怎么把它造出来」不在这里——**导入整包是详情页上的一条动作**
 * （`VDFS_ACTION_IMPORT`，载荷见 `DetailAction.pack`），与「导出」「删除」同级。
 * 曾经它带 `source` / `import` 两个字段承载导入，那让一个呈现结构承担了操作语义，
 * 也让 `VdfsProvider` 上多出一个系统级接口（见 `docs/DECISIONS.md` ADR-029）。
 *
 * ## 两条独立的键：`ext` 与 `node_ext`
 *
 * `ext` 是**呈现扩展名**——地址末段可能带的后缀（后端 `id_of` 按它剥出条目 id），
 * 它**不**决定详情怎么渲染：配置型资源（model / mcp / skill）落成后统一是
 * `ext = form`。`node_ext` 才是**新元素落成后的节点 `ext`**（渲染器键），
 * 缺省 = 用 `ext`。
 *
 * 分开声明是为了让**草稿节点**（还没创建、无 id 无名字）能用上与该类型落成后
 * **完全相同**的渲染器与 `schema`——这正是「点新建与选中一项进入同一个详情页」。
 */
export interface VdfsNewType {
  /** 新元素**呈现扩展名**（地址末段后缀；**不是**渲染器键） */
  ext: string
  /** 展示标题（如「会话」「模型」） */
  title: string
  /** 语义说明 */
  description?: string
  /** 图标名（纯 UI 映射） */
  icon?: string
  /** 新元素落成后的节点 `ext`（**详情渲染器键**）；缺省 = 与 `ext` 相同 */
  node_ext?: string
  /** 新元素的呈现描述（与节点 `schema` 同义）；草稿详情页据此渲染出同一张详情 */
  schema?: unknown
}

/**
 * 虚拟文件系统节点（文件或目录）——**一份自述，不含地址**。
 *
 * 目录与文件不做类型区分：`access.list` 为真即可列（目录），`access.read` 为真
 * 即可读（文件）。`kind` 只承载场景语义，不得用于能力判定。
 *
 * ## 为什么这里没有 `path`
 *
 * 地址是**某一份列表**给这个节点的定位，不是节点自己的属性——同一个节点可以在
 * 不同列表里以不同地址出现（实证：设置页的一项指向插件自己那份配置文档
 * `<目录名>/PLUGIN.yml`，而同一份文档在自己的目录里就叫 `PLUGIN.yml`）。
 * 于是地址落在**条目**上（{@link VdfsItem}），由分发层按 `<父地址>/<name>`
 * 回填；需要「可寻址的一项」时用 `VdfsItem`。
 *
 * 与后端 `symbio_core::vdfs_provider::VdfsNode` 逐字同构（`scripts/protocol-mirror-audit.mjs` 校验）。
 */
export interface VdfsNode {
  /** 父节点内的唯一标识（路径段） */
  name: string
  title: string
  description?: string
  kind: string
  status: string
  /** 访问位紧凑表示（如 'rw' / 'lt'），由 vdfsAccessOf 解析 */
  access: string
  /** 呈现扩展名 —— 前端选择详情渲染器的唯一键 */
  ext?: string
  size?: number
  updated_at?: number
  children?: number
  binary?: boolean
  /**
   * 呈现描述（宿主方言，VDFS 只透传）。
   * ext = 'form' 时本字段为 DetailDefinition（schemas/vdfs-form.ts）。
   */
  schema?: unknown
  /**
   * 本目录可接受的**新建元素类型**（至多一个；缺省 = 不可新建）。
   *
   * 一个目录接受的是**一类**东西——「可新建两类」是伪命题：那其实是同一个类型的
   * 两种落盘路径，而第二条（整包导入）是详情页上的一条**动作**，不是第二种类型
   * （见 {@link VdfsNewType} 与 `docs/DECISIONS.md` ADR-029）。
   */
  new_type?: VdfsNewType
  /** 场景扩展字段（flatten 到顶层） */
  [attribute: string]: unknown
}

/**
 * 列表条目 = **地址 + 节点**（后端 `VdfsItem`）。
 *
 * 线格式与「带 `path` 的节点」逐字节相同（后端用 `#[serde(flatten)]`），所以
 * 消费端读到的仍是同一个 `{path, name, title, …}`——地址只是**从节点挪到了条目上**。
 *
 * 清单（`VdfsListResponse.items` / `VdfsTreeResponse.nodes`）里给的每一项都是它：
 * 那是唯一「知道自己在哪」的形态。目录自身的 `node` 字段是纯 {@link VdfsNode}，
 * 因为它的地址在请求里已经有了。
 */
export interface VdfsItem extends VdfsNode {
  /** 条目地址（展示口径：根锚点打头的虚拟地址，或工作目录相对地址） */
  path: string
}

/** 节点内容（文本或二进制，互斥）——**不带地址**（内容总是「请求的那个节点」的内容） */
export interface VdfsContent {
  text?: string
  b64?: string
  binary: boolean
  size: number
  mime?: string
  etag?: string
}

/** 节点动作结果 */
export interface VdfsActionResponse {
  /** 回显的动作标识（便于配对请求） */
  action: string
  ok: boolean
  /** 结果说明（成功摘要 / 失败原因，可直接展示） */
  message: string
  data?: unknown
}

/**
 * 动作结果带回的**文件载荷**（`VdfsActionResponse.data` 的宿主方言形状之一）。
 *
 * 与 `VdfsContent.b64` 同构：只要结果里同时有 `filename` 与 `b64`，
 * 前端就把它当文件下载——**不必认识「导出」这个动作**，新增带回文件的
 * 动作无需改动前端。
 */
export interface VdfsActionFile {
  /** 建议文件名 */
  filename: string
  /** 文件字节（base64） */
  b64: string
}

/**
 * 读取动作结果里的文件载荷（没有则 `null`）。
 *
 * 这是**形状判定**而非动作判定：任何动作只要按此形状回传数据，都能被下载。
 */
export function actionFileOf(data: unknown): VdfsActionFile | null {
  if (!data || typeof data !== 'object') return null
  const d = data as Record<string, unknown>
  if (typeof d.filename !== 'string' || typeof d.b64 !== 'string') return null
  if (!d.filename || !d.b64) return null
  return { filename: d.filename, b64: d.b64 }
}

/** 已知动作标识：连接测试（后端 `VDFS_ACTION_TEST`） */
export const VDFS_ACTION_TEST = 'test'
/** 已知动作标识：导出打包（后端 `VDFS_ACTION_EXPORT`） */
export const VDFS_ACTION_EXPORT = 'export'
/**
 * 已知动作标识：**导入整包**（后端 `VDFS_ACTION_IMPORT`）——与「导出」互为逆向。
 *
 * 它声明 `pack`（载荷是一个本地文件，见 `DetailAction.pack`），因此前端这一侧
 * 只需「取文件 → 交给机制层编码」，**不认识这个动作本身**；反过来「导出」的
 * 结果里带 `filename` + `b64`（见 `actionFileOf`），前端也只认那个形状。
 * 两个方向都是形状判定，故新增同类动作无需改前端。
 */
export const VDFS_ACTION_IMPORT = 'import'
/**
 * 已知动作标识：**截断**（后端 `VDFS_ACTION_TRUNCATE`）——列表资源删除
 * 「该条目及其之后」的全部条目，`data` 带回被删条目 id 列表。
 */
export const VDFS_ACTION_TRUNCATE = 'truncate'
/**
 * 已知动作标识：**清空**（后端 `VDFS_ACTION_CLEAR`）——列表资源保留容器、
 * 清掉全部条目。
 *
 * 为什么「截断 / 清空」是动作而不是 `delete`：见后端
 * `symbio_core::vdfs_provider` 的 `VDFS_ACTION_TRUNCATE` 文档——`delete` 是
 * **逐节点**语义，表达不了「从这里删到末尾」这类集合操作；而清空虽然也能用
 * `delete` 表达（`deleted` 落在列表目录上无歧义），仍与截断一起走动作，
 * 好让**同一个区段的删除只有一种入口形态**。
 */
export const VDFS_ACTION_CLEAR = 'clear'

export interface VdfsListResponse {
  /** **本列表自身**的地址——唯一必须保留的地址字段：`vdfs/root` 的调用方无从知道根叫什么 */
  path: string
  /** 目录自身节点（纯自述；它的地址就是上面的 `path`） */
  node: VdfsNode
  /** 条目（地址 + 节点） */
  items: VdfsItem[]
}

/**
 * 写入回执。
 *
 * **不带地址**：写哪儿是调用方自己说的（请求里就有）。唯一需要 provider 交回的
 * 是**匿名写**（打在目录自身上的那一次）里它自己生成的名字——具名写为 `undefined`，
 * 地址由调用方拿自己的请求地址 + 这个名字拼。
 */
export interface VdfsWriteResponse {
  /** 匿名写时 provider 生成的名字（具名写为 undefined） */
  name?: string
  created: boolean
  etag?: string
}

/** **重同步指令**：后端通道曾满，消费端可能漏了变更，请按自己的作用域重读。
 *
 *  它是一条**指令**而不是一条变更（后端 `symbio_core::event_bus::RESYNC_MARKER_TYPE`）。
 *
 *  它刻意**不带 `path`**——消费端的作用域判定要求 `path` 是字符串，因此本指令会
 *  被既有消费者自然忽略，只在显式登记了重读动作的地方生效（`subscribeVdfsChanged`
 *  的 `onResync`）。与转写流的 `transcript_resync` 同构：都是「别猜漏了哪一段，
 *  按作用域整份重读」。 */
export const VDFS_BUS_RESYNC = 'resync'

/** 数据变更事件（总线上下发的形状；与后端 `VdfsChange` 逐字同构，由
 *  `scripts/protocol-mirror-audit.mjs` 校验）。
 *
 *  路径即对外展示地址（根锚点打头的虚拟地址，或工作目录相对地址），消费方直接比对。
 *
 *  ## 形状：`path` + 可选 `data`——**信封没有操作枚举**
 *
 *  信封只回答「**哪条路径、带来了什么**」；`data` 是该路径的**业务载荷**：
 *
 *  | `data` 形状 | 生产者 | 消费端动作 |
 *  |---|---|---|
 *  | `ChatMessage`（含 `delta`） | 消息域（`Transcript::apply`） | **按字段落地，零回读**：`delta` 追加 / `content` 替换 / `status = removed` 移除 |
 *  | `ChatMessage`（全量） | 同上（首帧发合并后的全量副本） | **零回读**就地替换该消息 |
 *  | `ChatMessage`（只有状态） | 同上（`state_frame` 剥掉了正文） | 本地已有 ⇒ 零回读迁移状态；身份未知 ⇒ 回读补基线 |
 *  | `VdfsNode` | 会话运行态（`emit_session_state`） | 就地收敛节点状态，零回读 |
 *  | 缺失 | 全部资源信号（`notify_change`） | 回读 / 重拉（幂等）；对资源删除，回读 `NotFound` 即删除 |
 *
 *  **path 的含义**：**恒为被变更节点自身的地址**。会话是**容器**，其下是若干**并列的
 *  集合**（消息 / 子会话 / 记忆 / 工作目录，后续还会有任务列表、请求队列……），因此
 *  集合项的地址形状统一为 `<sid>/<集合段>/<项 id>`——消息的落点是 `<sid>/message/<mid>`
 *  这个节点，**身份就是地址末段**（与 `data.id` 是同一个事实，以地址为准）。
 *  无载荷变更的 `path` 同样是节点自身地址。
 *
 *  早先这里发的是**目录**（`<sid>/message`）而把身份交给 `data.id`：那样 `path` 的含义
 *  随帧类型漂移（资源信号是节点自身、消息是它所在的目录），消费端必须**反推地址**
 *  才能回读，且这种寻址**推广不到第二类集合**。
 *
 *  ## 为什么没有操作枚举
 *
 *  「资源层面发生了什么」与「业务数据变成了什么」是同一件事的两种说法，而消费端
 *  真正消费的只有后者——保留枚举只会让每个消费端都背上一次「枚举 → 分派」的翻译。
 *  消息的删除由 `ChatMessage.status = removed` 承载（消息词汇本就有它），资源删除
 *  由「载荷缺失 + 回读 `NotFound`」表达。
 */
export interface VdfsChange {
  /** **被变更节点自身**的地址（集合项形状 `<sid>/<集合段>/<项 id>`，身份即末段） */
  path: string
  /** 业务载荷（`ChatMessage` / `VdfsNode` 的 JSON）；缺失 = 无载荷（回读收敛） */
  data?: unknown
}

/** 字段级校验错误（provider 自持校验的产物） */
export interface VdfsFieldError {
  field: string
  message: string
}

export interface VdfsValidationError {
  message: string
  fields: VdfsFieldError[]
}

// ==================== 路径代数（纯函数，机制级唯一实现） ====================
//
// 虚拟路径是 VDFS 的唯一寻址方式；下列函数是全部路径运算的单一来源，
// 任何模块不得再手写字符串拼接/切分。**口径恒为「根锚点」**（§3）：根叫什么
// 由 `schemas/vdfsRoot` 在启动期引导，这里不持有它的字面量。

import { vdfsRoot } from './vdfsRoot'

/** 规整目录段：空 / 根 → 根；否则去尾部斜杠 */
function normalizeDir(dir: string): string {
  const root = vdfsRoot()
  if (!dir || dir === root) return root
  const trimmed = dir.replace(/\/+$/, '')
  return trimmed || root
}

/** 拼接（父目录 + 单段名；自动规整多余斜杠） */
export function vdfsJoin(dir: string, name: string): string {
  const d = normalizeDir(dir)
  const n = name.replace(/^\/+|\/+$/g, '')
  return n ? `${d}/${n}` : d
}

/** 父目录（类别根的父 = 虚拟根；虚拟根的父 = 虚拟根） */
export function vdfsParent(path: string): string {
  const root = vdfsRoot()
  const p = path.replace(/\/+$/, '')
  if (!p || p === root) return root
  const i = p.lastIndexOf('/')
  return i <= 0 ? root : p.slice(0, i)
}

/** 末段名（虚拟根 → 虚拟根） */
export function vdfsBase(path: string): string {
  const root = vdfsRoot()
  const p = path.replace(/\/+$/, '')
  if (!p || p === root) return root
  const i = p.lastIndexOf('/')
  return i < 0 ? p : p.slice(i + 1)
}

/** 解析访问位紧凑串（'rw' → { read, write, list: false, traverse: false }） */
export function vdfsAccessOf(node: { access?: string } | null | undefined): VdfsAccess {
  const flags = node?.access ?? ''
  return {
    read: flags.includes('r'),
    write: flags.includes('w'),
    list: flags.includes('l'),
    traverse: flags.includes('t'),
  }
}

/** 节点是否为目录（机制判定只看访问位） */
export function isVdfsDir(node: { access?: string } | null | undefined): boolean {
  return vdfsAccessOf(node).list
}

/**
 * 是否**系统资源地址**（根锚点打头）。
 *
 * 地址空间只有两个半边：根之下是各 provider 挂载的虚拟资源，其余是工作目录里的
 * 物理文件。这个划分**曾经是能力差异的来源**——物理半边由文件系统 provider 承载
 * `move`（改名），虚拟半边一律没有，于是「重命名」入口只对物理地址给出。
 *
 * 那个差异随 `vdfs/move` 整条下线消失了（移动不是核心原语：跨子树时它是
 * copy+delete，见后端 `VdfsRequest` 的「没有 `Move`」一节）。本函数因此**不再
 * 参与任何能力判定**，只作为「这个地址是不是系统资源」的判据保留。
 */
export function isVdfsSystemAddr(path: string): boolean {
  const root = vdfsRoot()
  return path === root || path.startsWith(`${root}/`)
}

/**
 * 草稿节点（机制「新建」态）的**唯一判据**：没有路径。
 *
 * 草稿还没落盘，于是没有地址、也没有名字——三者说的是同一件事，但只有
 * `path` 是**定义**（地址唯一标识一个节点），「没有名字」是它的推论。
 * 此前这条判据在四处各写了一遍（`!node.path` / `!node.name`），当前取值恰好
 * 等价，靠注释解释而非靠机制保证；收敛到这里后，各处一律引用本函数。
 */
export function isVdfsDraft(node: { path?: string } | null | undefined): boolean {
  return !node?.path
}

// ==================== 会话转写地址（地址代数的会话特例） ====================
//
// 会话在 VDFS 上是「叶子 + 内部区段」：
//
//   <根>/session/<sid>             会话叶子（ext = session，点开即聊天工作区）
//   <根>/session/<sid>/message        转写列表（`l`）—— 会话的**本体**
//   <根>/session/<sid>/message/<mid>  单条消息（ext = message，`r`）
//
// 两个段名都由后端 provider 决定，**不是前端的知识**：
// - 挂载段 `session`  —— 后端 `symbio_core::ids::PLUGIN_SESSION`，是 provider 注册时
//   自选的挂载名（与 `model` / `skill` 等同族），前端按「挂载点声明可新建
//   `ext = session`」把它**认出来**；
// - 转写段 `消息`     —— 后端 `session::plugin::nodes::SEG_MESSAGES`，是 provider 的
//   私有段名且**同时是展示名**，前端按 `kind = VDFS_KIND_MESSAGES` 认出来。
//
// 因此本文件**不持有这两个段名**：地址方案是运行期数据（`VdfsSessionScheme`），
// 由 `services/vdfsScheme` 解析后作为参数传入。这里是纯函数，拼与解仍然成对
// ——只是两边的「同一份实现」变成了「同一份数据」。

/** 转写列表的场景类型（后端 `VDFS_KIND_MESSAGES`）。
 *
 * 稳定 ASCII 协议词：**与展示名解耦**——段名可以随文案调整，
 * `kind` 才是对外承诺的标识。跨栈一致性由 `scripts/protocol-mirror-audit.mjs`
 * 的 X-002 组校验。 */
export const VDFS_KIND_MESSAGES = 'messages'

/**
 * 会话地址方案（**运行期数据**）。
 *
 * - `mountDir`    挂载目录地址（如 `<根>/session`）
 * - `messagesSeg` 转写列表的段名（展示名，随后端下发）
 *
 * 解析见 `services/vdfsScheme.ensureVdfsSessionScheme()`。之所以是值而不是
 * 常量：`schemas/` 不允许反向依赖 `services/`，而这两项是列目录才能拿到的。
 */
export interface VdfsSessionScheme {
  mountDir: string
  messagesSeg: string
}

/**
 * 单个会话的地址：`<挂载目录>/<id>`（叶子）。
 *
 * 只用到挂载目录——转写段与本地址无关，因此不要求完整方案。
 */
export function vdfsSessionAddr(mountDir: string, sessionId: string): string {
  return vdfsJoin(mountDir, sessionId)
}

/** 会话转写列表的地址：`<挂载目录>/<id>/<转写段>` */
export function vdfsMessagesAddr(s: VdfsSessionScheme, sessionId: string): string {
  return vdfsJoin(vdfsSessionAddr(s.mountDir, sessionId), s.messagesSeg)
}

/** 单条消息的地址：`<mountDir>/<id>/<转写段>/<mid>` */
export function vdfsMessageAddr(
  s: VdfsSessionScheme,
  sessionId: string,
  messageId: string,
): string {
  return vdfsJoin(vdfsMessagesAddr(s, sessionId), messageId)
}

// ==================== 会话运行态（会话节点的场景属性） ====================
//
// 会话「忙不忙 / 上一轮怎么结束的」是**会话节点的属性**，不是事件：
//
//   status             = 'working' | 'active' | 'failed'
//   attributes.outcome = 'completed' | 'aborted' | 'failed'   （上一轮的结局）
//   attributes.error   = "<面向用户的短消息>"                   （仅 failed 时存在）
//
// 于是前端不需要「知道上一条事件是什么」就能渲染正确——这正是
// `eventBus.replayBuffer`（切会话防乱序缓冲）可以被删掉的原因。
// 见 `symbio/src/plugins/session/docs/node-state-streaming.md` §3.3。

/** 结局：正常结束 */
export const OUTCOME_COMPLETED = 'completed'
/** 结局：用户中止 */
export const OUTCOME_ABORTED = 'aborted'
/** 结局：以错误结束 */
export const OUTCOME_FAILED = 'failed'

export type SessionOutcome = 'completed' | 'aborted' | 'failed'

/**
 * 结束提示音的类型 —— **就是** `SessionOutcome`（音色由结局决定，故取别名而非另写一份）。
 *
 * 用别名的收益是**穷尽性检查**：提示音表是 `Record<CompletionKind, …>`，后端若
 * 新增一个结局取值，音色表会**编译报错**，逼着人决定它该响什么——比静默落到
 * 默认音色好。此前这三个取值有三份独立定义（`stores/soundSettings.ts` 一份、
 * `services/completionChime.ts` 引用它、`Appearance.vue` 里还有一份内联字面量联合）。
 */
export type CompletionKind = SessionOutcome

/** 会话运行态（会话节点的三个取值 + 三个场景属性） */
export interface SessionRuntime {
  /** 节点状态：`working` / `active` / `failed` */
  status: string
  /** 上一轮的结局（从未跑过任何一轮时为 undefined） */
  outcome?: SessionOutcome
  /** 面向用户的错误短消息（仅 `failed` 时存在） */
  error?: string
  /** 会话级告警（可恢复；新一轮开始时后端清除，此处随之回落 null） */
  warning?: string
}

/**
 * 从节点视图读出会话运行态（纯函数：缺字段即缺省，**不做推断**）。
 *
 * `error` / `warning` 取节点自述的 `attributes.error` / `attributes.warning`——
 * 它们覆盖「状态发生在任何消息节点创建之前」这一类（能力收集失败 / provider
 * 解析失败 / transport 级失败 / 持久化失败），此时没有任何失败节点可承载它。
 *
 * **漏读一个属性的代价不是"少显示一条"，而是整条状态被静默丢弃**：后端
 * `SessionStateChange` 写进节点、前端却读不出来，UI 永远看不到它，且不报错。
 * 三个属性（`outcome` / `error` / `warning`）都必须在此落地。
 */
export function sessionRuntimeOf(node: VdfsNode): SessionRuntime {
  const attrs = node as unknown as Record<string, unknown>
  const outcome = attrs.outcome
  const error = attrs.error
  const warning = attrs.warning
  const rt: SessionRuntime = { status: node.status }
  if (outcome === OUTCOME_COMPLETED || outcome === OUTCOME_ABORTED || outcome === OUTCOME_FAILED) {
    rt.outcome = outcome
  }
  if (typeof error === 'string' && error) rt.error = error
  if (typeof warning === 'string' && warning) rt.warning = warning
  return rt
}

/** 结局 → 提示音音色（纯映射；未知结局按正常结束处理） */
export function chimeKindOfOutcome(outcome?: SessionOutcome): CompletionKind {
  if (outcome === OUTCOME_ABORTED) return 'aborted'
  if (outcome === OUTCOME_FAILED) return 'failed'
  return 'completed'
}

/** 生效的呈现扩展名：显式 ext 优先，否则由 name 推导 */
export function vdfsExtOf(node: VdfsNode): string | undefined {
  const explicit = node.ext?.trim()
  if (explicit) return explicit
  const base = node.name.split('/').pop() ?? node.name
  const dot = base.lastIndexOf('.')
  if (dot <= 0 || dot === base.length - 1) return undefined
  return base.slice(dot + 1).toLowerCase()
}

/**
 * 从插件错误文案位还原字段级校验错误。
 *
 * 后端把 `VdfsValidationError` 序列化为 JSON 放在错误文案里；非本协议载荷
 * （纯文本）返回 null，调用方按纯文本展示。
 */
export function parseVdfsValidation(raw: unknown): VdfsValidationError | null {
  const text = typeof raw === 'string' ? raw : String((raw as Error)?.message ?? '')
  // 后端错误前缀（PluginError::ValidationError 的 Display）后接 JSON 载荷
  const start = text.indexOf('{')
  if (start < 0) return null
  try {
    const parsed = JSON.parse(text.slice(start)) as VdfsValidationError
    if (Array.isArray(parsed?.fields) || typeof parsed?.message === 'string') return parsed
    return null
  } catch {
    return null
  }
}
