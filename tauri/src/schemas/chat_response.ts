import { ChatMessage } from './chat_message';

export interface Response {
  message: ChatMessage;
}

export enum ChatEventType {
  Update = 'update',
  Error = 'error',
  Connected = 'connected',
  Disconnected = 'disconnected',
  Status = 'status',
  Abort = 'abort',
  SessionResumed = 'session_resumed',
  Delete = 'delete'
}

export type StreamEvent =
  | { type: ChatEventType.Update; message: ChatMessage }
  | { type: ChatEventType.Error; error: string }
  | { type: ChatEventType.Connected; session_id: string; is_working: boolean; messages: ChatMessage[] }
  | { type: ChatEventType.Disconnected }
  | { type: ChatEventType.Status; status: string }
  | { type: ChatEventType.Abort }
  | { type: ChatEventType.SessionResumed; session_id: string; parent_session_id: string; failed: boolean; result: string | null }
  | { type: ChatEventType.Delete; message_id: string };
