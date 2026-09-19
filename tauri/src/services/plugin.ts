// Corresponding Backend: symbio/src/symbio_core/transport.rs
/**
 * Symbio 插件通讯内核 (V2.7 统一路由对称版)
 * 
 * 核心目标：
 * 1. 统一消息模型 (PluginMessage)，对称处理请求与响应。
 * 2. 统一术语，使用 metadata 替代 head，使用 payload 替代 body。
 * 3. 增强类型安全性与协议严格性。
 */

import { invoke } from '@tauri-apps/api/core'
import { listen, UnlistenFn } from '@tauri-apps/api/event'
import { logger } from '@/utils/logger'
import {
  loadSystemLocation,
  setCurrentLocation,
  setLocationError,
} from './systemLocation'
import { GATEWAY_PATH } from '@/constants/pluginPaths'

// ==================== 1. 协议定义 (与后端 transport.rs 严格对齐) ====================

export type PluginFrame =
  | { Data: any }
  | { Error: [string, any] }

export const HEAD_PATH = "path";
export const HEAD_WORKDIR = "workdir";
export const HEAD_AGENT_ID = "agent_id";
export const HEAD_SESSION_ID = "session_id";
export const HEAD_TRACE_ID = "trace_id";

/**
 * 统一消息结构 (V2.7)
 */
export interface PluginMessage {
  metadata: Record<string, string>;
  payload?: any;
}

/**
 * 载荷包装结构 (与后端 PluginPayloadWire 对齐)
 */
export type PluginPayloadWire =
  | { type: 'Data', data: any }
  | { type: 'Connection', data: string }

export interface ConnectEvent { type: string; data?: any }

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

  async send(data: any): Promise<void> {
    if (!this.active) {
      throw new Error(`[Protocol Error] Cannot send to a disconnected session: ${this.connectionId} (${this.path})`);
    }
    if (!this.isConnected) {
      throw new Error(`[Protocol Error] Connection is stale or inactive: ${this.connectionId} (${this.path})`);
    }
    this.lastActivity = Date.now();
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
  static validate(frame: any, path: string): PluginFrame {
    if (!frame || typeof frame !== 'object') {
      const msg = `[Protocol Violation] ${path} sent non-object input: ${JSON.stringify(frame)}`;
      logger.error('Protocol', msg);
      throw new Error(msg);
    }
    const keys = Object.keys(frame);
    if (keys.length !== 1 || !['Data', 'Extension', 'Error'].includes(keys[0])) {
      const msg = `[Protocol Violation] ${path} sent invalid frame structure (expected Data|Extension|Error)`;
      logger.error('Protocol', msg, frame);
      throw new Error(msg);
    }
    return frame as PluginFrame;
  }

  /**
   * 业务载荷提取
   */
  static extract(frame: PluginFrame, path: string): { type: string; data: any } {
    if ('Data' in frame) {
      const d = frame.Data;
      // 场景 1: 标准业务格式 { type: "...", data: ... }
      if (d && typeof d === 'object' && 'type' in d && 'data' in d) {
        return { type: d.type, data: d.data };
      }
      // 场景 2: 兼容桥接期存量格式 { success: true, data: ... }
      if (d && typeof d === 'object' && 'success' in d && 'data' in d) {
        logger.debug('Protocol', `${path} unwrapping legacy success/data wrapper`);
        return { type: 'legacy_response', data: d.data };
      }
      // 场景 3: 裸数据帧 (视作 type=message)
      return { type: 'message', data: d };
    }
    if ('Error' in frame) return { type: 'error', data: frame.Error[0] };

    throw new Error(`[Protocol Error] Unhandleable frame type in ${path}`);
  }
}

// ==================== 2.5 出站传输选择（由「系统目录」切换器 / localStorage 驱动） ====================

export type TransportMode = 'native' | 'http'

interface OutboundConfig {
  protocol: TransportMode
  endpoint: string
  token: string
}

/**
 * 出站配置：前端以何种协议连接后端。
 * - native：进程内直连本机后端（默认，本地系统目录）
 * - http：连接另一个 Symbio 实例的网关入站服务（远端系统目录）
 *
 * 其值由 `initGatewayTransport()` 依据前端持久化的「系统目录」(`systemLocation`) 决定；
 * 连接目标完全由左下角「系统目录」切换器统一管理，网关插件不再持有出站配置。
 * 远端不可达时自动回退 native，确保主页面始终可渲染。
 */
let outbound: OutboundConfig = { protocol: 'native', endpoint: '', token: '' }
let outboundReady: Promise<void> | null = null

/**
 * 应用启动时调用一次：读取「系统目录」持久化选择，决定前端连接方式。
 *
 * 前端 localStorage 为权威（避免被坏配置卡死）：
 * - 远端模式做健康探测：可达 → 出站切 HTTP；不可达 → 安全回退 native（绝不卡死主页面），
 *   并通过 `locationError` 提示用户，便于从系统目录按钮切回。
 * - 本地模式（含 localStorage 为空时的默认）直连本机后端。
 *
 * 返回的 Promise 被缓存，供 `sendRouteRequest` 在首次调用前等待就绪（消除竞态）。
 */
export function initGatewayTransport(): Promise<void> {
  if (!outboundReady) outboundReady = doInitGatewayTransport()
  return outboundReady
}

/**
 * 探测远端网关是否可达（健康检查端点不校验令牌）。
 */
async function pingRemote(url: string): Promise<boolean> {
  try {
    const controller = new AbortController()
    const timer = setTimeout(() => controller.abort(), 3000)
    const res = await fetch(`${url}/api/v1/health`, { method: 'GET', signal: controller.signal })
    clearTimeout(timer)
    return res.ok
  } catch {
    return false
  }
}

