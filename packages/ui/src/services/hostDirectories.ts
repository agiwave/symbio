/**
 * Server 宿主目录 API。
 *
 * 这两个接口属于宿主 HTTP 控制面，不是插件路由，因此不能走 callPlugin。
 * endpoint / Bearer token 一律从 plugin.ts 的出站配置读取，避免 localStorage 出现第二份真相。
 */
import {
  getOutboundEndpoint,
  getOutboundToken,
  initGatewayTransport,
} from './plugin'

export interface HostRuntime {
  homedir: string
  bootstrap_path: string
  deployment: 'server'
  capabilities: { directory_browser: boolean }
}

export interface HostDirectoryEntry {
  name: string
  path: string
  kind: 'directory'
}

export interface HostDirectoryRoot {
  name: string
  path: string
}

export interface HostDirectoryListing {
  path: string
  parent: string | null
  roots: HostDirectoryRoot[]
  entries: HostDirectoryEntry[]
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

async function fetchHostJson(pathname: string): Promise<unknown> {
  await initGatewayTransport()
  const endpoint = getOutboundEndpoint().replace(/\/+$/, '')
  if (!endpoint) throw new Error('当前连接没有可用的 Server HTTP 地址')

  const headers: Record<string, string> = { Accept: 'application/json' }
  const token = getOutboundToken()
  if (token) headers.Authorization = `Bearer ${token}`

  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), 15000)
  try {
    const response = await fetch(`${endpoint}${pathname}`, { headers, signal: controller.signal })
    const text = await response.text().catch(() => '')
    let parsed: unknown = null
    try {
      parsed = text ? JSON.parse(text) : null
    } catch {
      // 非 JSON 错误由 HTTP 状态分支统一报出。
    }
    if (!response.ok || (isRecord(parsed) && parsed.error != null)) {
      const detail = isRecord(parsed) && parsed.error != null
        ? String(parsed.error)
        : `HTTP ${response.status}`
      throw new Error(`Server 目录接口失败：${detail}`)
    }
    return parsed
  } catch (error) {
    if (error instanceof Error && error.name === 'AbortError') {
      throw new Error('Server 目录接口请求超时')
    }
    throw error
  } finally {
    clearTimeout(timer)
  }
}

/** GET /api/v1/host/runtime；路径描述的是当前 Server 的宿主。 */
export async function getHostRuntime(): Promise<HostRuntime> {
  const value = await fetchHostJson('/api/v1/host/runtime')
  if (!isRecord(value) || typeof value.homedir !== 'string' ||
      typeof value.bootstrap_path !== 'string' || value.deployment !== 'server' ||
      !isRecord(value.capabilities) || typeof value.capabilities.directory_browser !== 'boolean') {
    throw new Error('Server 运行时响应格式无效')
  }
  return {
    homedir: value.homedir,
    bootstrap_path: value.bootstrap_path,
    deployment: 'server',
    capabilities: { directory_browser: value.capabilities.directory_browser },
  }
}

function normalizeRoots(value: unknown): HostDirectoryRoot[] {
  if (!Array.isArray(value)) return []
  const roots: HostDirectoryRoot[] = []
  for (const item of value) {
    if (typeof item === 'string' && item) {
      roots.push({ name: item, path: item })
    } else if (isRecord(item) && typeof item.path === 'string' && item.path) {
      roots.push({ name: typeof item.name === 'string' && item.name ? item.name : item.path, path: item.path })
    }
  }
  return roots
}

/**
 * GET /api/v1/host/directories?path=<Server 绝对路径>。
 * 省略 path 时由 Server 返回可浏览根；返回值已过滤为目录。
 */
export async function listHostDirectories(path?: string): Promise<HostDirectoryListing> {
  const query = path?.trim() ? `?${new URLSearchParams({ path: path.trim() }).toString()}` : ''
  const value = await fetchHostJson(`/api/v1/host/directories${query}`)
  if (!isRecord(value) || typeof value.path !== 'string' ||
      !(value.parent === null || typeof value.parent === 'string') || !Array.isArray(value.entries)) {
    throw new Error('Server 目录列表响应格式无效')
  }

  const entries: HostDirectoryEntry[] = []
  for (const item of value.entries) {
    if (!isRecord(item) || item.kind !== 'directory' ||
        typeof item.name !== 'string' || typeof item.path !== 'string') continue
    entries.push({ name: item.name, path: item.path, kind: 'directory' })
  }
  return {
    path: value.path,
    parent: value.parent,
    roots: normalizeRoots(value.roots),
    entries,
  }
}
