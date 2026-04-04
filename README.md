# Symbio - 分形插件系统

基于 Tauri (Vue 3 + Rust) 构建的插件系统应用。

## 核心设计理念

### 1. 三命令接口
系统只提供三个核心 Tauri 命令：
- **`meta`**: 通过路径获取插件的 `PluginMeta` 信息
- **`invoke`**: 同步调用插件
- **`stream`**: 流式调用插件

### 2. 路径寻址
所有命令都通过路径来定位插件：
- 空路径：访问 root 插件
- `["plugin_name"]`：访问指定插件
- `["plugin_name", "child_name", ...]`：逐级访问子插件（分形模式）

### 3. 插件平等
- 所有插件实现统一的 `Plugin` 接口
- Root Agent 也是 `Arc<dyn Plugin>`，无特殊概念

### 4. 全局工厂注册表
- `PluginFactoryRegistry` 是全局单例
- 每个插件有自己的 `PluginFactory` 实现
- 插件主动注册自己的工厂到全局注册表

### 5. 分形嵌套
- Agent 通过 `AgentFactory` 创建，支持嵌套
- Agent 可以包含 Agent 作为子插件

## 目录结构

```
src-tauri/src/
├── core/
│   ├── mod.rs
│   ├── traits.rs           # Plugin/PluginFactory Trait
│   ├── types.rs            # 核心类型
│   └── registry.rs         # PluginFactoryRegistry (全局单例)
├── plugins/
│   ├── agent/
│   │   ├── mod.rs          # Agent 插件实现
│   │   ├── factory.rs      # AgentFactory
│   │   ├── add.rs          # 添加子插件
│   │   ├── list.rs         # 列出子插件
│   │   └── remove.rs       # 删除子插件
│   ├── echo/
│   ├── calculator/
│   └── formatter/
├── commands.rs             # 三个核心命令：meta/invoke/stream
└── main.rs
```

## 核心接口

### Plugin Trait
```rust
#[async_trait::async_trait]
pub trait Plugin: Send + Sync {
    fn meta(&self) -> PluginMeta;
    fn plugin(&self, path: &[String]) -> Option<Arc<dyn Plugin>>;
    async fn invoke(&self, input: Value) -> PluginResult<Value>;
    async fn stream(&self, input: Value) -> PluginResult<Vec<StreamChunk>>;
}
```

### PluginFactory Trait
```rust
#[async_trait::async_trait]
pub trait PluginFactory: Send + Sync {
    fn meta(&self) -> PluginMeta;
    fn create(&self, parent: Option<&dyn Plugin>, config: Option<&Value>) -> Arc<dyn Plugin>;
}
```

## API 使用示例

### 1. 获取插件元数据
```typescript
// 获取 root 元数据
const rootMeta = await invoke('meta', { path: [] })

// 获取指定插件元数据
const echoMeta = await invoke('meta', { path: ['echo'] })
```

### 2. 同步调用插件
```typescript
// 调用 echo 插件
const result = await invoke('invoke', {
  path: ['echo'],
  input: { message: 'Hello' }
})
```

### 3. 流式调用插件
```typescript
// 流式调用 formatter 插件
const chunks = await invoke('stream', {
  path: ['formatter'],
  input: { text: 'Hello World' }
})
```

## 插件实现示例

```rust
// ==================== Plugin ====================
pub struct EchoPlugin { meta: PluginMeta }

impl EchoPlugin {
    pub fn new() -> Self { /* ... */ }
}

#[async_trait::async_trait]
impl Plugin for EchoPlugin {
    fn meta(&self) -> PluginMeta { self.meta.clone() }
    
    fn plugin(&self, _path: &[String]) -> Option<Arc<dyn Plugin>> {
        None
    }
    
    async fn invoke(&self, input: Value) -> PluginResult<Value> {
        Ok(input)
    }
}

// ==================== Factory ====================
pub struct EchoFactory;

#[async_trait::async_trait]
impl PluginFactory for EchoFactory {
    fn meta(&self) -> PluginMeta { /* ... */ }
    
    fn create(&self, _parent: Option<&dyn Plugin>, _config: Option<&Value>) -> Arc<dyn Plugin> {
        Arc::new(EchoPlugin::new())
    }
}
```

## 注册工厂

```rust
fn main() {
    // 初始化全局注册表
    PluginFactoryRegistry::init();
    let registry = PluginFactoryRegistry::global();

    // 插件主动注册自己的工厂
    registry.register(Arc::new(EchoFactory::new()));
    registry.register(Arc::new(CalculatorFactory::new()));
    registry.register(Arc::new(FormatterFactory::new()));
    registry.register(Arc::new(AgentFactory::new()));

    // 使用 AgentFactory 创建 root agent
    let root: Arc<dyn Plugin> = registry
        .list()
        .into_iter()
        .find(|f| f.meta().name == "agent")
        .expect("AgentFactory should be registered")
        .create(None, None);
    // ...
}
```

## 开发指南

```bash
# 安装依赖
npm install

# 开发模式
npm run tauri dev

# 构建
npm run tauri build
```

## 许可证

MIT
