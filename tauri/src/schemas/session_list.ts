// Corresponding Backend: symbio/src/symbio_core/schemas/session_list.rs

export interface SessionListItem {
  id: string;
  /** 会话显示名（后端 display_title：metadata.title 优先，否则从会话内容自动生成） */
  name?: string;
  message_count: number;
  updated_at: number;
  /** 运行状态：与 VDFS 节点 `status` 同一词表（`working` / `active` / …）。
   *
   * 不存 `is_working` 布尔——「忙不忙」是 `status` 的一个取值，
   * 由 `isWorkingStatus()` 派生；这里存原值，其余状态（error / disabled）才不丢。 */
  status?: string;
  /** 会话元数据摘要（workdir / title / agent_id 等） */
  metadata: Record<string, any>;
}

export interface Response {
  sessions: SessionListItem[];
}
