// Corresponding Backend: symbio/src/symbio_core/schemas/session/session_delete_message.rs
export interface Request {
  session_id: string
  message_id: string
}
export interface Response {
  /** 被删除的消息总数（含目标消息及其之后的所有消息） */
  deleted: number
  /** 被删除消息的 id 列表（前端据此精确移除本地状态） */
  deleted_ids: string[]
}