async function doInitGatewayTransport(): Promise<void> {
  const loc = loadSystemLocation()

  // 远端模式：健康探测，不可达则安全回退 native
  if (loc.kind === 'remote' && loc.remoteUrl) {
    const ok = await pingRemote(loc.remoteUrl)
    if (ok) {
      outbound = { protocol: 'http', endpoint: loc.remoteUrl, token: loc.remoteKey ?? '' }
      setCurrentLocation(loc)
      setLocationError(null)
      logger.info('Transport', `系统目录=远端，出站协议已切换为 HTTP: ${outbound.endpoint}`)
      return
    }
    // 不可达：回退 native，但保留远端配置，方便用户切回/重试
    outbound = { protocol: 'native', endpoint: '', token: '' }
    setCurrentLocation(loc)
    setLocationError(`远端地址不可达，已回退到本地连接：${loc.remoteUrl}`)
    logger.warn('Transport', '远端地址不可达，回退 native:', loc.remoteUrl)
    return
  }

  // 本地模式（含 localStorage 为空时的默认）
  outbound = { protocol: 'native', endpoint: '', token: '' }
  setCurrentLocation(loc)
  setLocationError(null)
}

export function getOutboundConfig(): OutboundConfig {
  return outbound
}

/**
 * 重新读取「系统目录」选择（切换后调用）。
 * 清除缓存并依据最新 localStorage 重新决定出站协议，使切换立即生效。
 */
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
  async send(data: any): Promise<void> {
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
  metadata?: any;
  /**
   * 强制走原生 Tauri IPC 传输（native），绕过出站 http。
   * 控制面操作（切换系统目录、读写本机网关配置等）必须命中本机后端，
   * 不能被当前 outbound 指到远端，否则会出现「切到远端后无法切回」的死锁。
   */
  forceNative?: boolean;
}

interface SendRouteOptions extends PluginOptions {
  path: string;
  payload?: any;
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

  // 网关自身的接口（gateway/*）始终是本机 native 调用：
  // 出站配置若设为 http，会指向远端实例，届时读取/修改「本机网关设置」反而会落到远端，
  // 既看不到本机配置也无法切换回 native。故强制 native，与 initGatewayTransport 启动期读取一致。
  const target = request.path.replace(/^\/+/, '');
  // 网关自身 = `gateway` 或 `gateway/…`（插件名见 constants/pluginPaths）
  const isGatewaySelf = target === GATEWAY_PATH || target.startsWith(`${GATEWAY_PATH}/`);

  // 控制面操作（forceNative）一律命中本机后端，避免被当前 outbound 指到远端而陷入死锁。
  const useNative = request.forceNative || isGatewaySelf || !(ob.protocol === 'http' && ob.endpoint);
  if (!useNative) {
    return httpTransport(request, mode, ob, onFrame, onEof);
  }
  return nativeTransport(request, onFrame, onEof);
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

  // 1. 预监听 (消除竞态)
  currentUnlisten = await listen<any>(`route/${sessionId}`, (event) => {
    try {
      const frame = ProtocolEnforcer.validate(event.payload, request.path);
      onFrame?.(frame);
    } catch (err) {
      logger.error('Protocol', `Critical Violation in ${request.path}`, err);
    }
  });

  currentEofUnlisten = await listen<any>(`route/${sessionId}/eof`, () => {
    onEof?.();
  });

  try {
    const response = await invoke<PluginMessage>('route_v2', {
      request: wireRequest,
      clientId: sessionId,
    });

    const payload = response.payload as PluginPayloadWire;

    if (payload && payload.type === 'Connection') {
      const actualId = payload.data;

      // 处理后端强制修改 Session ID 的情况 (无缝切换监听器)
      if (actualId !== sessionId) {
        const oldUnlisten = currentUnlisten;
        const oldEofUnlisten = currentEofUnlisten;
        currentUnlisten = await listen<any>(`route/${actualId}`, (event) => {
          try {
            const frame = ProtocolEnforcer.validate(event.payload, request.path);
            onFrame?.(frame);
          } catch (err) {
            logger.error('Protocol', `Critical Violation in ${request.path}`, err);
          }
        });
        currentEofUnlisten = await listen<any>(`route/${actualId}/eof`, () => {
          onEof?.();
        });
        oldUnlisten?.(); // 启动新监听后再销毁旧监听，确保数据帧不丢失
        oldEofUnlisten?.();
        return { response, connection: new Connection(actualId, request.path, currentUnlisten, currentEofUnlisten) };
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
  let parsed: any = null;
  try { parsed = text ? JSON.parse(text) : null; } catch { /* 非 JSON 忽略 */ }

  if (!res.ok || parsed?.error) {
    throw new Error(`[Gateway HTTP] ${parsed?.error ?? `invoke 失败 ${res.status}`}`);
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
  const wsUrl = `${ob.endpoint}/api/v1/ws` + (ob.token ? `?token=${encodeURIComponent(ob.token)}` : '');
  const ws = new WebSocket(wsUrl);

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
    response: { metadata: {}, payload: { type: 'Connection', data: 'ws' } as any },
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
  let lastData: any = null;
  let hasError = false;
  let connection: Connection | undefined;
  let resolveFn!: (val: TOutput) => void;
  let rejectFn!: (reason?: any) => void;

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

  return Promise.race([
    resultPromise,
    new Promise<TOutput>((_, reject) => setTimeout(() => {
      connection?.close();
      reject(new Error(`[Timeout] ${path} call timed out after ${timeoutMs}ms`));
    }, timeoutMs))
  ]);
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
