use super::skill_response::assemble_response;
use crate::types::Skill;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use symbio_plugin_api::{
    port_result, CapabilityMeta, ExecEnv, PluginError, PluginInvokeRequest, PortErrorClass,
    PortExecContext, PortExecutor, PortInput, PortOutcome,
};

pub struct SkillExecuteTool {
    skills: Vec<Skill>,
}

impl SkillExecuteTool {
    pub fn new(skills: Vec<Skill>) -> Self {
        Self { skills }
    }

    fn build_skill_names(&self) -> Vec<String> {
        self.skills
            .iter()
            .filter(|s| !s.disable_model_invocation)
            .map(|s| s.name.clone())
            .collect()
    }

    fn execute_skill(&self, args: Value) -> Result<Value, PluginError> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 name 参数".to_string()))?;

        let skill = self
            .skills
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| PluginError::NotFound(format!("Skill not found: {}", name)))?;

        let response = assemble_response(skill, args.get("args").and_then(Value::as_str))?;
        Ok(serde_json::to_value(&response)?)
    }

    pub fn meta(&self) -> CapabilityMeta {
        let skill_names = self.build_skill_names();

        CapabilityMeta {
            name: "read_skill".to_string(),
            description: "获取指定技能的执行指导（从 `name` 的 enum 中选）。".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "技能名称",
                        "enum": skill_names
                    },
                    "args": {
                        "type": "string",
                        "description": "技能参数（JSON 格式）"
                    }
                },
                "required": ["name"]
            }),
            category: Some(symbio_plugin_api::CapabilityCategory::Skill),
            examples: Some(vec![format!(
                "name='{}'",
                skill_names.first().unwrap_or(&"skill-name".to_string())
            )]),
            ..Default::default()
        }
    }
}

#[async_trait]
impl PortExecutor for SkillExecuteTool {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        let Some(ctx) = exec.ctx else {
            return PortOutcome::failed(
                PortErrorClass::NonRetryable,
                "工具端口没有调用方上下文，无法拆信封".to_string(),
            );
        };
        let env = ExecEnv::from_request(&*ctx);
        port_result(self.run(input.payload, &env, ctx).await)
    }
}

impl SkillExecuteTool {
    async fn run(
        &self,
        args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        self.execute_skill(args)
    }
}
