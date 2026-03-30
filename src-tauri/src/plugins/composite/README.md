# Composite 组合插件使用指南

## 概述

`Composite` 是一个通用组合插件，可以动态包含多个子插件，通过配置参数创建子插件实例，并提供子插件的管理功能。

## 核心特性

1. **多子插件支持** - 可以包含任意数量的子插件
2. **工厂创建** - 通过工厂注册表动态创建子插件实例
3. **子插件管理** - 提供列表/添加/删除功能
4. **标准属性** - 支持 name, title, description, version, author 等属性
5. **自动路由** - meta/invoke 函数自动路由到子插件

## 文件结构

```
src/plugins/composite/
├── mod.rs      # 模块导出
├── plugin.rs   # CompositePlugin 实现
└── factory.rs  # CompositeFactory 实现
```

## 使用方式

### 1. 基本使用（通过配置创建）

```rust
use crate::plugins::composite::{CompositeFactory, CompositeFactoryConfig, CompositeMetaConfig};
use serde_json::json;

// 创建元数据配置
let meta_config = CompositeMetaConfig {
    name: "my_composite".to_string(),
    title: "我的组合插件".to_string(),
    description: "自定义组合插件".to_string(),
    version: "1.0.0".to_string(),
    author: Some("Your Name".to_string()),
};

// 创建工厂配置
let config = CompositeFactoryConfig {
    meta: meta_config,
    sub_plugins: vec![
        SubPluginConfig {
            name: "echo_plugin".to_string(),
            factory: "echo".to_string(),  // 使用已注册的工厂名称
            config: Some(json!({"message": "Hello"})),
        },
        SubPluginConfig {
            name: "calc_plugin".to_string(),
            factory: "calculator".to_string(),
            config: None,
        },
    ],
};

// 创建工厂并生成插件实例
let factory = CompositeFactory::new(config);
let plugin = factory.create(None, None);
```

### 2. 链式调用（Builder 模式）

```rust
use crate::plugins::composite::CompositeFactory;

let factory = CompositeFactory::with_defaults()
    .add_sub_plugin_config(
        "echo_plugin".to_string(),
        "echo".to_string(),
        Some(json!({"message": "Hello"}))
    )
    .add_sub_plugin_config(
        "calc_plugin".to_string(),
        "calculator".to_string(),
        None
    );

let plugin = factory.create(None, None);
```

### 3. 通过 meta 命令获取元数据

```typescript
// 获取组合插件自身元数据
const meta = await invoke('meta', { path: '' });
// 返回：{ name: "my_composite", description: "...", ... }

// 获取子插件元数据
const echoMeta = await invoke('meta', { path: 'echo_plugin' });
const calcMeta = await invoke('meta', { path: 'calc_plugin' });
```

### 4. 通过 invoke 命令管理子插件

```typescript
// 列出所有子插件
const result = await invoke('invoke', {
    path: '',
    input: { action: 'list' }
});
// 返回：{ success: true, data: { plugins: [...] } }

// 调用子插件
const echoResult = await invoke('invoke', {
    path: '',
    input: {
        action: 'invoke',
        plugin_name: 'echo_plugin',
        input: { message: 'Hello World' }
    }
});

// 直接通过路径调用子插件（推荐）
const directResult = await invoke('invoke', {
    path: 'echo_plugin',
    input: { message: 'Hello World' }
});
```

## API 参考

### CompositeMetaConfig

| 字段 | 类型 | 说明 |
|------|------|------|
| name | String | 插件名称 |
| title | String | 插件标题（用于 UI 显示） |
| description | String | 插件描述 |
| version | String | 版本号 |
| author | Option<String> | 作者信息 |

### SubPluginConfig

| 字段 | 类型 | 说明 |
|------|------|------|
| name | String | 子插件在组合中的标识 |
| factory | String | 工厂名称（必须是已注册的工厂） |
| config | Option<Value> | 插件配置（传递给工厂） |

### 管理操作

| action | 说明 | 参数 |
|--------|------|------|
| list | 列出所有子插件 | 无 |
| add | 添加子插件 | factory, config |
| remove | 移除子插件 | plugin_name |
| invoke | 调用子插件 | plugin_name, input |

## 路由机制

Composite 插件支持通过路径自动路由到子插件：

- `path = ""` - 操作 Composite 自身（管理操作）
- `path = "echo_plugin"` - 操作名为 echo_plugin 的子插件
- `path = "echo_plugin/nested"` - 如果子插件支持嵌套，继续路由

## 示例：创建工作台组合插件

```rust
// 在 main.rs 中注册自定义组合插件
let work_composite = CompositeFactory::new(CompositeFactoryConfig {
    meta: CompositeMetaConfig {
        name: "workspace".to_string(),
        title: "工作空间".to_string(),
        description: "集成工作工具的组合插件".to_string(),
        version: "1.0.0".to_string(),
        author: Some("Symbio Team".to_string()),
    },
    sub_plugins: vec![
        SubPluginConfig {
            name: "formatter".to_string(),
            factory: "formatter".to_string(),
            config: None,
        },
        SubPluginConfig {
            name: "calculator".to_string(),
            factory: "calculator".to_string(),
            config: None,
        },
    ],
});

registry.register(Arc::new(work_composite));
```

## 注意事项

1. **工厂注册** - 子插件的工厂必须在全局注册表中已注册
2. **名称唯一** - 子插件名称在组合内必须唯一
3. **状态管理** - add/remove 操作需要可变引用，建议通过外部管理接口实现
4. **路径格式** - 子插件路径使用 `/` 分隔符支持嵌套路由
