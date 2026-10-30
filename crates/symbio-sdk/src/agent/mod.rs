//! Agent 包的**权限声明**与它的执行方（`DE-011` §5.3 / D-102 / D-140 / D-153）。
//!
//! ## 这个模块为什么在 core
//!
//! 写侧只有一个——`crates/symbio-plugin-agent`（它扫描 Agent 目录、读 manifest，是唯一能把
//! `agent_id` 解析成一份声明的地方）；读侧是**一堆互不相认的宿主工具**
//! （`web` 的三个出站工具、`vdfs` 的九个路径工具）。插件之间互相不可见
//! （判据 `plugin-entry-audit` E-009），所以「声明长什么样、怎么判」必须作为
//! 共享设施住 core，与能力收集期错误通道（`capability::error`）同一条归属规则。
//!
//! ## 语义：grant，不是 ceiling
//!
//! 四个字段（`network` / `fs.read` / `fs.write` / `limits`）是**授权清单**：
//! 列了才能用，没列就是没有。空清单 = 一律拒（fail-closed），与授权表
//! （`authz` 域的授权表）同一条纪律——「不声明就有」的边界不是边界，
//! 那正是 D-102 判过的坏形状。完整裁决（含被否候选）见
//! [D-153](../../../../docs/003-design/011-Agent目录规范.md#d-153-权限四字段的执行面同批落地空清单即拒glob-只管自己的包 "DE-011-Agent目录规范")。
//!
//! ## 执行面与边界（哪些地方真的在守）
//!
//! | 字段 | 执行方 | 判据 |
//! |---|---|---|
//! | `network.domains` | `web` 三工具出站前 | [`AgentPermissions::host_allowed`] |
//! | `fs.read` / `fs.write` | `vdfs` 路径入口（LLM 链路） | [`AgentPermissions::path_allowed`] |
//! | `limits` | 装载期（能力收集那一趟） | [`AgentPermissions::tools_ok`] / [`AgentPermissions::prompt_bytes_ok`] |
//!
//! **不在守的范围**（写在这里，省得下一个人以为漏了）：shell 的整条命令是**不透明**
//! 的，按参数字符串做 glob 判定是安全表演（命令可以 `cat`、可以 `python -c`、可以
//! 软链接）；MCP 服务器自己发的请求同样出不了一层；模型 API 往返不是工具出站，
//! 不该被 Agent 声明闸住（否则没有任何 Agent 能说话）。这三类的既有控制
//! （命令白名单 + 审批闸 / MCP 授权 / 模型选择）各归其位。
//!
//! ## glob 的管辖范围：**自己的包**
//!
//! `fs` 的 glob 相对 **Agent 目录根**判定（规范 §4.3），管的是 Agent 访问
//! **它自己那个目录**（`<根>/agent/<id>/…`，含它子树里的 `memory/AGENTS.md`）。
//! 用户工作目录里的文件不归它管——那是宿主策略（工作区闸 + 审批）的地界；
//! **别的** Agent 的包也不归它管（没有谁的 manifest 能描述别人的树）。

use crate::keys::SymbioKey;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use symbio_kernel::plugin::{PluginInvokeRequest, PluginInvokeRequestExt};

// ═══════════════════════════════════════════════════════════════════════════
// 声明形状（线格式 = manifest.yaml 的 `permissions`）
// ═══════════════════════════════════════════════════════════════════════════

/// Agent 包的权限声明（manifest `permissions` 四字段）。
///
/// 线格式由规范 §5.3 定；Rust 侧曾叫 `declared_permissions`（D-102：当时没有
/// 执行方），四字段同批落地后改回 [`Self`] 本名（D-140 的两态之名）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentPermissions {
    /// 宿主 API 名清单——**仍然只是声明**（四字段之外，没有执行方）。
    ///
    /// 不接判据的理由不是懒：宿主侧没有一张可枚举的「host API」注册表可对照，
    /// 此时接一个恒空 / 恒真的判定就是死旋钮（DE-016 批的形态）。字段留在形状里
    /// 是为了**线格式兼容**——包作者今天写着它，不能被静默吞掉。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub host_apis: Vec<String>,
    /// 网络权限：出站域名白名单（`web` 三工具的全部出站主机）
    #[serde(skip_serializing_if = "NetworkPermissions::is_empty")]
    pub network: NetworkPermissions,
    /// 文件系统权限：相对 Agent 目录根的 glob（读 / 写分开）
    #[serde(skip_serializing_if = "FsPermissions::is_empty")]
    pub fs: FsPermissions,
    /// 资源限额（装载期判，超限即拒载）
    #[serde(skip_serializing_if = "ResourceLimits::is_empty")]
    pub limits: ResourceLimits,
}

