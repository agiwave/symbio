// Corresponding Backend: symbio/src/symbio_core/schemas/agent_config.rs

export interface AgentConfig {
  /** 存储目录 */
  storage_dir: string;
  /** 最大条目数 */
  max_entries: number;
  /** 预定义分类 */
  categories: string[];
}
