use serde::{Deserialize, Serialize};
use symbio_plugin_api::PluginError;

use super::types::Skill;

/// Skill 执行响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillResponse {
    pub name: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub base_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<serde_json::Value>,
}

/// 技能指令的固定前缀：告诉模型「脚本与引用文件都从这个基目录解析」。
fn with_base_dir(base_dir: &str, body: &str) -> String {
    format!(
        "Base directory for this skill: {base_dir}\n\
         Important: ALWAYS resolve absolute paths from this base directory when working \
         with scripts or referenced files in this skill.\n\n{body}"
    )
}

/// 把一条技能组装成**返回值**（`skill/execute`、名称路由与 `read_skill` 共用这一处）。
///
/// 这里是**唯一**的组装点：`skill/execute`、名称路由与 `read_skill` 都走它，
/// 前缀与参数校验不会在两条路径上分叉。规则：
///
/// 1. 声明了 `argument_hint` 就**必须有** `args`（缺了返回可行动的校验错误）；
/// 2. `args` 是 JSON 字符串，原样解析后交给模板替换；
/// 3. 正文前恒加基目录前缀——技能里的脚本 / 引用路径以技能文件所在目录为准。
pub(crate) fn assemble_response(
    skill: &Skill,
    raw_args: Option<&str>,
) -> Result<SkillResponse, PluginError> {
    if let Some(hint) = &skill.argument_hint {
        if raw_args.is_none() {
            return Err(PluginError::ValidationError(format!(
                "技能 '{}' 需要参数，但未提供。请提供以下参数：\n{}",
                skill.name, hint
            )));
        }
    }

    let args: Option<serde_json::Value> = raw_args.map(serde_json::from_str).transpose()?;
    let body = skill.substitute_variables(
        &skill.body,
        args.as_ref().unwrap_or(&serde_json::Value::Null),
    );
    let base_dir = std::path::Path::new(&skill.file_path)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_string_lossy()
        .to_string();

    Ok(SkillResponse {
        name: skill.name.clone(),
        body: with_base_dir(&base_dir, &body),
        allowed_tools: skill.allowed_tools.clone(),
        model: skill.model.clone(),
        base_dir,
        args,
    })
}

#[cfg(test)]
#[path = "skill_response.test.rs"]
mod tests;
