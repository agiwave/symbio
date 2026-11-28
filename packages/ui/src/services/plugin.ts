// Corresponding Backend: crates/symbio-platform/src/kernel/plugin/transport.rs
/**
 * Symbio 插件通讯内核：统一消息模型 + 出站传输选择（native IPC / HTTP / gateway）。
 * 
 * 核心目标：
 * 1. 统一消息模型 (PluginMessage)，对称处理请求与响应。
 * 2. 统一术语，使用 metadata 替代 head，使用 payload 替代 body。
 * 3. 增强类型安全性与协议严格性。
 */

import type { UnlistenFn } from '@tauri-apps/api/event'
import { logger } from '@/utils/logger'
import {
  isBrowserEnvironment,
  loadSystemLocation,
  parseRemoteAddress,
  setCurrentLocation,
  setLocationError,
  type ConnectionTarget,
  type FrontendConnectionModel,
  type RuntimeEnvironment,
} from './systemLocation'

/**
 * 网关插件名（它的路由都是**控制面**接口：读写当前目标的网关设置）。
 *
 * embedded 中控制面固定走本机 native；browser 中固定走当前 Server 的控制面 HTTP。
 * 两种环境都不允许 browser 回退 IPC。判定在 `sendRouteRequest`。
 */

/** 未知值是否为可索引的对象——IPC 边界所有结构判定的唯一守卫。 */
function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v)
}

// ==================== 1. 协议定义 (与后端 transport.rs 严格对齐) ====================

/**
 * 线上的帧载荷是 JSON 反序列化结果，静态类型只能是 `unknown`；
 * 业务形状由 `ProtocolEnforcer.extract` 在边界处用守卫收窄，不让 `any` 越过 IPC 边界。
 */
export type PluginFrame =
  | { Data: unknown }
  | { Error: [string, unknown] }

export const HEAD_PATH = "path";
export const HEAD_WORKDIR = "workdir";
export const HEAD_AGENT_ID = "agent_id";
export const HEAD_SESSION_ID = "session_id";
export const HEAD_TRACE_ID = "trace_id";

/**
 * 调用**来源**（诊断键，不是协议键）。
 *
 * ## 它解决什么
 *
 * `trace_id` 只回答「哪几次请求属于同一条链」，不回答「谁发的、为什么发」。
 * 于是「一轮会话里出现多次 `vdfs/stat`，正常吗」这类问题**只能从时间戳反推**：
 * 同一个路由名有好几个调用方（实时面缺基线 / 资源信号分辨删除 / 清单刷新 /
 * 引导解析……），留痕里却一字不差。反推既慢又容易推错。
 *
 * `origin` 把来源与理由变成**随请求走的事实**：`Start routing` 那一行直接给出
 * 「谁、因为什么」。这是它唯一的用途——后端不读它做任何决策，缺省即 `-`。
 *
 * ## 取值是闭集，且由**类型**保证
 *
 * 回读类动词（`listVdfs` / `statVdfs` / `readVdfs`）的来源取自
 * `services/vdfs.ts` 的 `READBACK_REASON`，并作为**必填形参**——漏填是编译错误，
 * 不是「记得写」。其余路由把来源放进 [`PluginOptions.origin`]。
 */
export const HEAD_ORIGIN = "origin";

/**
 * 统一消息结构（与后端 `PluginMessageWire` 对齐）
 */
export interface PluginMessage {
  metadata: Record<string, string>;
  payload?: unknown;
}

/**
 * 载荷包装结构 (与后端 PluginPayloadWire 对齐)
 */
export type PluginPayloadWire =
  | { type: 'Data', data: unknown }
  | { type: 'Connection', data: string }

export interface ConnectEvent { type: string; data?: unknown }

// 最近一次使用的工作目录（lastWorkdir）。
// 仅用于"新建会话"时的默认工作区；不承载"全局当前目录"语义——
// 会话自身的工作目录以会话 metadata.workdir 为准（后端 orchestrator 已据此兜底）。
let lastWorkdir: string | undefined;

