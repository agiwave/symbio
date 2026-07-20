use std::path::PathBuf;
use std::sync::Arc;

use crate::plugins::agent::core::AgentConfig;

#[derive(Debug, Clone)]
pub struct CognitionContext {
    pub agent_config: Arc<AgentConfig>,
    /// Agent 配置目录路径
    ///
    /// 当前由 store 层工厂函数（`build_store` / `create_store`）通过参数传递，
    /// 本字段作为上下文保留，供未来扩展使用（如相对路径解析、日志标注等）。
    #[allow(dead_code)] // 预留字段：当前 store 层通过函数参数获取 agent_dir
    pub agent_dir: PathBuf,
}

impl CognitionContext {
    pub fn new(agent_config: Arc<AgentConfig>, agent_dir: PathBuf) -> Self {
        Self {
            agent_config,
            agent_dir,
        }
    }
}
