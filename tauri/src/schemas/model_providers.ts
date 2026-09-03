// Corresponding Backend: symbio/src/symbio_core/schemas/model_providers.rs
import type { ReasoningConfig } from './model_config'

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
