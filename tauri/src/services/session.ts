/**
 * Session 服务
 *
 * 管理聊天会话历史
 */

import { callPlugin } from './plugin'
import { listVdfs } from './vdfs'
import {
  VDFS_EXT_SESSION,
  VDFS_ROOT,
  VDFS_STATUS_WORKING,
  vdfsExtOf,
  vdfsJoin,
} from '@/schemas/vdfs'
import { ChatMessage as SessionMessage } from '../schemas/chat_message'
import * as SessionList from '../schemas/session_list'
import * as SessionClear from '../schemas/session_clear'
import * as SessionUpdate from '../schemas/session_update'
import * as SessionClearMessages from '../schemas/session_clear_messages'
import * as SessionDeleteMessage from '../schemas/session_delete_message'
import * as SessionUpdateMessage from '../schemas/session_update_message'
import type { SessionMetadata } from '../schemas/session_meta'
import { SESSION_PATH } from '../constants/pluginPaths'

export type { SessionMessage }

export type { SessionListItem } from '../schemas/session_list'
export type { SessionMetadata } from '../schemas/session_meta'

/**
 * 获取会话列表（VDFS：`.vdfs/session` 的目录内容）
 *
 * 会话挂载点已把清单所需字段挂在节点上（`message_count` / `metadata` /
 * `meta_tags`，VDFS 只透传场景字段），故这里把节点直接映射为
 * `SessionListItem`，以便既有 `sessions` store / 对话组件保持兼容。
 *
 * `vdfs/list` 是会话清单的**唯一**读入口：不存在与之并行的第二套清单协议。
 *
 * 挂载根下与资源**并列**的还有本插件的配置文件（`PLUGIN.yml`，`ext = form`，
 * 见 docs/design/vdfs.md §3.4）。它是给通用列表 / 表单视图用的，不属于会话清单，
 * 因此这里按 `ext` 过滤——**只认会话节点**，不按名字特判（文件名可改，语义不变）。
 */
export async function listSessions(): Promise<SessionList.SessionListItem[]> {
  const resp = await listVdfs(vdfsJoin(VDFS_ROOT, 'session'))
  return (resp.items || [])
    .filter((n) => vdfsExtOf(n) === VDFS_EXT_SESSION)
    .map((n) => {
      const v = n as Record<string, any>
      return {
        id: n.name,
        // 后端 display_title：metadata.title 优先，否则从会话内容自动生成
        name: n.title ?? '',
        message_count: Number(v.message_count ?? 0),
        updated_at: n.updated_at ?? 0,
        is_working: n.status === VDFS_STATUS_WORKING,
        metadata: v.metadata ?? {},
      }
    })
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

