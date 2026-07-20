// Corresponding Backend: symbio/src/symbio_core/schemas/session/session_update_message.rs
import type { ChatMessage } from './chat_message'

export interface Request {
  session_id: string
  /** 必须携带 id；其余字段为可选覆盖项 */
  message: ChatMessage
}
export interface Response {
  updated: boolean
}
