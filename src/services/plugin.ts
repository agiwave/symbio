/**
 * 插件调用服务
 *
 * 提供统一的后端插件调用接口
 */

// 检测是否在 Tauri 环境中
const isTauri = typeof window !== 'undefined' && '__TAURI__' in window

// 动态导入 Tauri API
let invoke: any = null
if (isTauri) {
  import('@tauri-apps/api/core').then((module) => {
    invoke = module.invoke
  })
}

export interface StreamChunk {
  data: unknown
  done: boolean
  error?: string
}

interface PluginResponse {
  success?: boolean
  data?: unknown
  message?: string
  error?: string
}

// 浏览器环境的 mock 数据
const mockResponses: Record<string, Record<string, unknown>> = {
  work: {
    init: { success: true },
    list: { documents: [] },
    create: { id: 'mock-doc-1', title: '新文档', content: '', parentId: null },
    get: { id: 'mock-doc-1', title: '文档', content: '# 示例内容', parentId: null },
    update: { success: true },
    delete: { success: true },
  },
  setting: {
    get: { success: true },
    set: { success: true },
  },
  config: {
    get: { plugins: {} },
    save: { success: true },
    load: { success: true },
    collect: { success: true, config: {} },
  },
  'agent/openai': {
    get: {
      api_base: 'https://api.openai.com/v1',
      api_key_set: false,
      model: 'gpt-4o-mini',
      temperature: 0.7,
      max_tokens: 4096,
      max_context_tokens: 128000,
    },
    set: { success: true },
  },
  'agent/session': {
    get: {
      success: true,
      session: {
        id: 'mock-session',
        messages: [],
        created_at: Date.now(),
        updated_at: Date.now(),
        metadata: {}
      }
    },
    set: { success: true },
    list: {
      success: true,
      sessions: []
    },
  },
  'agent/memory': {
    get: {
      storage_dir: '',
      max_entries: 1000,
      categories: ['general', 'code', 'project'],
    },
    set: { success: true },
  },
  'agent/tools': {
    get: {
      shell_enabled: true,
      file_enabled: true,
      web_enabled: true,
      allowed_paths: ['~'],
      blocked_commands: ['rm -rf', 'sudo', 'chmod 777'],
      shell_timeout: 60,
      web_timeout: 30,
    },
    set: { success: true },
  },
  'work/config': {
    get: {
      workspace_path: '~/projects',
      auto_save: true,
      auto_save_interval: 30000,
      recent_files: [],
    },
    set: { success: true },
  },
  'setting/config': {
    get: {},
    set: { success: true },
  },
  'agent/chat': {
    send: { content: '这是一个模拟的 AI 响应。实际使用时需要接入 AI API。' },
  },
  'agent/@llm': {
    configure: { message: '配置已保存' },
    get_config: {
      success: true,
      config: {
        api_base: 'https://api.openai.com/v1',
        api_key_set: false,
        model: 'gpt-4o-mini',
        temperature: 0.7,
        max_tokens: 4096,
        max_context_tokens: 128000,
      },
    },
  },
}

/**
 * 调用插件
 * @param path 插件路径，如 'work', 'agent/chat', 'setting'
 * @param input 输入参数
 */
export async function callPlugin<T = unknown>(
  path: string,
  input: Record<string, unknown>
): Promise<T> {
  console.log('[plugin] calling:', path, 'with input:', input)

  // 浏览器环境：返回 mock 数据
  if (!isTauri || !invoke) {
    console.log('[plugin] Running in browser mode, returning mock data')
    
    // 首先尝试完整路径匹配
    if (mockResponses[path]) {
      const action = input.action as string
      if (mockResponses[path][action]) {
        return mockResponses[path][action] as T
      }
    }
    
    // 然后尝试按第一段路径匹配（兼容旧格式）
    const [pluginName] = path.split('/')
    const action = input.action as string
    
    if (mockResponses[pluginName]?.[action]) {
      return mockResponses[pluginName][action] as T
    }
    
    // 默认返回成功
    return { success: true } as T
  }

  // Tauri 环境：调用后端
  const result = await invoke<StreamChunk[]>('invoke', {
    path,
    input
  })

  console.log('[plugin] result:', result)

  // invoke 返回 StreamChunk 数组，取第一个
  if (Array.isArray(result) && result.length > 0) {
    const chunk = result[0]
    
    // 检查错误
    if (chunk.error) {
      throw new Error(chunk.error)
    }
    
    // 解包响应
    const data = chunk.data as PluginResponse
    
    // 如果有 error 字段，抛出错误
    if (data.error) {
      throw new Error(data.error)
    }
    
    // 如果有 data 字段，返回 data 内容
    if (data.data !== undefined) {
      return data.data as T
    }
    
    // 否则直接返回整个 data
    return data as T
  }
  
  throw new Error('插件调用返回空结果')
}

/**
 * 获取插件元数据
 */
export async function getPluginMeta(path: string) {
  if (!isTauri || !invoke) {
    return { path, meta: { name: path, description: 'Mock plugin' } }
  }
  return invoke<{ path: string; meta: unknown }>('meta', { path })
}
