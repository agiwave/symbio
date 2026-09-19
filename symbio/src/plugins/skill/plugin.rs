use super::skill_response::SkillResponse;
use crate::plugins::skill::loader::{load_skills_from_dirs_with_budget, LoadBudget};
use crate::plugins::skill::skill_tool::SkillExecuteTool;
use crate::plugins::skill::types::{Skill, SkillConfig};
use crate::symbio_core::{
    dir_from_ctx, HomedirRegistry, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin,
    PluginDir, PluginError, PluginMeta, PluginPayload, PLUGIN_SKILL, TRAVERSE_AVAILABLE_TOOLS,
};
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct SkillPlugin {
    config: Arc<RwLock<SkillConfig>>,
    /// 本插件自己的目录（构造时由父插件经 `PLUGIN_DIR` 告知）
    dir: PluginDir,
}

impl SkillPlugin {
    /// 把 skill_dirs 里的 `{HOMEDIR}` 占位符解析为当前系统目录
    fn resolve_skill_dirs_template(dirs: &mut [String]) {
        // `{HOMEDIR}` 是**用户配置里的占位符**（配置文件可手改），其语义就是
        // 「系统目录」，由 home 插件持有；这里只做替换，不代表本插件知道自己落在哪。
        let homedir = HomedirRegistry::get()
            .join("skills")
            .to_string_lossy()
            .to_string();
        for d in dirs.iter_mut() {
            if d.contains("{HOMEDIR}/skills") {
                *d = d.replace("{HOMEDIR}/skills", &homedir);
            } else if d == "{HOMEDIR}/skills" {
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
                // 2. 系统级别：`<本插件目录>`（symbio 系统级）
                // 3. 第三方工具兼容：`.qwen/skills`、`.sixth/skills`、`.qoder/skills`
                skill_dirs: vec![".symbio/skills".to_string(), "{HOMEDIR}/skills".to_string()],
                // 预算字段使用 SkillConfig::default() 的值
                ..SkillConfig::default()
            });

        // 解析 {HOMEDIR} 占位符
        Self::resolve_skill_dirs_template(&mut config.skill_dirs);

        let dir = dir_from_ctx(&*ctx, PLUGIN_SKILL);
        Arc::new(SkillPlugin::new(config, dir)) as Arc<dyn Plugin>
    }

    pub fn new(config: SkillConfig, dir: PluginDir) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            dir,
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
        // **本插件自己的目录**是第一来源（其后才是配置里的 `skill_dirs`）。
        // 于是子 Agent 的 skill 实例自包含——技能就放在 `<agent dir>/skill/` 下，
        // 不需要任何全局配置，复制整个目录即复制这个 Agent 的全部技能。
        let mut dirs: Vec<String> = Vec::with_capacity(config.skill_dirs.len() + 1);
        dirs.push(self.dir.dir().to_string_lossy().to_string());
        dirs.extend(config.skill_dirs.iter().cloned());
        let mut skills = load_skills_from_dirs_with_budget(&dirs, workdir_path, budget).await?;

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

// ==================== VDFS 挂载点（`<根>/skill`） ====================
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
    VdfsNode, VdfsProvider, VdfsResult, VdfsWriteResponse, VDFS_ACTION_EXPORT, VDFS_EXT_FORM,
    VDFS_EXT_ZIP, VDFS_NEW_SOURCE_FILE, VDFS_STATUS_ACTIVE,
};

const LABEL: &str = "技能";

/// 技能主文件（Markdown，**必须按纯文本落盘**）
const MANIFEST: &str = "SKILL.md";

impl SkillPlugin {
    /// 本插件的磁盘底座（根 = 自己的目录）
    fn store(&self) -> DirVdfs {
        store(&self.dir)
    }
}

/// 磁盘底座（每次现取，跟随 homedir 切换）
///
/// 根 = **本插件自己的目录**（构造时由父插件经 `PLUGIN_DIR` 告知）——
/// 这里不按插件名反推落位，插件不知道、也不该知道自己被放在哪。
fn store(dir: &PluginDir) -> DirVdfs {
    DirVdfs::at(dir.dir(), PLUGIN_SKILL, MANIFEST).with_label(LABEL)
}

/// 路径末段 → 条目 id（去掉 `.skill` 呈现扩展名）
fn id_of(path: &str) -> String {
    crate::providers::vdfs_service::entry::id_of(path, PLUGIN_SKILL)
}

