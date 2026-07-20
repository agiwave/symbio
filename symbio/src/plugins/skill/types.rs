use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when_to_use: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub disable_model_invocation: bool,
    pub file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillConfig {
    #[serde(default = "default_skill_dirs")]
    pub skill_dirs: Vec<String>,

    /// 单次加载的最大 skill 数量（防止 LLM context 爆炸）
    ///
    /// 默认 20。超过此数量时按目录顺序截断并 warn。
    #[serde(default = "default_max_skills")]
    pub max_skills: usize,

    /// 单个 skill body 的最大字符数（防止单 skill 过大）
    ///
    /// 默认 8000（约 2k tokens）。超过时截断并 warn。
    #[serde(default = "default_max_body_chars")]
    pub max_body_chars: usize,

    /// 是否在 system_prompt 中显示 token 估算（默认 true）
    #[serde(default = "default_true")]
    pub report_token_estimate: bool,
}

impl Default for SkillConfig {
    fn default() -> Self {
        Self {
            skill_dirs: default_skill_dirs(),
            max_skills: default_max_skills(),
            max_body_chars: default_max_body_chars(),
            report_token_estimate: default_true(),
        }
    }
}

impl Skill {
    /// 模板变量替换：`${var}` → `value`（来自 variables JSON）
    ///
    /// ## 支持
    /// - 标准占位符：`${name}` 被 `variables["name"]` 替换
    /// - 转义：`$$` → `$`（用于在文本中输出字面量 `$`，例如 `$${var}` → `${var}`）
    /// - 多轮扫描：替换后重新扫描，防止新引入的占位符被下一轮替换
    ///
    /// ## 不支持
    /// - 嵌套对象（variables 值是 object 时，仅 to_string 序列化）
    pub fn substitute_variables(&self, text: &str, variables: &serde_json::Value) -> String {
        let obj = match variables.as_object() {
            Some(o) => o,
            None => return text.to_string(),
        };

        let mut result = text.to_string();
        // 最多 3 轮扫描（防止循环）
        for _ in 0..3 {
            // 1) 先把 `$$` 转义为内部占位符
            let with_escape = result.replace("$$", "\x00__DOLLAR__\x00");
            // 2) 替换占位符
            let mut after_subs = with_escape;
            let mut changed = false;
            for (key, value) in obj {
                let placeholder = format!("${{{}}}", key);
                if after_subs.contains(&placeholder) {
                    let replacement = match value {
                        serde_json::Value::String(s) => s.clone(),
                        _ => value.to_string(),
                    };
                    after_subs = after_subs.replace(&placeholder, &replacement);
                    changed = true;
                }
            }
            // 3) 还原转义
            let restored = after_subs.replace("\x00__DOLLAR__\x00", "$");
            if !changed {
                return restored;
            }
            result = restored;
        }
        result
    }
}

fn default_skill_dirs() -> Vec<String> {
    vec![".symbio/skills".to_string()]
}

fn default_max_skills() -> usize {
    20
}

fn default_max_body_chars() -> usize {
    8000
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests;
