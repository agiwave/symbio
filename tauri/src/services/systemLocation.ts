/**
 * 系统目录 (System Location) —— 前端唯一的「连接目标」事实源
 *
 * ## 为什么需要它
 *
 * 历史上「系统目录」被拆成两个互不相通的概念：
 * - 后端 `HomedirRegistry`：仅本地路径（`~/.symbio` 之类）
 * - 网关插件 `outbound_*`：仅远端 HTTP 地址（且会在「开放接口」设置表单暴露）
 *
 * 两者在 UI 上无法统一，且一旦把 outbound 写成无效远端地址并在设置页提交，前端
 * `initGatewayTransport` 会把它当成全局连接目标，导致主页面所有数据请求都打到死端点、
 * 且切换按钮自身也被路由到死端点 —— 永久卡死，无恢复路径。
 *
 * 本模块把二者收敛为单一概念 **SystemLocation**，并以**前端 localStorage 为权威**：
 * - `local`：连接本机后端（native 传输），并可切换本地 homedir 路径
 * - `remote`：连接远端 Symbio 网关（HTTP/WS），地址字符串可内嵌 key（`?token=`）
 *
 * 连接目标完全由左下角「系统目录」切换器管理；网关插件不再持有出站配置
 * （`outbound_*` 字段已移除），「开放接口」设置页仅保留入站（对外提供服务）配置。
 *
 * 好处：
 * 1. 启动自动恢复上次选择（纯前端逻辑）
 * 2. 控制面操作（切换目录、读写本机网关配置）永远走 `forceNative`，命中本机后端，
 *    不会被当前出站协议指到远端 —— 因此**永不卡死**，无效远端可随时切回
 * 3. 远端不可达时 `initGatewayTransport` 安全回退 native，并通过 `locationError` 提示
 *
 * 对应后端：
 * - 本地模式：`home/reload`（经 `forceNative`）热重载本机 homedir
 * - 远端模式：仅前端侧记录，经网关入站服务连接远端实例
 */

import { ref, type Ref } from 'vue'

/** 系统目录类型 */
export type SystemLocationKind = 'local' | 'remote'

/** 系统目录描述 */
export interface SystemLocation {
  /** 连接目标类型 */
  kind: SystemLocationKind
  /** 本地模式：homedir 路径；空串表示默认 `~/.symbio` */
  localPath?: string
  /** 远端模式：网关基础地址（不含 token），如 `http://host:port` */
  remoteUrl?: string
  /** 远端模式：访问密钥（Bearer Token） */
  remoteKey?: string
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

/** 远端地址格式是否合法（http/https 且含 host） */
export function isValidRemoteUrl(url: string): boolean {
  if (!/^https?:\/\//i.test(url)) return false
  const host = url.replace(/^https?:\/\//i, '').split('/')[0].split('?')[0]
  if (!host) return false
  const isIp = host
    .split(':')
    .every((seg) => seg === '' || /^\d+$/.test(seg) || seg.startsWith('['))
  return host === 'localhost' || host.includes('.') || isIp
}

/** 展示用文本 */
export function formatLocation(loc: SystemLocation): string {
  if (loc.kind === 'remote') return loc.remoteUrl || '(远端)'
  return loc.localPath ? loc.localPath : '默认 (~/.symbio)'
}
