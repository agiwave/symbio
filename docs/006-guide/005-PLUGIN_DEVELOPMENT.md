---
kind: guide
id: GU-005-PLUGIN_DEVELOPMENT
title: Symbio 插件开发指南
status: active
created: 2026-09-07
---

# Symbio 插件开发指南

> **文档类型：操作指南** — 从零编写一个业务插件。

## 插件类型

| 类型 | 用途 | 示例 |
|------|------|------|
| **容器插件** | 包含子插件，按目录名分发 | `home`, `composite` |
| **叶子插件** | 执行业务逻辑 | `agent`, `model`, `local` |

两者实现**完全相同**的 `Plugin` trait（`crates/symbio-kernel/src/plugin/mod.rs`）：
容器与叶子的差别只在 `route` 里是否把路径转发给子插件。

---

## 开发流程

### 1. 建 crate（一插件一 crate）

```bash
mkdir -p crates/symbio-plugin-my_plugin/src
```

`crates/symbio-plugin-my_plugin/Cargo.toml`：

```toml
[package]
name = "symbio-plugin-my_plugin" # 目录名 = 插件 id = 路由前缀，三者逐字一致
version = "0.1.0"
edition = "2021"
rust-version = "1.91"

[dependencies]
symbio-core = { path = "../symbio-core" }
symbio-kernel = { path = "../symbio-kernel" }
symbio-deploy = { path = "../symbio-deploy" }
async-trait = { workspace = true }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
tokio = { workspace = true, features = ["full"] }
tracing = { workspace = true }
```

**外部依赖只写 `{ workspace = true }`**，版本唯一源在根 `[workspace.dependencies]`。
根清单的 `members = ["crates/*", …]` 会自动收进新目录，不用改 `Cargo.toml`。

`PLUGIN.yml` 是**编译期装配声明**（`scripts/facts/assembly.mjs` 读取）：

```yaml
plugin_id: my_plugin # 必须等于目录名，否则生成器报错
plugin_comment: 我的插件
plugin_assembly: sub_agent # 进标准插件树；或 none（不进）
plugin_order: 300 # sub_agent 必填，整数，决定挂载顺序
```

**加插件 = 建目录 + 写 `PLUGIN.yml` + 写代码。** `symbio-deploy` 的 `PLUGIN_ID_*` 与装配清单、
`symbio` 的依赖行与 `pub use` 行都由生成器从目录投影（门禁自动重生成），**两侧都不手改**；
手改生成区会被 `gen-facts --check` 判红。

### 2. 实现 `Plugin`

```rust
// crates/symbio-plugin-my_plugin/src/lib.rs
pub mod plugin;
```

```rust
// crates/symbio-plugin-my_plugin/src/plugin.rs
use async_trait::async_trait;
use std::sync::Arc;
use symbio_core::{
    CapabilityRow, Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload, CAPABILITY_VISITOR, NAME, PATH,
    TRAVERSE_AVAILABLE_TOOLS,
};
use symbio_deploy::PLUGIN_ID_MY_PLUGIN;

pub struct MyPlugin;

impl MyPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(MyPlugin) as Arc<dyn Plugin>
    }

    pub fn metadata() -> PluginMeta {
        // 首参 = 插件目录名（E-001）：目录名才是路由前缀
        PluginMeta::new(PLUGIN_ID_MY_PLUGIN, "我的插件")
            .with_description("演示插件")
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
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        // 容器已剥掉插件前缀，`PATH` 是**相对臂**
        let path = ctx.get(PATH).unwrap_or_default();
        match path.as_str() {
            "greet" => {
                let name = ctx.get(NAME).unwrap_or_default();
                Ok(PluginPayload::new(
                    &serde_json::json!({ "message": format!("Hello, {name}!") }),
                ))
            }
            _ => Err(PluginError::NotFound(format!("[my_plugin] 未知路径: {path}"))),
        }
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        // 遍历路径同样读 `ctx` 的 `PATH`（容器传进来的第一个参数恒为空串）
        let path = ctx.get(PATH).unwrap_or_default();
        if path == TRAVERSE_AVAILABLE_TOOLS {
            if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
                // 工具由**实例**提供：停用 ⇒ 不构造 ⇒ 工具从模型工具面消失
                visitor.put(CapabilityRow::tool(Arc::new(MyTool))).await;
            }
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

symbio_core::submit_object_creator!(PLUGIN_ID_MY_PLUGIN, MyPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
```

