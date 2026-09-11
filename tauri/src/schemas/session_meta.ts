/**
 * Session 元数据约定
 *
 * 写入后端的 `Session.metadata` JSON 字段：
 *  - workdir:  会话级工作目录（决定 AI 工具调用上下文与右侧实体树）
 *  - title:    会话标题（用于会话列表展示）
 *  - agent_id: 使用的 Agent 标识
 *  - provider_id: 选定的 Model Provider ID（与 agent_id 同级别）
 *  - risk_level: 执行风险等级阈值 low/medium/high（与 agent_id 同级别）
 *  - mode: 运行模式 auto/interactive（与 agent_id 同级别）
 *  - created_via: "ui" | "api"
 *  - last_message_preview: 首条消息的简短摘要（用于列表显示优化）
 *  - heartbeat: 会话心跳任务配置（空闲指定时间后自动触发提示词）
 *
 * ## 会话参数统一模型
 *
 * agent_id / provider_id / risk_level / mode / workdir 级别相同，统一走：
 * `session.metadata` 持久化（写入唯一经**级联选项机制**——
 * 会话页选项行选择 → `worker/session/update` 浅合并）
 * + 后端各解析链按 metadata 回退取值。
 * 前端不持有任何业务字段名，详见 `docs/design/cascading-options-mechanism.md`。
 */
export interface SessionMetadata {
  workdir?: string;
  title?: string;
  agent_id?: string;
  /** 选定的 Model Provider ID（与 agent_id 同级别：随 chat_send 传输 + session.metadata 持久化） */
  provider_id?: string;
  /** 执行风险等级阈值：low / medium / high（与 agent_id 同级别） */
  risk_level?: 'low' | 'medium' | 'high';
  /** 运行模式：auto（无人值守）/ interactive（默认，会话流内可交互） */
  mode?: 'auto' | 'interactive';
  created_via?: 'ui' | 'api';
  last_message_preview?: string;
  /** 心跳任务配置：会话空闲 interval_seconds 后自动以 prompt 触发一次对话 */
  heartbeat?: SessionHeartbeatConfig;
  [key: string]: unknown;
}

/**
 * 会话心跳任务配置
 *
 * 存储于 `Session.metadata.heartbeat`，由会话页「心跳任务」选项表单写入
 * （级联选项机制，`bind = metadata.heartbeat`）。
 * 后端 `SessionPlugin` 的后台调度器会按 interval_seconds 检测空闲并触发。
 */
export interface SessionHeartbeatConfig {
  /** 是否启用心跳任务（默认 false） */
  enabled: boolean;
  /** 空闲多少秒后触发（默认 300） */
  interval_seconds: number;
  /** 触发的任务提示词 */
  prompt: string;
  /** 触发时是否携带历史会话上下文（默认 true） */
  include_history: boolean;
}