export function setLastWorkdir(path: string) {
  lastWorkdir = path;
}

export function getLastWorkdir(): string | undefined {
  return lastWorkdir;
}

/**
 * 插件连接对象
 * 封装了会话 ID、监听器生命周期及通讯方法
 */
export class Connection {
  protected active = true;
  private lastActivity = Date.now();

  constructor(
    public readonly connectionId: string,
    public readonly path: string,
    private unlistenFn: UnlistenFn,
    private eofUnlistenFn: UnlistenFn
  ) { }

  get isConnected() { 
    const isHealthy = this.active && (Date.now() - this.lastActivity < 60000);
    return isHealthy;
  }

  updateActivity() {
    this.lastActivity = Date.now();
  }

  async send(data: unknown): Promise<void> {
    if (!this.active) {
      throw new Error(`[Protocol Error] Cannot send to a disconnected session: ${this.connectionId} (${this.path})`);
    }
    if (!this.isConnected) {
      throw new Error(`[Protocol Error] Connection is stale or inactive: ${this.connectionId} (${this.path})`);
    }
    this.lastActivity = Date.now();
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('route_v2_send', {
      connectionId: this.connectionId,
      frame: { Data: data }
    });
  }

  async close(): Promise<void> {
    if (!this.active) return;
    this.active = false;
    this.unlistenFn();
    this.eofUnlistenFn();
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('route_v2_close', { connectionId: this.connectionId });
    } catch (err) {
      logger.warn('Connection', `Close error for ${this.path}:`, err);
    }
  }

  markDisconnected() {
    if (!this.active) return;
    this.active = false;
    this.unlistenFn();
    this.eofUnlistenFn();
  }
}

// ==================== 2. 协议执行器 (约束机制) ====================

class ProtocolEnforcer {
  /**
   * 严格验证帧结构
   */
  static validate(frame: unknown, path: string): PluginFrame {
    if (!isRecord(frame)) {
      const msg = `[Protocol Violation] ${path} sent non-object input: ${JSON.stringify(frame)}`;
      logger.error('Protocol', msg);
      throw new Error(msg);
    }
    const keys = Object.keys(frame);
    if (keys.length !== 1 || !['Data', 'Error'].includes(keys[0])) {
      const msg = `[Protocol Violation] ${path} sent invalid frame structure (expected Data|Error)`;
      logger.error('Protocol', msg, frame);
      throw new Error(msg);
    }
    return frame as PluginFrame;
  }

  /**
   * 业务载荷提取
   */
  static extract(frame: PluginFrame, path: string): { type: string; data: unknown } {
    if ('Data' in frame) {
      const d = frame.Data;
      // 场景 1: 标准业务格式 { type: "...", data: ... } —— 用守卫收窄未知载荷
      if (isRecord(d) && typeof d.type === 'string' && 'data' in d) {
        return { type: d.type, data: d.data };
      }
      // 场景 2: 裸数据帧 (视作 type=message)
      return { type: 'message', data: d };
    }
    if ('Error' in frame) return { type: 'error', data: frame.Error[0] };

    throw new Error(`[Protocol Error] Unhandleable frame type in ${path}`);
  }
}

// ==================== 2.5 出站传输选择（由「系统目录」切换器 / localStorage 驱动） ====================

export type TransportMode = 'native' | 'http'

/** 出站配置的唯一真相；宿主 HTTP API 也从此处取 endpoint/token。 */
export interface OutboundConfig {
  protocol: TransportMode
  endpoint: string
  token: string
}

const browser = (): RuntimeEnvironment => isBrowserEnvironment() ? 'browser' : 'embedded'

function sameOriginEndpoint(): string {
  return typeof location === 'undefined' ? '' : location.origin
}