impl AgentPermissions {
    /// 没有任何**授权**声明（manifest 里整个 `permissions` 块缺席时的形状）。
    ///
    /// `limits` 不参与判定：它恒有缺省值（32 / 65536），是「上限」不是「授权」——
    /// 把缺省值算成「声明过」会让 `permissions:` 块在只写 limits 时也照样输出。
    ///
    /// **空 = 三族授权都没有**（不是「不限制」）——判据见
    /// [`Self::host_allowed`] / [`Self::path_allowed`] 各自对空清单的处理。
    pub fn is_empty(&self) -> bool {
        self.host_apis.is_empty() && self.network.is_empty() && self.fs.is_empty()
    }

    /// 出站主机是否在声明范围内（**空清单 = 拒**）。
    ///
    /// 匹配规则与 `http_allowed_domains` 的宿主白名单同源：`*` 放行全部、
    /// `*.example.com` = 裸域名本身或任一子域（按**标签边界**比，不是字符串后缀——
    /// `evil-example.com` 必须不中）。
    pub fn host_allowed(&self, host: &str) -> bool {
        self.network.allows(host)
    }

    /// Agent 目录内的相对路径是否放行（`write` 区分读 / 写两个清单）。
    ///
    /// `rel` 是**相对 Agent 根**的路径（`memory/AGENTS.md`）；空串 = 根本身。
    pub fn path_allowed(&self, rel: &str, write: bool) -> bool {
        let globs = if write { &self.fs.write } else { &self.fs.read };
        if globs.is_empty() {
            return false;
        }
        globs.iter().any(|g| glob_matches(g, rel))
    }

    /// 装载期判据之一：子智能体贡献的工具数是否超限。
    pub fn tools_ok(&self, count: usize) -> bool {
        count <= self.limits.max_tools
    }

    /// 装载期判据之二：人格文件（`AGENTS.md`）字节数是否超限。
    pub fn prompt_bytes_ok(&self, bytes: usize) -> bool {
        bytes <= self.limits.max_prompt_bytes
    }

    /// 装载期 glob 校验（规范 §5.1）：读 / 写清单里不得有空白、不得含 `..`。
    ///
    /// 判据住在本类型上而不是 manifest 解析器：glob 是**执行面的输入**
    /// （`vdfs` 路径入口照它放行），一个含 `..` 的 glob 会让整份读 / 写清单
    /// 失去意义（`../..` + `**` = 整个磁盘）。装载期拒，而不是运行期逐次
    /// 拒——后者要把同一个判据抄进每一个调用点。
    pub fn check_globs(&self) -> Result<(), String> {
        for (field, globs) in [
            ("permissions.fs.read", &self.fs.read),
            ("permissions.fs.write", &self.fs.write),
        ] {
            for g in globs {
                if !glob_is_valid(g) {
                    return Err(format!(
                        "{field} 的 glob `{g}` 不合法：不得为空白、不得含 `..`（§5.1）"
                    ));
                }
            }
        }
        Ok(())
    }
}

/// 网络权限：出站域名白名单。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkPermissions {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<String>,
}

impl NetworkPermissions {
    /// 域名清单为空（= 没有任何出站授权）。
    pub fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }

    /// 域名匹配（标签边界，大小写不敏感）。
    fn allows(&self, host: &str) -> bool {
        if self.domains.is_empty() {
            return false;
        }
        let host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .trim_end_matches('.')
            .to_ascii_lowercase();
        self.domains.iter().any(|allowed| {
            let allowed = allowed.trim().to_ascii_lowercase();
            if allowed == "*" {
                return true;
            }
            match allowed.strip_prefix("*.") {
                Some(suffix) => host == suffix || host.ends_with(&format!(".{suffix}")),
                None => host == allowed,
            }
        })
    }
}

