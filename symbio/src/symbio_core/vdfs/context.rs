//! 不透明宿主上下文与调用级参数：VdfsContext / VdfsParams。

use serde_json::Value;
use std::any::Any;
use std::sync::Arc;

use super::error::{VdfsError, VdfsResult};

// ==================== 宿主上下文（不透明） ====================

/// 调用级自定义参数的键值表（**使用方注入 → provider 取用**）。
///
/// 键名是**约定**而非类型：使用方与 provider 通过共享常量对齐（如
/// [`VDFS_PARAM_WORKDIR`](super::VDFS_PARAM_WORKDIR)）。之所以用 JSON 值而非类型化槽位，是为了让机制不依赖
/// 任何具体资源语义——**新增一个约定参数不需要改动接口**。
pub type VdfsParams = serde_json::Map<String, Value>;

/// 不透明宿主上下文：VDFS 不假设宿主形态，宿主把运行时状态放进袋子里，
/// provider 按需 `downcast` 取用。
///
/// 除宿主句柄外还携带一袋**调用级参数**（[`VdfsParams`]）：不透明、无类型约束、
/// 由使用方按约定键名注入，provider 按同一约定取出。资源语义因此不必进入接口。
///
/// ```ignore
/// // 使用方（访问层）
/// let vctx = vdfs_context(&ctx).with_param(VDFS_PARAM_WORKDIR, workdir);
/// // provider
/// let workdir = ctx.param_str(VDFS_PARAM_WORKDIR)?;
/// ```
#[derive(Clone)]
pub struct VdfsContext {
    host: Arc<dyn Any + Send + Sync>,
    params: Arc<VdfsParams>,
    /// **当前父地址**：本 provider 挂载点的绝对地址（由转发方写入）。
    ///
    /// provider 收到的地址一律是自身子树内的相对地址；绝大多数操作只需要相对
    /// 地址。只有少数协议级场合需要全局地址，此时从本字段 + 相对地址拼出。
    /// 顶层为空串（可诊断的降级：绝对地址退化为根相对地址）。
    parent_addr: String,
}

impl VdfsContext {
    /// 由宿主状态构造
    pub fn new<H: Send + Sync + 'static>(host: H) -> Self {
        Self {
            host: Arc::new(host),
            params: Arc::new(VdfsParams::new()),
            parent_addr: String::new(),
        }
    }

    /// 转发方写入当前父地址（同名覆盖；vdfs 核心协议把操作派发给 provider 的
    /// 那一跳调用——见 `plugins/composite/vdfs.rs` 的 `dispatch`）
    pub fn with_parent_addr(mut self, addr: impl Into<String>) -> Self {
        self.parent_addr = addr.into();
        self
    }

    /// 当前父地址（空串 = 未设置，绝对地址随之退化为根相对地址）
    pub fn parent_addr(&self) -> &str {
        &self.parent_addr
    }

    /// 无宿主状态（纯计算 / 测试场景）
    pub fn empty() -> Self {
        Self::new(())
    }

    /// 注入一个调用级参数（同名覆盖）
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        Arc::make_mut(&mut self.params).insert(key.into(), value.into());
        self
    }

    /// 批量注入调用级参数（同名覆盖）
    pub fn with_params(mut self, params: VdfsParams) -> Self {
        let slot = Arc::make_mut(&mut self.params);
        for (k, v) in params {
            slot.insert(k, v);
        }
        self
    }

    /// 取一个参数（原始 JSON）
    pub fn param(&self, key: &str) -> Option<&Value> {
        self.params.get(key)
    }

    /// 取一个字符串参数
    pub fn param_str(&self, key: &str) -> Option<&str> {
        self.param(key)?.as_str()
    }

    /// 取一个参数并反序列化为 `T`（失败 / 缺失均为 `None`）
    pub fn param_as<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        serde_json::from_value(self.param(key)?.clone()).ok()
    }

    /// 全部参数（只读）
    pub fn params(&self) -> &VdfsParams {
        &self.params
    }

    /// 取用宿主状态（类型不匹配返回 `None`）
    pub fn host<H: Send + Sync + 'static>(&self) -> Option<&H> {
        self.host.downcast_ref::<H>()
    }

    /// 取用宿主状态，缺失时报 [`VdfsError::Internal`]
    pub fn require<H: Send + Sync + 'static>(&self) -> VdfsResult<&H> {
        self.host::<H>()
            .ok_or_else(|| VdfsError::internal("宿主上下文类型不匹配（provider 需要的类型未注入）"))
    }
}

impl Default for VdfsContext {
    fn default() -> Self {
        Self::empty()
    }
}

impl std::fmt::Debug for VdfsContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VdfsContext(<opaque>)")
    }
}
