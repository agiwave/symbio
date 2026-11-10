//! 子 Agent 注册的**代理层**：给一切注册项加上来源前缀
//!
//! ## 为什么是前缀，而不是"后注册者胜"
//!
//! 插件容器遍历子插件用的是 `HashMap`，**顺序不确定**。若让系统树与子 Agent 树
//! 用**同一个名字**注册（例如 `skill` 插件的每个实例都注册 `read_skill`），
//! 谁生效就取决于容器那一次随机的遍历顺序——结果不可预测。
//!
//! 加前缀后**名字根本不冲突**，于是：
//!
//! - 两份都注册（并集天然成立）；
//! - 判定与顺序无关，不需要给注册体系引入优先级概念；
//! - 也不必修改架构 crate 的架构定义——本模块只在 agent 插件内做一层代理。
//!
//! **去重判定不在这一层做**：这里只保证「两份都能按各自名字取到」，并顺手把
//! 「这个前缀名原本叫什么」交给会话装配（`SCOPE_TWIN` 声明行）。两份工具谁进模型的
//! 那**一张**表，由 `symbio-plugin-session` 按声明与根工具集合统一折叠，判定与顺序无关。
//! 原因是写入顺序不确定——按内表现判「根上有没有同名」会重新引入依赖遍历顺序的结果；
//! 而按名字形状猜前缀副本（`agent_` + 切点）会把子 Agent 的命名规则搬进控制面（D-428）。
//!
//! ## 两套前缀形态
//!
//! | 注册项 | 形态 | 理由 |
//! |---|---|---|
//! | 系统提示词段 | `agent/<id>/<段名>` | 只作键，`/` 可读且天然分层 |
//! | 工具 | `agent_<safe_id>_<name>` | 工具名进模型的 function-calling 协议，多数厂商只接受 `[A-Za-z0-9_-]` |
//!
//! VDFS 挂载族不在表里：它只剩根一形，而根**丢弃**（单槽归系统 Agent）。
//!
//! ## 单槽位 / 系统级注册不转发
//!
//! VDFS 根（`put` 的根行 `CapabilityRow::mount_root`）与整个**模型族**（模型行的生效位 + 目录）都是
//! **单槽**或系统级事实。子 Agent 的插件树里也有容器（它会把自己登记成 VDFS 根）
//! 与 model 插件实例，原样转发会让子 Agent **劫持**整个 `<根>` 与模型服务——
//! 系统侧的全部挂载点瞬间消失。因此这两项**丢弃**（记 debug）。
//!
//! 模型族的丢弃写在 [`Registry::put`] 上（按行的族别判），因为 4b 之后
//! 模型服务**不再是另一套方法**：model 插件逐行 `put` 写入，作用域这一层拦在同一个
//! 入口上——写入口只有一个，作用域规则也就只有一处（见 `capability/model.rs`）。

use async_trait::async_trait;
use std::sync::Arc;
use symbio_plugin_api::{
    family, port::PORT_FAMILY, CapabilityFamily, CapabilityRow, PortExecutor, PortPayload,
    Registry, SCOPE_TWIN,
};

/// 子 Agent 插件树的注册代理
pub struct SubAgentVisitor {
    inner: Arc<dyn Registry>,
    /// 智能体 id（目录名）
    agent_id: String,
    /// 本作用域登记进来的**工具数**（`max_tools` 限额的读数，见
    /// [`Self::tool_count`]）。只数工具行——提示词段 / 选项行不是「工具」。
    tools: std::sync::atomic::AtomicUsize,
    /// 同批端口行的执行体暂存（端口名 → 执行体）。
    ///
    /// `register_tool` 先写端口行后写工具行（D-440 ③ 的写入顺序）：工具行的执行面
    /// 住在**同批到达的端口行**上，这里按端口名暂存，等工具行到齐时取出——前缀化
    /// 重注册的执行体就是它（注册收口点已过闸，原样透传）。未配对的端口行（子树
    /// 自己的端口面）留在这里随轮丢弃：与旧行为一致——端口行此前就在本层被丢弃。
    /// tokio 的锁：不毒化、无 panic 面，锁卫只在取/放一条语句内，不跨 await。
    pending: tokio::sync::Mutex<std::collections::HashMap<String, Arc<dyn PortExecutor>>>,
}

