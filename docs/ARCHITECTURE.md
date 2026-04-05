# Symbio 架构文档

## 概述

Symbio 是一个基于分形插件架构的系统，核心设计目标：
- **平台无关性**：核心库不依赖任何特定平台框架
- **极简 API**：宿主应用只需调用一个函数
- **依赖注入**：通过 trait 抽象实现平台特定功能
- **分形嵌套**：插件可以包含子插件，形成自相似结构

## 项目结构

```
symbio/                           # 项目根目录
├── symbio/                       # 核心 Rust 库（平台无关）
├── tauri/                        # Tauri 桌面应用（前端 + Rust 后端）
├── docs/                         # 项目文档
│   ├── ARCHITECTURE.md           # 架构文档（本文件）
│   └── BUILD_GUIDE.md            # 构建指南
├── scripts/                      # 构建和发布脚本
├── .github/                      # GitHub 配置
├── .gitignore
├── LICENSE
└── README.md
```

## 架构分层

```
┌──────────────────────────────────────────────────┐
│              Host Application Layer              │
│  (Tauri / Web / CLI / Custom)                    │
│  - 平台特定实现 (EventSender, UI, etc.)          │
│  - 调用 create_root_plugin() 初始化              │
│  - 通过三命令接口与插件系统交互                   │
└────────────────────┬─────────────────────────────┘
                     │ 依赖
┌────────────────────▼─────────────────────────────┐
│           Symbio Core Library                    │
│                                                  │
│  ┌──────────────────────────────────────────┐   │
│  │  init.rs                                 │   │
│  │  - create_root_plugin()                  │   │
│  │  - 注册所有内置插件工厂                   │   │
│  └──────────────────────────────────────────┘   │
│                                                  │
│  ┌──────────────────────────────────────────┐   │
│  │  symbio_core/                            │   │
│  │  - traits.rs (Plugin, PluginFactory)     │   │
│  │  - types.rs (PluginMeta, StreamChunk)    │   │
│  │  - registry.rs (PluginFactoryRegistry)   │   │
│  │  - event.rs (EventSender trait)          │   │
│  └──────────────────────────────────────────┘   │
│                                                  │
│  ┌──────────────────────────────────────────┐   │
│  │  plugins/                                │   │
│  │  - home/ (root plugin)                   │   │
│  │  - agent/ (AI assistant)                 │   │
│  │  - explorer/ (file browser)              │   │
│  │  - work/ note/ setting/ etc.             │   │
│  └──────────────────────────────────────────┘   │
└──────────────────────────────────────────────────┘
```

## 核心组件

### 1. Plugin Trait

所有插件实现统一的 `Plugin` trait：

```rust
#[async_trait::async_trait]
pub trait Plugin: Send + Sync {
    /// 获取插件元数据
    fn meta(&self, path: &str) -> PluginResult<PluginMeta>;
    
    /// 调用插件（同步）
    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream>;
    
    /// 获取插件能力列表
    fn capabilities(&self) -> Vec<&'static str>;
    
    /// 获取可用工具列表
    fn available_tools(&self) -> Vec<PluginMeta>;
}
```

**关键设计**：
- `path` 参数支持分形嵌套路由
- 返回 `InvokeStream` 支持同步和流式两种模式
- 能力路由 (`@capability`) 支持动态查找

### 2. PluginFactory Trait

插件工厂负责创建插件实例：

```rust
#[async_trait::async_trait]
pub trait PluginFactory: Send + Sync {
    fn meta(&self) -> PluginMeta;
    fn create(&self, parent: Option<Weak<dyn Plugin>>, config: Option<&Value>) -> Arc<dyn Plugin>;
}
```

**关键设计**：
- `parent` 使用 `Weak` 引用避免循环引用
- `config` 支持运行时配置注入
- 每个插件类型对应一个工厂实现

### 3. PluginFactoryRegistry

全局单例注册表：

```rust
impl PluginFactoryRegistry {
    pub fn init();                          // 初始化（仅调用一次）
    pub fn global() -> &'static Self;       // 获取全局实例
    pub fn register(&self, factory: Arc<dyn PluginFactory>);
    pub fn get(&self, name: &str) -> Option<Arc<dyn PluginFactory>>;
    pub fn list(&self) -> Vec<Arc<dyn PluginFactory>>;
}
```

### 4. EventSender Trait (依赖注入)

平台无关的事件发送机制：

```rust
pub trait EventSender: Send + Sync {
    fn emit(&self, event_name: &str, payload: serde_json::Value) -> Result<(), String>;
}

pub struct OptionalEventSender {
    sender: Option<Arc<dyn EventSender>>,
}
```

**使用场景**：
- Explorer 插件发送文件变化事件
- 其他插件需要通知宿主时

**平台实现示例**（Tauri）：

```rust
pub struct TauriEventSender {
    app_handle: tauri::AppHandle,
}

impl EventSender for TauriEventSender {
    fn emit(&self, event_name: &str, payload: serde_json::Value) -> Result<(), String> {
        self.app_handle.emit(event_name, &payload)
            .map_err(|e| format!("Failed to emit event: {}", e))
    }
}
```

## 初始化流程

### 统一初始化函数

