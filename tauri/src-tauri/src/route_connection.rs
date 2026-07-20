//! V2.6 分形路由专用会话管理 (支持外部 ID)

use symbio::symbio_core::PluginFrame;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// 分形路由连接句柄
pub struct RouteConnection {
    pub tx: mpsc::Sender<PluginFrame>,
    pub last_active: Instant,
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
                    guard.remove(&id); // Drop tx naturally causes EOF
                }
            }

            debug!("Cleanup task stopped");
        });
    }

    /// 自动生成 ID 并注册
    pub async fn register(
        &self,
        tx: mpsc::Sender<PluginFrame>,
    ) -> String {
        let id = format!("route_conn_{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        self.register_fixed(id.clone(), tx).await;
        id
    }

    /// 使用固定 ID 注册 (用于解决前端握手竞态)
    pub async fn register_fixed(
        &self,
        id: String,
        tx: mpsc::Sender<PluginFrame>,
    ) {
        let mut guard = self.connections.write().await;
        info!(conn_id = %id, "Registering connection");
        guard.insert(
            id,
            RouteConnection {
                tx,
                last_active: Instant::now(),
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


    pub async fn remove_connection(&self, id: &str) {
        info!(conn_id = %id, "Removing connection (without cancelling)");
        self.connections.write().await.remove(id);
    }

    pub async fn remove_all(&self) {
        info!("Removing all connections (without cancelling)");
        self.connections.write().await.clear();
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
