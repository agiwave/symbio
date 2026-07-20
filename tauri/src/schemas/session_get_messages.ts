// Corresponding Backend: symbio/src/symbio_core/schemas/session_get_messages.ts
import { ChatMessage } from './chat_message';

/**
 * 获取会话消息请求
 * 
 * 设计说明：
 * - 不分页获取完整会话历史
 * - AI插件和前端共用此接口
 * - 如果会话历史被压缩或剪裁，返回的是压缩/剪裁后的内容
 */
export interface Request {
  session_id: string;
}

export interface Response {
  messages: ChatMessage[];
}

