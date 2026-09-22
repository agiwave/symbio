# Symbio 插件开发指南

> **文档类型：操作指南** — 从零编写一个业务插件。

## 插件类型

| 类型 | 用途 | 示例 |
|------|------|------|
| **容器插件** | 包含子插件，动态挂载 | `home`, `composite` |
| **叶子插件** | 执行业务逻辑 | `agent`, `model`, `local` |

两者实现**完全相同**的 `Plugin` trait。

---

## 开发流程

### 1. 创建目录

```bash
mkdir -p symbio/src/plugins/my_plugin
```

### 2. 创建 mod.rs

```rust
// symbio/src/plugins/my_plugin/mod.rs
pub mod plugin;
```

### 3. 实现 Plugin Trait

```rust
// symbio/src/plugins/my_plugin/plugin.rs

use async_trait::async_trait;
use std::sync::Arc;
use symbio::symbio_core::{
    InvokeRequest, InvokeResponse, Plugin, PluginError, PluginMeta, PluginPayload,
};

pub struct MyPlugin;

impl MyPlugin {
    pub fn build(_ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(Self)
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("my_plugin", "My Plugin")
            .with_description("我的自定义插件")
            .with_version("0.1.0")
    }
}

#[async_trait]
impl Plugin for MyPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(symbio::symbio_core::PATH).unwrap_or_default();
        
        match path.as_str() {
            "greet" => {
                let name = ctx.get(symbio::symbio_core::NAME).unwrap_or_default();
                Ok(PluginPayload::new(&serde_json::json!({

---

## Trait 方法详解

### route()

```rust
async fn route(
    self: Arc<Self>,
    ctx: Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload>
```

**上下文键**：
- `PATH` - 目标路径
- `PAYLOAD` - 请求数据
- `WORKDIR` - 工作区路径
- `SESSION_ID` - 会话 ID

**返回**：
- `Ok(PluginPayload::Data(...))` - 一次性数据
- `Ok(PluginPayload::Session(channel))` - 流式会话（**跨进程传输**）
- `Err(PluginError)` - 错误

> ⚠️ **`Session` 是传输原语，不是「事件流」的通用答案**。它用于**跨进程**场景
> （前端实时面 / CLI / gateway）。**执行期**（`Capability::execute` 被会话编排层调用）
> 不要用它回传事件：出口与中止由**执行期环境** `ExecEnv` 承载，作为 `execute` 的
> 显式参数给出（`env.sink()` / `env.abort()`），事件直接 `sink.apply(message)`（一条
> `ChatMessage`；会话级告警走 `sink.warn(...)` 独立通道）。
> 工具侧不再自己读信封里的键——`ExecEnv` 由分发点 `symbio_core::invoke_capability`
> 从 `ctx` 装配（缺席 ⇒ 静默 / 永不中止，故 `route()` 直连调用照常可用）。
> 理由与代价见 [ADR-020](../DECISIONS.md)、[ADR-021](../DECISIONS.md) 与
> [`session/docs/core-loop.md`](../../symbio/src/plugins/session/docs/core-loop.md) §9、§11。

### traverse()

```rust
async fn traverse(
    self: Arc<Self>,
    path: String,
    ctx: Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload>
```

**用途**：
- `available_tools` - 返回工具定义列表
- 自定义诊断/内省

---

## 容器插件实现

```rust
use std::collections::HashMap;

pub struct MyContainer {
    children: Arc<RwLock<HashMap<String, Arc<dyn Plugin>>>>,
}

#[async_trait]
impl Plugin for MyContainer {
    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        let (current, sub_path) = path.split_once('/').map_or((path.as_str(), ""), |(a, b)| (a, b));
        
        match current {
            "config" => handle_config(ctx).await,
            _ => {
                let children = self.children.read().await;
                if let Some(child) = children.get(current) {
                    let sub_ctx = ctx.fork();
                    sub_ctx.set(PATH, sub_path);
                    child.clone().route(sub_ctx).await
                } else {
                    Err(PluginError::NotFound(format!("子插件不存在: {current}")))
                }
            }
        }
    }
}
```

---

## 流式会话实现

```rust
use symbio::symbio_core::PluginChannel;

async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
    let (my_channel, peer_channel) = PluginChannel::pair(64);
    
    tokio::spawn(async move {
        for i in 0..10 {
            let frame = PluginFrame::Data(serde_json::json!({
                "type": "progress",
                "value": i * 10
            }));
            if my_channel.tx.send(frame).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
    
    Ok(PluginPayload::Session(peer_channel))
}
```

---

## 前端集成

### 添加路由常量

```typescript
// tauri/src/constants/pluginPaths.ts
export const PLUGIN_PATHS = {
  MY_PLUGIN: 'worker/my_plugin',
};
```

### 创建服务客户端

```typescript
// tauri/src/services/myPlugin.ts
import { invoke } from '@tauri-apps/api/core';
import { PLUGIN_PATHS } from '@/constants/pluginPaths';

export async function greet(name: string) {
  return invoke('route_v2', {
    request: {
      metadata: { path: `${PLUGIN_PATHS.MY_PLUGIN}/greet` },
      payload: { name }
    }
  });
}
```

---

## 测试

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_greet() {
        let plugin = MyPlugin::new();
        let ctx = Arc::new(SimpleRequest::new(None, None));
        ctx.set(PATH, "greet");
        ctx.set(NAME, "World");
        
        let result = plugin.route(ctx).await;
        assert!(result.is_ok());
    }
}
```

---

> **维护原则**：插件接口变更必须保持向后兼容，并在 DECISIONS.md 记录原因。

                    "message": format!("Hello, {name}!")
                })))
            }
            _ => Err(PluginError::NotFound(format!("未知路径: {path}"))),
        }
    }

    async fn traverse(
        self: Arc<Self>,
        path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        if path == symbio::symbio_core::TRAVERSE_AVAILABLE_TOOLS {
            let tools = vec![
                symbio::symbio_core::CapabilityMeta {
                    name: "my_plugin/greet".to_string(),
                    description: "打招呼".to_string(),
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" }
                        }
                    }),
                    ..Default::default()
                }
            ];
            return Ok(PluginPayload::new(&tools));
        }
        Err(PluginError::NotFound(format!("未知遍历路径: {path}")))
    }
}

