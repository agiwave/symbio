export const protocolLabels: Record<string, string> = {
  openai_responses: 'OpenAI Responses API (Beta)',
  openai_chat: 'OpenAI Chat Completions API',
  anthropic_messages: 'Anthropic Messages API',
  gemini_api: 'Google Gemini API'
}

export interface ProviderPreset {
  apiBase: string
  models: string[]
  protocols: string[]
}

/** 供应商标识 → 中文展示名（不在映射内的回退到原始标识） */
export const providerLabels: Record<string, string> = {
  openai: 'OpenAI',
  anthropic: 'Anthropic (Claude)',
  gemini: 'Google Gemini',
  deepseek: 'DeepSeek',
  xai: 'xAI (Grok)',
  groq: 'Groq',
  siliconflow: '硅基流动 (SiliconFlow)',
  alibaba: '阿里云百炼 (Qwen)',
  tencent: '腾讯混元',
  baidu: '百度千帆 (ERNIE)',
  moonshot: '月之暗面 (Kimi)',
  zhipu: '智谱 (GLM)',
  aiyuanjing: '爱媛景 (GLM 兼容)',
  lmstudio: 'LM Studio（本地）',
  local: 'Ollama（本地）',
  mistral: 'Mistral',
  azure: 'Azure OpenAI',
  openrouter: 'OpenRouter',
  perplexity: 'Perplexity',
  volcengine: '火山方舟 (豆包)',
  spark: '讯飞星火',
  minimax: 'MiniMax',
  baichuan: '百川智能',
  step: '阶跃星辰 (StepFun)',
  cerebras: 'Cerebras',
  together: 'Together AI',
  github: 'GitHub Models',
  custom: '自定义 (OpenAI 兼容)'
}

