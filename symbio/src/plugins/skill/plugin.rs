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

// ==================== VDFS 挂载点（`.vdfs/skill`） ====================
//
// 本插件**直接实现 `VdfsProvider`**：VDFS 是唯一协议、唯一地址空间，列 / 读 /
// 写 / 删 / 动作的语义都在这里表达。
//
// 存储走 `providers::vdfs_service::DirVdfs`（一个技能 = 一个目录，主文件
// `SKILL.md`）：条目寻址、原子写、mtime、整包 zip、变更广播都在集中实现里，
// 本模块只剩 **skill 特有的两件事**——SKILL.md 的摘要解析（frontmatter →
// 标题/摘要/config）与写前的表单校验。

use crate::providers::vdfs_service::DirVdfs;
use crate::symbio_core::vdfs::{from_plugin_error, unwatch_changes, watch_changes};
use crate::symbio_core::vdfs_provider::{
    VdfsAccess, VdfsActionResult, VdfsChangeSink, VdfsContent, VdfsContext, VdfsError, VdfsNewType,
    VdfsNode, VdfsProvider, VdfsResult, VdfsWriteResponse, VFDS_ACTION_EXPORT, VFDS_EXT_FORM,
    VFDS_EXT_ZIP, VFDS_NEW_SOURCE_FILE,
};

const LABEL: &str = "技能";

/// 技能主文件（Markdown，**必须按纯文本落盘**）
const MANIFEST: &str = "SKILL.md";

/// 磁盘底座（每次现取，跟随 homedir 切换）
fn store() -> DirVdfs {
    DirVdfs::for_category(PLUGIN_SKILL, MANIFEST).with_label(LABEL)
}

/// 路径末段 → 条目 id（去掉 `.skill` 呈现扩展名）
fn id_of(path: &str) -> String {
    crate::providers::vdfs_service::entry::id_of(path, PLUGIN_SKILL)
}

/// 导入的**建议名**：末段再去掉 `.zip`（新建地址是 `<name>.zip`）
fn import_name_of(path: &str) -> String {
    crate::providers::vdfs_service::entry::pack_name_of(path, PLUGIN_SKILL)
}

/// 主文件原文 → VDFS 节点（`ext = form` + 详情定义随节点 `schema` 下发）
///
/// 摘要优先 YAML frontmatter（name / description），无 frontmatter 时回落到旧的
/// 标题 / Description 行解析；主文件缺失（`raw = None`）时降级为以 id 呈现的
/// 占位条目——列表不得因单个坏条目而少一项或多失败。
fn node_of(id: &str, raw: Option<&str>) -> VdfsNode {
    let mut n = VdfsNode::file(id, id, VdfsAccess::READ_WRITE);
    n.kind = PLUGIN_SKILL.to_string();
    n.ext = Some(VFDS_EXT_FORM.to_string());
    n.schema = serde_json::to_value(super::detail::skill_detail_definition()).ok();
    n.status = "active".to_string();
    let Some(text) = raw else { return n };

    // frontmatter 路径：名称 / 摘要
    if let Some((yaml, _body)) = super::detail::parse_skill_md(text) {
        if let Some(name) = yaml.get("name").and_then(|v| v.as_str()) {
            n.title = name.to_string();
        }
        if let Some(desc) = yaml.get("description").and_then(|v| v.as_str()) {
            n.description = Some(desc.to_string());
        }
        return n;
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
        n.title = first_line.to_string();
    }
    let mut summary = cleaned
        .lines()
        .find(|l| l.trim().starts_with("**Description**") || l.trim().starts_with("Description"))
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
        n.description = Some(summary);
    }
    n
}

