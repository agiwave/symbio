use crate::plugins::skill::loader::{load_skills_from_dirs_with_budget, LoadBudget};
use crate::plugins::skill::skill_tool::SkillExecuteTool;
use crate::plugins::skill::types::{Skill, SkillConfig};
use crate::symbio_core::schemas::agent::skill::SkillResponse;
use crate::symbio_core::schemas::agent::skill_get as skill_get_schema;
use crate::symbio_core::schemas::agent::skill_list;
use crate::symbio_core::{
    HomedirRegistry, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError,
    PluginMeta, PluginPayload, PLUGIN_SKILL, TRAVERSE_AVAILABLE_TOOLS,
};
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct SkillPlugin {
    config: Arc<RwLock<SkillConfig>>,
}

impl SkillPlugin {
    /// 把 skill_dirs 里的 `{HOMEDIR}` 占位符解析为当前系统目录
    fn resolve_skill_dirs_template(dirs: &mut [String]) {
        let homedir = HomedirRegistry::get()
            .join("plugins")
            .join("skills")
            .to_string_lossy()
            .to_string();
        for d in dirs.iter_mut() {
            if d.contains("{HOMEDIR}/plugins/skills") {
                *d = d.replace("{HOMEDIR}/plugins/skills", &homedir);
            } else if d == "{HOMEDIR}/plugins/skills" {
                *d = homedir.clone();
            }
        }
    }

    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let mut config: SkillConfig = ctx
            .config()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_else(|| SkillConfig {
                // 加载路径优先级：
                // 1. 工作区级别：`.symbio/skills`（项目内）
                // 2. 系统级别：`<homedir>/plugins/skills`（symbio 系统级）
                // 3. 第三方工具兼容：`.qwen/skills`、`.sixth/skills`、`.qoder/skills`
                skill_dirs: vec![
                    ".symbio/skills".to_string(),
                    "{HOMEDIR}/plugins/skills".to_string(),
                ],
                // 预算字段使用 SkillConfig::default() 的值
                ..SkillConfig::default()
            });

        // 解析 {HOMEDIR} 占位符
        Self::resolve_skill_dirs_template(&mut config.skill_dirs);

