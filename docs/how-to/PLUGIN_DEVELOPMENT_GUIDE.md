# Symbio 插件开发实战指南

> **文档类型：How-to guide（操作指南）** — 从零写一个业务插件，任务导向。

> 本文档基于**当前代码**（纯 Rust 核心库 + E2E CLI）。
> 早期"Tauri + Vue 前端"叙述中关于前端注册、TS schema 同步等内容已不再适用。

本指南带你从零编写一个**业务插件**（如 `weather`），并通过配置把它接入分形路由树。

## 1. 准备：理解分形接口

每个插件实现同一个 `Plugin` Trait：

```rust
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    fn meta(&self) -> PluginMeta;
    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>)
        -> InvokeResponse<PluginPayload>;
    async fn traverse(self: Arc<Self>, path: String, ctx: Arc<dyn InvokeRequest>)
        -> InvokeResponse<PluginPayload>;
}
```

* `meta()`：返回插件元数据（id / name / description / version）。
* `route(ctx)`：处理分配到本节点的请求（按 `PATH` 字符串）。
* `traverse(path, ctx)`：递归暴露能力（最常用的是 `traverse("available_tools", …)`）。

容器类插件（`Home` / `Composite`）同样实现这两个方法，差别仅在于
它们收到 `route` 后**继续剥 `PATH` 前缀**转发给子插件。

## 2. 目录约定

```text
symbio/src/plugins/weather/
├── mod.rs
├── plugin.rs         # WeatherPlugin: Plugin 实现 + submit_object_creator! 注册
├── config.rs         # 解析 plugin 自己的配置块（可选）
├── tools.rs          # 具体 tool 描述（供 traverse("available_tools") 返回）
└── docs/
    └── README.md     # 业务说明（自包含）
```

> 注意：当前架构**没有**独立的 `factory.rs` / `PluginFactory` 概念。插件通过
> `submit_object_creator!` 宏把构造函数注册到全局 `ObjectCreatorRegistry`，
> 宿主用 `create_object::<dyn Plugin>(id, ctx)` 实例化。

## 3. 最小插件示例

### 3.1 `plugin.rs`

```rust
use std::sync::Arc;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError, PluginFrame,
    PluginMeta, PluginPayload,
};

#[derive(Deserialize)]
struct ForecastReq {
    city: String,
}

pub struct WeatherPlugin {
    meta: PluginMeta,
}

impl WeatherPlugin {
    /// 构造函数：被 submit_object_creator! 注册，签名固定为
    /// `fn(Arc<dyn InvokeRequest>) -> Arc<dyn Plugin>`
    pub fn build(_ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(WeatherPlugin {
            meta: PluginMeta::new("weather", "Weather")
                .with_description("查询城市天气"),
        })
    }

    fn handle_forecast(&self, ctx: Arc<dyn InvokeRequest>) -> Result<PluginPayload, PluginError> {
        let req: ForecastReq = ctx.payload()?;
        // 实际项目里调 API；这里仅返回 mock
        Ok(PluginPayload::new(&json!({
            "city": req.city,
            "temp_c": 23,
            "summary": "sunny"
        })))
    }
}

#[async_trait]
impl Plugin for WeatherPlugin {
    fn meta(&self) -> PluginMeta { self.meta.clone() }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>)
        -> InvokeResponse<PluginPayload>
    {
        match ctx.path().as_str() {
            p if p == "forecast" || p.is_empty() => {
                self.handle_forecast(ctx)
            }
            other => Err(PluginError::NotFound(format!("weather/{other}"))),
        }
    }

    async fn traverse(self: Arc<Self>, path: String, _ctx: Arc<dyn InvokeRequest>)
        -> InvokeResponse<PluginPayload>
    {
        if path == symbio_core::TRAVERSE_AVAILABLE_TOOLS {
            return Ok(PluginPayload::new(&json!([{
                "name": "weather/forecast",
                "description": "根据 city 查询天气",
                "schema": {
                    "type": "object",
                    "properties": { "city": { "type": "string" } },
                    "required": ["city"]
                }
            }])));
        }
        Err(PluginError::NotFound(format!("weather/{path}")))
    }
}

// 注册构造函数：id = "weather"，目标类型 = dyn Plugin
crate::submit_object_creator!(symbio_core::PLUGIN_WEATHER, WeatherPlugin::build, dyn Plugin);
```

> 说明：
> * `ctx.path()` 来自 `InvokeRequestExt`，返回当前路由路径字符串。
> * `ctx.payload::<T>()` 把 `PAYLOAD` 解析为强类型 `T`（优先零拷贝，失败回退 JSON 反序列化），解析失败返回 `PluginError::ValidationError`。
> * `Plugin` 的 `route` / `traverse` 直接返回 `Result<PluginPayload, PluginError>`（`InvokeResponse<PluginPayload>` 即该 Result），无需 `.into_invoke_response()` 之类的转换方法。
> * 插件 id 常量统一放在 `symbio_core::ids`（如 `PLUGIN_WEATHER`）。本例为示意，真实插件应在此追加对应常量，或在宏中直接写字符串字面量 `"weather"`。

