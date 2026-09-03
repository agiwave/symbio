/**
 * Model Provider 管理服务
 *
 * 基于统一资源协议：chat 侧通过 `listResources('model')` 读取完整 Provider 注册表
 * （列表项 `extra` 携带完整 `config` 与 `is_default`），与资源管理页共用同一入口，
 * 不再存在独立的 `worker/model/providers/*` 协议。
 *
 * 对应的 schema：tauri/src/schemas/model_providers.ts
 */

import { listResources } from './resources'
import type { ModelProvidersConfig } from '../schemas/model_providers'

/** 列出全部 Model Provider（含默认 ID） */
export async function listModelProviders(): Promise<ModelProvidersConfig> {
  try {
    const resp = await listResources('model')
    const providers: Record<string, ModelProvidersConfig['providers'][string]> = {}
    let default_provider_id: string | null = null
    for (const it of resp.items ?? []) {
      const cfg = it.config as ModelProvidersConfig['providers'][string] | undefined
      if (cfg && cfg.id) {
        providers[cfg.id] = cfg
      }
      if (it.is_default) {
        default_provider_id = it.id
      }
    }
    return { providers, default_provider_id }
  } catch (err) {
    return { providers: {}, default_provider_id: null }
  }
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
