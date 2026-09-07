/**
 * 统一实体服务 — 五类实体（model / mcp / skill / agent / session）共享同一套 entities/* 协议
 *
 * 与后端对齐：symbio/src/symbio_core/schemas/entities.rs + 各插件 entities/* 路由。
 * 能力开关驱动 UI：zip 上传 / 独立表单 / 实时状态 / 可删除 / 连接测试。
 */

import { callPlugin } from './plugin'
import {
  type DetailDefinition,
  type DetailDefinitionResponse,
  type ProviderInfo,
  type ProvidersResponse,
  type EntityCapabilities,
  type EntityStatusResponse,
  type EntitySummary,
  type EntitiesListResponse,
  type EntityUploadResponse,
} from '../schemas/entities'
import { logger } from '@/utils/logger'

/** 未知类型兜底能力：一律只读空态（不存在可创建/删除/表单等） */
const UNKNOWN_CAPABILITIES: EntityCapabilities = {
  zip_upload: false,
  independent_form: false,
  realtime_status: false,
  // 未登记类型：刷新幂等无害，保守保留入口（与后端 capabilities_for 兜底一致）
  refreshable: true,
  mutable: false,
  test_connection: false,
  read_only: true,
}

/**
 * 拉取已注册实体 provider（宿主级单一真相源）。
 * 前端据此动态生成左侧导航与统一实体页类型集合。
 * 同时填充 `providerPrefix` 缓存，供各 entities 操作拼接实体路径前缀。
 */
export async function fetchProviders(): Promise<ProviderInfo[]> {
  try {
    const resp = await callPlugin<ProvidersResponse>('entities/providers', {})
    const providers = resp?.providers ?? []
    providerPrefix = {}
    for (const p of providers) {
      providerPrefix[p.kind] = p.prefix
    }
    return providers
  } catch (err) {
    logger.error('entities-service', 'fetchProviders failed:', err)
    return []
  }
}

/**
 * 实体操作路径前缀缓存（kind → prefix，如 model→worker/model、session→worker/session）。
 * 由 fetchProviders 填充；未加载时回退 kind（仅 mcp/agent/skill 等顶层前缀可直接用）。
 */
let providerPrefix: Record<string, string> = {}

/** 解析 kind 的实体操作前缀（未加载记录时回退 kind，由调用方保证先 fetchProviders） */
function opPrefix(type: string): string {
  return providerPrefix[type] ?? type
}

function entitiesOp<T>(type: string, op: string, payload?: unknown): Promise<T> {
  return callPlugin<T>(`${opPrefix(type)}/entities/${op}`, payload)
}

/**
 * 列出某类型全部实体。
 *
 * `opts.container` 存在时为**容器语义**：列出该容器条目内部的子实体
 * （如某 agent bundle 的 prompts/skills/mcps），items 的 kind 字段区分子类型。
 * 失败时返回空态 + 只读兜底能力。
 */
export async function listEntities(
  type: string,
  opts?: { container?: string; subKind?: string; parent?: string }
): Promise<EntitiesListResponse> {
  try {
    const resp = await entitiesOp<EntitiesListResponse>(type, 'list', {
      container: opts?.container || undefined,
      sub_kind: opts?.subKind || undefined,
      parent: opts?.parent || undefined,
    })
    return resp ?? { kind: type, capabilities: UNKNOWN_CAPABILITIES, items: [] }
  } catch (err) {
    logger.error('entities-service', `listEntities(${type}) failed:`, err)
    return { kind: type, capabilities: UNKNOWN_CAPABILITIES, items: [] }
  }
}

/**
 * 读取单个实体详情。
 *
 * `container` 存在时为容器语义：读取容器条目内部的子实体（id 为容器内相对
 * 路径），文件内容随 `EntitySummary.extra.content` 返回。失败返回 null。
 */
export async function getEntity(
  type: string,
  id: string,
  container?: string
): Promise<EntitySummary | null> {
  try {
    const resp = await entitiesOp<EntitySummary>(type, 'get', {
      kind: type,
      id,
      container: container || undefined,
    })
    return resp ?? null
  } catch (err) {
    logger.error('entities-service', `getEntity(${type}/${id}) failed:`, err)
    return null
  }
}

