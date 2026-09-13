/**
 * VDFS 协议（前端侧类型契约）
 *
 * 域类型与常量对齐后端纯接口：symbio/src/symbio_core/vdfs_provider.rs
 * 操作路径/信封对齐后端线路层：symbio/src/plugins/vdfs/protocol.rs
 * 规范：docs/design/vdfs.md
 *
 * 前端只消费协议形状；**资源类型、能力、标签、路径模板一律来自后端**，
 * 前端仅持有 ext → 渲染器、mount → 图标这类纯 UI 映射（见 registry/vdfsTypes.ts）。
 */

/** 挂载点清单（虚拟根 `/` 的目录内容） */
export const VFDS_PROVIDERS = 'vdfs/providers'
export const VFDS_LIST = 'vdfs/list'
export const VFDS_TREE = 'vdfs/tree'
export const VFDS_STAT = 'vdfs/stat'
export const VFDS_READ = 'vdfs/read'
export const VFDS_WRITE = 'vdfs/write'
export const VFDS_DELETE = 'vdfs/delete'
export const VFDS_MKDIR = 'vdfs/mkdir'
export const VFDS_MOVE = 'vdfs/move'
export const VFDS_WATCH = 'vdfs/watch'
export const VFDS_UNWATCH = 'vdfs/unwatch'

/** 虚拟根路径（前端口径：`.vdfs`）。
 *
 * 线路协议仍是规范化全路径 `/…`；两者在服务层一处翻译（见 toWirePath/toVdfsPath），
 * 页面与渲染器一律只认 `.vdfs` 口径。 */
export const VFDS_ROOT = '.vdfs'

/** 前端地址前缀（与 LLM 侧 ToolVdfs 的 VIRTUAL_PREFIX 同源） */
export const VFDS_PREFIX = '.vdfs'

/** 线路口径的虚拟根 */
export const VFDS_WIRE_ROOT = '/'

/** 节点状态（与实体机制同一约定） */
export const VFDS_STATUS_ACTIVE = 'active'
export const VFDS_STATUS_WORKING = 'working'
export const VFDS_STATUS_DISABLED = 'disabled'
export const VFDS_STATUS_ERROR = 'error'
export const VFDS_STATUS_UNKNOWN = 'unknown'

/** 节点基础类型 */
export const VFDS_KIND_DIR = 'dir'
export const VFDS_KIND_FILE = 'file'
export const VFDS_KIND_MOUNT = 'mount'

/** 约定呈现扩展名（宿主可自行扩展） */
export const VFDS_EXT_FORM = 'form'
export const VFDS_EXT_SESSION = 'session'
export const VFDS_EXT_TEXT = 'text'
export const VFDS_EXT_JSON = 'json'
export const VFDS_EXT_MARKDOWN = 'md'
export const VFDS_EXT_DIR = 'dir'

/** 变更事件的总线 kind（后端 host::VFDS_EVENT_KIND） */
export const VFDS_EVENT_KIND = 'vdfs'

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
}

/**
 * 虚拟文件系统节点。
 *
 * 目录与文件不做类型区分：`access.list` 为真即可列（目录），`access.read` 为真
 * 即可读（文件）。`kind` 只承载场景语义，不得用于能力判定。
 */
export interface VdfsNode {
  /** 全路径（含挂载点） */
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
   * ext = 'form' 时本字段为 DetailDefinition（schemas/entities.ts）。
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

/** 挂载点信息 */
export interface VdfsMountInfo {
  mount: string
  label: string
  description?: string
  order: number
  access: string
  status: string
  root: string
  icon?: string
  /** 该挂载根可接受的新建类型（导航项据此出添加入口） */
  new_types?: VdfsNewType[]
  [attribute: string]: unknown
}

export interface VdfsProvidersResponse {
  providers: VdfsMountInfo[]
}

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

/** 变更类型 */
export const VFDS_CHANGE_CREATED = 'created'
export const VFDS_CHANGE_UPDATED = 'updated'
export const VFDS_CHANGE_DELETED = 'deleted'
export const VFDS_CHANGE_RENAMED = 'renamed'

/** 数据变更事件（总线下发的使用方形状；后端 `VdfsChangeEvent`）。
 *  后端 provider 侧只报子树内相对路径，`mount` 与全路径由分发层补齐。 */
export interface VdfsChange {
  mount: string
  path: string
  change: string
  to?: string
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
  if (!dir || dir === VFDS_ROOT || dir === VFDS_WIRE_ROOT) return VFDS_ROOT
  const trimmed = dir.replace(/\/+$/, '')
  return trimmed || VFDS_ROOT
}

/** 拼接（父目录 + 单段名；自动规整多余斜杠） */
export function vdfsJoin(dir: string, name: string): string {
  const d = normalizeDir(dir)
  const n = name.replace(/^\/+|\/+$/g, '')
  return n ? `${d}/${n}` : d
}

/** 父目录（挂载点根的父 = 虚拟根；虚拟根的父 = 虚拟根） */
export function vdfsParent(path: string): string {
  const p = path.replace(/\/+$/, '')
  if (!p || p === VFDS_ROOT) return VFDS_ROOT
  const i = p.lastIndexOf('/')
  return i <= 0 ? VFDS_ROOT : p.slice(0, i)
}

/** 末段名（虚拟根 → 虚拟根） */
export function vdfsBase(path: string): string {
  const p = path.replace(/\/+$/, '')
  if (!p || p === VFDS_ROOT) return VFDS_ROOT
  const i = p.lastIndexOf('/')
  return i < 0 ? p : p.slice(i + 1)
}

/** 归属挂载点（虚拟根 → ''） */
export function vdfsMountOf(path: string): string {
  if (!path || path === VFDS_ROOT) return ''
  const p = path.startsWith(`${VFDS_ROOT}/`)
    ? path.slice(VFDS_ROOT.length + 1)
    : path.replace(/^\/+/, '')
  const i = p.indexOf('/')
  return i < 0 ? p : p.slice(0, i)
}

// ==================== 口径翻译（唯一翻译点，仅服务层可用） ====================
//
// 前端地址 `.vdfs/…` ↔ 线路路径 `/…`。二者在**服务层一处**完成翻译；
// 页面逻辑、渲染器、路径代数一律只认 `.vdfs` 口径（§3.2）。

/** 前端地址（`.vdfs` 口径）→ 线路路径（`/` 口径） */
export function toWirePath(path: string): string {
  if (!path || path === VFDS_ROOT) return VFDS_WIRE_ROOT
  if (path.startsWith(`${VFDS_PREFIX}/`)) return path.slice(VFDS_PREFIX.length)
  // 容错：已是线路口径则原样规整
  return path.startsWith(VFDS_WIRE_ROOT) ? path : `${VFDS_WIRE_ROOT}${path}`
}

/** 线路路径（`/` 口径）→ 前端地址（`.vdfs` 口径）；幂等 */
export function toVdfsPath(path: string): string {
  if (!path || path === VFDS_WIRE_ROOT) return VFDS_ROOT
  if (path === VFDS_ROOT || path.startsWith(`${VFDS_ROOT}/`)) return path
  return `${VFDS_PREFIX}${path.startsWith(VFDS_WIRE_ROOT) ? path : `${VFDS_WIRE_ROOT}${path}`}`
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
