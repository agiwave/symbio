/**
 * Model Provider 管理服务
 *
 * 包装后端 `worker/model/providers/*` 路由：
 * - list  : 列出全部 Provider
 * - get   : 读取单个 Provider
 * - set   : 创建或更新（按 id upsert）
 * - delete: 删除
 * - setDefault: 设置默认 Provider
 *
 * 对应的后端：symbio/src/plugins/model/plugin.rs
 * 对应的 schema：tauri/src/schemas/model_providers.ts
 */

import { callPlugin } from './plugin'
import type {
  ModelProvidersConfig,
  ModelProviderConfig
} from '../schemas/model_providers'
import {
  ModelProvidersList,
  ModelProvidersGet,
  ModelProvidersSet,
  ModelProvidersDelete,
  ModelProvidersSetDefault,
  ModelProvidersTest
} from '../schemas/model_providers'
import { logger } from '@/utils/logger'

const MODEL_PROVIDERS_PATH = 'worker/model/providers'

/** 列出全部 Model Provider（含默认 ID） */
export async function listModelProviders(): Promise<ModelProvidersConfig> {
  try {
    const resp = await callPlugin<ModelProvidersList.Response>(
      `${MODEL_PROVIDERS_PATH}/list`,
      {} satisfies ModelProvidersList.Request
    )
    return resp?.config ?? { providers: {}, default_provider_id: null }
  } catch (err) {
    logger.error('model-providers-service', 'listModelProviders failed:', err)
    return { providers: {}, default_provider_id: null }
  }
}

/** 获取单个 Model Provider */
export async function getModelProvider(providerId: string): Promise<ModelProviderConfig> {
  const resp = await callPlugin<ModelProvidersGet.Response>(
    `${MODEL_PROVIDERS_PATH}/get`,
    { provider_id: providerId } satisfies ModelProvidersGet.Request
  )
  return resp.provider
}

/** 创建或更新一个 Model Provider */
export async function setModelProvider(
  provider: ModelProviderConfig,
  options: { skipValidation?: boolean } = {}
): Promise<ModelProviderConfig> {
  const resp = await callPlugin<ModelProvidersSet.Response>(
    `${MODEL_PROVIDERS_PATH}/set`,
    {
      provider,
      skip_validation: options.skipValidation ?? false
    } satisfies ModelProvidersSet.Request
  )
  return resp.provider
}

/** 删除一个 Model Provider */
export async function deleteModelProvider(providerId: string): Promise<void> {
  await callPlugin<ModelProvidersDelete.Response>(
    `${MODEL_PROVIDERS_PATH}/delete`,
    { provider_id: providerId } satisfies ModelProvidersDelete.Request
  )
}

/** 设置默认 Model Provider */
export async function setDefaultModelProvider(providerId: string): Promise<void> {
  await callPlugin<ModelProvidersSetDefault.Response>(
    `${MODEL_PROVIDERS_PATH}/set_default`,
    { provider_id: providerId } satisfies ModelProvidersSetDefault.Request
  )
}

/**
 * 测试 Model Provider 连接（无副作用——不写入注册表、不落盘）
 *
 * 用于"保存前测试连接"：未保存的草稿配置也可以直接测试。
 * 失败时后端返回 `PluginError::ValidationError`（含具体校验错误）。
 */
export async function testModelProvider(
  provider: ModelProviderConfig,
  options: { skipValidation?: boolean } = {}
): Promise<void> {
  await callPlugin<ModelProvidersTest.Response>(
    `${MODEL_PROVIDERS_PATH}/test`,
    {
      provider,
      skip_validation: options.skipValidation ?? false
    } satisfies ModelProvidersTest.Request
  )
}

/** 把 ModelProvidersConfig 拍平为数组（按 name 排序） */
export function flattenProviders(cfg: ModelProvidersConfig): ModelProviderConfig[] {
  return Object.values(cfg.providers ?? {}).sort((a, b) =>
    (a.name || a.id).localeCompare(b.name || b.id)
  )
}

/**
 * 依据名称/模型/提供商自动生成一个不冲突的 Provider ID（用户不可见）
 *
 * - 由名称等派生可读 slug；`<base>`, `<base>-2`, `<base>-3` … 递增直到不与现有 ID 冲突
 */
export function generateUniqueProviderId(
  base: string,
  existingIds: Iterable<string>
): string {
  const used = new Set(existingIds)
  const slug =
    base
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9-_]+/g, '-')
      .replace(/^-+|-+$/g, '') || 'provider'
  let id = slug
  let counter = 2
  while (used.has(id)) {
    id = `${slug}-${counter}`
    counter++
  }
  return id
}