/// 文件系统权限：相对 Agent 目录根的 glob 清单（读 / 写分开）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FsPermissions {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub read: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub write: Vec<String>,
}

impl FsPermissions {
    /// 读 / 写两个清单都为空（= 对自己的包零授权）。
    pub fn is_empty(&self) -> bool {
        self.read.is_empty() && self.write.is_empty()
    }
}

/// 资源限额（装载期判，**不是运行时截断**）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ResourceLimits {
    /// 本 Agent 贡献的工具数上限（超出 ⇒ 拒绝接入）
    pub max_tools: usize,
    /// 人格文件（`AGENTS.md`）字节数上限（超出 ⇒ 拒绝接入）
    pub max_prompt_bytes: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_tools: 32,
            max_prompt_bytes: 65_536,
        }
    }
}

impl ResourceLimits {
    /// 恒不空：两个字段都有缺省值，"没声明" = 用缺省，不是跳过判定。
    pub fn is_empty(&self) -> bool {
        false
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// glob（manifest 的 `fs.read` / `fs.write` 语义）
// ═══════════════════════════════════════════════════════════════════════════

/// manifest glob 匹配：`*` 不跨段、`**` 跨任意段、`?` 单字符；段按 `/` 分。
///
/// 特例：**空路径**（Agent 根本身）只被 `**` 与 `.` 命中——`.` 是规范里
/// 「相对 Agent 根」的写法（§4.3），不是通配符。
///
/// glob 语义（匹配 + 合法性）只此一份，全在域内：运行期判定走
/// [`AgentPermissions::path_allowed`]，装载期校验走
/// [`AgentPermissions::check_globs`]，不各写一半。
fn glob_matches(pattern: &str, path: &str) -> bool {
    let pat = normalize_glob(pattern);
    let segs: Vec<&str> = if path.is_empty() {
        Vec::new()
    } else {
        path.split('/').collect()
    };
    match_segments(&pat, &segs)
}

/// 模式归一：去空白与首尾 `/`，`.` → 空（根本身）。
fn normalize_glob(pattern: &str) -> Vec<String> {
    let p = pattern.trim().trim_matches('/');
    if p == "." || p.is_empty() {
        return Vec::new();
    }
    p.split('/').map(|s| s.trim().to_string()).collect()
}
fn match_segments(pat: &[String], segs: &[&str]) -> bool {
    match pat.split_first() {
        None => segs.is_empty(),
        Some((head, rest)) => {
            if head == "**" {
                // `**` 吃掉 0..n 段：逐个尝试切点。
                (0..=segs.len()).any(|i| match_segments(rest, &segs[i..]))
            } else if segs.is_empty() {
                false
            } else {
                seg_matches(head, segs[0]) && match_segments(rest, &segs[1..])
            }
        }
    }
}

/// 段内匹配（`*` / `?`，不跨段）。
fn seg_matches(pat: &str, s: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = s.chars().collect();
    seg_rec(&p, &t)
}

fn seg_rec(p: &[char], t: &[char]) -> bool {
    match p.split_first() {
        None => t.is_empty(),
        Some((&'*', rest)) => (0..=t.len()).any(|i| seg_rec(rest, &t[i..])),
        Some((&'?', rest)) => !t.is_empty() && seg_rec(rest, &t[1..]),
        Some((&c, rest)) => !t.is_empty() && t[0] == c && seg_rec(rest, &t[1..]),
    }
}

/// manifest 里的 glob 是否合法（规范 §5.1：不得含 `..`）。
///
/// 装载期判：一个能穿越到目录外的 glob 让整个读 / 写清单失去意义
/// （`../..` + `**` 就是「整个磁盘」）。空模式同样拒——它什么都匹不中，
/// 写在那里只会让人以为read/write 清单里有一条。
///
/// 只经 [`AgentPermissions::check_globs`] 一个入口调用（域内私有：
/// 合法性与匹配同住，消费方经那一个方法拿到判据）。
fn glob_is_valid(pattern: &str) -> bool {
    if pattern.trim().is_empty() || pattern.contains('\0') {
        return false;
    }
    !pattern.split(['/', '\\']).any(|seg| seg.trim() == "..")
}

// ═══════════════════════════════════════════════════════════════════════════
// 运行期作用域（解析后的那一份，住 ctx）
// ═══════════════════════════════════════════════════════════════════════════

/// 已解析的权限作用域：**哪个** Agent、它的**目录在哪**、它**声明了什么**。
///
/// 与 [`AgentPermissions`] 分开是因为两者寿命不同：声明是 manifest 的一部分
/// （可序列化、随包走），作用域是「这一次会话以谁的身份跑」的运行期事实
/// （带目录根，因为文件闸要判物理路径）。
#[derive(Debug, Clone)]
pub struct AgentScope {
    /// Agent id（= 会话 `metadata.agent_id`，也是 `<根>/agent/<id>` 的那一段）
    pub id: String,
    /// Agent 目录根（`{homedir}/agent/<id>`）
    pub root: PathBuf,
    /// 它的声明
    pub permissions: AgentPermissions,
}

impl AgentScope {
    /// 目标是否落在**本 Agent 自己的目录**内；是则给出目录内相对路径。
    ///
    /// 两种形态都认（漏一个就是一条能绕过的旁路）：
    /// 1. 虚拟地址 `<根>/agent/<id>` 或 `<根>/agent/<id>/<rel>`——模型从提示词里
    ///    拿到的可编辑地址；
    /// 2. 物理绝对路径落在 [`Self::root`] 下——模型从 `vdfs_list` 的 `dir`
    ///    属性里看得见真实目录（`agent_dir_info` 把它印在概览里），
    ///    因此这条不是理论上的。
    ///
    /// `addr` 须已归一（`vdfs::fs::normalize_addr`：无 `..`、无空段；首部的 `/` 是
    /// **绝对锚点**，会原样保留——POSIX 下它因此落进形态 2）。
    pub fn own_rel(&self, root_name: &str, addr: &str) -> Option<String> {
        // 形态 1：虚拟地址。root_name 由 vdfs 插件经静态声明给出（core 对根名零知识）。
        let base = format!("{root_name}/agent/{}", self.id);
        if addr == base {
            return Some(String::new());
        }
        if let Some(rel) = addr.strip_prefix(&format!("{base}/")) {
            return Some(rel.trim_matches('/').to_string());
        }
        // 形态 2：物理绝对路径。相对地址恒在工作目录内，与 Agent 目录无关
        // （工作目录是用户的地界，不归 manifest 管）。
        let target = Path::new(addr);
        if !target.is_absolute() {
            return None;
        }
        let key_target = path_key(target);
        let key_root = path_key(&self.root);
        if key_target == key_root {
            return Some(String::new());
        }
        // 段边界：`<root>-evil/x` 不是 `<root>` 的里面——按字符串前缀切会把
        // 兄弟目录当成自己的子树（与域名按标签边界比同一条教训）。
        let rel = key_target.strip_prefix(&key_root)?.strip_prefix('/')?;
        Some(rel.trim_matches('/').to_string())
    }

    /// 路径闸：不在自己目录内 ⇒ 放行（不归本声明管）；在 ⇒ 按清单判。
    pub fn path_allowed_addr(&self, root_name: &str, addr: &str, write: bool) -> bool {
        match self.own_rel(root_name, addr) {
            Some(rel) => self.permissions.path_allowed(&rel, write),
            None => true,
        }
    }

    /// 把本作用域发布进收集期的共享槽（[`AGENT_PERMS`]）。
    ///
    /// **先到先得**：一次收集只应有一个 Agent 被选中（会话级单选），第二个
    /// 发布被忽略并留下日志——静默覆盖会让「以谁的身份跑」取决于插件遍历顺序。
    ///
    /// 自备槽：没有槽就地建一个。收集方的 pre-init（[`agent_perms_init_slot`]）
    /// 负责把槽放进**它的**那条 ctx lineage——容器对每个子插件各 fork 一份，
    /// 插件写在自己那份上，兄弟与收集方都看不见，可变的共享槽因此是必需的
    /// （与能力收集期错误桶同一条路）。发布方的自备只保证「自己这份上有」，
    /// 与 `capability_report_error` 自备错误桶同一条纪律。
    pub fn publish(&self, ctx: &Arc<dyn PluginInvokeRequest>) {
        agent_perms_init_slot(ctx);
        if let Some(slot) = ctx.get(AGENT_PERMS) {
            if slot.set(self.clone()).is_err() {
                crate::plugin_warn!(
                    "agent",
                    "权限作用域重复发布（同一趟收集里两个 Agent？）——保留先到的那一份"
                );
            }
        }
    }
}

/// 路径比较键：分隔符归一 + Windows 大小写不敏感（与 `local/policy` 同口径，
/// 但那是插件内的工具，core 不反向依赖插件）。
fn path_key(p: &Path) -> String {
    let mut s = p.to_string_lossy().replace('\\', "/");
    if let Some(rest) = s.strip_prefix("//?/") {
        s = rest.to_string();
    }
    if cfg!(windows) {
        s.make_ascii_lowercase();
    }
    s
}

// ═══════════════════════════════════════════════════════════════════════════
// ctx 通道（收集期槽 → 轮 ctx → 工具 ctx）
// ═══════════════════════════════════════════════════════════════════════════

/// Agent 权限作用域 Key —— **键与值同住**（D-095）。
///
/// 值类型带 [`AgentScope`]（本域类型），故键住本域。键名（`agent_perms`）是 ctx
/// 上的内部协议面。
///
/// ## 为什么值是 `Arc<OnceLock<..>>` 而不是 `AgentScope`
///
/// 解析方（agent 插件）与消费方（web / vdfs 工具）在**同一次 traverse 里根本
/// 见不到彼此的 ctx**：容器对每个子插件各 fork 一份（`Composite::traverse`），
/// 插件写在自己那份上，兄弟与收集方都看不见。可变的共享槽因此是必需的——
/// 与能力收集期错误桶（`CAPABILITY_ERRORS`）同一条路，差别只在那边是
/// `Arc<Mutex<Vec<_>>>`（可多次追加），这边是 `Arc<OnceLock<_>>`（一次解析）。
///
/// 收集方（session 的 `collect_capabilities`）在 traverse **之前**建槽、
/// **之后**把同一个 `Arc` 挂回轮 ctx——于是工具执行期从 ctx 读到的就是它。
pub struct AgentPermsKey;
impl SymbioKey for AgentPermsKey {
    type Value = Arc<OnceLock<AgentScope>>;
    /// 进程内专用：`Arc<OnceLock<..>>` 无字符串 / JSON 形态
    const WIRE: bool = false;
    fn name(&self) -> &'static str {
        "agent_perms"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, _v: &Self::Value) -> String {
        "agent_perms".to_string()
    }
}
pub const AGENT_PERMS: AgentPermsKey = AgentPermsKey;

/// 收集方在 traverse 前建槽（重复调用无副作用——已有槽就复用）。
///
/// 发布方（[`AgentScope::publish`]）也自备一遍：两个入口共用一个幂等建槽，
/// 与 `capability` 错误桶（`capability_init_error_bucket` /
/// `capability_report_error`）同一条形态。
pub fn agent_perms_init_slot(ctx: &Arc<dyn PluginInvokeRequest>) {
    if ctx.get(AGENT_PERMS).is_none() {
        ctx.set(AGENT_PERMS, Arc::new(OnceLock::new()));
    }
}

/// 消费方（工具）读作用域。**没有槽 / 槽是空的 = 本会话没有选中 Agent**
/// ⇒ 宿主态（`agent:main`，无 manifest 可声明），不做 Agent 级过滤。
pub fn agent_perms_of(ctx: &dyn PluginInvokeRequest) -> Option<AgentScope> {
    ctx.get(AGENT_PERMS).and_then(|slot| slot.get().cloned())
}

#[cfg(test)]
mod tests;
