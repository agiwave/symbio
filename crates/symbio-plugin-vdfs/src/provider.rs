//! VDFS 工具链路的封装 provider —— 把「对文件系统的操作」原样交给 [`UnifiedFs`]
//!
//! 大模型工具（`tools/` 下的 `vdfs_*`）不认识能力管理器、不认识虚拟层与物理层的
//! 区分，它们只持有本类型：
//!
//! ```text
//! vdfs_* 工具 ──▶ ToolVdfs（取根 + 透传 workdir）──▶ UnifiedFs ──▶ 虚拟层 / 物理层
//! ```
//!
//! ## 本层只做三件事
//!
//! 1. **取虚拟层根**：从持有的 `Registry` 里取容器注册的组合根
//!    （[`host::root_of`]），与前端链路取到的是**同一个根**；
//! 2. **透传调用级参数**：把请求 ctx 的运行时状态（workdir）翻译成 provider 的
//!    约定键（[`call_params`]），两条链路共用同一份翻译；
//! 3. **Agent 权限路径闸**（[`Self::entry`]）：地址落在**当前 Agent 自己的包**里时，
//!    按它 manifest 声明的 `fs.read` / `fs.write` glob 判（D-153）。
//!
//! 第 3 条是九个工具**共用**的执行面：判据放在这一个入口上，而不是每个工具各抄
//! 一遍——漏一个工具就是「列得出、读不到」的同形事故（本模块文档里记着那次）。
//!
//! 地址规则、两半分流、路径回填、根守卫**全部在 [`UnifiedFs`] 一处**，
//! 本文件不再重复实现——这也是它最需要的收敛：不在这里做的
//! 「裸地址补 `local/` 前缀 → 再拆挂载名 → 按名取 provider」三步翻译，
//! 现在只需要把地址原样交给门面。
//!
//! `host::root_of`

use super::fs::{UnifiedFs, VDFS_ADDR_ROOT};
use super::host::call_params;
use super::protocol::{VdfsEditResponse, VdfsSearchResult};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use symbio_plugin_api::vdfs_context;
use symbio_plugin_api::{
    DynVdfsProvider, VdfsContent, VdfsContext, VdfsError, VdfsItem, VdfsNode, VdfsRequest,
    VdfsResult, VdfsWriteResponse, VDFS_PARAM_WORKDIR,
};
use symbio_plugin_api::{PluginInvokeRequest, PluginInvokeRequestExt, Registry};

/// 把路径归一成可做前缀比较的键（大小写、分隔符、Windows `\\?\` 前缀）。
fn audit_path_key(p: &Path) -> String {
    let mut s = p.to_string_lossy().replace('\\', "/");
    if let Some(rest) = s.strip_prefix("//?/").map(str::to_string) {
        s = rest;
    }
    if cfg!(windows) {
        s.make_ascii_lowercase();
    }
    s
}

/// 目标是否落在工作目录之外。
///
/// 判定先用**物理层那一条解析规则**（[`super::physical::join_target`]）把地址变成真实
/// 路径，再比前缀——不在这里重算一遍「什么叫绝对」：首部 `/` 是绝对锚点，Windows 下
/// 它会带着工作目录的盘符落到盘根（`Path::is_absolute()` 对它返回假，照它判就等于漏判）。
fn outside_workdir(workdir: &str, addr: &str) -> bool {
    let base = Path::new(workdir);
    let target = super::physical::join_target(base, addr);
    !PathBuf::from(audit_path_key(&target)).starts_with(PathBuf::from(audit_path_key(base)))
}

/// 工作区外**变更**的审计点：记，不拦。
///
/// `workspace_only` 的缺省仍是放开——改成「只允许工作区」会改变地址规则里
/// 文档承诺的绝对路径语义，而当前配置面没有白名单根的入口；在拿到真实使用分布
/// 之前，用审计让这类动作可追溯，而不是替用户做决定（与 D-101 同一取舍）。
/// 只记**变更**（写 / 编辑 / 删除 / 建目录），不记读取：审计要回答「改了什么」，
/// 不是全量访问日志。
fn audit_outside_workdir(
    ctx: &Arc<dyn PluginInvokeRequest>,
    vctx: &VdfsContext,
    op: &str,
    addr: &str,
) {
    let Some(workdir) = vctx.param_str(VDFS_PARAM_WORKDIR) else {
        return;
    };
    let Ok(addr) = super::fs::normalize_addr(addr) else {
        return;
    };
    if outside_workdir(workdir, &addr) {
        let session = ctx
            .get(symbio_plugin_api::SESSION_ID)
            .unwrap_or_else(|| "-".to_string());
        symbio_plugin_api::plugin_warn!(
            "vdfs",
            "LLM 工作区外变更：session={session} op={op} path={addr} workdir={workdir}"
        );
    }
}

/// VDFS 工具链路的封装 provider。
///
/// 工具构造时持有 `Arc<Self>`；执行时它把操作交给统一文件系统，工具因此不必
/// 感知能力管理器与目录拓扑。
pub struct ToolVdfs {
    visitor: Arc<dyn Registry>,
}

impl ToolVdfs {
    /// 持有能力管理器构造（工具注册广播的 ctx 中取到的即是它）
    pub fn new(visitor: Arc<dyn Registry>) -> Self {
        Self { visitor }
    }

