---
kind: guide
id: GU-007-TROUBLESHOOTING
title: Symbio 故障排查指南
status: active
created: 2026-09-07
---

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
INFO trace_id=abc123 path=session/stats Start routing
DEBUG trace_id=abc123 path=session/stats Routing finished
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

### 先确认「这份产物由哪份源码产出」，再去读代码

日志里出现一条当前源码中**不存在**的行（或反过来，某条新加的日志始终不出现）时，
第一反应往往是去源码里找「是不是没接上」——而真相可能是**跑的是过期产物**，排查
方向从第一步就错了。所以这一步要**先做**。

桌面原生窗口壳拉起 `symbio-cli` 二进制（前后端与 UI 都在这一份），窗口只负责显示
二进制里。CLI 侧有构建戳机制：产物旁 `.symbio-cli.build-stamp` 内容是构建输入的
内容指纹，使用前比对当前指纹。

```bash
node scripts/cli-binary.mjs --check  # 只判不建：产物是否对应当前源码（不一致则退出码 1）
node scripts/cli-binary.mjs          # 需要就重建并写戳（等价于 cargo build --release）
```

| 状态 | 结论 |
|---|---|
| 与当前源码一致 | 产物可信，可以放心去读代码 |
| 构建输入已改变 | 先重建再复现，别在过期产物上找原因 |
| 没有构建戳 / 二进制不存在 | 来源不可判断或还没构建，先跑一次 `node scripts/cli-binary.mjs` |

指纹覆盖 `apps/cli/`、`crates/` 整棵插件树、根清单与 `packages/ui/dist`（前端产物由
CLI 的 `build.rs` 编译期内嵌，改了 UI 却复用旧 exe 就等于 serve 出旧页面）。详见
`scripts/cli-binary.mjs` 的文件头。

桌面原生窗口壳（`apps/desktop/src`）不进入插件树，只拉起 CLI serve、等就绪并加载本机地址。
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
1. 插件 `plugin.rs` 末尾是否调用了 `submit_object_creator!`
2. 插件 ID 是否与配置中的 `plugin_provider` 匹配
3. 插件是否由 `crates/symbio-plugin-<id>/` 目录投影进 `symbio` 的依赖行（`plugin-links` 生成段）

### 问题：Clippy 警告

**解决**：

```bash
cargo clippy --all-targets --workspace -- -D warnings
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

### 问题：会话跑起来了但界面上没有输出

**症状**：写入收件箱（`vdfs/write <根>/session/<sid>/inbox`）后被受理，但没有增量正文、
状态卡在「处理中」

**排查步骤**：
1. 检查 API Key 是否配置
2. 检查网络连接
3. 查看后端日志是否有 LLM API 错误
4. 确认**两步订阅都装上了**：`event_bus/subscribe`（总线广播口）+ `vdfs/watch`（provider
 侧的 sink）。只装前者的症状与「模型没有产出」逐字相同——一条变更都收不到，且不报错。
 CLI 上判据是 `SYMBIO_ROUTE_LOG=1`：只有出站请求、没有任何 `[frame]` 行，就是少了一半
 （帧到了但没认出来也会有 `[frame]` 行）。

### 问题：前端无法连接后端

**症状**：桌面原生窗口白屏或报错

**排查步骤**：
1. 确认 `packages/ui` 下 `npm ci && npm run build` 已执行，且 CLI 是在那之后重建的
   （UI 由 CLI 编译期内嵌，改了 UI 不重建 CLI 就会 serve 出旧页面或占位页）
2. 确认窗口壳找到了 CLI：同目录 `symbio-cli` 或 `SYMBIO_CLI` 显式指定的路径存在
3. 查看浏览器控制台 (F12) 是否有错误：前端一律走同源 HTTP/WS，无 IPC 分支
4. 确认 CLI 进程仍在运行：窗口关闭时会终止 CLI；CLI 若先退出，窗口页面会失去后端
5. 单机离线时确认 serve 监听的是回环地址（默认 `127.0.0.1`）且未被其它进程占用

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

**症状**：鉴权失败。Key 含非法字符（非 ASCII / 控制字符）时 `Authorization` 头**不会发出**，
协议适配层只记一条 `plugin_warn!("model", "Invalid characters in … API key")`——日志里搜这句

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

**排查方法**：先用发布档观察常驻内存（桌面交付物实际运行的是 CLI
`target/release/symbio-cli`，原生窗口壳 `symbio-desktop` 本身不含插件树），再换 debug 档
对比——两者差值就是「没被优化掉的那部分」。
需要堆剖析时用平台工具（Linux `valgrind --tool=massif`，Windows 用 Visual Studio 的
「诊断工具」附加到进程）。

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
cargo test --workspace -- --test-threads=1

# 或等待其他构建完成
```

### 问题：测试失败

**调试方法**：

```bash
# 显示测试输出
cargo test --workspace -- --nocapture

# 运行单个测试（过滤名是位置参数）
cargo test <测试名> -- --nocapture
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
