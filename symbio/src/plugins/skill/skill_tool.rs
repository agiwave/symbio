use super::skill_response::assemble_response;
use crate::plugins::skill::types::Skill;
use crate::symbio_core::{Capability, CapabilityMeta, ExecEnv, PluginError, PluginInvokeRequest};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct SkillExecuteTool {
    skills: Vec<Skill>,
}

impl SkillExecuteTool {
    pub fn new(skills: Vec<Skill>) -> Self {
        Self { skills }
    }

    fn build_skill_details(&self) -> Vec<String> {
        self.skills
            .iter()
            .filter(|s| !s.disable_model_invocation)
            .map(|s| format!("{} - {}", s.name, s.description))
            .collect()
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
}

#[async_trait]
impl Capability for SkillExecuteTool {
    fn meta(&self) -> CapabilityMeta {
        let skill_names = self.build_skill_names();
        let skill_details = self.build_skill_details();

        CapabilityMeta {
            name: "read_skill".to_string(),
            description: format!(
                "获取指定技能的执行指导。\n\n可用技能：\n{}",
                skill_details.join("\n")
            ),
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
            category: Some(crate::symbio_core::CapabilityCategory::Skill),
            examples: Some(vec![format!(
                "name='{}'",
                skill_names.first().unwrap_or(&"skill-name".to_string())
            )]),
            ..Default::default()
        }
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        self.execute_skill(args)
    }
}
