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

export * from './vdfs-form'

export const VDFS_LIST = 'vdfs/list'
export const VDFS_TREE = 'vdfs/tree'
export const VDFS_STAT = 'vdfs/stat'
export const VDFS_READ = 'vdfs/read'
export const VDFS_WRITE = 'vdfs/write'
export const VDFS_DELETE = 'vdfs/delete'
export const VDFS_MKDIR = 'vdfs/mkdir'
export const VDFS_MOVE = 'vdfs/move'
export const VDFS_WATCH = 'vdfs/watch'
export const VDFS_UNWATCH = 'vdfs/unwatch'
/** 执行节点动作（provider 自持的动词，如「测试连接」） */
export const VDFS_ACTION = 'vdfs/action'

/** 虚拟根路径（系统资源类别统一挂接在此目录之下）。
 *
 * 地址规则与后端 [`UnifiedFs`] 同源：`.vdfs` 打头 = 系统资源，其余 = 磁盘文件。
 * 前端与后端共用同一套地址口径，**不存在另一套线路翻译**。 */
export const VDFS_ROOT = '.vdfs'

/** 节点状态：进行中（其余状态词由 provider 自定，前端只做呈现映射） */
export const VDFS_STATUS_WORKING = 'working'
/** 节点状态：空闲 / 正常 */
export const VDFS_STATUS_ACTIVE = 'active'
/** 节点状态：**以错误结束**（消息与会话共用这个词）。
 *
 * 会话用它取代「`active` + `last_failed` 布尔」这种「状态 + 平行标志位」写法：
 * 「上一轮失败了吗」= `status == VDFS_STATUS_FAILED`，一处判定，不会漏读。
 * 见 `symbio/src/plugins/session/docs/node-state-streaming.md` §2.3.1。 */
export const VDFS_STATUS_FAILED = 'failed'
/** 节点状态：未开始（消息；会话没有这个态——它是长驻容器） */
export const VDFS_STATUS_PENDING = 'pending'
/** 节点状态：正在产生内容（消息的「运行中」） */
export const VDFS_STATUS_STREAMING = 'streaming'
/** 节点状态：等待用户响应（审批 / 提问） */
export const VDFS_STATUS_WAITING_USER_ACTION = 'waiting_user_action'
/** 节点状态：已结束（消息的终态）。
 *
 * **不与 `VDFS_STATUS_ACTIVE` 合并**：`active` 是「无特殊状态」，`completed`
 * 是「终态」，二者曾被后端映射成同一个字符串，导致消费端必须把 `active`
 * **猜回** `completed`（一次信息丢失 + 一次猜测还原）。现在状态原样透传。 */
export const VDFS_STATUS_COMPLETED = 'completed'

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
 * 目录可接受的新建元素类型（「新建」入口的类型清单元素）。
 *
 * 对齐后端 `symbio_core/vdfs_provider.rs` 的 `VdfsNewType`。
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
 *
 * `source` 说明**写进去的内容从哪来**（后端声明、前端照做）：
 * 缺省 = 在详情页里边看边填；`'file'` = 选一个本地文件，字节走二进制通道
 * （典型场景：zip 整包导入）。
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
  /** 内容来源（后端 `VDFS_NEW_SOURCE_FILE`）：'file' = 选择本地文件 */
  source?: string
  /** 新元素落成后的节点 `ext`（**详情渲染器键**）；缺省 = 与 `ext` 相同 */
  node_ext?: string
  /** 新元素的呈现描述（与节点 `schema` 同义）；草稿详情页据此渲染出同一张详情 */
  schema?: unknown
}

/** 新建内容来源：本地文件（后端 `VDFS_NEW_SOURCE_FILE`） */
export const VDFS_NEW_SOURCE_FILE = 'file'

/**
 * 虚拟文件系统节点。
 *
 * 目录与文件不做类型区分：`access.list` 为真即可列（目录），`access.read` 为真
 * 即可读（文件）。`kind` 只承载场景语义，不得用于能力判定。
 */
export interface VdfsNode {
  /** 全路径（`.vdfs/…` 或工作目录相对地址，与后端展示口径一致） */
  path: string
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
  /** 本目录可接受的新建类型（空 / 缺省 = 不可新建） */
  new_types?: VdfsNewType[]
  /** 场景扩展字段（flatten 到顶层） */
  [attribute: string]: unknown
}

