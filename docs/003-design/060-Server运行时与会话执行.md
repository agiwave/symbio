---
kind: design
id: DE-060-Server运行时与会话执行
title: Server 运行时与会话执行
status: active
created: 2026-11-24
impl: partial
deps: [DE-002-架构总览, DE-005-协议规范, DE-058-沙箱机制]
---

# Server 运行时与会话执行

## 统一运行时与入站边界

- CLI、Tauri 内嵌与 `symbio-cli serve` 共用一个 `RuntimeHost`，不维护第二套会话或前端。
- `RuntimeHost` 在构造插件树前注册统一分发桥。控制面先处理会话删除，再由 Supervisor 派发会话动作，
  未命中才回落到宿主插件 `route`。
- `symbio-http` 只管 HTTP/1.1、WS、SSE、鉴权、限制与连接生命周期；Ingress 决定公开端点、身份与策略。
- 浏览器只走 HTTP/WS，失败不回退 Tauri IPC。目录接口只列目录，不读取任意文件。
- WS 令牌从 `Sec-WebSocket-Protocol` 的 `bearer.<token>` 读取，响应不回显。显式跨源 Origin
  必须同时提供访问令牌；Host/Origin 与回环同源条件先通过校验。
- 转播标记归 runtime/Server；SDK 只保留发布端口。

## 会话执行体

- 一会话只有一个活动执行体。默认 host 模式与宿主同进程，container 模式为独立进程。
- `WorkerSupervisor` 统一管理 `WorkerState`、`WorkerId`、durable inbox、epoch 与 broker 登记。
- `session/stats`、`session/changes` 与 VDFS read/list/stat/tree/search 留在控制面，避免长驻读进程占用 WAL 写令牌。
- 控制面不直接执行会话工具；执行体不启动 heartbeat 或 gateway 入站。
- `WorkerId = { session_id, epoch, build_id }`。epoch 同时 fencing 请求、事件、审批、子会话流与 homedir reload。
- 本地执行体与宿主共享事件总线，事件直接到达，不再次回灌；跨 worker 通道满时丢弃当前帧并补 resync，
  不注销订阅、不阻塞慢目标。
- 宿主执行绑定随请求快照经 `SessionFactSource`、`DriveTarget`、`FactWorld` 显式传递，
  不依赖调用栈或全局可变状态。

## 持久化、恢复与取消

- 执行体 ready 前取得 WAL 写锁；控制面只读。durable inbox 是重启后请求恢复的唯一入口。
- 恢复只重放未认领 pending。已认领但结果未知的请求标记 `Abandoned`，不可逆动作不自动重放。
- WAL 恢复、终态回执落盘与 worker ready 完成后才接受新请求。
- 收件箱判空、占轮和出队在同一会话状态写锁内。首条 `user.message` 的局部 append 结果决定回滚，
  不看全局 WAL head。
- 首条未落则恢复原槽并 FIFO 回队；已落但装配失败则不重放，补失败终态并投影。
- 同一锁内先登记中止信号再公开忙态；终态 VDFS 帧必须先于落闲与唤醒。
- 派发前取消不中止既有子会话；派发后取消经受权 VDFS abort。停机使在途启动失败，
  future 丢弃必须撤销 reply 槽和未认领 pending。
- 恢复先等原轮收束并完成投影，再原子重接原轮根与中止信号。

## 子会话与删除

- 子会话由 Supervisor 校验 epoch 与持久父子归属，经 `session.subagent` 请求级端口和 durable inbox
  启动或复用；父执行体不持有子对象。
- 删除会话先 `stop_session_tree(session_id)`，再由控制面执行 VDFS 删除。worker 不能在 durable 请求中自删。
- 路由以显式 VDFS 资源路径为准；metadata 会话上下文不能覆盖路径叶节点，嵌套子会话按逻辑地址叶节点路由。
- `answer/approve/reject` 落事实前重建可信来源主体、Agent 权限与 VDFS 上下文；失败不落任何答复或审批事实。

## 容器与出口

- container 镜像固定为 `repository@sha256:<64 hex>`，只支持 Docker。默认 host；container 必须提供
  `SYMBIO_WORKER_IMAGE`，缺失或配置无效时失败关闭。
- 启动参数包含 `--network none`、`--read-only`、`--cap-drop ALL`、`--security-opt no-new-privileges`、
  pids/cpu/memory 限制、`/tmp` tmpfs 与非 root 用户。
- 挂载面只包含只读运行时投影、可写会话目录与显式工作目录。worker 保持原绝对路径投影，
  不挂会话、模型或 MCP 原配置、符号链接与宿主凭证。
- 显式 workdir 不得覆盖 homedir、会话树或模型/MCP 目录，树内不得出现凭证文件。
- 模型与 MCP 凭据只经 broker/proxy；没有受控出口时禁用相关能力。
- Supervisor 只接受启动投影登记的精确目标，剥离 worker 凭据头，以宿主可信配置覆盖，
  并过滤逐跳响应头与 Location。broker 禁用自动重定向，限制 connect/read/request 超时。
- 错误不泄露真实上游 URL；断流表现为传输失败。慢头部、请求体和响应写入均有时限；
  断连、取消、epoch 替换与 EOF 都取消上游。

## 实现入口

- 运行时与路由顺序：`crates/symbio-runtime/src/runtime.rs`。
- Supervisor、恢复与分发：`crates/symbio-runtime/src/worker/supervisor.rs`。
- inbox、epoch 与进程入口：`crates/symbio-runtime/src/worker/`。
- 容器投影：`crates/symbio-runtime/src/worker/container.rs`。
- 出口：`crates/symbio-runtime/src/worker/broker.rs`。
- HTTP 与 WS：`crates/symbio-http/src/`。
- 轮级装配与收件箱：`crates/symbio-plugin-session/src/plugin/round.rs`。
