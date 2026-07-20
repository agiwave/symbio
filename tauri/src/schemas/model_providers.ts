// Corresponding Backend: symbio/src/symbio_core/schemas/model_providers.rs
import type { ModelConfig, ReasoningConfig } from './model_config'

/**
 * 单个 Model Provider 配置
 *
 * 在 ModelConfig 之上扩展：
 * - id/name : 注册表内唯一标识与展示名
 * - rate_limit_ms : 最小请求间隔（毫秒），0 表示不限制
 * - enabled : 是否启用；禁用后无法被选为活动 Provider
 */
export interface ModelProviderConfig {
  id: string
  name?: string

  // 复用 ModelConfig 的全部字段（运行时直接转换为 ModelConfig）
  provider: string
  api_base: string
  api_key?: string
  model: string
  temperature?: number
  max_tokens?: number
  api_protocol?: string
  system_prompt?: string
  max_context_tokens?: number
  reserved_tokens?: number
  timeout_secs?: number
  store?: boolean
  reasoning?: ReasoningConfig

  /** 最小请求间隔（毫秒）；0 表示不限制 */
  rate_limit_ms?: number

  /** 是否启用；禁用后无法被选为活动 Provider */
  enabled?: boolean
}

/** Provider 注册表 */
export interface ModelProvidersConfig {
  /** 全部 Provider，key 为 ModelProviderConfig.id */
  providers: Record<string, ModelProviderConfig>
  /** 默认 Provider ID */
  default_provider_id?: string | null
}

// ==================== CRUD 请求/响应 ====================

/** providers/list - 列出全部 Model Provider */
export namespace ModelProvidersList {
  export interface Request {}
  export interface Response {
    config: ModelProvidersConfig
  }
}

/** providers/get - 获取单个 Provider */
export namespace ModelProvidersGet {
  export interface Request {
    provider_id: string
  }
  export interface Response {
    provider: ModelProviderConfig
  }
}

/** providers/set - 创建或更新一个 Provider */
export namespace ModelProvidersSet {
  export interface Request {
    provider: ModelProviderConfig
    /** 是否跳过 API 校验；true 时不发起实际校验请求 */
    skip_validation?: boolean
  }
  export interface Response {
    provider: ModelProviderConfig
  }
}

/** providers/delete - 删除一个 Provider */
export namespace ModelProvidersDelete {
  export interface Request {
    provider_id: string
  }
  export interface Response {}
}

/** providers/set_default - 设置默认 Provider */
export namespace ModelProvidersSetDefault {
  export interface Request {
    provider_id: string
  }
  export interface Response {}
}

/** 将 ModelProviderConfig 转为 ModelConfig（运行期视图） */
export function providerToModelConfig(p: ModelProviderConfig): ModelConfig {
  return {
    provider: p.provider,
    api_base: p.api_base,
    api_key: p.api_key,
    model: p.model,
    temperature: p.temperature,
    max_tokens: p.max_tokens,
    api_protocol: p.api_protocol,
    system_prompt: p.system_prompt,
    max_context_tokens: p.max_context_tokens,
    reserved_tokens: p.reserved_tokens,
    timeout_secs: p.timeout_secs,
    store: p.store,
    reasoning: p.reasoning,
  }
}