        Arc::new(SkillPlugin::new(config)) as Arc<dyn Plugin>
    }

    pub fn new(config: SkillConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("skill", "技能插件")
            .with_description("提供行业标准技能 (Skill) 加载与执行能力")
            .with_version("0.2.0")
    }

    /// 为 API 端点（skill/list、skill/get）加载技能
    ///
    /// 与 LLM 工具路径不同，API 路径**不做** exe-path fallback：
    /// - `~` 前缀路径（系统级）通过 expand_tilde 展开为绝对路径，与 workdir 无关
    /// - 相对路径（工作区级）相对 workdir 解析
    /// - 不从 exe 路径推算项目根目录，避免在生产环境加载非预期的 skill
    async fn load_skills_for_api(
        &self,
        workdir: Option<String>,
    ) -> Result<Vec<Skill>, PluginError> {
        let config = self.config.read().await;
        let workdir_path = workdir
            .as_deref()
            .map(Path::new)
            .unwrap_or_else(|| Path::new("."));
        let budget = LoadBudget {
            max_skills: config.max_skills,
            max_body_chars: config.max_body_chars,
        };
        load_skills_from_dirs_with_budget(&config.skill_dirs, workdir_path, budget).await
    }

    /// 为 LLM 工具（traverse / execute）加载技能
    ///
    /// 保留 exe-path fallback：当 workdir 下找不到任何 skill 时，
    /// 尝试从可执行文件所在目录向上推算项目根目录，兼顾开发阶段使用场景。
    async fn load_skills_for_tool(
        &self,
        workdir: Option<String>,
    ) -> Result<Vec<Skill>, PluginError> {
        let config = self.config.read().await;
        let workdir_path = workdir
            .as_ref()
            .map(Path::new)
            .unwrap_or_else(|| Path::new("."));
        let budget = LoadBudget {
            max_skills: config.max_skills,
            max_body_chars: config.max_body_chars,
        };
        let mut skills =
            load_skills_from_dirs_with_budget(&config.skill_dirs, workdir_path, budget).await?;

        // 如果从工作目录没有加载到技能，尝试从项目根目录加载
        if skills.is_empty() {
            let project_root = if let Ok(exe_path) = std::env::current_exe() {
                exe_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf()
            } else {
                Path::new(".").to_path_buf()
            };
            let global_skills =
                load_skills_from_dirs_with_budget(&config.skill_dirs, &project_root, budget)
                    .await?;
            skills.extend(global_skills);
        }
        Ok(skills)
    }

    async fn list_skills(&self, workdir: Option<String>) -> Result<Vec<Skill>, PluginError> {
        self.load_skills_for_tool(workdir).await
    }

    /// 列出 skill 并标注来源
    ///
    /// 来源规则：
    /// - 包含 `~` 或 `~/.symbio/plugins/skills` → `system`
    /// - 以 `.qwen` `.sixth` `.qoder` 开头 → `external`
    /// - 其它 → `workspace`
    async fn list_skills_with_source(
        &self,
        workdir: Option<String>,
    ) -> Result<Vec<crate::symbio_core::schemas::agent::skill_list::SkillInfo>, PluginError> {
        let config = self.config.read().await;
        let skills = self.load_skills_for_api(workdir).await?;

        // 用 dir 路径反查每个 skill 的来源
        Ok(skills
            .into_iter()
            .map(|s| {
                let source = Self::classify_skill_source(&s.file_path, &config.skill_dirs);
                crate::symbio_core::schemas::agent::skill_list::SkillInfo {
                    name: s.name,
                    description: s.description,
                    file_path: s.file_path,
                    source,
                    argument_hint: s.argument_hint,
                    when_to_use: s.when_to_use,
                }
            })
            .collect())
    }

    /// 根据 SKILL.md 所在目录，反查它在哪个 skill_dir 下，从而判定来源
    fn classify_skill_source(file_path: &str, skill_dirs: &[String]) -> String {
        let file_path = file_path.replace('\\', "/");
        let file_path_norm = file_path.trim_end_matches('/');

        for dir in skill_dirs {
            // 把 dir 展开为若干候选路径
            // - `~/...` → `<home>/...`
            // - `<homedir>/plugins/skills` 等绝对路径保持不变
            // - 相对路径 `.symbio/skills` 也尝试在 home 下展开（兼容旧配置）
            let mut candidates: Vec<String> = Vec::new();
            if let Some(stripped) = dir.strip_prefix("~/") {
                if let Some(home) = dirs::home_dir() {
                    candidates.push(home.join(stripped).to_string_lossy().replace('\\', "/"));
                }
                candidates.push(dir.clone());
            } else if dir.starts_with('/') || dir.contains(':') {
                // 绝对路径（Unix 或 Windows）
                candidates.push(dir.clone());
            } else {
                // 相对路径：也尝试在 home 下展开（兼容 `.symbio/skills` 这种用法）
                if let Some(home) = dirs::home_dir() {
                    candidates.push(home.join(dir).to_string_lossy().replace('\\', "/"));
                }
                candidates.push(dir.clone());
            }

            let matches = candidates.iter().any(|c| {
                let c_norm = c.trim_end_matches('/');
                !c_norm.is_empty()
                    && (file_path_norm == c_norm
                        || file_path_norm.starts_with(&format!("{c_norm}/")))
            });

            if matches {
                let lower = dir.to_lowercase();
                // 注意：判断"系统级"的特征是"路径以 .symbio/plugins/skills 结尾"，
                // 不管 homedir 怎么切，特征都一致
                if lower.starts_with("~")
                    || lower.contains(".symbio/plugins/skills")
                    || lower.contains(".symbio/skills")
                {
                    return "system".to_string();
                }
                if lower.contains(".qwen") || lower.contains(".sixth") || lower.contains(".qoder") {
                    return "external".to_string();
                }
                return "workspace".to_string();
            }
        }
        "unknown".to_string()
    }

    /// BUG-FR9：根据 name 查找 skill 详情（含 body）
    ///
    /// 复用 `list_skills` 的结果做 O(1) 查找，避免重复磁盘 IO。
    /// body 已按 `max_body_chars` 截断。
    async fn get_skill_detail(
        &self,
        name: &str,
        workdir: Option<String>,
    ) -> Result<skill_get_schema::Response, PluginError> {
        let config = self.config.read().await;
        let skills = self.load_skills_for_api(workdir).await?;
        let s = skills
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| PluginError::NotFound(format!("Skill not found: {name}")))?;

        let source = Self::classify_skill_source(&s.file_path, &config.skill_dirs);
        let body_truncated = s.body.contains("[... body truncated");
        let body_chars = s.body.chars().count();

        Ok(skill_get_schema::Response {
            name: s.name,
            description: s.description,
            file_path: s.file_path,
            source,
            argument_hint: s.argument_hint,
            when_to_use: s.when_to_use,
            body: s.body,
            body_chars,
            body_truncated,
        })
    }
}

