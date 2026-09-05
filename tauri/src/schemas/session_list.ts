// Corresponding Backend: symbio/src/symbio_core/schemas/session_list.rs

export interface SessionListItem {
  id: string;
  /** 会话显示名（后端 display_title：metadata.title 优先，否则从会话内容自动生成） */
  name?: string;
  message_count: number;
  updated_at: number;
  /** 实时运行状态：是否正在与 AI 通信 */
  is_working: boolean;
  /** 会话元数据摘要（workdir / title / agent_id 等） */
  metadata: Record<string, any>;
}

export interface Response {
  sessions: SessionListItem[];
}