/// 目标地址 → 条目 id（`write` 与测试共用的**唯一**判据）。
///
/// 末段非空 ⇒ 名字由使用方给；地址为空 ⇒ **使用方没给名字**（写挂载点目录自身，
/// 即「点新建 → 在详情页填好 → 保存」），id 由本插件生成。目录自身没有可覆盖的
/// 目标，所以必须带 `create` 意图（见 [`VdfsProvider::write`]）。
fn resolve_id(path: &str, create: bool) -> VdfsResult<String> {
    if !path.trim_matches('/').is_empty() {
        return Ok(id_of(path));
    }
    if !create {
        return Err(VdfsError::invalid(format!(
            "写{LABEL}挂载根需要 create 意图：目录自身没有可覆盖的目标"
        )));
    }
    Ok(crate::providers::vdfs_service::entry::auto_id(PLUGIN_SKILL))
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
/// 详情定义（JSON 形态）——**唯一出处**：节点 `schema` 与新建类型 `schema` 都读它，
/// 因此「点新建」的草稿表单与「选中一项」的详情表单是同一张。
fn detail_definition() -> serde_json::Value {
    serde_json::to_value(super::detail::skill_detail_definition())
        .unwrap_or(serde_json::Value::Null)
}

fn node_of(id: &str, raw: Option<&str>) -> VdfsNode {
    let mut n = VdfsNode::file(id, id, VdfsAccess::READ_WRITE);
    n.kind = PLUGIN_SKILL.to_string();
    n.ext = Some(VDFS_EXT_FORM.to_string());
    n.schema = Some(detail_definition());
    n.status = VDFS_STATUS_ACTIVE.to_string();
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

    /// 根下可新建两类：表单新建 + 整包导入（zip）
    ///
    /// `ext = skill` 是**呈现扩展名**（`id_of` 按它剥地址后缀），落成后的节点
    /// `ext = form`——两者不同，故显式声明 `node_ext` 与详情定义（草稿详情页据此
    /// 渲染出与落成后同一张表单）。
    fn root_new_types(&self) -> Vec<VdfsNewType> {
        vec![
            VdfsNewType::new(PLUGIN_SKILL, LABEL)
                .with_description(format!("新建{LABEL}（在详情页里填好，保存时一次写入）"))
                .with_node_ext(VDFS_EXT_FORM)
                .with_schema(detail_definition()),
            VdfsNewType::new(VDFS_EXT_ZIP, format!("{LABEL}包"))
                .with_description(format!("导入{LABEL}整包（.zip）——整目录覆盖同名条目"))
                .with_source(VDFS_NEW_SOURCE_FILE),
        ]
    }

    async fn list(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        if !path.is_empty() {
            return Err(VdfsError::not_found(format!(
                "{LABEL}是叶子资源，没有子项：{path}"
            )));
        }
        Ok(self
            .store()
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
        let e = self.store().entry(&id_of(path)).await?;
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
        let text = self.store().read_text(&id_of(path)).await?;
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
        let s = self.store();
        // 二进制写入 = 整包导入（导入不是第二条协议，它就是「新建」的一种内容来源）。
        // 导入的**名字来自目标地址末段**（使用方由文件名推导），所以必须有名字：
        // 「无名字导入」无从命名，直接拒绝。
        if content.binary {
            if path.trim_matches('/').is_empty() {
                return Err(VdfsError::invalid(format!(
                    "{LABEL}整包导入需要目标名（地址末段）：{path}"
                )));
            }
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
        // 无名字（写在挂载点目录自身）→ 「新建一项，名字由本插件生成」。
        // 目录自身没有可覆盖的目标，因此必须有 create 意图（见 `VdfsProvider::write`）。
        let id = resolve_id(path, content.create)?;
        let text = content.as_text().unwrap_or_default();
        // `create` 只管「不存在时怎么办」，**不改变内容的处理方式**：草稿详情页
        // 填好的字段必须原样落盘。唯一例外是**内容为空**——「先建一个，随后再填」
        // 是合法形态，此时落一份最小内容。
        let manifest = if content.create && text.trim().is_empty() {
            new_manifest(&id)
        } else {
            serde_json::from_str::<serde_json::Value>(text)
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
        let s = self.store();
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
        if action != VDFS_ACTION_EXPORT {
            return Err(VdfsError::NotImplemented);
        }
        if path.is_empty() {
            return Err(VdfsError::invalid(format!(
                "「导出」只对{LABEL}条目可用：{path}"
            )));
        }
        let id = id_of(path);
        let pack = self.store().export_pack(&id).await?;
        let data = serde_json::to_value(&pack)
            .map_err(|e| VdfsError::internal(format!("导出结果序列化失败: {e}")))?;
        Ok(VdfsActionResult {
            action: VDFS_ACTION_EXPORT.to_string(),
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

            // VDFS 挂载点：本插件自身就是 provider（`<根>/skill`）——
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
#[path = "plugin.test.rs"]
mod tests;