function defaultOutbound(): OutboundConfig {
  return isBrowserEnvironment()
    ? { protocol: 'http', endpoint: sameOriginEndpoint(), token: '' }
    : { protocol: 'native', endpoint: '', token: '' }
}

/**
 * 出站配置：embedded 默认 native，browser 默认同源 Server HTTP。
 * remote 选择另一台 Server；browser 永不切换到 native。
 */
let outbound: OutboundConfig = defaultOutbound()
let outboundReady: Promise<void> | null = null

/** 当前运行环境与连接目标；供 UI 区分“本机”与“当前 Server”。 */
export function getConnectionModel(): FrontendConnectionModel {
  const target: ConnectionTarget = outbound.protocol === 'native' ? 'native' : 'server'
  return { environment: browser(), target }
}

/**
 * 应用启动时调用一次：读取持久化选择并决定传输。
 *
 * browser + local → 同源 HTTP；embedded + local → native；remote → HTTP。
 * 远端不可达时保留所选 Server HTTP 并显示错误；任何运行环境都不回退 native。
 * 返回的 Promise 被缓存，供首次出站请求等待，消除初始化竞态。
 */
export function initGatewayTransport(): Promise<void> {
  if (!outboundReady) outboundReady = doInitGatewayTransport()
  return outboundReady
}

/** 探测远端网关是否可达（健康检查端点不校验令牌）。 */
async function pingRemote(url: string): Promise<boolean> {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), 3000)
  try {
    const res = await fetch(`${url}/api/v1/health`, { method: 'GET', signal: controller.signal })
    return res.ok
  } catch {
    return false
  } finally {
    clearTimeout(timer)
  }
}

async function doInitGatewayTransport(): Promise<void> {
  const loc = loadSystemLocation()

  if (loc.kind === 'remote' && loc.remoteUrl) {
    const parsedRemote = parseRemoteAddress(loc.remoteUrl)
    const endpoint = parsedRemote.url
    const token = loc.remoteKey || parsedRemote.key
    const ok = await pingRemote(endpoint)
    outbound = { protocol: 'http', endpoint, token }
    setCurrentLocation(loc)
    setLocationError(ok ? null : `远端地址不可达：${endpoint}（保留 Server HTTP 连接）`)
    if (ok) logger.info('Transport', `系统目录=远端，出站协议已切换为 HTTP: ${endpoint}`)
    else logger.warn('Transport', '远端地址不可达，保留 Server HTTP 连接:', endpoint)
    return
  }

  outbound = defaultOutbound()
  if (outbound.protocol === 'http' && !outbound.endpoint) {
    setLocationError('无法确定浏览器同源 Server 地址')
  } else {
    setLocationError(null)
  }
  setCurrentLocation(loc)
}

export function getOutboundConfig(): OutboundConfig {
  return outbound
}

/** 宿主 HTTP API 的统一 endpoint 出口；不得再读 localStorage 推导地址。 */
export function getOutboundEndpoint(): string {
  return outbound.endpoint
}

/** 宿主 HTTP API 的统一 Bearer token 出口。 */
export function getOutboundToken(): string {
  return outbound.token
}

/** 重新读取「系统目录」选择（切换后调用），立即重算传输。 */
export function reloadGatewayTransport(): Promise<void> {
  outboundReady = doInitGatewayTransport()
  return outboundReady
}

/**
 * 基于 WebSocket 的连接（出站协议为 http 时，`connectPlugin` 使用）。
 * 复用 Connection 的对外接口，底层走 WebSocket 而非 Tauri IPC 事件。
 */
class WsConnection extends Connection {
  constructor(public ws: WebSocket, path: string) {
    super('ws', path, () => {}, () => {})
  }
  get isConnected(): boolean {
    return this.ws.readyState === WebSocket.OPEN
  }
  async send(data: unknown): Promise<void> {
    if (!this.active) {
      throw new Error(`[Protocol Error] Cannot send to a disconnected session: ws (${this.path})`)
    }
    this.ws.send(JSON.stringify({ Data: data }))
  }
  async close(): Promise<void> {
    if (!this.active) return
    this.active = false
    try {
      this.ws.close()
    } catch {
      /* ignore */
    }
  }
  markDisconnected(): void {
    if (!this.active) return
    this.active = false
  }
}

