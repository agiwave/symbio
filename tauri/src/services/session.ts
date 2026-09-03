/**
 * Session 服务
 *
 * 管理聊天会话历史
 */

import { callPlugin } from './plugin'
import { listResources } from './resources'
import { ChatMessage as SessionMessage } from '../schemas/chat_message'
import * as SessionGetMessages from '../schemas/session_get_messages'
import * as SessionList from '../schemas/session_list'
import * as SessionAppend from '../schemas/session_append'
import * as SessionClear from '../schemas/session_clear'
import * as SessionUpdate from '../schemas/session_update'
import * as SessionClearMessages from '../schemas/session_clear_messages'
import * as SessionDeleteMessage from '../schemas/session_delete_message'
import * as SessionUpdateMessage from '../schemas/session_update_message'
import type { SessionMetadata } from '../schemas/session_meta'
import { SESSION_PATH } from '../constants/pluginPaths'

export type { SessionMessage }

export type { SessionListItem } from '../schemas/session_list'
export type { SessionMetadata, SessionHeartbeatConfig } from '../schemas/session_meta'

/**
 * 获取会话列表（统一资源协议：`worker/session/resources/list`）
 *
 * 统一 `ResourceSummary` 项经映射还原为 `SessionListItem` 形状，
 * 以便既有 `sessions` store / 对话组件保持兼容。
 */
export async function listSessions(): Promise<SessionList.SessionListItem[]> {
  const resp = await listResources('session')
  // 后端 ResourceSummary.extra 为 #[serde(flatten)]，类型特有字段(message_count/
  // is_working/metadata)会被平铺到 JSON 顶层，而非套在 it.extra 下；这里直接从顶层读。
  return (resp.items || []).map((it) => {
    const v = it as Record<string, any>
    return {
      id: v.id,
      message_count: Number(v.message_count ?? 0),
      updated_at: v.updated_at ?? 0,
      is_working: v.is_working ?? v.status === 'working',
      metadata: v.metadata ?? {},
    }
  })
}

/**
 * 分页获取会话消息
 */
export async function getSessionMessages(
  sessionId: string,
  limit: number = 20,
  before?: number
): Promise<{ messages: SessionMessage[]; hasMore: boolean; total: number }> {
  // 注：当前后端 schema 仅返回 messages；保留 hasMore/total 用于向前兼容
  const result = await callPlugin<SessionGetMessages.Response>(
    `${SESSION_PATH}/get_messages`,
    {
      session_id: sessionId,
      limit,
      ...(before ? { before } : {})
    },
    undefined,
    { session_id: sessionId }
  )
  const msgs = result.messages || []
  return {
    messages: msgs,
    hasMore: false,
    total: msgs.length
  }
}

export async function appendMessages(
  sessionId: string,
  messages: SessionMessage[]
): Promise<SessionAppend.Response> {
  return await callPlugin<SessionAppend.Response, SessionAppend.Request>(
    `${SESSION_PATH}/append`,
    { session_id: sessionId, messages },
    undefined,
    { session_id: sessionId }
  )
}

export async function clearSession(sessionId: string): Promise<void> {
  await callPlugin<void, SessionClear.Request>(
    `${SESSION_PATH}/clear`,
    { session_id: sessionId },
    undefined,
    { session_id: sessionId }
  )
}

/**
 * 清空会话历史消息（保留会话本身 / 工作目录 / 标题等元数据）。
 * 路由：`worker/session/chat/clear_messages`
 */
export async function clearMessages(sessionId: string): Promise<SessionClearMessages.Response> {
  return await callPlugin<SessionClearMessages.Response, SessionClearMessages.Request>(
    `${SESSION_PATH}/chat/clear_messages`,
    { session_id: sessionId },
    undefined,
    { session_id: sessionId }
  )
}

/**
 * 删除单条会话消息（连同其之后的所有消息一并删除）。
 * 路由：`worker/session/chat/delete_message`
 */
export async function deleteMessage(
  sessionId: string,
  messageId: string
): Promise<SessionDeleteMessage.Response> {
  return await callPlugin<SessionDeleteMessage.Response, SessionDeleteMessage.Request>(
    `${SESSION_PATH}/chat/delete_message`,
    { session_id: sessionId, message_id: messageId },
    undefined,
    { session_id: sessionId }
  )
}

/**
 * 更新单条会话消息（手工编辑 / 标错重试等）。
 * 路由：`worker/session/chat/update_message`
 */
export async function updateMessage(
  sessionId: string,
  message: SessionMessage
): Promise<SessionUpdateMessage.Response> {
  return await callPlugin<SessionUpdateMessage.Response, SessionUpdateMessage.Request>(
    `${SESSION_PATH}/chat/update_message`,
    { session_id: sessionId, message },
    undefined,
    { session_id: sessionId }
  )
}

/**
 * 合并写入会话 metadata（workdir / title / agent_id 等）。
 * 后端会保留已有字段，浅合并新字段。
 */
export async function updateSession(
  sessionId: string,
  metadata: SessionMetadata,
  title?: string
): Promise<SessionUpdate.Response> {
  return await callPlugin<SessionUpdate.Response, SessionUpdate.Request>(
    `${SESSION_PATH}/update`,
    { session_id: sessionId, metadata, ...(title ? { title } : {}) },
    undefined,
    { session_id: sessionId }
  )
}

/**
 * 创建新会话（生成 ID 并初始化）
 */
export function createSessionId(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2)
}

/**
 * 手动触发一次会话心跳任务。
 * 路由：`worker/session/heartbeat/trigger`
 * - 会话未启用心跳 / 提示词为空 → 抛错
 * - 会话正在工作中 → 返回 `{ status: "skipped" }`
 * - 正常触发 → 返回 `{ status: "triggered", session_id, include_history }`
 */
export async function triggerHeartbeat(sessionId: string): Promise<{
  status: 'triggered' | 'skipped'
  session_id: string
  include_history?: boolean
  reason?: string
}> {
  return await callPlugin(
    `${SESSION_PATH}/heartbeat/trigger`,
    {},
    undefined,
    { session_id: sessionId }
  )
}

