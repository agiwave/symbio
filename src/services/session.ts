/**
 * Session 服务
 *
 * 管理聊天会话历史
 */

import { callPlugin } from './plugin'

const SESSION_PATH = 'agent/session'

export interface SessionMessage {
  role: 'user' | 'assistant' | 'system'
  content: string
  timestamp: number
  tool_calls?: Array<{
    id?: string
    type?: string
    function?: {
      name: string
      arguments: string
    }
  }>
  tool_call_id?: string
}

export interface Session {
  id: string
  messages: SessionMessage[]
  created_at: number
  updated_at: number
  metadata: Record<string, unknown>
}

export interface SessionListItem {
  id: string
  message_count: number
  updated_at: number
}

/**
 * 获取会话列表
 */
export async function listSessions(): Promise<SessionListItem[]> {
  const result = await callPlugin<{ success: boolean; sessions: SessionListItem[] }>(SESSION_PATH, {
    action: 'list'
  })
  return result.sessions || []
}

/**
 * 获取会话详情
 */
export async function getSession(sessionId: string): Promise<Session> {
  const result = await callPlugin<{ success: boolean; session: Session }>(SESSION_PATH, {
    action: 'get',
    session_id: sessionId
  })
  console.log('[session] getSession result:', result)
  return result.session
}

/**
 * 分页获取会话消息
 * @param sessionId 会话 ID
 * @param limit 每次获取的消息数量（默认 20）
 * @param before 获取此时间戳之前的消息（用于加载更多）
 */
export async function getSessionMessages(
  sessionId: string,
  limit: number = 20,
  before?: number
): Promise<{ messages: SessionMessage[]; hasMore: boolean; total: number }> {
  const result = await callPlugin<{
    success: boolean
    messages: SessionMessage[]
    has_more: boolean
    total: number
  }>(SESSION_PATH, {
    action: 'get_messages',
    session_id: sessionId,
    limit,
    ...(before ? { before } : {})
  })
  return {
    messages: result.messages || [],
    hasMore: result.has_more || false,
    total: result.total || 0
  }
}

/**
 * 追加消息到会话
 */
export async function appendMessages(sessionId: string, messages: SessionMessage[]): Promise<{ success: boolean; message_count: number }> {
  return callPlugin<{ success: boolean; message_count: number }>(SESSION_PATH, {
    action: 'append',
    session_id: sessionId,
    messages
  })
}

/**
 * 清除会话
 */
export async function clearSession(sessionId: string): Promise<{ success: boolean }> {
  return callPlugin<{ success: boolean }>(SESSION_PATH, {
    action: 'clear',
    session_id: sessionId
  })
}

/**
 * 创建新会话（生成 ID 并初始化）
 */
export function createSessionId(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2)
}