// ==================== 3. 统一通讯链路 ====================

export interface PluginOptions {
  workdir?: string;
  agent_id?: string;
  session_id?: string;
  metadata?: Record<string, unknown>;
  /**
   * 调用来源（诊断键，见 [`HEAD_ORIGIN`]）。进 `metadata.origin`，由
   * `Start routing` 留痕打出。缺省不写这个键——**不给来源时信封形状与从前逐字节一致**。
   */
  origin?: string;
  /**
   * 控制面提示：native 目标走 Tauri IPC；已选 Server HTTP 时仍走该 Server。
   * browser 中此选项绝不会触发原生调用，也不会改变已选择的 Server 地址。
   */
  forceNative?: boolean;
}

interface SendRouteOptions extends PluginOptions {
  path: string;
  payload?: unknown;
}

/**
 * 构造路由请求的 metadata（native 与 http 两套传输共用，保证对称）
 */
function buildMetadata(request: SendRouteOptions): Record<string, string> {
  const sessionId = `v2_sess_${Math.random().toString(36).substring(2, 10)}`
  const traceId = `trace_${Date.now()}_${Math.random().toString(36).substring(2, 6)}`

  const metadata: Record<string, string> = {
    [HEAD_PATH]: request.path,
    [HEAD_SESSION_ID]: request.session_id ?? sessionId,
    [HEAD_TRACE_ID]: traceId,
  };

  const workdir = request.workdir ?? lastWorkdir;
  if (workdir) {
    metadata[HEAD_WORKDIR] = workdir;
  }

  if (request.agent_id) {
    metadata[HEAD_AGENT_ID] = request.agent_id;
  }

  // 来源键**只在给了来源时**才写：没给就与从前的信封逐字节一致，
  // 免得每条路由都多带一个恒为空的字段（那会让「有没有来源」不可判）。
  if (request.origin) {
    metadata[HEAD_ORIGIN] = request.origin;
  }

  if (request.metadata) {
    for (const [k, v] of Object.entries(request.metadata)) {
      metadata[k] = typeof v === 'string' ? v : JSON.stringify(v);
    }
  }

  return metadata;
}

/**
 * 内部核心：发起路由请求并处理握手。
 *
 * mode 决定「出站语义」：
 * - 'call'（默认）：一次性/同步语义。native 走 `route_v2`；http 走 `POST /api/v1/invoke`
 *   （服务端把会话折叠到 EOF 的最后一帧，与 callPlugin 的一次性语义一致）。
 * - 'connect'：持久会话语义。native 走 `route_v2` + 事件通道；http 走 `WS /api/v1/ws`
 *   （首帧发送 PluginMessageWire，之后双向转发 PluginFrame）。
 *
 * 出站协议由 `initGatewayTransport()` 依据「系统目录」切换器的选择决定；每次调用前
 * await 该 Promise（已缓存），天然消除首调竞态。
 */
async function sendRouteRequest(
  request: SendRouteOptions,
  onFrame?: (frame: PluginFrame) => void,
  onEof?: () => void,
  mode: 'call' | 'connect' = 'call'
): Promise<{ response: PluginMessage; connection?: Connection }> {
  await initGatewayTransport();
  const ob = getOutboundConfig();
  const serverSelected = ob.protocol === 'http' && !!ob.endpoint;

  // 浏览器只有 Server HTTP 一个出口；embedded 一旦显式选择 Server，控制面提示
  // （forceNative / gateway 路径）也不得把请求偷偷送回本机 IPC。
  if (!serverSelected && isBrowserEnvironment()) {
    throw new Error('[Transport] 浏览器需要可用的 Server HTTP 地址，不能回退 Tauri IPC')
  }
  return serverSelected
    ? httpTransport(request, mode, ob, onFrame, onEof)
    : nativeTransport(request, onFrame, onEof);
}

