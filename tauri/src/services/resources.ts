/**
 * 统一资源服务 — 五类资源（model / mcp / skill / agent / session）共享同一套 resources/* 协议
 *
 * 与后端对齐：symbio/src/symbio_core/schemas/resources.rs + 各插件 resources/* 路由。
 * 能力开关驱动 UI：zip 上传 / 独立表单 / 实时状态 / 可删除 / 连接测试。
 */

import { callPlugin } from './plugin'
import {
  type ProviderInfo,
  type ProvidersResponse,
  type ResourceCapabilities,
  type ResourceStatusResponse,
  type ResourcesListResponse,
  type ResourceUploadResponse,
} from '../schemas/resources'
import { logger } from '@/utils/logger'

/** 未知类型兜底能力：一律只读空态（不存在可创建/删除/表单等） */
const UNKNOWN_CAPABILITIES: ResourceCapabilities = {
  zip_upload: false,
  independent_form: false,
  realtime_status: false,
  mutable: false,
  test_connection: false,
  read_only: true,
}

/**
 * 拉取已注册资源 provider（宿主级单一真相源）。
 * 前端据此动态生成左侧导航与统一资源页类型集合。
 * 同时填充 `providerPrefix` 缓存，供各 resources 操作拼接资源路径前缀。
 */
export async function fetchProviders(): Promise<ProviderInfo[]> {
  try {
    const resp = await callPlugin<ProvidersResponse>('resources/providers', {})
    const providers = resp?.providers ?? []
    providerPrefix = {}
    for (const p of providers) {
      providerPrefix[p.kind] = p.prefix
    }
    return providers
  } catch (err) {
    logger.error('resources-service', 'fetchProviders failed:', err)
    return []
  }
}

/**
 * 资源操作路径前缀缓存（kind → prefix，如 model→worker/model、session→worker/session）。
 * 由 fetchProviders 填充；未加载时回退 kind（仅 mcp/agent/skill 等顶层前缀可直接用）。
 */
let providerPrefix: Record<string, string> = {}

/** 解析 kind 的资源操作前缀（未加载记录时回退 kind，由调用方保证先 fetchProviders） */
function opPrefix(type: string): string {
  return providerPrefix[type] ?? type
}

function resourcesOp<T>(type: string, op: string, payload?: unknown): Promise<T> {
  return callPlugin<T>(`${opPrefix(type)}/resources/${op}`, payload)
}

/** 列出某类型全部资源（含能力开关），失败时返回空态 + 只读兜底能力 */
export async function listResources(type: string): Promise<ResourcesListResponse> {
  try {
    const resp = await resourcesOp<ResourcesListResponse>(type, 'list', {})
    return resp ?? { kind: type, capabilities: UNKNOWN_CAPABILITIES, items: [] }
  } catch (err) {
    logger.error('resources-service', `listResources(${type}) failed:`, err)
    return { kind: type, capabilities: UNKNOWN_CAPABILITIES, items: [] }
  }
}

/** 上传 zip 创建/更新资源（name 即资源目录名）。返回新资源 id */
export async function uploadResourceZip(
  type: string,
  name: string,
  zipBytes: ArrayBuffer
): Promise<ResourceUploadResponse> {
  const zip_b64 = arrayBufferToBase64(zipBytes)
  const resp = await resourcesOp<ResourceUploadResponse>(type, 'upload', {
    kind: type,
    name,
    zip_b64,
    replace: true,
  })
  return resp
}

/** 以 JSON 表单（manifest）创建/更新资源（independent_form 类型：model / session） */
export async function uploadResourceForm(
  type: string,
  name: string,
  manifest: Record<string, unknown>
): Promise<ResourceUploadResponse> {
  const resp = await resourcesOp<ResourceUploadResponse>(type, 'upload', {
    kind: type,
    name,
    manifest,
    replace: true,
  })
  return resp
}

/** 删除资源 */
export async function deleteResource(type: string, id: string): Promise<void> {
  await resourcesOp(type, 'delete', { kind: type, id })
}

/** 查询单个资源实时/连接状态（capabilities.realtime_status 为 true 时使用） */
export async function getResourceStatus(
  type: string,
  id: string
): Promise<ResourceStatusResponse | null> {
  try {
    const resp = await resourcesOp<ResourceStatusResponse>(type, 'status', { kind: type, id })
    return resp ?? null
  } catch (err) {
    logger.debug('resources-service', `getResourceStatus(${type}/${id}) failed:`, err)
    return null
  }
}

/** ArrayBuffer → base64（zip 经 JSON payload 上传） */
export function arrayBufferToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer)
  let binary = ''
  const CHUNK = 0x8000
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK))
  }
  return btoa(binary)
}