/// 表单 manifest → SKILL.md 全文（写盘前的校验/规范化）。
///
/// 强制 BUG-SR6（名称 == 目录 id）与 BUG-SR7（description ≥ 10 字符），
/// 错误在保存时即给出（而非下次加载时）。zip 上传路径不经过本函数。
///
/// 返回**纯文本**而不是 JSON 值：SKILL.md 是 Markdown，必须原样落盘——包成
/// JSON 字符串会让落盘内容带上引号与转义的 `\n`，读回来 frontmatter 就解析不
/// 出来了（`DirVdfs::write_json` 因此对本资源禁用）。
fn validate_manifest(id: &str, manifest: &serde_json::Value) -> Result<String, PluginError> {
    super::detail::manifest_to_skill_md(id, manifest)
}

/// `write { create }` 的最小清单。
///
/// `manifest_to_skill_md` 要求 `name` 与目录名（id）一致、`description`
/// 至少 10 字符，故这里给出同名的骨架描述，用户随后在详情里完善。
fn new_manifest(id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "name": id,
        "description": format!("{id}：请填写该技能的用途与使用时机（至少 10 字）"),
    })
}

#[async_trait]
impl VdfsProvider for SkillPlugin {
    fn label(&self) -> Option<&str> {
        Some(LABEL)
    }

    fn description(&self) -> Option<&str> {
        Some("Skill 技能包（每项一份 SKILL.md），是可复用能力片段的唯一来源。")
    }

    fn order(&self) -> i32 {
        4
    }

    fn icon(&self) -> Option<&str> {
        Some(PLUGIN_SKILL)
    }

    /// 根下可新建两类：表单新建（最小 SKILL.md）+ 整包导入（zip）
    fn root_new_types(&self) -> Vec<VdfsNewType> {
        vec![
            VdfsNewType::new(PLUGIN_SKILL, LABEL)
                .with_description(format!("新建{LABEL}（先落一份默认配置，随后在详情里完善）")),
            VdfsNewType::new(VFDS_EXT_ZIP, format!("{LABEL}包"))
                .with_description(format!("导入{LABEL}整包（.zip）——整目录覆盖同名条目"))
                .with_source(VFDS_NEW_SOURCE_FILE),
        ]
    }

    async fn list(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        if !path.is_empty() {
            return Err(VdfsError::not_found(format!(
                "{LABEL}是叶子资源，没有子项：{path}"
            )));
        }
        Ok(store()
            .entries()
            .await?
            .iter()
            .map(|e| node_of(&e.id, e.raw.as_deref()))
            .collect())
    }

    async fn stat(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        if path.is_empty() {
            return Ok(VdfsNode::dir("", LABEL, VdfsAccess::LIST));
        }
        let e = store().entry(&id_of(path)).await?;
        Ok(node_of(&e.id, e.raw.as_deref()))
    }

    /// 详情读的是**表单能填的形状**（config JSON），不是 Markdown 原文——
    /// 原文由 `read(<id>/SKILL.md)` 这一真实地址给出（目录型天然支持）。
    async fn read(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        if path.is_empty() {
            return Err(VdfsError::invalid(format!(
                "该路径是目录，不可读取内容：{path}"
            )));
        }
        let text = store().read_text(&id_of(path)).await?;
        let value = super::detail::skill_md_to_config(&text).unwrap_or_else(
            || serde_json::json!({ "name": id_of(path), "description": "", "content": text }),
        );
        let body = serde_json::to_string_pretty(&value)
            .map_err(|e| VdfsError::internal(format!("配置序列化失败：{e}")))?;
        Ok(VdfsContent::text(path, body).with_mime("application/json"))
    }

    async fn write(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        if path.is_empty() {
            return Err(VdfsError::invalid(format!(
                "{LABEL}整包只能导入到挂载根下：{path}"
            )));
        }
        let s = store();
        // 二进制写入 = 整包导入（导入不是第二条协议，它就是「新建」的一种内容来源）
        if content.binary {
            let bytes = crate::providers::vdfs_service::decode_b64(
                content.b64.as_deref().unwrap_or_default(),
            )
            .map_err(|e| VdfsError::invalid(e.0))?;
            let name = import_name_of(path);
            let created = s.import_pack(&name, &bytes).await?;
            return Ok(VdfsWriteResponse {
                path: name,
                created,
                etag: None,
            });
        }
        let id = id_of(path);
        let manifest = if content.create {
            new_manifest(&id)
        } else {
            serde_json::from_str::<serde_json::Value>(content.as_text().unwrap_or_default())
                .map_err(|e| VdfsError::invalid(format!("manifest 不是合法 JSON：{e}")))?
        };
        // SKILL.md 是 Markdown：走**纯文本**写入，不能被 JSON 序列化
        let normalized = validate_manifest(&id, &manifest).map_err(from_plugin_error)?;
        let created = s.write_text(&id, &normalized).await?;
        Ok(VdfsWriteResponse {
            path: id,
            created,
            etag: None,
        })
    }