// ==================== 原生传输（Tauri IPC，现状） ====================

async function nativeTransport(
  request: SendRouteOptions,
  onFrame?: (frame: PluginFrame) => void,
  onEof?: () => void
): Promise<{ response: PluginMessage; connection?: Connection }> {
  const metadata = buildMetadata(request);
  const sessionId = metadata[HEAD_SESSION_ID];
  const wireRequest: PluginMessage = {
    metadata,
    payload: request.payload,
  };

  let currentUnlisten: UnlistenFn | undefined;
  let currentEofUnlisten: UnlistenFn | undefined;

  // 只有 embedded 分支能到达这里；动态导入避免浏览器静态初始化原生能力。
  const [{ invoke }, { listen }] = await Promise.all([
    import('@tauri-apps/api/core'),
    import('@tauri-apps/api/event'),
  ]);

  // 1. 预监听 (消除竞态)
  currentUnlisten = await listen<unknown>(`route/${sessionId}`, (event) => {
    try {
      const frame = ProtocolEnforcer.validate(event.payload, request.path);
      onFrame?.(frame);
    } catch (err) {
      logger.error('Protocol', `Critical Violation in ${request.path}`, err);
    }
  });

  currentEofUnlisten = await listen<unknown>(`route/${sessionId}/eof`, () => {
    onEof?.();
  });

  try {
    const response = await invoke<PluginMessage>('route_v2', {
      request: wireRequest,
      clientId: sessionId,
    });

    const payload = response.payload as PluginPayloadWire;

    if (payload && payload.type === 'Connection') {
      // ## 契约：`conn_id` 恒等于 `clientId`
      //
      // 后端 `route_v2` 在收到 `clientId` 时**恒**用它作为 conn_id
      // （`apps/desktop/src-tauri/src/commands.rs` 的 `register_fixed`），而本函数恒传
      // `clientId: sessionId`。因此事件名与**预注册**的监听器必然一致——这正是
      // 「先 listen 再 invoke」能消除握手竞态的原因。
      //
      // 历史上这里有一段「后端可能改 id ⇒ 重新 listen 并迁移监听器」的补救分支，
      // 它**不可达**。留着它的危害不是浪费几行代码，而是掩盖真实缺口：若后端将来
      // 真改了 id，预注册的监听器会挂在旧事件名上，**切换窗口内的帧静默丢失且不报错**。
      // 故删除该分支，把契约写成断言——破坏契约时当场报错，而不是安静地收空气。
      if (payload.data !== sessionId) {
        // 交给下方 catch 统一清理——避免同一对监听器被 unlisten 两次
        throw new Error(
          `[Protocol Error] route_v2 返回的 conn_id (${payload.data}) 与 clientId ` +
            `(${sessionId}) 不一致。前端已在 clientId 上预注册监听器，后端改 id 会导致` +
            `帧静默丢失。请检查后端 register_fixed 契约（${request.path}）。`,
        );
      }

      return { response, connection: new Connection(sessionId, request.path, currentUnlisten, currentEofUnlisten) };
    }

    // 如果是同步响应，立即清理监听器
    currentUnlisten?.();
    currentEofUnlisten?.();
    return { response };
  } catch (err) {
    currentUnlisten?.();
    currentEofUnlisten?.();
    throw err;
  }
}

// ==================== HTTP 传输（连接远端 Symbio 实例的网关入站） ====================

async function httpTransport(
  request: SendRouteOptions,
  mode: 'call' | 'connect',
  ob: OutboundConfig,
  onFrame?: (frame: PluginFrame) => void,
  onEof?: () => void
): Promise<{ response: PluginMessage; connection?: Connection }> {
  const wireRequest: PluginMessage = {
    metadata: buildMetadata(request),
    payload: request.payload,
  };
  return mode === 'connect'
    ? httpWsTransport(request, wireRequest, ob, onFrame, onEof)
    : httpInvokeTransport(wireRequest, ob, onFrame, onEof);
}

