//! 「开跑一轮」端口：跑完一个 Turn 所需的宿主切片。
//!
//! ## 为什么是手柄而不是插件（[D-075](../../../../../docs/003-design/002-架构总览.md#d-075-卸载能力先于目录分离session-切分立三条前置会话记忆与存储留守 "DE-002-架构总览") 前置 P2 第五处）
//!
//! 一轮执行是 session 里**最长命的后台任务**：`start_turn` 把主体 `tokio::spawn` 出去，
//! 任务体一路跑到消费循环收尾。而 provider 侧的 `dispatch` 只有 `&self`——要把句柄移进
//! 后台任务，就只有两条路：升级出一个 `Arc<SessionPlugin>`（代价是插件得持一份自身弱引用），
//! 或者**把这一轮真正读到的东西按字段列出来**。本结构走第二条：任务钉住的就是它用得着
//! 的那六样，provider 注册、路由、工作目录监听与常驻任务册都不在里面。
//!
//! 它比插件窄的地方是可数的——没有配置文件、没有监听器、没有停机位与句柄册。
//!
//! ## 各字段都是谁在用
//!
//! | 字段 | 用途 |
//! |---|---|
//! | `convergence` | 取会话摘要 / 打开会话引擎 / 广播运行态与失败收口（存储、配置、名册都在它身上） |
//! | `active_mgr` | 取或建活跃会话状态；转写发布走名册出口（`ActiveSessionManager::transcript_apply`） |
//! | `config` | 轮首开关与策略快照（classify / compose / progress / 建树档） |
//! | `change_subs` | 轮边界补充抽干的通知表，以及自动命名那一次的资源信号 |
//! | `dir` | 本插件目录（作用域正确性的来源） |
//! | `parent` | 能力收集要遍历的父插件（弱引用，与插件上那一格同源） |
//!
//! 字段全是 `Arc` / `Weak` / 值类型，因此 `Clone` 廉价；构造点唯一（[`super::super::plugin::SessionPlugin::new`]），
//! 与插件共用同一批 Arc——**不是**第二份状态。
//!
//! ## 名字为什么不带 Runner
//!
//! [`super::super::turn_runner::TurnRunner`] 是**一轮的执行引擎**（流循环 + 落格纪律，AUC 见
//! D-050）；本结构负责的是**装配与派发**——
//! 从宿主切片凑出这一轮要读的那几样，把引擎 `tokio::spawn` 出去，之后只剩轮边界的事。
//! 两个名字都叫「跑一轮」会逼读者逐个文件确认自己在哪一层，故本端口取名 Launcher。

use super::super::config::SessionConfig;
use super::super::store::SessionStore;
use super::super::types::Session;
use super::*;
use symbio_core::{PluginDir, VdfsChange, VdfsChangeSubscriptions};
use std::sync::Weak;
use tokio::sync::RwLock;

/// 「开跑一轮」端口（构造与用途见模块文档）。
#[derive(Clone)]
pub(crate) struct TurnLauncher {
    pub(super) convergence: Convergence,
    pub(super) active_mgr: Arc<super::super::active::ActiveSessionManager>,
    pub(super) config: Arc<RwLock<SessionConfig>>,
    pub(super) change_subs: Arc<VdfsChangeSubscriptions>,
    pub(super) dir: PluginDir,
    pub(super) parent: Option<Weak<dyn Plugin>>,
}

impl TurnLauncher {
    pub(crate) fn new(
        convergence: Convergence,
        active_mgr: Arc<super::super::active::ActiveSessionManager>,
        config: Arc<RwLock<SessionConfig>>,
        change_subs: Arc<VdfsChangeSubscriptions>,
        dir: PluginDir,
        parent: Option<Weak<dyn Plugin>>,
    ) -> Self {
        Self {
            convergence,
            active_mgr,
            config,
            change_subs,
            dir,
            parent,
        }
    }

    /// 父插件引用（能力收集遍历它）——与 [`super::super::plugin::SessionPlugin::get_parent`] 同源的那一格弱引用。
    pub(crate) fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.as_ref().and_then(|w| w.upgrade())
    }

    /// 存储（与插件共用**同一个** `OnceCell`，实现在 [`Convergence::get_store`]）。
    pub(crate) async fn get_store(&self) -> Result<Arc<SessionStore>, PluginError> {
        self.convergence.get_store().await
    }

    /// 按 id 取会话（缺失即初始化）——一轮里所有会话读取都走这里。
    pub(crate) async fn load_session(&self, session_id: &str) -> Result<Session, PluginError> {
        self.get_store().await?.load_session(session_id).await
    }

    pub(crate) async fn save_session(&self, session: &Session) -> Result<(), PluginError> {
        self.get_store().await?.save_session(session).await
    }

    /// 会话叶子的**资源信号**（粗粒度、不带载荷，消费方重拉清单）——自动命名用。
    pub(super) fn notify_change(&self, session_id: &str) {
        self.change_subs.notify(&VdfsChange::bare(session_id));
    }
}
