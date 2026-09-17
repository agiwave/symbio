/**
 * Model \u5bf9\u8bdd服务
 *
 * - 使用连接模式与后端通信
 * - 支持请求中止
 * - 支持断线重连
 * - 支持多模态消息（文本+图片）
 */

import { ChatMessage, ContentPart, MessageContent, ChatRole, ChatMessageType, MessageStatus } from '../schemas/chat_message'


// ==================== Chat 事件协议（原 schemas/chat_response，仅本模块使用，内联） ====================

/**
 * 会话事件类型（对齐后端 `session_chat_response::StreamEvent`）。
 *
 * ## 前端已不再订阅这条通道（S20）
 *
 * 会话的显示——运行态 / 结局 / 错误 / 消息——**全部**由 VDFS 节点状态承载
 * （`kind = "vdfs"` 变更，见 `symbio/src/plugins/session/docs/node-state-streaming.md`）。
 * 本枚举保留为**线路协议的类型描述**：后端仍发布这些帧，因为**进程内**消费者
 * 依赖它们（`agent/host/subagent.rs` 以 `Update` 做审批透传与文本累积、
 * 以 `Status idle` 判定子会话结束）。
 */
export enum ChatEventType {
  Update = 'update',
  Error = 'error',
  Connected = 'connected',
  Disconnected = 'disconnected',
  Status = 'status',
  Abort = 'abort',
  Delete = 'delete'
}

export type { ChatMessage, ContentPart, MessageContent, ChatMessageType, MessageStatus, ChatRole }

// ==================== 类型定义 ====================

/**
 * 连接事件
 */
export interface ChatEvent {
  type: ChatEventType
  request_id?: number
  message?: ChatMessage
  patch?: Partial<ChatMessage>
  error?: string
  connection_id?: string
  session_id?: string
  done?: boolean
  is_working?: boolean
  [key: string]: any
}
