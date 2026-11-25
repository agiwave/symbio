/**
 * 系统目录 (System Location) —— 前端持久化的「连接目标」事实源。
 *
 * 运行环境与连接目标是两条独立坐标，不能互相推断：
 * - 运行环境：`embedded`（Tauri WebView，有 IPC）或 `browser`（普通浏览器，无 IPC）；
 * - 连接目标：`native`（embedded 进程内后端）或 `server`（HTTP/WS Server）。
 *
 * `local` / `remote` 只是连接目标的 UI 兼容写法：
 * - embedded + local：native，homedir 位于这台机器；
 * - browser + local：同源 Server，homedir 位于该 Server，而不是浏览器本机；
 * - remote：另一台 Server 的 HTTP/WS 地址，Bearer token 只放 Authorization 请求头。
 *
 * 浏览器没有 native 出口：远端不可达只记录 `locationError`，绝不回退 Tauri IPC。
 * `forceNative` 只是控制面提示；embedded 已选 Server 时仍走该 Server，browser 中表示
 * 当前 Server 的控制面 HTTP。homedir 是 Symbio 配置根，workdir 是会话工作区，两者不互换。
 */

import { ref, type Ref } from 'vue'

/** 前端运行环境；由 Tauri IPC 是否存在决定，不由持久化配置决定。 */
export type RuntimeEnvironment = 'embedded' | 'browser'

/** 传输连接目标；server 同时覆盖同源 Server 与远端 Server。 */
export type ConnectionTarget = 'native' | 'server'

/** 运行环境 + 连接目标的显式模型。 */
export interface FrontendConnectionModel {
  environment: RuntimeEnvironment
  target: ConnectionTarget
}

/** 系统目录类型（保留 local / remote 作为持久化与 UI 兼容层）。 */
export type SystemLocationKind = 'local' | 'remote'

/** 系统目录描述。localPath 始终是目标后端的 homedir，不是会话 workdir。 */
export interface SystemLocation {
  /** 连接目标类型 */
  kind: SystemLocationKind
  /** local：目标后端 homedir；embedded 为本机，browser 为同源 Server。空串表示后端默认值。 */
  localPath?: string
  /** remote：Server 基础地址（不含 token），如 `http://host:port` */
  remoteUrl?: string
  /** remote：访问密钥（Bearer Token），不得拼进 WS URL。 */
  remoteKey?: string
}

/**
 * 是否运行在无 Tauri IPC 的浏览器中。
 *
 * Tauri v2 在注入原生桥时提供 `window.__TAURI_INTERNALS__.invoke`；Node 单测或普通
 * 浏览器没有它，一律按 browser 处理。宁可保守判定，也不能在无 IPC 时尝试原生调用。
 */
export function isBrowserEnvironment(): boolean {
  if (typeof window === 'undefined') return true
  const internals = (window as Window & {
    __TAURI_INTERNALS__?: { invoke?: unknown }
  }).__TAURI_INTERNALS__
  return typeof internals?.invoke !== 'function'
}

const STORAGE_KEY = 'symbio.systemLocation'

/** 默认：本地默认目录 */
export const DEFAULT_LOCATION: SystemLocation = { kind: 'local', localPath: '' }

// ── 响应式状态（供 UI 展示当前连接目标 / 连接错误）──
export const currentLocation: Ref<SystemLocation> = ref({ ...DEFAULT_LOCATION })
export const locationError: Ref<string | null> = ref(null)

export function setCurrentLocation(loc: SystemLocation): void {
  currentLocation.value = loc
}

export function setLocationError(msg: string | null): void {
  locationError.value = msg
}

/**
 * 读取已持久化的系统目录；无记录或损坏时返回默认（本地默认）。
 */
export function loadSystemLocation(): SystemLocation {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return { ...DEFAULT_LOCATION }
    const parsed = JSON.parse(raw)
    if (parsed && (parsed.kind === 'local' || parsed.kind === 'remote')) {
      return {
        kind: parsed.kind,
        localPath: typeof parsed.localPath === 'string' ? parsed.localPath : '',
        remoteUrl: typeof parsed.remoteUrl === 'string' ? parsed.remoteUrl : '',
        remoteKey: typeof parsed.remoteKey === 'string' ? parsed.remoteKey : '',
      }
    }
  } catch {
    /* 损坏的 storage 直接忽略，回退默认 */
  }
  return { ...DEFAULT_LOCATION }
}

/**
 * 持久化系统目录到 localStorage，并同步响应式状态。
 */
export function saveSystemLocation(loc: SystemLocation): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(loc))
  } catch {
    /* storage 可能不可用（隐私模式等）；非致命 */
  }
  setCurrentLocation(loc)
}

/**
 * 解析远端地址字符串：支持将 key 直接写在 URL 的 `?token=` / `?key=` 查询参数中
 * （满足「远端地址字符串包含 key」的需求），分离出不含 token 的 baseUrl 与 key。
 */
export function parseRemoteAddress(input: string): { url: string; key: string } {
  const trimmed = input.trim()
  let url = trimmed
  let key = ''
  const qIndex = trimmed.indexOf('?')
  if (qIndex !== -1) {
    const base = trimmed.slice(0, qIndex)
    const query = trimmed.slice(qIndex + 1)
    url = base
    for (const pair of query.split('&')) {
      const eq = pair.indexOf('=')
      if (eq === -1) continue
      const k = pair.slice(0, eq)
      const v = pair.slice(eq + 1)
      if (k === 'token' || k === 'key') key = decodeURIComponent(v)
    }
  }
  // 去掉尾斜杠，统一格式
  url = url.replace(/\/+$/, '')
  return { url, key }
}

/**
 * 远端地址格式是否合法（http/https 且含 host）
 *
 * ⚠️ 判据说的是**主机名**，不是「主机:端口」：必须先去掉端口再判定。
 * 此前拿 `localhost:8080` 整体去比 `'localhost'`，于是最常见的开发目标
 * （本机网关跑在 8080）被误判为非法——带端口的 localhost 进不去。
 */
export function isValidRemoteUrl(url: string): boolean {
  if (!/^https?:\/\//i.test(url)) return false
  const hostPort = url.replace(/^https?:\/\//i, '').split('/')[0].split('?')[0]
  if (!hostPort) return false
  // 去端口：IPv6 字面量是 `[::1]:8080` 形式，方括号内的冒号不能当端口分隔符
  const host = hostPort.startsWith('[')
    ? hostPort.slice(0, hostPort.indexOf(']') + 1)
    : hostPort.split(':')[0]
  if (!host) return false
  const isIp = /^\d+(\.\d+){3}$/.test(host) || host.startsWith('[')
  return host === 'localhost' || host.includes('.') || isIp
}

/** 展示用文本 */
export function formatLocation(loc: SystemLocation): string {
  if (loc.kind === 'remote') return loc.remoteUrl || '(远端)'
  if (isBrowserEnvironment()) return loc.localPath ? `当前 Server：${loc.localPath}` : '当前 Server'
  return loc.localPath ? loc.localPath : '默认 (~/.symbio)'
}
