/**
 * 统一资源协议（前端侧类型契约）
 *
 * 与后端大面积对齐：symbio/src/symbio_core/schemas/resources.rs
 * 覆盖 model / mcp / agent / skill / session 等资源类型。
 * 所有资源共享同一套 resources/* 操作与能力开关，前端据此驱动统一页面。
 *
 * 资源类型（kind）已开放为 string：类型的**存在性/能力/前缀**以后端
 * `resources/providers` 下发的 ProviderInfo 为单一真相源，前端不再硬编码类型清单。
 */

/**
 * 资源能力开关——决定统一页面启用哪些模块 */
export interface ResourceCapabilities {
  /** 以上传 zip 为主（文件名即资源目录名） */
  zip_upload: boolean
  /** 是否有独立表单 */
  independent_form: boolean
  /** 列表项是否有实时状态 */
  realtime_status: boolean
  /** 是否可写（可上传新增 / 删除） */
  mutable: boolean
  /** 是否支持连接测试 */
  test_connection: boolean
  /** 是否默认只读 */
  read_only: boolean
}

/**
 * 资源类型（provider）注册信息 —— 前端从后端 `resources/providers` 拉取。
 * 对应后端 symbio_core::schemas::resources::ProviderInfo。
 */
export interface ProviderInfo {
  kind: string
  /** 提供方显示名，用于路径 [provider]/[id].[kind] */
  provider_name: string
  /** 资源操作路径前缀（resourcesOp 拼接 `${prefix}/resources/<op>`） */
  prefix: string
  capabilities: ResourceCapabilities
  /** 展示顺序（导航 / 类型选择排序） */
  order: number
  /** 展示标签 */
  label: string
  /** 是否支持在资源管理器内创建/删除（session=false） */
  supports_upload: boolean
  /** 列表简洁模式：仅显示类型图标 + 标题（如设置分区） */
  compact_list?: boolean
  /** 列表项是否显示运行状态图示（如设置分区为 false，隐藏状态点）；缺省 true */
  status_indicator?: boolean
  /** 容器声明：条目内部托管的子资源类型（空/缺省 = 条目不是容器） */
  container_kinds?: ContainerKindInfo[]
}

/** resources/providers 响应 */
export interface ProvidersResponse {
  providers: ProviderInfo[]
}

/**
 * 容器子资源类型声明 —— 该 provider 的条目本身是「容器」，内部托管这些子类型。
 *
 * 如 agent（OAB bundle）内部托管 prompt / skill / mcp。容器资源页的左侧类别导航、
 * 新建路径模板、新建内容模板均由此下发——后端控制，前端零硬编码。
 */
export interface ContainerKindInfo {
  /** 子资源类型（如 prompt / skill / mcp） */
  kind: string
  /** 展示标签 */
  label: string
  /** 语义说明（新建/编辑表单提示文本） */
  description?: string
  /** 新建路径模板（<name> 占位符），如 prompts/<name>.md */
  path_hint?: string
  /** 新建内容模板（编辑器初始内容） */
  default_content?: string
  /** 子资源能力开关 */
  capabilities: ResourceCapabilities
}

/** 统一资源概要（列表项） */
export interface ResourceSummary {
  kind: string
  /** 提供方（插件）显示名，用于资源路径 [provider]/[id].[kind]；后端 dispatch 统一回填 */
  provider?: string
  name: string
  id: string
  description?: string
  summary?: string
  updated_at?: number
  status: string
  status_detail?: string
  // 类型特有扩展字段（flatten）
  [extra: string]: unknown
}

/** resources/list 响应（请求携带 container 时为容器语义，container 回显容器 id） */
export interface ResourcesListResponse {
  kind: string
  capabilities: ResourceCapabilities
  items: ResourceSummary[]
  /** 容器作用域（容器语义时下发；顶层列表缺省） */
  container?: string
}

/** resources/upload 响应 */
export interface ResourceUploadResponse {
  kind: string
  id: string
  created: boolean
}

/** resources/status 响应 */
export interface ResourceStatusResponse {
  kind: string
  id: string
  status: string
  status_detail?: string
}

/** 各类型标签（前端兜底展示用；后端 ProviderInfo.label 为权威，未下发时用此表） */
export const RESOURCE_LABELS: Record<string, string> = {
  session: '会话',
  model: '模型',
  agent: '智能体',
  skill: '技能',
  mcp: 'MCP',
  setting: '设置',
}