/**
 * 一次性调用：POST /api/v1/invoke（请求体 = PluginMessageWire，响应 = PluginPayloadWire）。
 * 会话型路径由服务端折叠到 EOF 的最后一帧，与 callPlugin 的一次性语义一致。
 */
async function httpInvokeTransport(
  wireRequest: PluginMessage,
  ob: OutboundConfig,
  _onFrame?: (frame: PluginFrame) => void,
  _onEof?: () => void
): Promise<{ response: PluginMessage; connection?: Connection }> {
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (ob.token) headers['Authorization'] = `Bearer ${ob.token}`;

  const res = await fetch(`${ob.endpoint}/api/v1/invoke`, {
    method: 'POST',
    headers,
    body: JSON.stringify(wireRequest),
  });

  const text = await res.text().catch(() => '');
  let parsed: unknown = null;
  try { parsed = text ? JSON.parse(text) : null; } catch { /* 非 JSON 忽略 */ }

  // JSON 是未知值，用守卫读 error 字段，不让 any 从 fetch 响应渗入
  const parsedRecord = isRecord(parsed) ? parsed : null;
  const gatewayError = parsedRecord?.error;
  if (!res.ok || gatewayError) {
    const errText = gatewayError != null ? String(gatewayError) : `invoke 失败 ${res.status}`;
    throw new Error(`[Gateway HTTP] ${errText}`);
  }

  // 服务端返回的 PluginPayloadWire（Data / Connection）
  const wire = parsed as PluginPayloadWire;
  const response: PluginMessage = { metadata: {}, payload: wire };
  return { response };
}

/**
 * 持久会话：WS /api/v1/ws。
 * 连接建立后客户端首帧发送 PluginMessageWire，此后该连接双向转发 PluginFrame，
 * 与 connectPlugin 的会话语义一致。
 */
async function httpWsTransport(
  request: SendRouteOptions,
  wireRequest: PluginMessage,
  ob: OutboundConfig,
  onFrame?: (frame: PluginFrame) => void,
  onEof?: () => void
): Promise<{ response: PluginMessage; connection?: Connection }> {
  const wsUrl = new URL(`${ob.endpoint.replace(/\/+$/, '')}/api/v1/ws`)
  wsUrl.protocol = wsUrl.protocol === 'https:' ? 'wss:' : 'ws:'
  // token 只走 Sec-WebSocket-Protocol；URL 不得残留 query / hash。
  wsUrl.search = ''
  wsUrl.hash = ''
  const protocols = ['symbio.v1', ...(ob.token ? [`bearer.${ob.token}`] : [])]
  const ws = new WebSocket(wsUrl.toString(), protocols);

  await new Promise<void>((resolve, reject) => {
    ws.onopen = () => resolve();
    ws.onerror = () => reject(new Error(`[Gateway WS] 无法连接到 ${ob.endpoint} (${request.path})`));
  });

  // 首帧：PluginMessageWire（与 route_v2 请求体完全一致）
  ws.send(JSON.stringify(wireRequest));

  const conn = new WsConnection(ws, request.path);

  ws.onmessage = (ev) => {
    try {
      const frame = ProtocolEnforcer.validate(JSON.parse(ev.data as string), request.path);
      onFrame?.(frame);
    } catch (err) {
      logger.error('Protocol', `WS 帧违规 in ${request.path}`, err);
    }
  };
  ws.onclose = () => {
    conn.markDisconnected();
    onEof?.();
  };
  ws.onerror = () => {
    conn.markDisconnected();
  };

  // 与 native 对称：返回 Connection 型 payload，connectPlugin 据此拿到连接对象
  return {
    response: { metadata: {}, payload: { type: 'Connection', data: 'ws' } as PluginPayloadWire },
    connection: conn,
  };
}

