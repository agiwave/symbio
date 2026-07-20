// Corresponding Backend: symbio/src/symbio_core/schemas/session_append.rs
import { ChatMessage } from './chat_message';

export interface Request {
  session_id: string;
  messages: ChatMessage[];
}

export interface Response {
  message_count: number;
}

