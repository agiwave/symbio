/**
 * Model Provider 管理服务
 *
 * 基于统一实体协议：chat 侧通过 `listEntities('model')` 读取完整 Provider 注册表
 * （列表项 `extra` 携带完整 `config` 与 `is_default`），与实体管理页共用同一入口，
 * 不再存在独立的 `worker/model/providers/*` 协议。
 *
 * 对应的 schema：tauri/src/schemas/model_providers.ts
 */

import { listEntities } from './entities'
import type { ModelProvidersConfig } from '../schemas/model_providers'

/** 列出全部 Model Provider（含默认 ID） */
export async function listModelProviders(): Promise<ModelProvidersConfig> {
  try {
    const resp = await listEntities('model')
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
