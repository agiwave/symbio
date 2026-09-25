//! V2.6 分形路由专用会话管理 (支持外部 ID)

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use symbio::symbio_core::PluginFrame;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// 分形路由连接句柄
pub struct RouteConnection {
    pub tx: mpsc::Sender<PluginFrame>,
    pub last_active: Instant,
    /// 该连接所持 `PluginChannel` 的取消令牌。
    ///
    /// ## 它只用来收口**订阅表**，绝不用来中止业务任务
    ///
    /// 会话与总线插件各自 spawn 了一个「等它触发后反注册订阅」的任务
    /// （`plugins/session/plugin.rs`、`plugins/event_bus/plugin.rs`）。若不触发，
    /// 那些任务永不执行——订阅表就只能靠「下一次发布时探测 `tx.is_closed()`」兜底，
    /// 而那要求**之后还有发布**；没有发布就一直留着。
    ///
    /// 会话主循环的中止走**另一条路**（`session/chat/abort` 路由 + `ExecAbortSignal`），
    /// 与本令牌无关。因此关闭前端连接**不会**中止正在跑的那一轮对话——
    /// 这正是 `route_v2_close` 想要的语义（后端任务独立于前端连接继续运行）。
    pub cancel: CancellationToken,
}

/// 分形路由连接管理器
pub struct RouteConnectionManager {
    connections: Arc<RwLock<HashMap<String, RouteConnection>>>,
    next_id: AtomicU64,
    cleanup_token: CancellationToken,
}

impl RouteConnectionManager {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
            next_id: AtomicU64::new(1),
            cleanup_token: CancellationToken::new(),
        }
    }

    /// 启动后台清理任务，定期扫描并关闭超时连接
    /// 注意：此方法必须在 Tokio runtime 上下文中调用
    pub fn start_cleanup_task(&self) {
        let connections = self.connections.clone();
        let token = self.cleanup_token.clone();

        tokio::spawn(async move {
            let timeout = Duration::from_secs(300); // 5 分钟
            let mut interval = tokio::time::interval(Duration::from_secs(60));

            loop {
                tokio::select! {
                    _ = interval.tick() => {},
                    _ = token.cancelled() => break,
                }

                let now = Instant::now();
                let mut guard = connections.write().await;
                let to_remove: Vec<String> = guard
                    .iter()
                    .filter(|(_, conn)| now.duration_since(conn.last_active) > timeout)
                    .map(|(id, _)| id.clone())
                    .collect();

                for id in to_remove {
                    warn!(
                        conn_id = %id,
                        timeout_secs = timeout.as_secs(),
                        "Connection timed out, removing"
                    );
                    // Drop tx naturally causes EOF；取消令牌让订阅表同步收口
                    if let Some(conn) = guard.remove(&id) {
                        conn.cancel.cancel();
                    }
                }
            }

            debug!("Cleanup task stopped");
        });
    }

    /// 自动生成 ID 并注册
    pub async fn register(
        &self,
        tx: mpsc::Sender<PluginFrame>,
        cancel: CancellationToken,
    ) -> String {
        let id = format!(
            "route_conn_{}",
            self.next_id.fetch_add(1, Ordering::Relaxed)
        );
        self.register_fixed(id.clone(), tx, cancel).await;
        id
    }

    /// 使用固定 ID 注册 (用于解决前端握手竞态)
    pub async fn register_fixed(
        &self,
        id: String,
        tx: mpsc::Sender<PluginFrame>,
        cancel: CancellationToken,
    ) {
        let mut guard = self.connections.write().await;
        info!(conn_id = %id, "Registering connection");
        guard.insert(
            id,
            RouteConnection {
                tx,
                last_active: Instant::now(),
                cancel,
            },
        );
    }

    /// 发送帧并更新最后活动时间戳
    pub async fn send(&self, id: &str, frame: PluginFrame) -> Result<(), String> {
        let mut guard = self.connections.write().await;
        if let Some(conn) = guard.get_mut(id) {
            conn.last_active = Instant::now();
            conn.tx.send(frame).await.map_err(|e| e.to_string())
        } else {
            Err(format!("Connection {id} not found"))
        }
    }

    /// 摘除连接并**取消其订阅令牌**（订阅表因此确定性地收口，不等下一次发布）。
    ///
    /// 「不中止业务任务」的语义不变：本方法只碰订阅令牌，不碰会话的 `ExecAbortSignal`。
    pub async fn remove_connection(&self, id: &str) {
        let removed = self.connections.write().await.remove(id);
        match removed {
            Some(conn) => {
                info!(conn_id = %id, "Removing connection");
                conn.cancel.cancel();
            }
            None => debug!(conn_id = %id, "Connection already gone"),
        }
    }

    pub async fn remove_all(&self) {
        let drained: Vec<(String, RouteConnection)> =
            self.connections.write().await.drain().collect();
        info!(count = drained.len(), "Removing all connections");
        for (_, conn) in drained {
            conn.cancel.cancel();
        }
    }
}

impl Default for RouteConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RouteConnectionManager {
    fn drop(&mut self) {
        info!("Dropping manager, stopping cleanup task");
        self.cleanup_token.cancel();
    }
}