// ==================== 4. 统一业务 API (对外接口) ====================

/**
 * 同步调用：支持超时机制与强类型验证
 *
 * 泛型默认值使用 `unknown` 而不是 `any`，强制调用方显式提供类型，
 * 避免类型安全从源头失守。
 */
export async function callPlugin<TOutput = unknown, TInput = unknown>(
  path: string,
  input?: TInput,
  timeoutMs = 30000,
  options?: PluginOptions
): Promise<TOutput> {
  let lastData: unknown = null;
  let hasError = false;
  let connection: Connection | undefined;
  let resolveFn!: (val: TOutput) => void;
  let rejectFn!: (reason?: unknown) => void;

  const resultPromise = new Promise<TOutput>((resolve, reject) => {
    resolveFn = resolve;
    rejectFn = reject;
  });

  (async () => {
    try {
      const result = await sendRouteRequest({
        path,
        payload: input !== undefined ? input : undefined,
        ...options
      }, (frame) => {
        if (frame !== undefined && 'Data' in frame) lastData = ProtocolEnforcer.extract(frame, path).data;
        if (frame !== undefined && 'Error' in frame) {
          hasError = true;
          connection?.close();
          rejectFn(new Error(frame.Error[0]));
        }
      }, () => {
        // EOF received
        connection?.markDisconnected();
        if (hasError) rejectFn(new Error(`[Plugin Error] ${path} failed during execution`));
        else resolveFn(lastData as TOutput);
      });

      connection = result.connection;
      const response = result.response;
      const payload = response.payload as PluginPayloadWire;

      // A. 处理立即响应
      if (payload && payload.type === 'Data') {
        resolveFn(payload.data as TOutput);
        return;
      }

      // B. 处理会话式同步响应 (直到收到 EOF 信号)
      if (!connection) {
        rejectFn(new Error(`[Protocol Error] ${path} returned unsupported response payload`));
      }
    } catch (e) {
      rejectFn(e);
    }
  })();

  if (timeoutMs <= 0) return resultPromise;

  // 超时定时器必须在请求 settle 后清掉：否则每次调用都挂一个 30s 定时器，
  // 即使早已 resolve，到点仍会 connection?.close() 把一条活跃连接静默关掉。
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeoutPromise = new Promise<TOutput>((_, reject) => {
    timer = setTimeout(() => {
      connection?.close();
      reject(new Error(`[Timeout] ${path} call timed out after ${timeoutMs}ms`));
    }, timeoutMs);
  });

  return Promise.race([resultPromise, timeoutPromise]).finally(() => {
    if (timer !== undefined) clearTimeout(timer);
  });
}

/**
 * 持久连接 (带类型支持)
 *
 * 泛型默认值用 `unknown`，理由同 callPlugin。
 */
export async function connectPlugin<TInput = unknown>(
  path: string,
  input?: TInput,
  onEvent?: (event: ConnectEvent) => void,
  options?: PluginOptions
): Promise<Connection> {
  let conn: Connection | undefined;

  const result = await sendRouteRequest({
    path,
    payload: input !== undefined ? input : null,
    ...options
  }, (frame) => {
    if (conn) conn.updateActivity();
    const { type, data } = ProtocolEnforcer.extract(frame, path);

    if (frame !== undefined && 'Error' in frame) {
      conn?.markDisconnected();
      onEvent?.({ type: 'error', data: frame.Error[0] });
      onEvent?.({ type: 'disconnected', data: { reason: 'error' } });
    } else {
      onEvent?.({ type, data });
    }
  }, () => {
    conn?.markDisconnected();
    onEvent?.({ type: 'disconnected', data: { reason: 'done' } });
  }, 'connect');

  if (!result.connection) throw new Error(`[Protocol Error] ${path} session failed to establish`);
  conn = result.connection;
  return conn;
}