    /// **路径闸**（D-153）：地址落在当前 Agent 自己的包里时，按它的 glob 判。
    ///
    /// 判据三句话：
    /// - 没有选中 Agent（宿主态）⇒ 恒过——没有 manifest 可声明；
    /// - 地址不在自己包里（别人的包 / 用户工作目录）⇒ 恒过——不归本声明管；
    /// - 在自己包里 ⇒ 按 `fs.read` / `fs.write` 判，**空清单即拒**。
    ///
    /// `..` 不在这里单独判：`normalize_addr` 已全局拒穿（地址规则的第一道），
    /// manifest 里的 glob 含 `..` 在装载期就拒了（`manifest::validate`）。
    fn ensure_allowed(
        ctx: &Arc<dyn PluginInvokeRequest>,
        path: &str,
        write: bool,
    ) -> VdfsResult<()> {
        let Some(scope) = symbio_plugin_api::agent_perms_of(ctx.as_ref()) else {
            return Ok(());
        };
        let Ok(addr) = super::fs::normalize_addr(path) else {
            // 地址本身不合法：交给下游出一致的错，不在这里抢先报第二种错。
            return Ok(());
        };
        if scope.path_allowed_addr(VDFS_ADDR_ROOT, &addr, write) {
            return Ok(());
        }
        Err(VdfsError::Forbidden(format!(
            "智能体 `{}` 的 permissions.fs.{} 未覆盖 `{addr}`（glob 相对 Agent 目录根，空清单 = 无授权）",
            scope.id,
            if write { "write" } else { "read" },
        )))
    }

    /// 本次调用的统一文件系统（虚拟层根来自容器注册，物理层是磁盘）
    async fn entry(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        path: &str,
        write: bool,
    ) -> VdfsResult<(DynVdfsProvider, VdfsContext)> {
        Self::ensure_allowed(ctx, path, write)?;
        let root = super::host::root_of(&self.visitor).await;
        let fs: DynVdfsProvider = Arc::new(UnifiedFs::new(root));
        Ok((fs, vdfs_context(ctx).with_params(call_params(ctx))))
    }

    /// 列出目录的直接子节点（**条目** = 地址 + 节点）
    pub async fn list(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        path: &str,
    ) -> VdfsResult<Vec<VdfsItem>> {
        let (fs, vctx) = self.entry(ctx, path, false).await?;
        fs.dispatch(&vctx, path, VdfsRequest::List {})
            .await?
            .into_list()
            .ok_or_else(|| VdfsError::internal("响应类型不匹配"))
    }

    /// 读取节点元数据
    pub async fn stat(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        path: &str,
    ) -> VdfsResult<VdfsNode> {
        let (fs, vctx) = self.entry(ctx, path, false).await?;
        fs.dispatch(&vctx, path, VdfsRequest::Stat)
            .await?
            .into_stat()
            .ok_or_else(|| VdfsError::internal("响应类型不匹配"))
    }

    /// 读取内容（`r` 位）
    pub async fn read(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        path: &str,
    ) -> VdfsResult<VdfsContent> {
        let (fs, vctx) = self.entry(ctx, path, false).await?;
        fs.dispatch(&vctx, path, VdfsRequest::Read)
            .await?
            .into_read()
            .ok_or_else(|| VdfsError::internal("响应类型不匹配"))
    }

    /// 写入内容（`w` 位）
    pub async fn write(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let (fs, vctx) = self.entry(ctx, path, true).await?;
        audit_outside_workdir(ctx, &vctx, "write", path);
        fs.dispatch(
            &vctx,
            path,
            VdfsRequest::Write {
                content: content.clone(),
            },
        )
        .await?
        .into_write()
        .ok_or_else(|| VdfsError::internal("响应类型不匹配"))
    }

    /// 删除节点
    pub async fn delete(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        path: &str,
        recursive: bool,
    ) -> VdfsResult<()> {
        let (fs, vctx) = self.entry(ctx, path, true).await?;
        audit_outside_workdir(ctx, &vctx, "delete", path);
        fs.dispatch(&vctx, path, VdfsRequest::Delete { recursive })
            .await?
            .into_unit()
            .ok_or_else(|| VdfsError::internal("响应类型不匹配"))
    }

    /// 新建目录
    pub async fn mkdir(&self, ctx: &Arc<dyn PluginInvokeRequest>, path: &str) -> VdfsResult<()> {
        let (fs, vctx) = self.entry(ctx, path, true).await?;
        audit_outside_workdir(ctx, &vctx, "mkdir", path);
        fs.dispatch(&vctx, path, VdfsRequest::Mkdir)
            .await?
            .into_unit()
            .ok_or_else(|| VdfsError::internal("响应类型不匹配"))
    }

    /// 内容编辑——**组合操作**：read → 精确替换 → write，
    /// 逻辑只在访问层一份（[`super::host::edit_via`]），任何一层只出原子操作
    pub async fn edit(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        path: &str,
        old_string: &str,
        new_string: &str,
    ) -> VdfsResult<VdfsEditResponse> {
        let (fs, vctx) = self.entry(ctx, path, true).await?;
        audit_outside_workdir(ctx, &vctx, "edit", path);
        super::host::edit_via(&fs, &vctx, path, old_string, new_string).await
    }

    /// 文件名 Glob 搜索（`path` 为可选搜索基目录）——**组合操作**：
    /// 递归 `list` + 模式过滤，逻辑只在访问层一份（[`super::host::search_via`]）
    pub async fn search(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        path: &str,
        pattern: &str,
    ) -> VdfsResult<VdfsSearchResult> {
        let (fs, vctx) = self.entry(ctx, path, false).await?;
        super::host::search_via(&fs, &vctx, path, pattern).await
    }
}

#[cfg(test)]
#[path = "provider.test.rs"]
mod tests;
