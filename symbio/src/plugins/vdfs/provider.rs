//! VDFS 工具链路的封装 provider —— 把「对文件系统的操作」原样交给 [`UnifiedFs`]
//!
//! 大模型工具（`tools/` 下的 `vdfs_*`）不认识能力管理器、不认识虚拟层与物理层的
//! 区分，它们只持有本类型：
//!
//! ```text
//! vdfs_* 工具 ──▶ ToolVdfs（取根 + 透传 workdir）──▶ UnifiedFs ──▶ 虚拟层 / 物理层
//! ```
//!
//! ## 本层只做两件事
//!
//! 1. **取虚拟层根**：从持有的 `Registry` 里取容器注册的组合根
//!    （[`host::root_of`]），与前端链路取到的是**同一个根**；
//! 2. **透传调用级参数**：把请求 ctx 的运行时状态（workdir）翻译成 provider 的
//!    约定键（[`call_params`]），两条链路共用同一份翻译。
//!
//! 地址规则、两半分流、路径回填、根守卫**全部在 [`UnifiedFs`] 一处**，
//! 本文件不再重复实现——这也是它最需要的收敛：不在这里做的
//! 「裸地址补 `local/` 前缀 → 再拆挂载名 → 按名取 provider」三步翻译，
//! 现在只需要把地址原样交给门面。
//!
//! `host::root_of`

use super::fs::UnifiedFs;
use super::host::call_params;
use super::protocol::{VdfsEditResponse, VdfsSearchResult};
use crate::symbio_core::vdfs_context;
use crate::symbio_core::{
    DynVdfsProvider, VdfsContent, VdfsContext, VdfsError, VdfsItem, VdfsNode, VdfsRequest,
    VdfsResult, VdfsWriteResponse, VDFS_PARAM_WORKDIR,
};
use crate::symbio_core::{PluginInvokeRequest, PluginInvokeRequestExt, Registry};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

/// 目标是否落在工作目录之外（只有**绝对地址**才可能；相对地址恒在工作目录内）。
fn outside_workdir(workdir: &str, addr: &str) -> bool {
    let target = PathBuf::from(addr);
    target.is_absolute()
        && !PathBuf::from(audit_path_key(&target))
            .starts_with(PathBuf::from(audit_path_key(Path::new(workdir))))
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
            .get(crate::symbio_core::SESSION_ID)
            .unwrap_or_else(|| "-".to_string());
        crate::plugin_warn!(
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

    /// 本次调用的统一文件系统（虚拟层根来自容器注册，物理层是磁盘）
    async fn fs(&self, ctx: &Arc<dyn PluginInvokeRequest>) -> (DynVdfsProvider, VdfsContext) {
        let root = super::host::root_of(&self.visitor).await;
        let fs: DynVdfsProvider = Arc::new(UnifiedFs::new(root));
        (fs, vdfs_context(ctx).with_params(call_params(ctx)))
    }

    /// 列出目录的直接子节点（**条目** = 地址 + 节点）
    pub async fn list(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        path: &str,
    ) -> VdfsResult<Vec<VdfsItem>> {
        let (fs, vctx) = self.fs(ctx).await;
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
        let (fs, vctx) = self.fs(ctx).await;
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
        let (fs, vctx) = self.fs(ctx).await;
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
        let (fs, vctx) = self.fs(ctx).await;
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
        let (fs, vctx) = self.fs(ctx).await;
        audit_outside_workdir(ctx, &vctx, "delete", path);
        fs.dispatch(&vctx, path, VdfsRequest::Delete { recursive })
            .await?
            .into_unit()
            .ok_or_else(|| VdfsError::internal("响应类型不匹配"))
    }

    /// 新建目录
    pub async fn mkdir(&self, ctx: &Arc<dyn PluginInvokeRequest>, path: &str) -> VdfsResult<()> {
        let (fs, vctx) = self.fs(ctx).await;
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
        let (fs, vctx) = self.fs(ctx).await;
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
        let (fs, vctx) = self.fs(ctx).await;
        super::host::search_via(&fs, &vctx, path, pattern).await
    }
}

#[cfg(test)]
#[path = "provider.test.rs"]
mod tests;
