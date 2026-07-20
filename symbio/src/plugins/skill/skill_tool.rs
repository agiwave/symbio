use crate::plugins::skill::types::Skill;
use crate::symbio_core::schemas::agent::skill::SkillResponse;
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError,
    PluginPayload,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;
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

    fn execute_skill(&self, args: Value) -> InvokeResponse<PluginPayload> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 name 参数".to_string()))?;

        let mut skill = self
            .skills
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| PluginError::NotFound(format!("Skill not found: {}", name)))?
            .clone();

        if let Some(hint) = &skill.argument_hint {
            if args.get("args").is_none() {
                return Err(PluginError::ValidationError(format!(
                    "技能 '{}' 需要参数，但未提供。请提供以下参数：\n{}",
                    skill.name, hint
                )));
            }
        }

        let skill_args: Value = args
            .get("args")
            .and_then(|v| v.as_str())
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or_default();

        let body_with_args = skill.substitute_variables(&skill.body, &skill_args);
        skill.body = body_with_args;

        let base_dir = Path::new(&skill.file_path)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_string_lossy()
            .to_string();

        let instructions = format!(
            "Base directory for this skill: {base_dir}\nImportant: ALWAYS resolve absolute paths from this base directory when working with scripts or referenced files in this skill.\n\n{}",
            skill.body
        );

        Ok(PluginPayload::new(&SkillResponse {
            name: skill.name,
            body: instructions,
            allowed_tools: skill.allowed_tools,
            model: skill.model,
            base_dir,
            args: Some(skill_args),
        }))
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

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let args: Value = ctx.payload()?;
        self.execute_skill(args)
    }
}
