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

## 目标形状

Symbio 有一个运行时实现和三种交付形态：CLI、Tauri 内嵌同一个 `RuntimeHost`，独立
`Server` 通过 HTTP/WS 把同一个运行时交给浏览器。浏览器复用 Tauri 的 Vue 构建产物，
不维护第二套前端。Server 是 `symbio-cli serve` 子命令，不是第二个可执行文件；CLI 默认
模式仍是终端客户端。

控制面持有 homedir（rootdir）、会话目录、审批、心跳与文件系统目录浏览。每个会话有且仅有
一个活动执行 worker；默认落在宿主独立进程中，隔离模式把同一个 worker 放进容器。沙箱
改变执行位置，不改变授权、审批或审计语义。

## 关键决策

### D-535 传输机制与接入策略分层

`symbio-http` 只拥有 HTTP/1.1、WebSocket、SSE、鉴权、限制与连接生命周期；接入方实现
`Ingress`，决定公开端点、调用者身份、路由与策略。Gateway 保留 Model 调用者、只读白名单、
幂等与自身配置策略；Server 使用 Manager 调用者并承载宿主控制 API。CLI / Tauri 通过
`symbio-server::RuntimeHost` 嵌入同一运行时，不各自复制装配或上下文规则。Server 只作为
`serve` 子命令交付，与 CLI 共用一个二进制和参数入口。`symbio-http` 属于插件可依赖的
通用线路设施，与 `symbio-providers` 同档：它只依赖 `symbio-plugin-api`、内核的线路语法
与第三方库，不依赖任何插件或应用门面；`PluginPayloadWire` 也由该设施转出，插件门面不再为它保留
只有宿主 IPC 在用的转发。

### D-536 浏览器只有 HTTP/WS 出口

浏览器默认同源 HTTP/WS，可显式连接另一台 Server；任何失败都不回退 Tauri IPC。WebSocket
令牌只走 `Sec-WebSocket-Protocol: bearer.<token>`，不出现在 URL。免令牌只适用于回环，
且仍校验 `Host` 与 `Origin`；显式跨源必须同时有令牌和一个精确 Origin，不接受通配。
调用入口只接受 JSON，目录选择是受鉴权的宿主控制 API，只列目录，不是物理 VDFS，也不读取
任意文件；返回路径属于 Server 文件系统。homedir 是系统数据根，workdir 是会话自有且可选的
工作区，两者不能互相取代。homedir 重载在运行时写锁内取消并排空其他实时连接，发起重载的
WebSocket 保留自身直至响应结束，避免连接自等待。

### D-537 一会话一 worker，沙箱只决定落点

一个会话的活动执行体始终是一个独立 worker。`host` 是默认进程落点，不声称隔离；
`container` 是同一 worker 的隔离落点。VM 后续复用同一启动边界。控制面不得再在 Server
进程内直接执行会话工具，也不增加第三种与 host / container 并列的 process 模式。
生产绑定已从 `RuntimeHost` 统一切换：会话作用域请求经 durable inbox 派发到同一二进制的
隐藏 worker 入口，worker 的 VDFS 事件回传控制面总线；CLI / Tauri / Server 不再各自直调
会话执行路径。worker 进程不启动心跳调度器。

### D-538 worker 身份、持久入口与单写者

`WorkerId = { session_id, epoch, build_id }`。epoch 对请求、事件、审批、子会话流与 homedir
重载统一 fencing；每个会话只有当前 epoch 能改变状态。worker 在 ready 前取得会话 WAL
写锁，运行期直接持有该锁，控制面只读。控制面落盘的 durable inbox 是重启后恢复请求的
唯一入口；会话元数据写操作只能交给 worker owner。心跳仍是控制面单例，不随会话复制。
inbox 的状态转换与 epoch 登记分别在各自跨进程锁内完成：接受序号持久化且唯一；完成回执
保留至会话退役，以支持跨重启的请求去重。

### D-539 子会话经 Supervisor 跨进程汇合

子 Agent 会话同样拥有独立 worker。Supervisor 负责启动、停止、epoch 登记与父子事件 /
审批路由；父 worker 不直接持有子进程对象。跨进程消息必须携带 session / epoch / build
身份，旧 epoch 的迟到帧一律丢弃。父子 worker 的路由与身份校验协议已经落地；子会话的
生产 spawn 仍待接入，当前子会话由父 worker 执行，D-539 尚未完全兑现。

### D-540 容器默认拒绝越界能力

容器镜像按 digest 固定，只挂会话目录与显式选择的 workdir；不挂完整 homedir、Docker
socket、宿主网络或任意宿主路径。rootfs 只读，临时目录用 tmpfs，以非 root 运行并丢弃
capabilities，启用 `no-new-privileges`、seccomp 与 CPU / 内存 / PID 限制。Docker 不可用
或配置无效时 fail closed。模型与 MCP 凭证只能经受控 broker / proxy 注入；没有该机制时
相关能力必须显式禁用，不能把宿主凭证目录挂进容器。

### D-541 崩溃恢复不重放外部副作用

持久 inbox 只保证已接受请求不丢失，不保证工具恰好执行一次。崩溃后未完成的外部动作
进入待核对状态，由幂等键、外部系统查询或用户确认收口；不得自动重放付款、发布、删除
等不可逆动作。核对只能写入显式的「确认完成 / 放弃」终态回执，不能把旧认领重新排回
pending。WAL 恢复与 worker ready 必须先完成，之后才接受新请求。

### D-542 切换绑定与删除旧执行路径同提交

per-session worker 成为唯一生产执行路径时，切换绑定与删除 Server / CLI / Tauri 的
进程内会话执行路径必须发生在同一提交。不得保留双实现、双写或靠配置在两套语义间切换。
顶层会话已完成这一步：切换点在 `RuntimeHost`，CLI 不再直接打开会话 WAL 或调用
`session/project`。子会话 spawn 与容器落点仍属后续工作，因此整体迁移保持 `partial`。
本条沿用 D-245 的迁移纪律。

## 验收边界

- Server 与 Gateway 共用传输实现，但调用者身份、公开面与路由策略不共享。
- 浏览器在 Tauri IPC 不可用时仍能完成连接、目录选择与会话操作；IPC 存在时也不得静默
  切回 native。
- 同一会话出现两个可写 epoch、worker 未持 WAL 锁即 ready、旧 epoch 事件被消费，均判失败。
- 容器缺 Docker、镜像摘要、受限挂载或凭证通道时，隔离模式不得退化为宿主执行。
- 旧执行路径删除前，不声称已完成一会话一进程迁移。
