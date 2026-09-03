/**
 * 统一资源协议（前端侧类型契约）
 *
 * 与后端大面积对齐：symbio/src/symbio_core/schemas/resources.rs
 * 覆盖 model / mcp / skill / agent / session 五类资源。
 * 所有资源共享同一套 resources/* 操作与能力开关，前端据此驱动统一页面。
 */

/** 统一资源类型 */
export type ResourceType = 'model' | 'mcp' | 'skill' | 'agent' | 'session'

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

/** 各类型中文标签（前端展示用） */
export const RESOURCE_LABELS: Record<ResourceType, string> = {
  model: 'Model',
  mcp: 'MCP',
  skill: 'Skill',
  agent: 'Agent',
  session: 'Session',
}

/** 前端兜底能力表（联网失败/未接线时用于渲染布局，与后端 capabilities_for 保持一致） */
export const DEFAULT_CAPABILITIES: Record<ResourceType, ResourceCapabilities> = {
  model: { zip_upload: false, independent_form: true, realtime_status: false, mutable: true, test_connection: true, read_only: false },
  mcp: { zip_upload: true, independent_form: false, realtime_status: true, mutable: true, test_connection: true, read_only: false },
  skill: { zip_upload: true, independent_form: false, realtime_status: false, mutable: true, test_connection: false, read_only: false },
  agent: { zip_upload: true, independent_form: false, realtime_status: false, mutable: true, test_connection: false, read_only: false },
  session: { zip_upload: false, independent_form: true, realtime_status: true, mutable: true, test_connection: false, read_only: false },
}