export const providerPresets: Record<string, ProviderPreset> = {
  openai: {
    apiBase: 'https://api.openai.com/v1',
    models: ['gpt-4o-mini', 'gpt-4o', 'gpt-4-turbo', 'gpt-3.5-turbo', 'o1', 'o1-mini', 'o3-mini'],
    protocols: ['openai_responses', 'openai_chat']
  },
  anthropic: {
    apiBase: 'https://api.anthropic.com/v1',
    models: ['claude-3-7-sonnet-latest', 'claude-3-5-sonnet-latest', 'claude-3-5-haiku-latest', 'claude-3-opus-latest'],
    protocols: ['anthropic_messages']
  },
  gemini: {
    apiBase: 'https://generativelanguage.googleapis.com/v1beta',
    models: ['gemini-2.5-pro', 'gemini-2.5-flash', 'gemini-2.0-pro-exp-02-05', 'gemini-2.0-flash', 'gemini-2.0-flash-lite-preview-02-05', 'gemini-2.0-flash-thinking-exp-01-21'],
    protocols: ['gemini_api']
  },
  deepseek: {
    apiBase: 'https://api.deepseek.com/v1',
    models: ['deepseek-chat', 'deepseek-coder', 'deepseek-reasoner'],
    protocols: ['openai_chat']
  },
  xai: {
    apiBase: 'https://api.x.ai/v1',
    models: ['grok-2-latest', 'grok-2-vision-latest'],
    protocols: ['openai_chat']
  },
  groq: {
    apiBase: 'https://api.groq.com/openai/v1',
    models: ['llama-3.3-70b-versatile', 'llama-3.1-8b-instant', 'mixtral-8x7b-32768'],
    protocols: ['openai_chat']
  },
  mistral: {
    apiBase: 'https://api.mistral.ai/v1',
    models: ['mistral-large-latest', 'mistral-small-latest', 'codestral-latest', 'open-mistral-nemo'],
    protocols: ['openai_chat']
  },
  azure: {
    apiBase: 'https://<your-resource>.openai.azure.com/openai/v1',
    models: [],
    protocols: ['openai_chat']
  },
  openrouter: {
    apiBase: 'https://openrouter.ai/api/v1',
    models: ['openrouter/auto', 'anthropic/claude-3.5-sonnet', 'openai/gpt-4o', 'meta-llama/llama-3.3-70b-instruct'],
    protocols: ['openai_chat']
  },
  perplexity: {
    apiBase: 'https://api.perplexity.ai',
    models: ['sonar-pro', 'sonar', 'sonar-reasoning'],
    protocols: ['openai_chat']
  },
  volcengine: {
    apiBase: 'https://ark.cn-beijing.volces.com/api/v3',
    models: ['doubao-seed-1-6-250615', 'doubao-1-5-pro-32k-250115', 'doubao-1-5-lite-32k-250115'],
    protocols: ['openai_chat']
  },
  spark: {
    apiBase: 'https://spark-api-open.xf-yun.com/v1',
    models: ['4.0Ultra', 'max-32k', 'lite'],
    protocols: ['openai_chat']
  },
  minimax: {
    apiBase: 'https://api.minimax.chat/v1',
    models: ['MiniMax-Text-01', 'abab6.5s-chat'],
    protocols: ['openai_chat']
  },
  baichuan: {
    apiBase: 'https://api.baichuan-ai.com/v1',
    models: ['Baichuan4', 'Baichuan4-Turbo'],
    protocols: ['openai_chat']
  },
  step: {
    apiBase: 'https://api.stepfun.com/v1',
    models: ['step-2-16k', 'step-1-8k'],
    protocols: ['openai_chat']
  },
  cerebras: {
    apiBase: 'https://api.cerebras.ai/v1',
    models: ['llama-3.3-70b', 'llama-3.1-8b'],
    protocols: ['openai_chat']
  },
  together: {
    apiBase: 'https://api.together.xyz/v1',
    models: ['meta-llama/Llama-3.3-70B-Instruct-Turbo', 'deepseek-ai/DeepSeek-V3'],
    protocols: ['openai_chat']
  },
  github: {
    apiBase: 'https://models.github.ai/inference',
    models: ['gpt-4o', 'gpt-4o-mini'],
    protocols: ['openai_chat']
  },
  siliconflow: {
    apiBase: 'https://api.siliconflow.cn/v1',
    models: ['deepseek-ai/DeepSeek-R1', 'deepseek-ai/DeepSeek-V3', 'Qwen/Qwen2.5-72B-Instruct'],
    protocols: ['openai_chat']
  },
  alibaba: {
    apiBase: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
    models: ['qwen-plus', 'qwen-max', 'qwen-turbo', 'qwen2.5-72b-instruct'],
    protocols: ['openai_chat']
  },
  tencent: {
    apiBase: 'https://api.hunyuan.cloud.tencent.com/v1',
    models: ['hunyuan-pro', 'hunyuan-standard', 'hunyuan-lite'],
    protocols: ['openai_chat']
  },
  baidu: {
    apiBase: 'https://qianfan.baidubce.com/v2',
    models: ['ernie-4.0-8k-latest', 'ernie-3.5-8k', 'ernie-speed-128k'],
    protocols: ['openai_chat']
  },
  moonshot: {
    apiBase: 'https://api.moonshot.cn/v1',
    models: ['moonshot-v1-8k', 'moonshot-v1-32k', 'moonshot-v1-128k'],
    protocols: ['openai_chat']
  },
  zhipu: {
    apiBase: 'https://open.bigmodel.cn/api/paas/v4',
    models: ['glm-4.7-flash','glm-4-plus', 'glm-4-flash', 'glm-4', 'glm-3-turbo'],
    protocols: ['openai_chat']
  },
  aiyuanjing: {
    apiBase: 'https://maas-api.ai-yuanjing.com/openapi/compatible-mode/v1',
    models: ['glm-5', 'glm-4-plus', 'glm-4'],
    protocols: ['openai_chat']
  },
  lmstudio: {
    apiBase: 'http://localhost:1234/v1',
    models: [],
    protocols: ['anthropic_messages', 'openai_chat', 'openai_responses']
  },
  local: {
    apiBase: 'http://localhost:11434/v1',
    models: ['llama3', 'qwen2', 'mistral', 'deepseek-coder-v2'],
    protocols: ['openai_chat']
  },
  custom: {
    apiBase: '',
    models: [],
    protocols: ['openai_responses', 'openai_chat', 'anthropic_messages', 'gemini_api']
  }
}