/** 上传 zip 创建/更新实体（name 即实体目录名）。返回新实体 id */
export async function uploadEntityZip(
  type: string,
  name: string,
  zipBytes: ArrayBuffer
): Promise<EntityUploadResponse> {
  const zip_b64 = arrayBufferToBase64(zipBytes)
  const resp = await entitiesOp<EntityUploadResponse>(type, 'upload', {
    kind: type,
    name,
    zip_b64,
    replace: true,
  })
  return resp
}

/** 以 JSON 表单（manifest）创建/更新实体（independent_form 类型：model / session） */
export async function uploadEntityForm(
  type: string,
  name: string,
  manifest: Record<string, unknown>
): Promise<EntityUploadResponse> {
  const resp = await entitiesOp<EntityUploadResponse>(type, 'upload', {
    kind: type,
    name,
    manifest,
    replace: true,
  })
  return resp
}

/** 删除实体（`container` 存在时为容器语义：id 为容器内相对路径） */
export async function deleteEntity(
  type: string,
  id: string,
  container?: string
): Promise<void> {
  await entitiesOp(type, 'delete', {
    kind: type,
    id,
    container: container || undefined,
  })
}

/** 查询单个实体实时/连接状态（capabilities.realtime_status 为 true 时使用） */
export async function getEntityStatus(
  type: string,
  id: string
): Promise<EntityStatusResponse | null> {
  try {
    const resp = await entitiesOp<EntityStatusResponse>(type, 'status', { kind: type, id })
    return resp ?? null
  } catch (err) {
    logger.debug('entities-service', `getEntityStatus(${type}/${id}) failed:`, err)
    return null
  }
}

/**
 * 获取详情页定义（definition-driven detail）。
 *
 * `id` 为空 = 「新建态」定义；provider 未实现定义钩子时返回 null
 * （前端回退注册 editor / 通用兜底面板）。
 */
export async function getDetailDefinition(
  type: string,
  id = ''
): Promise<DetailDefinition | null> {
  try {
    const resp = await entitiesOp<DetailDefinitionResponse>(type, 'detail', { kind: type, id })
    return resp?.definition ?? null
  } catch (err) {
    logger.debug('entities-service', `getDetailDefinition(${type}/${id}) failed:`, err)
    return null
  }
}

/**
 * 订阅/取消容器子实体数据变更 —— 统一协议 watch/unwatch 操作对。
 *
 * 树视图等实时场景在视图挂载时订阅、卸载时取消（生命周期与视图绑定）；
 * 变更经粗粒度 `data` 事件下发（kind = provider kind、sessionId = 容器 id）。
 * 两个函数均吞错（fire-and-forget：无实时能力的 provider 由后端默认 no-op，
 * 前端失败不影响视图功能）。
 */
export async function watchEntity(
  type: string,
  opts: { container: string; subKind?: string }
): Promise<void> {
  try {
    await entitiesOp(type, 'watch', { container: opts.container, sub_kind: opts.subKind || undefined })
  } catch (err) {
    logger.debug('entities-service', `watchEntity(${type}) failed:`, err)
  }
}

export async function unwatchEntity(
  type: string,
  opts: { container: string; subKind?: string }
): Promise<void> {
  try {
    await entitiesOp(type, 'unwatch', { container: opts.container, sub_kind: opts.subKind || undefined })
  } catch (err) {
    logger.debug('entities-service', `unwatchEntity(${type}) failed:`, err)
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

/**
 * 写入（创建/覆盖）容器子实体 —— 统一协议容器语义。
 *
 * `path` 为容器内相对路径（如 `prompts/persona.md`），内容经 `manifest.content`
 * 下发；后端做路径白名单校验。响应与顶层 upload 一致（EntityUploadResponse）。
 */
export async function putContainerEntity(
  type: string,
  path: string,
  content: string,
  container: string
): Promise<EntityUploadResponse> {
  return entitiesOp<EntityUploadResponse>(type, 'upload', {
    kind: type,
    name: path,
    manifest: { content },
    container,
  })
}