impl SubAgentVisitor {
    pub fn new(inner: Arc<dyn Registry>, agent_id: impl Into<String>) -> Self {
        Self {
            inner,
            agent_id: agent_id.into(),
            tools: std::sync::atomic::AtomicUsize::new(0),
            pending: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// 本作用域贡献的工具数（装载期 `max_tools` 判据的读数）。
    pub fn tool_count(&self) -> usize {
        self.tools.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 提示词段的名字：`agent/<id>/<段名>`
    fn seg(&self, name: &str) -> String {
        format!("agent/{}/{}", self.agent_id, name)
    }

    /// 工具名：`agent_<safe_id>_<name>`
    fn tool(&self, name: &str) -> String {
        format!("agent_{}_{}", safe_segment(&self.agent_id), name)
    }
}

/// 把 id 压进 `[A-Za-z0-9_-]`（工具名的协议字符集）
fn safe_segment(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[async_trait]
impl Registry for SubAgentVisitor {
    async fn put(&self, row: CapabilityRow) {
        // 模型族**整体丢弃**（生效者与目录都归系统 Agent，见模块文档）：
        // 写入方（model 插件的 traverse）逐行 put，规则只需在这一个入口上挡一次。
        if row.family() == family::MODEL {
            symbio_plugin_api::plugin_debug!(
                "agent",
                "丢弃子 Agent `{}` 的模型服务注册（生效者与目录都归系统 Agent）",
                self.agent_id
            );
            return;
        }
        // 提示词段（4c）走**段名前缀**再入内表（`agent/<id>/<段名>`）：
        // 与机制化之前「拼好前缀再 register」逐字一致，只是入口从 register 换成 put。
        if let Some(text) = row.prompt_text() {
            let name = self.seg(row.name());
            self.inner
                .put(CapabilityRow::prompt(&name, text.to_string()))
                .await;
            return;
        }
        // 挂载族（4d）：只剩根一形，而且**丢弃**（子 Agent 的容器会把自己登记成根——
        // 转发它等于劫持整个 `<根>`）。非根的挂载行已不存在（见 `capability/mount.rs`），
        // 所以这里不再有「加目录名前缀」那一支。
        if row.vdfs().is_some() {
            symbio_plugin_api::plugin_debug!(
                "agent",
                "丢弃子 Agent `{}` 的 VDFS 根注册（单槽，归系统 Agent）",
                self.agent_id
            );
            return;
        }
        // 选项行与可配置声明行**原样透传**（4e）：这两族走独立的广播通道
        // （OPTION_VISITOR / CONFIGURABLE_VISITOR），作用域投影此前就不接管它们——
        // 入口统一到 `put` 之后，规则必须显式写在这里，否则静默丢行。
        if row.option_entry().is_some() || row.configurable_item().is_some() {
            self.inner.put(row).await;
            return;
        }
        // 端口行（D-440：工具的执行面住在端口行上）：**不转发**——子树自己的端口面
        // （`port.tool` 等系统端口）归系统侧，与旧行为一致（此前端口行就在本层丢弃）。
        // 这里只把**工具行派生端口**的执行体按端口名暂存，等同批的工具行到齐后一并
        // 前缀化重注册（写入顺序：`register_tool` 先端口行后工具行）。
        if row.family() == PORT_FAMILY {
            if let Some(payload) = row.payload::<PortPayload>() {
                let provider = payload.provider();
                if let Some(executor) = provider.executor() {
                    let mut pending = self.pending.lock().await;
                    for port in provider.ports() {
                        pending.insert(port, Arc::clone(&executor));
                    }
                }
            }
            return;
        }
        // 子作用域的工具加名前缀再入内表（`agent_<safe_id>_<name>`）：
        // 前缀化发生在 meta 层（D-440：行是纯数据，改名装饰器随 `Capability` 一起
        // 退场）——取行的自述快照改 `name`，执行体用同批端口行带来的那一份，经
        // `register_tool` 一次写入前缀化的端口行 + 工具行（派发侧按前缀名查执行体）。
        // **前缀副本与根工具的去重不在写入侧做**（写入顺序不确定），但「这个前缀名
        // 对应哪个本名」由本层声明：装配面据此折叠，不必认识 `agent_` 这套命名规则
        // （D-428）。
        if row.family() == family::TOOL {
            let mut meta = row.meta();
            let base = meta.name.clone();
            let name = self.tool(&base);
            meta.name = name.clone();
            // 声明行的键不能和工具行同名（表键是全表唯一命名空间）；键的形状由本层
            // 自己给，读侧只取载荷，不解析它。
            self.inner
                .put(CapabilityRow::new(
                    SCOPE_TWIN,
                    format!("twin:{name}"),
                    (name.clone(), base.clone()),
                    false,
                ))
                .await;
            self.tools
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // 执行体缺席 = 纯数据行（无执行体可绑）：按数据形状照旧入表，
            // 派发侧对它诚实 NotFound——工具面（目录 / 风险 / 行为读数）不受影响。
            // （锁卫只在 remove 这条语句内，不跨后面的 await。）
            let executor = self.pending.lock().await.remove(&base);
            if let Some(executor) = executor {
                symbio_providers::register_tool(self.inner.as_ref(), meta, None, executor).await;
            } else {
                self.inner.put(CapabilityRow::tool(meta)).await;
            }
            return;
        }
        // 其余族（嵌套作用域的声明行等）不在转发清单里：此前它们无执行对象可取、
        // 同样在这一层止步。
    }

    async fn list(&self, family: CapabilityFamily) -> Vec<Arc<CapabilityRow>> {
        self.inner.list(family).await
    }

    async fn get(&self, name: &str) -> Option<Arc<CapabilityRow>> {
        self.inner.get(name).await
    }

    // 模型族（4b）、挂载族（4d）与选项 / 可配置两族（4e）的读写都不再是 trait 上的方法：
    // 写侧在 `put` 上按族丢弃 / 加前缀 / 透传，读侧经下面的 `get` / `list` 原样透传内层——子 Agent 作用域内的
    // 插件照常按 id 取模型、按目录名取自己的挂载点，而生效者与 `<根>` 都归系统 Agent。
}

#[cfg(test)]
#[path = "scope.test.rs"]
mod tests;