    async fn delete(&self, _ctx: &VdfsContext, path: &str, _recursive: bool) -> VdfsResult<()> {
        if path.is_empty() {
            return Err(VdfsError::Forbidden(format!("不可删除挂载点：{path}")));
        }
        let s = store();
        let id = id_of(path);
        // 存在性校验：删除不存在的条目应报 NotFound 而非静默成功
        s.entry(&id).await?;
        s.remove(&id).await
    }

    async fn action(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        action: &str,
        _payload: Option<&serde_json::Value>,
    ) -> VdfsResult<VdfsActionResult> {
        if action != VFDS_ACTION_EXPORT {
            return Err(VdfsError::NotImplemented);
        }
        if path.is_empty() {
            return Err(VdfsError::invalid(format!(
                "「导出」只对{LABEL}条目可用：{path}"
            )));
        }
        let id = id_of(path);
        let pack = store().export_pack(&id).await?;
        let data = serde_json::to_value(&pack)
            .map_err(|e| VdfsError::internal(format!("导出结果序列化失败: {e}")))?;
        Ok(VdfsActionResult {
            action: VFDS_ACTION_EXPORT.to_string(),
            ok: true,
            message: format!("已打包「{}」", pack.filename),
            data: Some(data),
        })
    }

    async fn watch(&self, _ctx: &VdfsContext, path: &str, sink: VdfsChangeSink) -> VdfsResult<()> {
        watch_changes(PLUGIN_SKILL, path, sink).await
    }

    async fn unwatch(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        unwatch_changes(PLUGIN_SKILL, path).await
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

            // VDFS 挂载点：本插件自身就是 provider（`.vdfs/skill`）——
            // 列 / 读 / 写 / 删 / 动作直接由 `impl VdfsProvider for SkillPlugin` 承载。
            // 无条件注册——技能清单为空也是合法的挂载点。
            if let Some(tool_visitor) = ctx.get(crate::symbio_core::CAPABILITY_VISITOR) {
                let vdfs_provider: Arc<dyn VdfsProvider> = self.clone();
                tool_visitor
                    .register_vdfs_provider(PLUGIN_SKILL, vdfs_provider)
                    .await;
            }
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_SKILL, SkillPlugin::build, dyn Plugin);

#[cfg(test)]
mod tests {
    use super::*;

    /// 路径末段才是 id：`.vdfs/skill/<id>.skill` 与裸 `<id>` 同解
    #[test]
    fn id_of_strips_presentation_extension() {
        assert_eq!(id_of("demo"), "demo");
        assert_eq!(id_of("demo.skill"), "demo");
    }

    /// 导入建议名：新建地址是 `<name>.zip`，建议名即去掉 `.zip` 的 `<name>`
    #[test]
    fn import_name_of_strips_zip() {
        assert_eq!(import_name_of("demo.zip"), "demo");
        assert_eq!(import_name_of("demo.skill"), "demo");
    }

    /// 摘要优先 YAML frontmatter：`name` 作标题、`description` 作摘要，
    /// 且详情定义随节点 `schema` 下发（详情页表单预填靠 `read`）
    #[test]
    fn node_prefers_frontmatter() {
        let md = "---\nname: 演示\ndescription: 一个用于演示的技能\n---\n\n正文\n";
        let n = node_of("demo", Some(md));
        assert_eq!(n.name, "demo");
        assert_eq!(n.title, "演示");
        assert_eq!(n.description.as_deref(), Some("一个用于演示的技能"));
        assert_eq!(n.ext.as_deref(), Some(VFDS_EXT_FORM));
        assert!(n.schema.is_some(), "详情定义必须随节点下发");
    }

