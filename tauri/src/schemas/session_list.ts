/**
 * 会话清单项 —— **前端投影**（不是后端回包类型）
 *
 * 来源：`<根>/session` 的 `vdfs/list` 节点，由 `services/session.listSessions`
 * 逐项映射（清单是会话的唯一读入口，不存在与之并行的第二套清单协议）。
 */

import type { ReadingSection } from './reading'

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
  /**
   * **上下文水位**（`node.attributes.context_watermark`，后端在交付出口排好的读数）。
   *
   * 类型是宿主方言的一格 [`ReadingSection`]（行 + 可选 `bar` 几何），**不是**会话专有
   * 形状：本项前端不认「已用」「上限」「阈值」任何一个词，换一个产者（成本、记忆）
   * 零改动。值里的单位、精度与除法都在后端定，前端不做第二次换算。
   *
   * 缺省 = 这个会话还没记录水位（存量会话 / 一轮没跑完），渲染器因此**整条不出现**，
   * 而不是画一条空条——「上下文是空的」与「这里没有数」是两件事。
   *
   * 真源：`symbio/src/plugins/session/stats_reading.rs::context_reading_of`。
   */
  context_watermark?: ReadingSection;
}

export interface Response {
  sessions: SessionListItem[];
}