// 注册到全局工厂
symbio::submit_object_creator!("my_plugin", MyPlugin::build, dyn Plugin);
```

### 4. 让它被装配

**不需要在任何地方登记**：容器**扫描自己的目录**（= 系统根，其下一层目录即一个插件），
逐目录读 `PLUGIN.yml`，`plugin_provider` 指向已注册的工厂（`has_creator`）即实例化，
并把该目录经 ctx 键 `PLUGIN_DIR` 告知插件。

> ⚠️ **插件不要推导自己的目录。** 目录一律用 `PLUGIN_DIR` 给的 `PluginDir`
> （`dir_from_ctx(ctx, PLUGIN_X)` / `self.dir`），**不要** `HomedirRegistry::get()`
> 再 `join("plugins")`、也不要 `entry::category_dir` 按插件名反推——那等于把「装配
> 决策」写死进插件，挪个位置就全错。按 id 派生路径的自由函数请让调用方把根当入参
> 传进来（见 `plugins/session/paths.rs`）。注释里写「本插件自己的目录」，不要写
> `<homedir>/…` 具体路径。

```yaml
# ~/.symbio/my_plugin/PLUGIN.yml
plugin_provider: my_plugin   # 工厂 id（submit_object_creator! 的第一个参数）
plugin_name: my_plugin       # 实例名，缺省 = 目录名
# ↓ 以下即本插件自己的配置字段，随你定义
```

若要随系统启动（`home` 的必需插件清单），把插件名加进 `symbio/src/plugins/home/plugin.rs`
的 `SYSTEM_PLUGINS`——**这是唯一的清单点**，且它属于**构造者**（`composite` 是通用容器，
可以嵌套，因此不内置任何清单）。

> 配置读写走 `symbio_core::plugin_dir` 的 `PluginDir` + `ConfigFile`：插件持有自己的
> `ConfigFile`，读 / 写**自己**的 `PLUGIN.yml`（`ConfigFile::apply` = 校验 → 落内存 →
> 落自己的文件 → 广播）。对外地址自动是 `<根>/my_plugin/PLUGIN.yml`，无需写任何路由。