```rust
pub fn create_root_plugin(event_sender: OptionalEventSender) -> Arc<dyn Plugin> {
    // 1. 初始化全局注册表
    PluginFactoryRegistry::init();
    let registry = PluginFactoryRegistry::global();

    // 2. 注册所有内置插件工厂（13+ 个）
    registry.register(Arc::new(WorkFactory::new()));
    registry.register(Arc::new(NoteFactory::new()));
    registry.register(Arc::new(SettingFactory::new()));
    registry.register(Arc::new(ExplorerFactory::new(event_sender.clone())));
    // ... 其他插件

    // 3. 创建并返回 root plugin
    HomeFactory::new().create(None, None)
}
```

### 宿主应用集成

宿主应用只需：
1. 实现平台特定的 `EventSender`（可选）
2. 调用 `create_root_plugin()` 获得完整插件系统
3. 通过三命令接口与插件系统交互

## 三命令接口

所有插件能力通过三个标准命令访问：

| 命令 | 用途 | 返回类型 |
|------|------|----------|
| `meta` | 获取插件元数据 | `PluginMeta` |
| `invoke` | 同步调用插件 | `Vec<StreamChunk>` |
| `stream` | 流式调用插件 | 通过事件推送 `StreamChunk` |

**路径寻址**：
- `""` 或 `"root"` - 访问根插件
- `"work"` - 访问 work 插件
- `"agent/chat"` - 访问 agent 的 chat 子插件
- `"agent/tools/file_read"` - 访问更深层嵌套

## 分形嵌套示例

```
home (root)
├── work
├── note
├── setting
├── explorer
└── agent
    ├── chat
    ├── tools
    │   ├── file_read
    │   ├── file_write
    │   └── shell
    ├── memory
    ├── session
    ├── openai
    └── telegram
```

每个节点都是 `Arc<dyn Plugin>`，通过 path 参数逐级路由。

## 数据流

### 同步调用 (invoke)

```
Frontend ──invoke("agent/chat", input)──> Tauri Command
                                              │
                                              ▼
                                         AppState.root
                                              │
                                              ▼
                                    home.invoke("agent/chat", input)
                                              │
                                    路由到 agent
                                              │
                                    agent.invoke("chat", input)
                                              │
                                    chat 处理并返回结果
                                              │
                                         StreamChunk
                                              │
                                              ▼
Frontend <─────────────────────────────── 返回结果
```

### 流式调用 (stream)

```
Frontend ──stream("agent/chat", input, event_id)──> Tauri Command
                                                        │
                                                        ▼
                                                   创建流
                                                        │
                                                        ▼
                                              逐 chunk 处理
                                                        │
                                              app.emit(event_id, chunk)
                                                        │
                                                        ▼
Frontend <────────────────────────────────── 接收事件
```

## 扩展指南

### 添加新插件

1. **创建插件实现**：

```rust
// plugins/my_plugin/plugin.rs
pub struct MyPlugin {
    meta: PluginMeta,
}

impl Plugin for MyPlugin {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path.is_empty() {
            Ok(self.meta.clone())
        } else {
            Err(PluginError::NotFound(path.to_string()))
        }
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        // 处理调用
        Ok(InvokeStream::single(json!({ "success": true })))
    }
}
```

2. **创建工厂**：

```rust
// plugins/my_plugin/factory.rs
pub struct MyFactory;

impl PluginFactory for MyFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "my_plugin".to_string(),
            description: "My custom plugin".to_string(),
            version: "0.1.0".to_string(),
            input: None,
            output: None,
            author: Some("Your Name".to_string()),
        }
    }

    fn create(&self, parent: Option<Weak<dyn Plugin>>, config: Option<&Value>) -> Arc<dyn Plugin> {
        Arc::new(MyPlugin::new(parent, config))
    }
}
```

3. **注册到 init.rs**：

```rust
// symbio/src/init.rs
pub fn create_root_plugin(event_sender: OptionalEventSender) -> Arc<dyn Plugin> {
    // ...
    registry.register(Arc::new(MyFactory::new()));
    // ...
}
```

4. **导出工厂**：

```rust
// symbio/src/plugins/mod.rs
pub mod my_plugin;
pub use my_plugin::MyFactory;
```

## 设计原则

1. **插件平等**：所有插件实现相同接口，root 插件无特殊待遇
2. **依赖注入**：外部依赖通过构造函数注入，不在插件内部创建
3. **弱引用父节点**：避免循环引用导致的内存泄漏
4. **配置分离**：插件配置通过工厂传入，运行时可动态更新
5. **能力路由**：支持通过 `@capability` 查找支持特定能力的插件

## 技术栈

- **Rust**: 核心库和插件系统
- **Tauri 2.x**: 桌面应用框架
- **Vue 3 + TypeScript**: 前端 UI
- **tokio**: 异步运行时
- **serde**: 序列化/反序列化

## 编译配置

### symbio 库

```toml
# symbio/Cargo.toml
[package]
name = "symbio"
version = "0.1.0"
edition = "2021"

[lints.rust]
unused_imports = "warn"
unused_variables = "warn"
dead_code = "allow"  # 开发阶段允许未使用代码
```

### Tauri 应用

```toml
# tauri/src-tauri/Cargo.toml
[package]
name = "symbio-tauri"
version = "0.1.5"
edition = "2021"

[dependencies]
symbio = { path = "../../symbio" }
tauri = { version = "2.0", features = [] }
# ... 其他 Tauri 特定依赖
```
