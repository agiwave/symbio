// Corresponding Backend: symbio/src/symbio_core/schemas/session/session_clear_messages.rs
export interface Request {
  session_id: string
}
export interface Response {
  cleared: boolean
}
