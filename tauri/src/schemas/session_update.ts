// Corresponding Backend: symbio/src/symbio_core/schemas/session_update.ts
//
// 与后端 snake_case 协议保持一致。
export interface Request {
  session_id: string;
  metadata: Record<string, any>;
  title?: string;
}

export interface Response {
  success: boolean;
  session: any;
}
