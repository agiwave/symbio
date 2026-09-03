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
  private active = true;
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

// ==================== 3. 统一通讯链路 ====================

export interface PluginOptions {
  workdir?: string;
  agent_id?: string;
  session_id?: string;
  metadata?: any;
}

interface SendRouteOptions extends PluginOptions {
  path: string;
  payload?: any;
}

/**
 * 内部核心：发起路由请求并处理握手
 */
async function sendRouteRequest(
  request: SendRouteOptions,
  onFrame?: (frame: PluginFrame) => void,
  onEof?: () => void
): Promise<{ response: PluginMessage; connection?: Connection }> {

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

  const wireRequest: PluginMessage = {
    metadata,
    payload: request.payload
  };

  let currentUnlisten: UnlistenFn | undefined
  let currentEofUnlisten: UnlistenFn | undefined

  // 1. 预监听 (消除竞态)
  currentUnlisten = await listen<any>(`route/${sessionId}`, (event) => {
    try {
      const frame = ProtocolEnforcer.validate(event.payload, request.path);
      onFrame?.(frame);
    } catch (err) {
      logger.error('Protocol', `Critical Violation in ${request.path}`, err);
    }
  })

  currentEofUnlisten = await listen<any>(`route/${sessionId}/eof`, () => {
    onEof?.()
  })

  try {
    const response = await invoke<PluginMessage>('route_v2', {
      request: wireRequest,
      clientId: sessionId
    })

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
        })
        currentEofUnlisten = await listen<any>(`route/${actualId}/eof`, () => {
          onEof?.()
        })
        oldUnlisten?.(); // 启动新监听后再销毁旧监听，确保数据帧不丢失
        oldEofUnlisten?.();
        return { response, connection: new Connection(actualId, request.path, currentUnlisten, currentEofUnlisten) }
      }

      return { response, connection: new Connection(sessionId, request.path, currentUnlisten, currentEofUnlisten) }
    }

    // 如果是同步响应，立即清理监听器
    currentUnlisten?.()
    currentEofUnlisten?.()
    return { response }
  } catch (err) {
    currentUnlisten?.()
    currentEofUnlisten?.()
    throw err
  }
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
  });

  if (!result.connection) throw new Error(`[Protocol Error] ${path} session failed to establish`);
  conn = result.connection;
  return conn;
}
