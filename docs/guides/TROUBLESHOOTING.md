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
INFO trace_id=abc123 path=session/chat/send Start routing
DEBUG trace_id=abc123 path=session/chat/send Routing finished
```

**`trace_id` 只回答「哪几次请求属于同一条链」，不回答「谁发的、为什么发」**——
而同一个路由名常有多个调用方。例如一轮会话里出现两次 `vdfs/stat`：一次是
「实时面缺基线补读」，一次是「无载荷资源信号分辨删除」，留痕里一字不差，
只能从时间戳反推。

因此回读类请求（`vdfs/list` / `stat` / `read`）还带一个**来源**，取值是闭集
（`services/readback.ts` 的 `READBACK_REASON`），由前端**必填**给出：

```
INFO trace_id=abc123 origin=missing-baseline path=vdfs/stat Start routing
INFO trace_id=abc123 origin=vdfs-browser path=vdfs/list Start routing
```

| `origin` | 含义 | 正常频次 |
|---|---|---|
| `bootstrap` | 引导：解析根锚点 / 挂载点 / 转写段 | 启动期各一次，之后走缓存 |
| `missing-baseline` | 实时面缺基线：状态帧到达，本端没有该节点 | 偶发（首帧丢失 / 订阅晚于节点出现） |
| `identity-unknown` | 实时面身份未知：窄增量帧到达，本端没有该节点 | 偶发 |
| `resource-signal` | 无载荷的会话节点变更（自动命名 / metadata / 删除） | 每次自动命名一次 |
| `list-refresh` | 会话清单整表重拉 | 打开界面 / 资源变更后的防抖收敛 |
| `vdfs-browser` | VDFS 浏览器的导航 / 分页 / 选中 / 手动刷新 | 用户动作；浏览器停在被写入的目录时随变更收敛 |
| `transcript-load` | 打开会话时读整份转写 | 每次打开会话一次 |

缺省打 `-`：**没给来源就说明调用方不是回读类动词**，这也是一条信息。判定口径
是「一轮里同一个 `origin` 出现几次」而不是「出现了几次 `vdfs/stat`」——前者才
对得上设计意图。

### 先确认「这段日志由哪份源码产出」，再去读代码

日志里出现一条当前源码中**不存在**的行（或反过来，某条新加的日志始终不出现）时，
第一反应往往是去源码里找「是不是没接上」——而真相可能是**跑的是过期产物**，排查
方向从第一步就错了。所以这一步要**先做**。

壳启动时会把自己的**构建指纹**打进日志首行：

```
INFO symbio_tauri: Symbio shell starting build=6ea35239b585b180… version="0.1.13"
```

与当前源码的指纹一比即知：

```bash
node scripts/tauri-binary.mjs --print    # 当前源码的指纹
node scripts/tauri-binary.mjs --check    # 本机产物是否对应当前源码（不可信则退出码 1）
```

| 日志里的 `build` | 结论 |
|---|---|
| 与 `--print` **一致** | 日志确实来自眼前这份源码，可以放心去读代码 |
| 与 `--print` **不一致** | 先重建再复现，别在过期产物上找原因 |
| `unknown` | 产物没带构建戳（打包分发、或直接用 `cargo build`）⇒ **来源不可判断**，请用 `npm run tauri dev` 重新构建后复现 |

指纹覆盖壳自身的源码 / 权限集 / 清单，以及**整棵 `symbio/src` 插件树**——壳把它
编译进去，插件那几类日志（`[session INFO]` / `[model INFO]` / `[Tool]`）正是从那里
来的。`tauri/src`（前端）**不在**其中：dev 下它由 Vite 现服、release 下由同一条命令
先行构建，所以「前端新不新鲜」不是这个指纹回答的问题。

戳写在产物旁边（`tauri/src-tauri/target/<profile>/`），由 `npm run tauri dev|build`
的 `before*Command` 在**构建前**写入。判据与实现见 `scripts/tauri-binary.mjs`。

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
   `<根>/<插件>/PLUGIN.yml`——两条路写的是**同一个文件**
4. 配置改完**不需要重启**（`PluginConfigFile::apply` 落盘后广播）；只有少数副作用
   （如网关重建监听）由插件自己在写完后处理

### 问题：API Key 无效

**症状**：`MODEL_AUTH_ERROR`

**排查步骤**：
1. 确认 API Key 未过期
2. 检查 `api_protocol` 与 Key 匹配
3. 确认 `api_base` 正确 (使用代理时)

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
- 嵌入模型加载（`ort` / ONNX 首次加载较慢）

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