**跨插件调用只走地址**（`ctx.parent()` + `route`），**不要**依赖兄弟插件的 crate
（E-009 判清单 / 源码 / `include!` 五条通路）——插件之间互不可见是架构公理。

### 3. 工具（`Capability`）

工具不是 `traverse` 里返回的 JSON，而是一个实现 `Capability` 的类型，由 `traverse` 装进收集器：

```rust
// crates/symbio-plugin-my_plugin/src/tool.rs
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use symbio_core::{
    Capability, CapabilityCategory, CapabilityMeta, CapabilityRiskLevel, ExecEnv, PluginError,
    PluginInvokeRequest,
};

pub(crate) struct MyTool;

#[async_trait]
impl Capability for MyTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: "my_greet".to_string(),
            description: "向某人打招呼".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"]
            }),
            category: Some(CapabilityCategory::Other),
            risk: Some(CapabilityRiskLevel::Low),
            examples: None,
            behaviors: &[], // 产出哪些效果由能力自己声明（DE-025）
        }
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
        Ok(json!({ "message": format!("Hello, {name}!") }))
    }
}
```

`execute` 期间的事件与中止走**执行期环境** `ExecEnv`（`env.sink` / `env.abort`），
不要用 `PluginPayload::Session` 回传——那条通道是**跨进程传输原语**（前端实时面 / CLI / 网关），
理由见 [D-020](../003-design/001-核心架构.md "DE-001-核心架构") 与
[`crates/symbio-plugin-session/src/docs/102-core-loop.md`](../../crates/symbio-plugin-session/src/docs/102-core-loop.md "DE-102-core-loop") §7、§9。

### 4. 让它被装配

**不需要在任何地方登记**：容器扫描自己的目录（= 系统根，其下一层目录即一个插件），
逐目录读 `PLUGIN.yml`，`plugin_provider` 指向已注册的工厂（`creator_has`）即实例化，
并把该目录经 ctx 键 `PLUGIN_DIR` 告知插件。标准树挂哪些插件由各插件 `PLUGIN.yml` 的
`plugin_assembly: sub_agent` 决定——生成器把它投影进 `symbio_deploy::ASSEMBLY_SUB_AGENT_PLUGINS`
（`home` 的 `SYSTEM_PLUGINS` 只是这份清单的别名，没有第二处要同步）。

> ⚠️ **插件不要推导自己的目录。** 目录一律用 `PLUGIN_DIR` 给的 `PluginDir`
> （`plugin_dir_from_ctx(ctx, PLUGIN_X)` / `self.dir`），**不要**按插件名反推落位
> （`HomedirRegistry::get` 再 `join(...)` 之类）——那等于把「装配决策」写死进插件，
> 挪个位置就全错。按 id 派生路径的自由函数请让调用方把根当入参传进来（见
> `crates/symbio-plugin-session/src/paths.rs`）。注释里写「本插件自己的目录」，
> 不要写 `<homedir>/…` 具体路径。

```yaml
# ~/.symbio/my_plugin/PLUGIN.yml
plugin_provider: my_plugin # 工厂 id（submit_object_creator! 的第一个参数）
plugin_name: my_plugin # 实例名，缺省 = 目录名
# ↓ 以下四个是**身份**保留键，由装配期从 Plugin::meta 一次性投影（D-032）
plugin_title: 我的插件 # 展示标题 / 挂载点标题
plugin_description: 演示插件 # 语义描述
plugin_version: 0.1.0 # 版本
plugin_author: me # 作者
# ↓ 以下即本插件自己的配置字段，随你定义
```

**身份不用手写**：装配期容器构造出插件后，会把 `Plugin::meta` 里的身份**投影**
进 manifest（键已存在则不动，因此你改过的值不会被覆盖）。此后 manifest 是权威——
运行期的插件列表、挂载点目录标题都读它，所以**停用的插件也有身份**（它只是不被
构造，不是不存在）。`meta` 的另一半（`order` / `icon` / `hidden` / `root_access`）
是**挂载点呈现**，只在运行期读，不投影——没有挂载点就没有这些属性。

> 配置读写走 `symbio_core::plugin::dir` 的 `PluginDir` + `PluginConfigFile`：插件持有自己的
> `PluginConfigFile`，读 / 写**自己**的 `PLUGIN.yml`（`PluginConfigFile::apply` = 校验 → 落内存 →
> 落自己的文件 → 广播）。对外地址自动是 `<根>/my_plugin/PLUGIN.yml`，无需写任何路由。
> 要出现在设置页，只需在 `traverse` 里 `capability_announce_configurable(&ctx, &self.config_file)`——
> 设置页列的就是各插件自己交出来的配置文档。