#[cfg(test)]
mod tests;

#[async_trait]
impl Plugin for SkillPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        match path {
            "list" => {
                // 列出所有已加载的 skill（按来源分类标注）
                let workdir = ctx.get(crate::symbio_core::WORKDIR).or_else(|| {
                    ctx.payload::<skill_list::Request>()
                        .ok()
                        .and_then(|r| r.workdir)
                });
                let skills = self.list_skills_with_source(workdir).await?;
                Ok(PluginPayload::new(&skill_list::Response { skills }))
            }
            // BUG-FR9：前端 SkillView 详情面板预览用
            "get" => {
                let req: skill_get_schema::Request = ctx.payload()?;
                let workdir = ctx.get(crate::symbio_core::WORKDIR).or(req.workdir);
                let detail = self.get_skill_detail(&req.name, workdir).await?;
                Ok(PluginPayload::new(&detail))
            }
            "execute" => {
                #[derive(serde::Deserialize, Clone)]
                struct ExecuteRequest {
                    name: String,
                    args: Option<String>,
                }
                let req: ExecuteRequest = ctx.payload()?;

                let skills = self
                    .list_skills(ctx.get(crate::symbio_core::WORKDIR))
                    .await?;
                let mut skill =
                    skills
                        .into_iter()
                        .find(|s| s.name == req.name)
                        .ok_or_else(|| {
                            PluginError::NotFound(format!("Skill not found: {}", req.name))
                        })?;

                // 检查参数：如果技能需要参数但未提供，返回错误提示
                if let Some(hint) = &skill.argument_hint {
                    if req.args.is_none() {
                        return Err(PluginError::ValidationError(format!(
                            "技能 '{}' 需要参数，但未提供。请提供以下参数：\n{}",
                            skill.name, hint
                        )));
                    }
                }

                // 解析参数
                let args: serde_json::Value = req
                    .args
                    .map(|s| serde_json::from_str(&s))
                    .transpose()?
                    .unwrap_or_default();

                // 参数替换
                let body_with_args = skill.substitute_variables(&skill.body, &args);
                skill.body = body_with_args;

                // 返回技能描述（符合行业标准的行为）
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
                    args: Some(args),
                }))
            }
            _ => {
                let skills = self
                    .list_skills(ctx.get(crate::symbio_core::WORKDIR))
                    .await?;
                if let Some(mut skill) = skills.into_iter().find(|s| s.name == path) {
                    let args: serde_json::Value = serde_json::Value::Object(serde_json::Map::new());
                    let body_with_args = skill.substitute_variables(&skill.body, &args);
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

                    return Ok(PluginPayload::new(&SkillResponse {
                        name: skill.name,
                        body: instructions,
                        allowed_tools: skill.allowed_tools,
                        model: skill.model,
                        base_dir,
                        args: None,
                    }));
                }
                Err(PluginError::NotFound(format!("路径不存在: {path}")))
            }
        }
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        let sub_path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        if sub_path == TRAVERSE_AVAILABLE_TOOLS {
            let skills = self
                .list_skills(ctx.get(crate::symbio_core::WORKDIR))
                .await?;

            if !skills.is_empty() {
                if let Some(tool_manager) = ctx.get(crate::symbio_core::CAPABILITY_MANAGER) {
                    let skill_tool = Arc::new(SkillExecuteTool::new(skills));
                    tool_manager.register(skill_tool).await;
                }
            }
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_SKILL, SkillPlugin::build, dyn Plugin);
