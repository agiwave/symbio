use super::skill_response::SkillResponse;
use crate::plugins::skill::loader::{load_skills_from_dirs_with_budget, LoadBudget};
use crate::plugins::skill::skill_tool::SkillExecuteTool;
use crate::plugins::skill::types::{Skill, SkillConfig};
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
}

// ==================== 统一实体协议 (entities/*) ====================
//
// 公共流程（列表包装 / zip 上传 / 幂等删除）由 `EntityProvider::dispatch` 承载，
// 这里只实现 skill 的差异化钩子（SKILL.md 摘要解析）。

#[async_trait]
impl crate::symbio_core::entities::EntityProvider for SkillPlugin {
    fn kind(&self) -> &'static str {
        crate::symbio_core::entities::ENTITY_SKILL
    }

    fn category(&self) -> Option<&'static str> {
        Some(crate::symbio_core::providers::categories::SKILL)
    }

    fn manifest_file(&self) -> Option<&'static str> {
        Some(crate::symbio_core::providers::manifests::SKILL)
    }

    /// 详情页定义：Skill 表单由后端下发（frontmatter 字段 + Markdown 正文），
    /// 表单 manifest ↔ SKILL.md 映射见 `super::detail`
    async fn detail_definition(
        &self,
        _ctx: &Arc<dyn crate::symbio_core::InvokeRequest>,
        _id: &str,
    ) -> Option<crate::symbio_core::schemas::entities::DetailDefinition> {
        Some(super::detail::skill_detail_definition())
    }

    /// 表单上传的校验/规范化：manifest → SKILL.md 全文（实际写盘内容）。
    ///
    /// 强制 BUG-SR6（名称 == 目录 id）与 BUG-SR7（description ≥ 10 字符），
    /// 错误在保存时即给出（而非下次加载时）。zip 上传路径不经过本钩子。
    async fn validate_manifest(
        &self,
        _ctx: &Arc<dyn crate::symbio_core::InvokeRequest>,
        id: &str,
        manifest: &serde_json::Value,
    ) -> Result<serde_json::Value, PluginError> {
        let md = super::detail::manifest_to_skill_md(id, manifest)?;
        Ok(serde_json::Value::String(md))
    }

    /// VDFS 新建（`write { create }`）的最小清单。
    ///
    /// `manifest_to_skill_md` 要求 `name` 与目录名（id）一致、`description`
    /// 至少 10 字符，故这里给出同名的骨架描述，用户随后在详情里完善。
    fn new_entity_manifest(&self, id: &str, _title: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "name": id,
            "description": format!("{id}：请填写该技能的用途与使用时机（至少 10 字）"),
        })
    }

    /// 从 SKILL.md 解析摘要：优先 YAML frontmatter（name / description），
    /// 无 frontmatter 时回落到旧的标题/Description 行解析
    async fn summarize(
        &self,
        _ctx: &Arc<dyn crate::symbio_core::InvokeRequest>,
        id: &str,
        manifest: Option<&str>,
    ) -> crate::symbio_core::entities::EntitySummary {
        let mut it = crate::symbio_core::entities::EntitySummary::new(
            crate::symbio_core::entities::ENTITY_SKILL,
            id,
            id,
        );
        it.status = "active".to_string();
        let Some(text) = manifest else {
            return it;
        };

        // frontmatter 路径：名称/摘要 + 完整 config（DetailForm 预填用）
        if let Some((yaml, _body)) = super::detail::parse_skill_md(text) {
            if let Some(name) = yaml.get("name").and_then(|v| v.as_str()) {
                it.name = name.to_string();
            }
            if let Some(desc) = yaml.get("description").and_then(|v| v.as_str()) {
                it.summary = Some(desc.to_string());
            }
            if let Some(cfg) = super::detail::skill_md_to_config(text) {
                if let serde_json::Value::Object(ref mut m) = it.extra {
                    let _ = m.insert("config".to_string(), cfg);
                }
            }
            return it;
        }

        // 旧格式回落：首行标题 + Description 行
        let cleaned = text.trim();
        let first_line = cleaned
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .trim_start_matches('#')
            .trim();
        if !first_line.is_empty() {
            it.name = first_line.to_string();
        }
        let mut summary = cleaned
            .lines()
            .find(|l| {
                l.trim().starts_with("**Description**") || l.trim().starts_with("Description")
            })
            .map(|l| {
                l.trim()
                    .trim_start_matches("**Description**")
                    .trim()
                    .trim_start_matches("Description")
                    .trim()
                    .to_string()
            })
            .unwrap_or_default();
        if summary.is_empty() {
            summary = cleaned.chars().take(120).collect();
        }
        if !summary.is_empty() {
            it.summary = Some(summary);
        }
        it
    }
}

#[async_trait]
impl Plugin for SkillPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        // 统一实体协议：entities/list / get / upload / delete / status
        if let Some(resp) = crate::symbio_core::entities::dispatch(self.as_ref(), path, &ctx).await
        {
            return resp;
        }

        match path {
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
                if let Some(tool_visitor) = ctx.get(crate::symbio_core::CAPABILITY_VISITOR) {
                    let skill_tool = Arc::new(SkillExecuteTool::new(skills));
                    tool_visitor.register(skill_tool).await;
                }
            }

            // VDFS 挂载点：把本插件的实体能力适配为一份 VDFS 资源（与实体机制
            // 共用同一份 list/read/write/delete 实现，见 `vdfs::EntityVdfsAdapter`）。
            // 无条件注册——技能清单为空也是合法的挂载点。
            if let Some(tool_visitor) = ctx.get(crate::symbio_core::CAPABILITY_VISITOR) {
                let me: Arc<dyn crate::symbio_core::entities::EntityProvider> = self.clone();
                let vdfs_provider = Arc::new(crate::symbio_core::vdfs::EntityVdfsAdapter::new(
                    crate::symbio_core::entities::ENTITY_SKILL,
                    me,
                ));
                tool_visitor
                    .register_vdfs_provider(PLUGIN_SKILL, vdfs_provider)
                    .await;
            }
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_SKILL, SkillPlugin::build, dyn Plugin);
