// Corresponding Backend: symbio/src/symbio_core/schemas/session_config.rs

export interface SessionConfig {
  /** 存储目录 (~ 表示当前工作区) */
  storage_dir: string;
  /** 最大消息数 */
  max_messages: number;
  /** 自动压缩 */
  auto_compress: boolean;
  /** 压缩阈值（消息数） */
  compress_threshold: number;
  /** 上下文消息数量限制（0 表示不限制） */
  context_messages: number;
  /** 默认认知人格 ID */
  default_agent_id: string;
  /** 会话ID（用于标识具体会话的配置） */
  session_id?: string;
}
