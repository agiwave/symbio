/**
 * 系统目录 (System Location) —— 前端持久化的「连接目标」事实源。
 *
 * 三端交付物（桌面启动器 / CLI serve / web）的 Vue 实现完全跑在普通浏览器里：
 * 桌面端由启动器拉起本机 `symbio-cli serve --browser`，浏览器同源 HTTP 访问本机
 * 后端；web 端可选择连接另一个 Server；两者都不存在 Tauri IPC 这条分支。
 *
 * `local` / `remote` 只是连接目标的 UI 兼容写法：
 * - local：同源 Server（桌面端就是本机 CLI serve，homedir 位于该 Server）；
 * - remote：另一台 Server 的 HTTP/WS 地址，Bearer token 只放 Authorization 请求头。
 *
 * 远端不可达只记录 `locationError`，绝不回退任何不存在的原生通道。
 * `forceNative` 只是控制面兼容提示，不改变已选择的 Server 地址。
 * homedir 是 Symbio 配置根，workdir 是会话工作区，两者不互换。
 */

import { ref, type Ref } from 'vue'

/** 前端运行环境：所有交付物都在普通浏览器里，不再有 WebView/IPC 形态。 */
export type RuntimeEnvironment = 'browser'

/** 传输连接目标：同源 Server 与远端 Server 统一为 server。 */
export type ConnectionTarget = 'server'

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
 * 是否运行在浏览器中。所有交付物都是普通浏览器环境，故此值恒为 true。
 *
 * 保留这个函数是为了让调用方（HomedirEntry / HomedirSwitcher / 测试）不必知道
 * 「原生形态已被移除」这件事；它不再是能力探测，只是一条环境事实。
 */
export function isBrowserEnvironment(): boolean {
  return true
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

/**
 * 展示用短标签：只给路径本身，不再叠加「当前 Server」之类的实现前缀
 * （入口按钮已有明确文案，再加前缀就是同一信息显示两遍）。
 */
export function formatLocation(loc: SystemLocation): string {
  if (loc.kind === 'remote') return loc.remoteUrl || '远端服务'
  return loc.localPath || '默认位置'
}
