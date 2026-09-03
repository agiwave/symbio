/**
 * 资源类型注册表 —— 前端侧"类型 → 描述符"单一真相源
 *
 * 集中维护每类资源的展示标签、路径前缀、专属表单与兜底能力，
 * 供 services（resourcesOp 路径拼接）、视图（ResourceManagerView 多类型
 * 实例化）与资源路径（[provider]/[id].[kind]）统一消费。
 *
 * ## 新增一种资源类型的三步扩展位（如未来的 setting）
 *
 * 1. 后端：`schemas/resources.rs::capabilities_for` 加能力 + 插件 route 顶部接入
 *    `resources::dispatch`（五操作即可用）；
 * 2. 前端：本注册表加一条 descriptor（kind/label/provider/prefix/capabilities）；
 * 3. 可选：注册专属表单组件（`form` 字段），未注册走通用兜底
 *    （zip 面板 / JSON 编辑器 / 只读详情）。
 */

import { markRaw, type Component } from 'vue'
import {
  DEFAULT_CAPABILITIES,
  RESOURCE_LABELS,
  type ResourceCapabilities,
  type ResourceType,
} from '@/schemas/resources'
import ModelProviderForm from '@/components/resources/ModelProviderForm.vue'
import { logger } from '@/utils/logger'

/** 资源类型描述符 */
export interface ResourceTypeDescriptor {
  kind: ResourceType
  /** 展示标签（Model / MCP / ...） */
  label: string
  /** 提供方显示名，用于资源路径 [provider]/[id].[kind]，默认与 kind 相同 */
  provider: string
  /** 后端插件统一路径前缀（resourcesOp 拼接 `${prefix}/resources/<op>`） */
  prefix: string
  /** 专属详情/编辑表单（未注册走通用兜底） */
  form?: Component
  /** 前端兜底能力表（联网失败时渲染布局用；后端下发 capabilities 为真相源） */
  capabilities: ResourceCapabilities
  /** 列表分组 / 类型选择器中的展示顺序 */
  order: number
}

/** 类型注册表：新增资源类型只需在此加一条 */
export const RESOURCE_TYPE_REGISTRY: Record<ResourceType, ResourceTypeDescriptor> = {
  model: {
    kind: 'model',
    label: RESOURCE_LABELS.model,
    provider: 'model',
    prefix: 'worker/model',
    form: markRaw(ModelProviderForm),
    capabilities: DEFAULT_CAPABILITIES.model,
    order: 1,
  },
  mcp: {
    kind: 'mcp',
    label: RESOURCE_LABELS.mcp,
    provider: 'mcp',
    prefix: 'mcp',
    capabilities: DEFAULT_CAPABILITIES.mcp,
    order: 2,
  },
  skill: {
    kind: 'skill',
    label: RESOURCE_LABELS.skill,
    provider: 'skill',
    prefix: 'skill',
    capabilities: DEFAULT_CAPABILITIES.skill,
    order: 3,
  },
  agent: {
    kind: 'agent',
    label: RESOURCE_LABELS.agent,
    provider: 'agent',
    prefix: 'agent',
    capabilities: DEFAULT_CAPABILITIES.agent,
    order: 4,
  },
  session: {
    kind: 'session',
    label: RESOURCE_LABELS.session,
    provider: 'session',
    prefix: 'worker/session',
    capabilities: DEFAULT_CAPABILITIES.session,
    order: 5,
  },
}

/**
 * 'all' 展开顺序：session 除外——其 upload/delete 在后端 dispatch 层为
 * NotImplemented（无实体目录），且会话有专属 SessionView 主 UX；
 * `/resources/session` 仍显式可达（列表 + 实时状态可用）。
 */
export const DEFAULT_RESOURCE_TYPES: ResourceType[] = ['model', 'mcp', 'skill', 'agent']

/**
 * 解析路由 `:types` 参数 → 有序描述符列表（纯函数）。
 *
 * - `undefined / '' / 'all'` → DEFAULT_RESOURCE_TYPES 按 order 排序
 * - 逗号分隔 → trim / 去重 / 过滤未注册 kind（未知 kind 记 warn 后丢弃）
 * - 过滤后为空 → 回退 all
 */
export function parseTypesParam(param?: string): ResourceTypeDescriptor[] {
  const raw = (param ?? '').trim()
  if (!raw || raw === 'all') {
    return DEFAULT_RESOURCE_TYPES.map((k) => RESOURCE_TYPE_REGISTRY[k])
  }
  const seen = new Set<string>()
  const kinds: ResourceType[] = []
  for (const seg of raw.split(',')) {
    const kind = seg.trim()
    if (!kind || seen.has(kind)) continue
    if (!(kind in RESOURCE_TYPE_REGISTRY)) {
      logger.warn('resourceTypes', `parseTypesParam: 未注册的资源类型 "${kind}"，已忽略`)
      continue
    }
    seen.add(kind)
    kinds.push(kind as ResourceType)
  }
  if (kinds.length === 0) {
    return DEFAULT_RESOURCE_TYPES.map((k) => RESOURCE_TYPE_REGISTRY[k])
  }
  return kinds
    .map((k) => RESOURCE_TYPE_REGISTRY[k])
    .sort((a, b) => a.order - b.order)
}

/** 构造资源路径唯一标识：`[provider]/[id].[kind]`（如 `model/openai.model`） */
export function resourcePath(provider: string, id: string, kind: string): string {
  return `${provider}/${id}.${kind}`
}
