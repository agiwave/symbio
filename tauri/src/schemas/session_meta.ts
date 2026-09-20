/**
 * Session 元数据约定
 *
 * 写入后端的 `Session.metadata` JSON 字段：
 *  - workdir:  会话级工作目录（决定 AI 工具调用上下文与右侧文件树）
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
 * 前端不持有任何业务字段名，详见 `symbio/src/plugins/session/docs/cascading-options-mechanism.md`。
 */
/**
 * 执行风险等级阈值（`metadata.risk_level`）。
 *
 * **取值即跨栈契约**：后端 `plugins/local/policy/policy_types.rs` 的 `RiskLevel`
 * 枚举以 `#[serde(rename_all = "lowercase")]` 序列化成这三个词，前端按它比较阈值。
 *
 * 写成**词表数组**而不是裸的字面量联合，是为了让它可被守卫比对：裸联合只是类型，
 * 运行期不存在，`protocol-mirror-audit` 的 C 组（后端闭集枚举 ↔ 前端词表数组）
 * 看不见它。数组 + `(typeof X)[number]` 是同一条信息的两种形态，不是两份真相。
 *
 * ⚠️ 注意 `lowercase` 与 `snake_case` 对多词变体的结果不同（`ReadOnly` → `readonly`
 * vs `read_only`）；本枚举全是单词，两者恰好一致，但**不要**据此认为可以互换。
 */
export const SESSION_RISK_LEVELS = ['low', 'medium', 'high'] as const
export type SessionRiskLevel = (typeof SESSION_RISK_LEVELS)[number]

/**
 * 运行模式（`metadata.mode`）：`auto` = 无人值守，`interactive` = 会话流内可交互。
 *
 * 后端目前**没有**对应的 Rust 枚举（`metadata.mode` 就是字符串），故它暂不是
 * 跨栈闭集、不进 C 组；但前端内部的**唯一定义处**仍应在此——先前
 * `stores/sessionLive.ts` 与 `stores/sessions.ts` 各写了一份同样的联合。
 *
 * 同样写成数组：`SessionListItem.metadata` 是 `Record<string, any>`（**未类型化**），
 * 校验后端回包只能靠运行期的取值枚举，而枚举必须只有一处。
 */
export const SESSION_MODES = ['auto', 'interactive'] as const
export type SessionMode = (typeof SESSION_MODES)[number]

export interface SessionMetadata {
  workdir?: string;
  title?: string;
  agent_id?: string;
  /** 选定的 Model Provider ID（与 agent_id 同级别：随 chat_send 传输 + session.metadata 持久化） */
  provider_id?: string;
  /** 执行风险等级阈值：low / medium / high（与 agent_id 同级别） */
  risk_level?: SessionRiskLevel;
  /** 运行模式：auto（无人值守）/ interactive（默认，会话流内可交互） */
  mode?: SessionMode;
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
