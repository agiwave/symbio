/**
 * 统一资源服务 — 五类资源（model / mcp / skill / agent / session）共享同一套 resources/* 协议
 *
 * 与后端对齐：symbio/src/symbio_core/schemas/resources.rs + 各插件 resources/* 路由。
 * 能力开关驱动 UI：zip 上传 / 独立表单 / 实时状态 / 可删除 / 连接测试。
 */

import { callPlugin } from './plugin'
import {
  DEFAULT_CAPABILITIES,
  type ResourceCapabilities,
  type ResourceStatusResponse,
  type ResourcesListResponse,
  type ResourceSummary,
  type ResourceType,
  type ResourceUploadResponse,
} from '../schemas/resources'
import { logger } from '@/utils/logger'

/** 各类型 → 后端插件统一路径前缀（前端传 `${prefix}/resources/<op>`） */
const RESOURCE_PATHS: Record<ResourceType, string> = {
  model: 'worker/model',
  mcp: 'mcp',
  skill: 'skill',
  agent: 'agent',
  session: 'worker/session',
}

/** 获取某类型统一资源路径 */
export function resourcePath(type: ResourceType): string {
  return RESOURCE_PATHS[type]
}

function resourcesOp<T>(type: ResourceType, op: string, payload?: unknown): Promise<T> {
  return callPlugin<T>(`${RESOURCE_PATHS[type]}/resources/${op}`, payload)
}

/** 兜底能力表 */
export function capabilitiesFor(type: ResourceType): ResourceCapabilities {
  return DEFAULT_CAPABILITIES[type]
}

/** 列出某类型全部资源（含能力开关），失败时返回空态 + 兜底能力 */
export async function listResources(type: ResourceType): Promise<ResourcesListResponse> {
  try {
    const resp = await resourcesOp<ResourcesListResponse>(type, 'list', {})
    return resp ?? { kind: type, capabilities: DEFAULT_CAPABILITIES[type], items: [] }
  } catch (err) {
    logger.error('resources-service', `listResources(${type}) failed:`, err)
    return { kind: type, capabilities: DEFAULT_CAPABILITIES[type], items: [] }
  }
}

/** 上传 zip 创建/更新资源（name 即资源目录名）。返回新资源 id */
export async function uploadResourceZip(
  type: ResourceType,
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
  type: ResourceType,
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
export async function deleteResource(type: ResourceType, id: string): Promise<void> {
  await resourcesOp(type, 'delete', { kind: type, id })
}

/** 查询单个资源实时/连接状态（capabilities.realtime_status 为 true 时使用） */
export async function getResourceStatus(
  type: ResourceType,
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

/** 由后端统一列表项普通化处理；若某类型尚未接线，则保持原输出 */
export function toSummary(type: ResourceType, raw: ResourceSummary): ResourceSummary {
  return raw.kind ? raw : { ...raw, kind: type }
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