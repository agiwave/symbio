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
 * 对齐后端 `symbio_core/vdfs_provider.rs` 的 `VdfsNewType`：以扩展名 `ext` 标识，
 * 与节点 `ext` 同一命名空间，因此**新建后的详情渲染器与既有节点一致**。
 * 清单非空 → 显示添加入口；多于一项 → 先选类型再命名。
 *
 * `source` 说明**写进去的内容从哪来**（后端声明、前端照做）：
 * 缺省 = 先命名后写入；`'file'` = 选一个本地文件，字节走二进制通道
 * （典型场景：zip 整包导入）。
 */
export interface VdfsNewType {
  /** 新元素扩展名（决定创建后的详情渲染器） */
  ext: string
  /** 展示标题（如「会话」「模型」） */
  title: string
  /** 语义说明 */
  description?: string
  /** 图标名（纯 UI 映射） */
  icon?: string
  /** 内容来源（后端 `VDFS_NEW_SOURCE_FILE`）：'file' = 选择本地文件 */
  source?: string
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
 * 反向解析：从一条变更路径取出会话 id 与（可选的）消息 id。
 *
 * 不是转写路径（含 `.vdfs/session` 清单本身、会话叶子、子会话 / 工作目录区段）
 * 一律返回 `null`——调用方据此跳过，无需自己切字符串。
 */
export function parseTranscriptPath(
  path: string,
): { sessionId: string; messageId?: string } | null {
  const prefix = `${VDFS_ROOT}/${VDFS_SESSION_DIR}/`
  if (!path.startsWith(prefix)) return null
  const segs = path.slice(prefix.length).split('/')
  const sessionId = segs[0]
  if (!sessionId || segs[1] !== VDFS_SEG_MESSAGES) return null
  if (segs.length === 2) return { sessionId }
  if (segs.length === 3 && segs[2]) return { sessionId, messageId: segs[2] }
  return null
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
