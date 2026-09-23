/**
 * 会话清单项 —— **前端投影**（不是后端回包类型）
 *
 * 来源：`<根>/session` 的 `vdfs/list` 节点，由 `services/session.listSessions`
 * 逐项映射（清单是会话的唯一读入口，不存在与之并行的第二套清单协议）。
 */

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
  /**
   * 会话的**选项定义**（后端产出的 `DetailDefinition`，`binding = "option"`）。
   *
   * 定义与值同源于会话节点（`node.schema` / `node.attributes.metadata`），因此
   * 与会话树解耦的面板（`ModelChatPanel`，只有 `sessionId`）能从清单里零额外请求
   * 拿到它——不必回读 `vdfs/stat`（那是热路径，定义不挂在那里）。
   */
  schema?: unknown;
}

export interface Response {
  sessions: SessionListItem[];
}