    /// 无 frontmatter 的旧格式回落：首行标题 + Description 行
    #[test]
    fn node_falls_back_to_legacy_heading() {
        let md = "# 旧技能\n\n**Description** 旧格式描述\n";
        let n = node_of("old", Some(md));
        assert_eq!(n.title, "旧技能");
        assert_eq!(n.description.as_deref(), Some("旧格式描述"));
    }

    /// 主文件缺失（坏条目）降级为 id 占位，不阻断整张列表
    #[test]
    fn node_degrades_to_the_id_when_manifest_unreadable() {
        let n = node_of("broken", None);
        assert_eq!(n.name, "broken");
        assert_eq!(n.title, "broken");
        assert!(n.description.is_none());
    }

    /// 回归：**写进去的必须能被读回来**（写读同源）。
    ///
    /// SKILL.md 是 Markdown，只能走纯文本落盘。若把 manifest 当 JSON 值写入，
    /// 落盘会变成 `"---\nname: ...\n"`（外层引号 + `\n` 被转义成字面两字符），
    /// 而 `parse_skill_md` 以 `strip_prefix("---\n")` 起手 ⇒ 必然失败：
    /// 保存报成功、文件却是坏的，下次加载解析不出 frontmatter。
    #[test]
    fn validate_manifest_produces_parsable_markdown() {
        let manifest = serde_json::json!({
            "id": "demo",
            "name": "demo",
            "description": "一个用于演示的技能描述",
        });
        let md = validate_manifest("demo", &manifest).expect("BUG-SR6 / SR7 均应满足");
        assert!(
            md.starts_with("---\n"),
            "SKILL.md 必须以 frontmatter 起始标记开头（不是引号），实际开头：{:?}",
            &md[..md.len().min(12)]
        );
        // 关键断言：落盘文本经同一份摘要解析能还原出填写的字段
        let n = node_of("demo", Some(&md));
        assert_eq!(n.title, "demo");
        assert_eq!(n.description.as_deref(), Some("一个用于演示的技能描述"));
    }

    /// 新建链路：最小清单同样必须产出合法 SKILL.md（否则一新建就是坏文件）
    #[test]
    fn new_manifest_passes_validation() {
        let m = new_manifest("demo");
        let md = validate_manifest("demo", &m).expect("新建的最小清单必须合法");
        assert!(md.starts_with("---\n"));
        let n = node_of("demo", Some(&md));
        assert_eq!(n.title, "demo");
        assert!(n.description.is_some());
    }

    /// 集中实现接上真实磁盘：写 → 列 → 读 → 删全程往返
    /// （`read` 给出**表单形状**，Markdown 原文走条目内部地址）
    #[tokio::test]
    async fn store_roundtrip_through_the_dir_impl() {
        let tmp = tempfile::tempdir().unwrap();
        let s = DirVdfs::at(tmp.path().join("plugins/skill"), PLUGIN_SKILL, MANIFEST);
        let ctx = VdfsContext::empty();

        let md = validate_manifest("demo", &new_manifest("demo")).unwrap();
        s.write_text("demo", &md).await.unwrap();

        let entries = s.entries().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            node_of(&entries[0].id, entries[0].raw.as_deref()).title,
            "demo"
        );
        assert_eq!(s.read_text("demo").await.unwrap(), md);
        assert!(
            s.list(&ctx, "").await.unwrap()[0].is_dir(),
            "条目内部可下钻"
        );
        assert_eq!(
            s.read(&ctx, "demo/SKILL.md").await.unwrap().as_text(),
            Some(md.as_str()),
            "原文地址读到的就是落盘原文"
        );

        s.remove("demo").await.unwrap();
        assert!(s.entries().await.unwrap().is_empty());
    }
}