### 3.2 `mod.rs`

```rust
pub mod plugin;
pub mod docs;
```

并在 `symbio/src/plugins/mod.rs` 中追加 `mod weather;`。

## 4. 接入分形路由树

无需改任何代码，只需在 `~/.symbio/config.yaml` 中挂载：

```yaml
symbio:
  plugins:
    worker:
      plugin_provider: composite
      plugins:
        weather:
          plugin_provider: weather
          config:
            default_city: "Shanghai"
```

之后调用 `route("weather/forecast", { "city": "Shanghai" })`
即可由 `Home → worker → weather` 路径命中。

## 5. 暴露给 LLM（工具发现）

```bash
# 程序内列出整棵树的工具
let tools = root.traverse(TRAVERSE_AVAILABLE_TOOLS, ctx).await?;
```

返回结果中 `weather/forecast` 会被自动注入到 LLM 的 tool list，
LLM 在对话中调 `weather/forecast` 时会被精准路由回本插件的 `route("forecast", …)`。

## 6. 容器类插件的特殊写法

容器（如 `composite`）的实现要点是**按 `PATH` 剥前缀后转发**：

```rust
async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>)
    -> InvokeResponse<PluginPayload>
{
    let path = ctx.path();
    let (head, rest) = path.split_once('/').unwrap_or((path.as_str(), ""));

    // 本地指令
    if head == "_root" {
        return Ok(PluginPayload::new(&self.list_children()));
    }

    // 转发给子插件
    let child = self.children.get(head)
        .ok_or_else(|| PluginError::NotFound(head.to_string()))?;
    let new_ctx = ctx.with_path(rest);
    child.clone().route(new_ctx).await
}
```

> `ctx.with_path(rest)` 来自 `InvokeRequestExt`，基于当前 ctx fork 出剥离前缀后的新上下文。

容器与叶子插件**接口完全一致**——这是分形架构的精髓。

## 7. 测试与调试

### 7.1 单元测试

使用真实 `SimpleRequest` 构造上下文（无 `MockInvokeRequest` / `testing` 模块）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Weak;
    use symbio_core::{SimpleRequest, PATH};

    #[tokio::test]
    async fn forecast_requires_city() {
        let p = WeatherPlugin::build(Arc::new(SimpleRequest::new(None, None)));
        let ctx = Arc::new(SimpleRequest::new(None, None));
        ctx.set(PATH, "weather/forecast".to_string());
        // 不设置 payload，payload::<ForecastReq>() 应返回 ValidationError
        let err = p.route(ctx).await.unwrap_err();
        assert!(matches!(err, PluginError::ValidationError(_)));
    }
}
```

### 7.2 端到端验证

通过 `seed_agents` 之外的宿主（Tauri 桌面端或自建 Rust host）调用 `route("weather/forecast", …)`；
也可在单元测试中直接 `p.route(ctx)` 验证业务。

### 7.3 整树自省

宿主层可调用 `root.traverse(TRAVERSE_AVAILABLE_TOOLS, ctx)` 列出全树工具，
或调用各插件的 `_root` 路径查询子插件拓扑。

## 8. 最佳实践

1. **机制化**：插件内不要硬编码业务规则（"哪些字段是 ID"、"哪些字段是关系"），
   能交给 `seed_cus.jsonl` + `RelationPropRegistry` 的就交出去。
2. **自相似**：不要写"特殊插件路径"；所有逻辑通过 `route("xxx", …)` 表达。
3. **错误稳定码**：跨端错误必须带稳定 `code`（`VALIDATION_ERROR` / `NOT_FOUND` / …），
   详见 [API_DESIGN.md §6](../architecture/API_DESIGN.md)。
4. **状态隔离**：用 `Arc<Atomic*>` 或 `Arc<RwLock<T>>` 持有可变状态；
   锁内不要 `await` 跨其他锁。
5. **文档自包含**：插件专属说明放在 `symbio/src/plugins/<name>/docs/`，随代码一起演进。

## 9. 进阶：把插件发布到外部

当前插件与库同 crate 编译。若要支持"外部动态加载"，可：

* 在 `symbio_core::creator` 暴露类似 `register_external(Arc<dyn Plugin>)` 的接口。
* 用 `abi_stable` / `wasmtime` 约束 ABI，并通过配置 `plugin_provider: "external:weather.wasm"` 加载。
* host 侧按 `libloading` 动态装载 + 进程隔离。

> 这是路线图方向，尚未实现。期间建议先用 in-tree 形式验证业务可行性。