/** 节点内容（文本或二进制，互斥） */
export interface VdfsContent {
  path: string
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

export interface VdfsListResponse {
  path: string
  node: VdfsNode
  items: VdfsNode[]
}

export interface VdfsTreeResponse {
  path: string
  nodes: VdfsNode[]
  truncated: boolean
}

export interface VdfsWriteResponse {
  path: string
  created: boolean
  etag?: string
}

export interface VdfsDeleteResponse {
  path: string
}

export interface VdfsMoveResponse {
  from: string
  to: string
}

/** 变更类型
 *
 *  后端还会发 `renamed`——前端不区分它（无专用常量），按通用变更走重拉即可。 */
export const VDFS_CHANGE_CREATED = 'created'
export const VDFS_CHANGE_UPDATED = 'updated'
export const VDFS_CHANGE_DELETED = 'deleted'
/** **追加型**变更：节点内容尾部新增了一段（携带 `delta`）。
 *  与 `updated` 的区别是增量的——消费者直接拼接，无需重读整个节点。
 *  列表型数据的流式输出（如会话转写里一条正在生成的消息）走这一种。 */
export const VDFS_CHANGE_APPENDED = 'appended'
/** **尾部截断**变更：`path` 所指节点**及其之后的全部兄弟**都已被移除。
 *
 *  与 `deleted`（「**这一个**节点没了」，移除一项即可、与顺序无关）是两种语义：
 *  本变更描述的是列表尾部的一段**区间**，消费者要按自己的顺序取「该节点及其后」。
 *
 *  分开的理由是**可分辨**与**代价**：逐条下发截断时，「删这一个」与「从这里删到
 *  末尾」在载荷上完全一样（只能靠外部知识去猜），且删一条早期消息要发 N 条变更。 */
export const VDFS_CHANGE_TRUNCATED = 'truncated'

/** 数据变更事件（总线下发的形状；后端 `VdfsChangeEvent`）。
 *  路径即对外展示地址（`.vdfs/…` 或工作目录相对地址），消费方直接比对。
 *
 *  ## 载荷按变更类型可选（不是装饰）
 *
 *  事件只说「哪里、怎么变」；「变成了什么」按类型附在下面两个字段上：
 *
 *  | 变更 | 载荷 | 消费者动作 | 额外往返 |
 *  |---|---|---|---|
 *  | `appended` | `delta` | 尾部拼接 | **0**（热路径，逐帧） |
 *  | `created` / `updated` | `node`（+ `content`） | 就地插入 / 替换 | **0** |
 *  | 未带载荷 | — | 回退 `stat` + `read` | 1–2 |
 *
 *  `delta`（多了什么）与 `content`（现在是什么）语义互斥，不会同时出现。 */
export interface VdfsChange {
  path: string
  change: string
  to?: string
  /** 追加型变更（`VDFS_CHANGE_APPENDED`）携带的**增量文本**；其余变更为 undefined */
  delta?: string
  /** **节点视图**（`created` / `updated` 可携带）：变更后该节点的元数据 */
  node?: VdfsNode
  /** **内容快照**（`created` / `updated` 可携带）：变更后该节点的正文 */
  content?: string
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
// 任何模块不得再手写字符串拼接/切分。**口径恒为 `.vdfs`**（§3）。

/** 规整目录段：空 / 根 → 根；否则去尾部斜杠 */
function normalizeDir(dir: string): string {
  if (!dir || dir === VDFS_ROOT) return VDFS_ROOT
  const trimmed = dir.replace(/\/+$/, '')
  return trimmed || VDFS_ROOT
}

/** 拼接（父目录 + 单段名；自动规整多余斜杠） */
export function vdfsJoin(dir: string, name: string): string {
  const d = normalizeDir(dir)
  const n = name.replace(/^\/+|\/+$/g, '')
  return n ? `${d}/${n}` : d
}

/**
 * 由本地文件名推导「新建名」：**保留原名主干 + 换成类型扩展名**。
 *
 * 服务于 `source = 'file'` 的新建类型（整包导入）：名称来自文件本身，
 * 扩展名由类型声明（`ext`），与后端「扩展名即类型」同口径。
 *
 * 例：`demo.zip` + `zip` → `demo.zip`；`pkg.tar.gz` + `zip` → `pkg.tar.zip`；
 * `README` + `zip` → `README.zip`。
 */
export function newFileNameOf(fileName: string, ext: string): string {
  // 个别环境给的是带路径的名字：只取末段
  const base = fileName.split(/[\\/]/).pop() || fileName
  const dot = base.lastIndexOf('.')
  const stem = dot > 0 ? base.slice(0, dot) : base
  return ext ? `${stem}.${ext}` : stem
}

/** 父目录（类别根的父 = 虚拟根；虚拟根的父 = 虚拟根） */
export function vdfsParent(path: string): string {
  const p = path.replace(/\/+$/, '')
  if (!p || p === VDFS_ROOT) return VDFS_ROOT
  const i = p.lastIndexOf('/')
  return i <= 0 ? VDFS_ROOT : p.slice(0, i)
}

/** 末段名（虚拟根 → 虚拟根） */
export function vdfsBase(path: string): string {
  const p = path.replace(/\/+$/, '')
  if (!p || p === VDFS_ROOT) return VDFS_ROOT
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

// ==================== 会话转写地址（地址代数的会话特例） ====================
//
// 会话在 VDFS 上是「叶子 + 内部区段」：
//
//   .vdfs/session/<sid>             会话叶子（ext = session，点开即聊天工作区）
//   .vdfs/session/<sid>/消息        转写列表（`l`）—— 会话的**本体**
//   .vdfs/session/<sid>/消息/<mid>  单条消息（ext = message，`r`）
//
// 与后端 `plugins/session/plugin.rs` 的 `SEG_MESSAGES` / `message_path` 同源：
// **地址的「拼」与「解」必须成对**，两边各只有一份实现，改地址方案时漏改一边
// 会被两侧的单测挡住。

/** 转写列表的路径段（后端 `session::SEG_MESSAGES`；同时是展示名） */
export const VDFS_SEG_MESSAGES = '消息'

/** 会话清单的挂载名（`.vdfs/session`） */
export const VDFS_SESSION_DIR = 'session'

/** 单个会话的地址：`.vdfs/session/<id>`（叶子） */
export function vdfsSessionAddr(sessionId: string): string {
  return vdfsJoin(vdfsJoin(VDFS_ROOT, VDFS_SESSION_DIR), sessionId)
}

/** 会话转写列表的地址：`.vdfs/session/<id>/消息` */
export function vdfsMessagesAddr(sessionId: string): string {
  return vdfsJoin(vdfsSessionAddr(sessionId), VDFS_SEG_MESSAGES)
}

/** 单条消息的地址：`.vdfs/session/<id>/消息/<mid>` */
export function vdfsMessageAddr(sessionId: string, messageId: string): string {
  return vdfsJoin(vdfsMessagesAddr(sessionId), messageId)
}

/**
 * 会话域地址 → 本域目标（**按地址分派**的唯一实现）。
 *
 * ## 为什么是「按地址」而不是「按事件类型」
 *
 * 事件流的分派必须写成 `switch (event.type)`，而每个分支都隐含「之前发生过什么」
 * ——顺序一变就错。地址分派不记录历史：每个地址只认**自己那份状态**，
 * 因此变更可以丢、可以重放、可以乱序，视图仍然正确。
 *
 * 不是会话域的地址一律返回 `null`（会话清单本身、子会话 / 工作目录区段、
 * `.vdfs/model` 等）——调用方据此跳过，无需自己切字符串。
 */
export type SessionRoute =
  /** `.vdfs/session/<sid>`：会话叶子（运行态的承载者） */
  | { target: 'session'; sessionId: string }
  /** `.vdfs/session/<sid>/消息`：转写列表（整表） */
  | { target: 'messages'; sessionId: string }
  /** `.vdfs/session/<sid>/消息/<mid>`：单条消息 */
  | { target: 'message'; sessionId: string; messageId: string }

/** 从一条变更路径解出会话域目标（纯函数，可单测） */
export function sessionRouteOf(path: string): SessionRoute | null {
  const prefix = `${VDFS_ROOT}/${VDFS_SESSION_DIR}/`
  if (!path.startsWith(prefix)) return null
  const segs = path.slice(prefix.length).split('/')
  const sessionId = segs[0]
  if (!sessionId) return null
  // 会话叶子：`.vdfs/session/<sid>`
  if (segs.length === 1) return { target: 'session', sessionId }
  if (segs[1] !== VDFS_SEG_MESSAGES) return null
  if (segs.length === 2) return { target: 'messages', sessionId }
  if (segs.length === 3 && segs[2]) return { target: 'message', sessionId, messageId: segs[2] }
  return null
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

/** 会话运行态（会话节点的三个取值 + 两个场景属性） */
export interface SessionRuntime {
  /** 节点状态：`working` / `active` / `failed` */
  status: string
  /** 上一轮的结局（从未跑过任何一轮时为 undefined） */
  outcome?: SessionOutcome
  /** 面向用户的错误短消息（仅 `failed` 时存在） */
  error?: string
}

/**
 * 从节点视图读出会话运行态（纯函数：缺字段即缺省，**不做推断**）。
 *
 * `error` 取节点自述的 `attributes.error`——它覆盖「错误发生在任何消息节点
 * 创建之前」这一类（能力收集失败 / provider 解析失败 / transport 级失败），
 * 此时没有任何失败节点可承载错误。
 */
export function sessionRuntimeOf(node: VdfsNode): SessionRuntime {
  const attrs = node as unknown as Record<string, unknown>
  const outcome = attrs.outcome
  const error = attrs.error
  const rt: SessionRuntime = { status: node.status }
  if (outcome === OUTCOME_COMPLETED || outcome === OUTCOME_ABORTED || outcome === OUTCOME_FAILED) {
    rt.outcome = outcome
  }
  if (typeof error === 'string' && error) rt.error = error
  return rt
}

/** 结局 → 提示音音色（纯映射；未知结局按正常结束处理） */
export function chimeKindOfOutcome(outcome?: SessionOutcome): 'completed' | 'aborted' | 'failed' {
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
