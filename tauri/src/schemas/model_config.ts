// Corresponding Backend: symbio/src/symbio_core/schemas/model_config.rs (ModelConfig)

export interface ReasoningConfig {
  effort: string;
}

export interface ModelConfig {
  provider: string;
  api_base: string;
  api_key?: string;
  model: string;
  temperature?: number;
  max_tokens?: number;
  api_protocol?: string;
  system_prompt?: string;
  max_context_tokens?: number;
  reserved_tokens?: number;
  timeout_secs?: number;
  store?: boolean;
  reasoning?: ReasoningConfig;
}
