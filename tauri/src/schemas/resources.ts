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
 * 统一资源类型（开放 string：可被后端 provider 注册表扩展的任意 kind）。
 * 类型是否存在、能力如何，来自 ProviderInfo，而非本联合类型。
 */
export type ResourceType = string

/** 资源能力开关——决定统一页面启用哪些模块 */
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
  /** 左侧主导航归属分组：'resources'（资源）/ 'settings'（设置）；缺省/空串表示不进导航 */
  nav?: string
}

/** resources/providers 响应 */
export interface ProvidersResponse {
  providers: ProviderInfo[]
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

/** resources/list 响应 */
export interface ResourcesListResponse {
  kind: string
  capabilities: ResourceCapabilities
  items: ResourceSummary[]
}

/** resources/upload 请求 */
export interface ResourceUploadRequest {
  kind: string
  name?: string
  zip_b64?: string
  manifest?: Record<string, unknown> | null
  replace?: boolean
}

/** resources/upload 响应 */
export interface ResourceUploadResponse {
  kind: string
  id: string
  created: boolean
}

/** resources/delete 请求 */
export interface ResourceDeleteRequest {
  kind: string
  id: string
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
  model: 'Model',
  mcp: 'MCP',
  skill: 'Skill',
  agent: 'Agent',
  session: 'Session',
}

// 主导航分组常量（与后端 ProviderInfo.nav 对齐，symbio_core::schemas::resources）
/** 资源分组：model/mcp/agent/skill 等可管理类型 */
export const NAV_RESOURCES = 'resources'
/** 设置分组：setting（左侧"设置"入口由该 provider 动态驱动） */
export const NAV_SETTINGS = 'settings'