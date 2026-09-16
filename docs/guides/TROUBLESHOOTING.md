# Symbio 故障排查指南

> **文档类型：操作指南** — 常见问题与解决方案。

## 调试工具

### 启用调试日志

```bash
# Rust 后端 (所有模块)
RUST_LOG=debug cargo run

# Rust 后端 (仅 symbio)
RUST_LOG=symbio=debug cargo run

# 前端
VITE_LOG_LEVEL=debug npm run dev
```

### 追踪请求链

每个请求的 `metadata` 应包含 `trace_id`：

```
INFO trace_id=abc123 path=agent/chat Start routing
DEBUG trace_id=abc123 path=agent/chat Routing finished
```

---

## 编译问题

### 问题：链接时间过长 (20+ 分钟)

**原因**：测试二进制含 ort/ONNX，完整调试信息使 MSVC 链接缓慢。

**解决**：已在 `Cargo.toml` 配置：

```toml
[profile.test]
debug = "line-tables-only"
```

### 问题：`submit_object_creator!` 宏报错

**症状**：`home plugin creator not found`

**原因**：插件未正确注册。

**检查清单**：
1. `plugin.rs` 末尾是否调用了 `submit_object_creator!`
2. 插件 ID 是否与配置中的 `plugin_provider` 匹配
3. 是否在 `mod.rs` 中正确导出

### 问题：Clippy 警告

**解决**：

```bash
cd symbio
cargo clippy --lib --tests -- -D warnings
```

常见警告修复：
- `unused_imports`：删除未使用的导入
- `dead_code`：删除未使用的代码或添加 `#[allow(dead_code)]`

---

## 运行时问题

### 问题：路由返回 NOT_FOUND

**症状**：`路由路径不存在: xxx/yyy`

**排查步骤**：
1. 检查路径拼写 (区分大小写)
2. 确认插件目录下有可解析的 `PLUGIN.yml`（且 `plugin_provider` 指向已注册的工厂）
3. 检查插件是否注册到 Composite（容器按目录扫描，不合格的目录会被跳过并点名）

**调试方法**：

```bash
# 查询根节点拓扑
curl -X POST ... -d '{"path": "_root"}'
```

### 问题：流式会话无响应

**症状**：调用 `session/chat/send` 后无数据返回

**排查步骤**：
1. 检查 API Key 是否配置
2. 检查网络连接
3. 查看后端日志是否有 LLM API 错误
4. 确认 `PluginChannel` 是否正确传递

### 问题：前端无法连接后端

**症状**：Tauri 应用白屏或报错

**排查步骤**：
1. 确认 `npm install` 已执行
2. 检查 `tauri.conf.json` 配置
3. 查看浏览器控制台 (F12) 是否有错误
4. 检查 `src-tauri` 是否编译成功

---

## 配置问题

### 问题：配置不生效

**排查步骤**：
1. 确认改的是**插件自己的**配置文件：`~/.symbio/<插件>/PLUGIN.yml`
   （系统级插件 `home` 在 `~/.symbio/PLUGIN.yml`）
2. 检查 YAML 格式 (缩进、冒号后空格)；两个身份字段 `plugin_provider` /
   `plugin_name` 不要写进配置字段里（它们由 `PluginDir` 自动剥离 / 补回）
3. 确认写入路径对：前端「设置」页点开对应条目，或直接 `vdfs/write`
   `.vdfs/<插件>/PLUGIN.yml`——两条路写的是**同一个文件**
4. 配置改完**不需要重启**（`ConfigFile::apply` 落盘后广播）；只有少数副作用
   （如网关重建监听）由插件自己在写完后处理
5. 若刚从旧版本升级：旧 `<homedir>/config.yaml` 已一次性迁移并改名
   `config.yaml.migrated`——查历史值看那个留档文件

### 问题：API Key 无效

**症状**：`MODEL_AUTH_ERROR`

**排查步骤**：
1. 确认 API Key 未过期
2. 检查 `provider_type` 与 Key 匹配
3. 确认 `base_url` 正确 (使用代理时)

---

## 性能问题

### 问题：首次加载慢

**原因**：`inventory` 静态注册需要收集所有插件构造函数。

**优化**：已内置，无需额外处理。

### 问题：内存占用高

**排查方法**：

```bash
# 检查内存使用
cargo build --release
valgrind --tool=massif ./target/release/symbio
```

**常见原因**：
- 大量 Session 未释放 (检查连接超时配置)
- 嵌入模型加载 (fastembed 首次加载较慢)

---

## 测试问题

### 问题：测试卡死

**原因**：多个 cargo 进程互抢 target 目录锁。

**解决**：

```bash
# 串行测试
cargo test --lib -- --test-threads=1

# 或等待其他构建完成
```

### 问题：测试失败

**调试方法**：

```bash
# 显示测试输出
cargo test --lib -- --nocapture

# 运行单个测试
cargo test test_name -- --nocapture
```

---

## 获取帮助

如果以上方法无法解决问题：

1. 收集信息：
   - 操作系统版本
   - Rust 版本 (`rustc --version`)
   - Node 版本 (`node --version`)
   - 相关日志 (`RUST_LOG=debug`)

2. 提交 Issue：
   - 使用 GitHub Issues 模板
   - 包含复现步骤
   - 附上关键日志

---

> **维护原则**：每个新发现的常见问题都应添加到此文档。