---

## Trait 方法详解

`Plugin` 的分形寻址入口只有两个（其余方法都有默认实现）：`route` 与 `traverse`。
`start`（装配后同步调用一次）与 `stop`（停用 / 卸载 / 收尾前异步调用）按需覆盖；
`get_vfs_provider` / `vdfs_dispatch` 默认不暴露 VDFS，自身即 provider 的插件才实现。

### route

```rust
async fn route(
    self: Arc<Self>,
    ctx: Arc<dyn PluginInvokeRequest>,
) -> PluginInvokeResponse<PluginPayload>
```

**上下文键**：
- `PATH` - 目标路径（容器已剥掉插件前缀，是**相对臂**）
- `PLUGIN_PAYLOAD_KEY` - 请求数据（`plugin_payload::<T>(&ctx)` 取）
- `WORKDIR` - 工作区路径
- `SESSION_ID` - 会话 ID

**返回**：
- `Ok(PluginPayload::Data(...))` - 一次性数据
- `Ok(PluginPayload::Empty)` - 成功但无返回值
- `Ok(PluginPayload::Session(channel))` - 流式会话（**跨进程传输**）
- `Err(PluginError)` - 错误；未知臂一律 `PluginError::NotFound`

### traverse

```rust
async fn traverse(
    self: Arc<Self>,
    path: String,
    ctx: Arc<dyn PluginInvokeRequest>,
) -> PluginInvokeResponse<PluginPayload>
```

**用途**：`TRAVERSE_AVAILABLE_TOOLS` 广播时把工具 / 上下文段装进 `CAPABILITY_VISITOR`；
`TRAVERSE_AVAILABLE_OPTIONS` 走平行的 `OPTION_VISITOR`。协议端点只有这两个常量，
写别的字符串由 E-004 判红。第一个参数由容器传空串，实际子路径读 `ctx` 的 `PATH`。

---

## 容器插件实现

```rust
#[async_trait]
impl Plugin for MyContainer {
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        let (current, sub_path) = path.split_once('/').unwrap_or((path.as_str(), ""));

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
use symbio_core::PluginChannel;

async fn route(
    self: Arc<Self>,
    ctx: Arc<dyn PluginInvokeRequest>,
) -> PluginInvokeResponse<PluginPayload> {
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
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    });

    Ok(PluginPayload::Session(peer_channel))
}
```

---

## 前端集成

新增一条控制面路由**不需要在前端登记**：`tauri/src/constants/routes.gen.ts` 是后端
`route` 的 `match` 臂的**生成物**（`scripts/facts/routes-ts.mjs`，门禁自动重跑并暂存），
加一条臂就多一个同名常量 `ROUTE_<插件目录名>_<操作>`。在前端别处写死同一条地址，
由 `mechanism-audit` 的 M-008 判红。

### 创建服务客户端

出站请求一律经 `services/`（M-003：组件不得直接 `invoke`）：

```typescript
// tauri/src/services/myPlugin.ts
import { callPlugin } from './plugin'
import { ROUTE_MY_PLUGIN_GREET } from '@/constants/routes.gen'

export async function greet(name: string) {
  return callPlugin<GreetResponse>(ROUTE_MY_PLUGIN_GREET, { name })
}
```

---

## 测试

测试跟着实现文件走（`node scripts/test-layout-audit.mjs` 判定）：一个实现文件一个
`<名字>.test.rs`，用 `#[path]` 挂到同一模块下——与 `plugin.rs` 末尾那三行一致。

```rust
// crates/symbio-plugin-my_plugin/src/plugin.test.rs
use super::*;
use symbio_core::PluginSimpleRequest;

#[tokio::test]
async fn greet_returns_message() {
    let plugin = MyPlugin::build(Arc::new(PluginSimpleRequest::new(None, None)));
    let ctx = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(PATH, "greet".to_string());
    ctx.set(NAME, "World".to_string());

    let payload = plugin.route(ctx).await.expect("greet 应当成功");
    let data: serde_json::Value = payload.get().expect("应当是可序列化数据");
    assert_eq!(data["message"], "Hello, World!");
}
```

---

> **维护原则**：插件接口变更必须保持向后兼容，并在 `docs/003-design/` 记一条决策。
