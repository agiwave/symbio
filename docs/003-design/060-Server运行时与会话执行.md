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
会话执行路径。worker 进程不启动心跳调度器，也不拉起 gateway 入站监听——它构造完整插件树只为执行端口，
入站与控制面同端口会让请求随机落到没有分发桥的那棵树上。

### D-538 worker 身份、持久入口与单写者

`WorkerId = { session_id, epoch, build_id }`。epoch 对请求、事件、审批、子会话流与 homedir
重载统一 fencing；每个会话只有当前 epoch 能改变状态。worker 在 ready 前取得会话 WAL
写锁，运行期直接持有该锁，控制面只读。控制面落盘的 durable inbox 是重启后恢复请求的
唯一入口；会话元数据写操作只能交给 worker owner。心跳仍是控制面单例，不随会话复制。
inbox 的状态转换与 epoch 登记分别在各自跨进程锁内完成：接受序号持久化且唯一；完成回执
保留至会话退役，以支持跨重启的请求去重。

### D-539 子会话经 Supervisor 跨进程汇合

子 Agent 会话同样拥有独立 worker。Supervisor 负责启动、停止、epoch 登记与父子事件 /
审批路由；父 worker 不直接持有子进程对象。父 worker 经请求级 `session.subagent` 端口
提交完整的 `PluginMessageWire`，Supervisor 先校验当前 epoch 与持久化父子归属，再启动
或复用子 worker；子会话请求与其响应都经 durable inbox，pipe 身份负责帧来源校验。
跨进程 VDFS 消息使用 `vdfs/root` 解析出的会话逻辑地址，并显式携带 `workdir`，不能把
容器内相对地址当作宿主路由地址。子 worker 就绪后才接受请求，旧 epoch 的迟到帧一律
丢弃。CLI / Tauri / Server 共用这一启动路径，不再保留父 worker 进程内执行子会话的
生产分支。

### D-540 容器默认拒绝越界能力

容器镜像按 digest 固定，只挂会话目录与显式选择的 workdir；不挂完整 homedir、Docker
socket、宿主网络或任意宿主路径。rootfs 只读，临时目录用 tmpfs，以非 root 运行并丢弃
capabilities，启用 `no-new-privileges`、seccomp 与 CPU / 内存 / PID 限制。Docker 不可用
或配置无效时 fail closed。模型与 MCP 凭证只能经受控 broker / proxy 注入；没有该机制时
相关能力必须显式禁用，不能把宿主凭证目录挂进容器。

### D-541 崩溃恢复不重放外部副作用

持久 inbox 只保证已接受请求不丢失，不保证工具恰好执行一次。只有尚未认领的 pending
请求可在新 epoch 重放；已认领但未完成的动作结果未知，生产恢复将其显式记为「放弃」终态，
不得自动重放付款、发布、删除等不可逆动作，也不得把旧认领重新排回 pending。若外部结果
可核对，只能写入显式的「确认完成 / 放弃」终态回执。WAL 恢复、终态回执落盘与 worker
ready 必须先完成，之后才接受新请求。

### D-542 切换绑定与删除旧执行路径同提交

per-session worker 成为唯一生产执行路径时，切换绑定与删除 Server / CLI / Tauri 的
进程内会话执行路径必须发生在同一提交。不得保留双实现、双写或靠配置在两套语义间切换。
顶层会话与子会话均已完成这一步：切换点在 `RuntimeHost` 与 Supervisor，CLI 不再
直接打开会话 WAL 或调用 `session/project`，子会话也没有进程内生产执行分支。容器
落点仍属后续工作，因此整体迁移保持 `partial`。本条沿用 D-245 的迁移纪律。

### D-543 宿主执行绑定随异步事实驱动显式传递

worker 注入的 `session.subagent` 是请求级宿主能力，不属于插件能力收集结果。事实源
`append` 会跨任务边界新建驱动上下文，因此绑定必须由请求快照显式经 `SessionFactSource`、
`DriveTarget` 与 `FactWorld` 传递，不能依赖原调用栈或进程级可变全局状态。能力收集与
宿主装配都在该快照的**新容器**上追加本轮插件端口：既保留宿主预置绑定，也不让多次
收集重复改写同一个容器。绑定只承载执行出口，不提升授权、审批或沙箱策略；子会话
归属、epoch 与路由白名单仍由 Supervisor 独立校验。

### D-544 取消、事件背压与停机都不改变授权语义

子会话启动的取消以 durable 派发为界：派发前只取消启动，不中止既有子会话；派发后
取消等待，并由控制面经同一受权 VDFS 动作入口向子会话发送 `abort`。启动端对中止或
future 丢弃发送 `CancelSubagent`，worker 停机时先失败化所有在途启动，避免父执行体
等待回复形成停机死锁。跨 worker 事件通道满时丢弃当前帧并补送 resync 标记，不注销
订阅，也不让源 reader 等待慢目标。启动端口只弱引用 worker 输出通道，停机释放本地
发送端后 writer 必须观察到 EOF。Windows Server 同时监听 Ctrl+C 与 Ctrl+Break，以便
新进程组触发与 Unix 一致的优雅停机。durable 派发从入队到发送完成都受取消清理保护：
future 在发送完成前丢弃时，同时撤销 reply 槽与尚未认领的 pending 记录，不得把未发送
请求留到下次重启重放。上述机制只管理执行生命周期，不伪造审批、不修改沙箱策略，也不把
沙箱事实写入承诺或义务记账。

### D-545 会话路由按资源节点与生命周期分界

全局 VDFS 管理调用即使携带尚未持久化的临时会话上下文也留在控制面；只有 `session/*`
动作或显式会话节点载荷才派发给 worker。具名会话仅在目标不存在且明确 `create` 时由控制面
创建，既有会话的写入仍归 worker。`session/stats` 与 `session/changes` 始终由控制面回答
（会话不存在时答「有据的空」，存在时读快照），不把「会话不存在」伪装成 worker 路由失败。
只读 VDFS 投影（`read` / `list` / `stat` / `tree` / `search`）同样留在控制面：读方一旦被派给
worker，长驻读进程就会持有 WAL 写令牌，挡住真正要写的进程。显式 VDFS 路径是资源
归属的唯一事实，metadata 的会话上下文可作 IPC 通道名，不得覆盖或否定路径中的叶节点。
显式会话节点只认声明根下的
`session/...` 或相对 `session/...`；其他挂载点与物理绝对路径里的同名片段不改变路由归属。
嵌套子会话按逻辑地址的叶节点
归属路由，不因物理归档在父目录下而误派给父 worker。删除会话本体是控制面生命周期操作：
先停止目标及其整棵 worker 子树，再执行同一 VDFS 删除语义，禁止 worker 在 durable 请求中
自删存储。子会话取消帧必须同时通过发送 worker 的 epoch fencing 与启动 owner 校验；分类放宽
不构成会话授权、审批或沙箱策略的放宽。

### D-546 驱动契约独立成域，不拆类型闭包

D-543 的请求级 `PortBindings` 绑定使 `DriveTarget` 必须引用 `capability`，而 `capability`
既有的效果与可配置声明消费 `plugin`。继续把完整驱动契约住在 `plugin` 会造出
`capability ⇄ plugin` 环。按 D-087「名字自证归属」的判据，驱动描述的是宿主事实落账后的
执行推进，不是能力收集或插件扩展点，故整个 `drive_entry` 类型闭包迁到
`crates/symbio-sdk/src/drive/mod.rs`，公开路径为 `symbio_sdk::drive`。

这是对 D-413 物理归属的细化，不是把机制搬回门面：`DriveEntry` / `DriveTarget`、
两个 `inventory` 注册表、部署根表与全部行为一字不改，`symbio-plugin-api` 仍只转出扩展层
真正消费的 `publish_deployment_root`。`plugin` 不留兼容别名——`drive → plugin` 与
`plugin → drive` 会重新成环。也不拆出 `DriveTarget` 或改写类型路径来隐藏扫描边；前者违背
D-408 / D-413 的类型闭包，后者没有改变真实依赖。`PortBindings` 仍归 `capability`，
驱动域单向消费它，授权、审批与沙箱策略不随本次迁址变化。

### D-547 恢复动作在落事实前重建可信执行上下文

worker 重启不恢复跨请求的来源主体与 Agent 权限槽。`answer` / `approve` / `reject`
在写入答复事实前，必须用本次受权宿主请求的 `source_principal` 和会话持久绑定重新执行
与开轮相同的能力收集、作用域发布与 VDFS provider 装配。来源主体不取自答复载荷或模型
可控字段；绑定 Agent 却未发布作用域时 fail closed，且不得落下任何答复或审批事实。

恢复动作先等进程内原轮收束并完成投影，再原子重接原轮根：登记新的续跑中止信号，并由同一轮收尾面等
驱动空闲后投影终态。worker 重启后同样按这条生命周期规则执行，不得只准备上下文并落答复事实。
恢复不重新选择工具，也不提升端口绑定、审批或沙箱策略。尚未 claim 的请求重放仍按 D-541 裁决；显式恢复
动作只是新的受权请求，不能借此重放未知结果。

### D-548 目录选择按真实出站协议定位

前端运行环境与连接目标是两条独立坐标。目录选择的目标后端由实际 outbound 决定：
browser 一律经 Server 目录 API；embedded 只有在 native 出站时才使用 Tauri dialog；
embedded + HTTP 同样操作 Server 文件系统。是否注入了 Tauri IPC 不能替代出站协议判定，
远端不可达也不回退本机目录 API。Server 目录入口沿用 D-536 的鉴权与只列目录边界。
embedded 的控制面提示（`forceNative` / gateway 路径）同样不得越过已选 Server HTTP；
homedir 切换与会话工作目录选择必须落在同一个连接目标上。

### D-549 轮级执行上下文与占轮原子性

每轮能力收集从可信请求基底 fork 独立的 `round_ctx`；`AGENT_PERMS`、`SANDBOX` 与端口收集容器
不得复用上一轮的实例，宿主预置的请求级端口绑定只复制进新容器。发布的作用域 id 必须与会话
持久绑定的 Agent id 完全一致，错配同样 fail closed。所有可能失败的准备完成前不发布来源主体或
权限槽；发布后、第一条事实落格前仍失败的，恢复原槽并把未落格条目按原 FIFO 顺序回队。

收件箱判空、占轮与出队在同一把会话状态写锁内完成，空队列不得留下忙态。能否回滚只按本次第一条
`user.message` append 的局部结果判定，不能拿全局 WAL head 推断该条目是否落格；其他写方推进 head 不代表
本条已落。首条事实未落时恢复原槽并按原 FIFO 回队；首条已落、后续开轮装配失败时不重放，由同一轮收尾面
等驱动收束，补失败终态并投影。

占轮在同一把 `inner` 锁内先登记本轮中止信号，再公开忙态；中止动作也读这把锁，不能出现
「已忙但无信号」。终态 VDFS 帧必须先于落闲与唤醒送达，后继轮次才能保证在旧 Finished
之后发布 Working；直接开轮与恢复续跑共用同一占轮路径，不得绕开信号登记。

### D-550 容器 worker 使用同路径投影与 stdio 凭证代理

容器落点由宿主进程环境 `SYMBIO_WORKER_MODE=container` 显式选择，不读会话或模型可控
metadata。镜像必须固定为 `repository@sha256:<64 hex>`，且镜像 OS 必须与宿主一致；Docker
不可用、镜像摘要或构建标签不匹配、安全参数不受支持、挂载目标无效时启动失败，绝不回退 host。
容器仍执行同一 `__worker` 与 durable inbox 协议，宿主绝对路径原样保留：只读运行时投影挂到
原 homedir，实际会话目录与显式 workdir 分别挂到原绝对路径，从而不改变 WAL、inbox、workdir
与子会话寻址语义。投影按运行时白名单复制技能与插件配置，排除会话、模型/MCP 原配置、
符号链接、宿主凭证目录及无关文件；配置键按归一化后的名字识别 credential（含 camelCase），
子 Agent 目录递归投影，其模型与 HTTP MCP 使用
`/subagent/<agent>/model/<id>` 与 `/subagent/<agent>/mcp/<id>` 的独立 broker 路由。模型与
HTTP MCP 只保留 broker 地址，stdio MCP 和没有受控出口的 gateway / telegram / web 插件
明确停用。显式 workdir 不得覆盖 homedir、会话树或模型/MCP 目录，其目录树内也不得出现
凭证文件或目录（含 `.env`、私钥与证书）；容器 UID/GID 取会话所有者，会话与 workdir 必须
由当前非 root 用户所有且可写。
rootless Docker 的 UID 映射与该契约不同，fail closed；Docker 检查与清理有超时，清理失败
必须显式告警。broker 错误不向 worker 暴露真实上游 URL；响应头后的断流必须让 HTTP 客户端
观察到传输失败，不得补正常 EOF。客户端断开会取消上游，慢头部、慢请求体与慢响应写入均有
时限，不让未认证连接无限占用 broker 流；单流分片积压只终止该流，不结束整个 worker。

模型与 HTTP MCP 的真实 URL、凭证只留在 Supervisor 的启动快照。worker 内 loopback shim
使用每 worker 随机令牌校验访问，把 HTTP 请求转成受 epoch fencing 的 stdio broker 流；响应头
与正文分片转发，SSE 不整缓冲。
Supervisor 只接受启动投影登记过的精确目标，剥离 worker 提供的凭证头，再由宿主可信配置覆盖，
并过滤逐跳响应头与 `Location` / `Content-Location`（避免预签名 URL 出容器）；重定向不得把
凭证带到其他 origin。流关闭、取消、epoch 替换和 EOF 都取消
上游请求。该代理只提供受控网络出口，不授予工具、审批或沙箱策略权限，也不声称 Docker 本身
提供容器、网络或进程隔离。

### D-552 事件转播标记归 Server：SDK 只留发布端口

**现场**：跨 worker 事件的转播标记（`__relayed` 及其打标 / 判定函数）只被 Server 的
Supervisor 与 worker 进程使用；SDK 里既有 event_bus 域的读者只有 Server，插件侧没有消费方。
它随 `D-546` 的驱动契约一起进了 SDK，使共享机制面多出三个公开项，EX-007 的欠账因此还剩
一笔。

**裁决**：把标记常量与 `mark_relayed` / `is_relayed` 连同同源测试迁到
`crates/symbio-server/src/worker/event_relay.rs`，SDK 的 event_bus 域只保留发布入口与发布
端口；`PluginFrame` 仍是现成内核语法，不为 Server 新增跨插件契约。事件语义、帧形状与
背压补送路径逐字不变，只是标记的所有者从共享机制回到唯一读者。EX-007 按真实机制面净减
记一笔迁移，抵掉 `D-546` 那一笔补件；不为消账改基线、不加到期豁免。

### D-554 HTTP 入站调用统一经 RuntimeHost 分发

**现场**：gateway 的 `POST /api/v1/invoke` 直接 `parent.route()` 控制面插件树，而 native IPC
经 `RuntimeHost::invoke` 先问 Supervisor。会话 worker 化之后，同一条 `vdfs/write` 从 HTTP
进来会落在控制面的 session 插件上；它拿不到 worker 持有的 WAL 写者令牌，于是忙窗插话、
压缩 / 分支重试都报 `user.message 落格失败: NotTheWriter`。

**裁决**：`symbio-http` 提供运行时调用分发桥（`InvokeDispatcher`），`RuntimeHost::create`
在构造插件树之前先注册分发器占位（gateway 入站随构造期异步监听，事后注册会留下写请求落到
控制面的窗口）；gateway ingress 只保留鉴权、幂等与只读策略，会话 worker 调用先交分发器，
控制面调用仍由 ingress 自己的插件树边界裁决，未注册分发器时同样退回构造期插件树以支撑独立
ingress 测试。HTTP / WS 的会话路径与 native IPC 从此共用同一条 worker 路由判据，transport
不持有宿主实现，停止宿主时注销分发桥。心跳是控制面单例，它的自主触发同样经这条桥投递；
worker 内的插件配置由每次能力收集从磁盘重读，避免控制面热切只更新自己那棵树。

## 验收边界

- Server 与 Gateway 共用传输实现，但调用者身份、公开面与路由策略不共享。
- 浏览器在 Tauri IPC 不可用时仍能完成连接、目录选择与会话操作；IPC 存在时也不得静默
  切回 native。
- 同一会话出现两个可写 epoch、worker 未持 WAL 锁即 ready、旧 epoch 事件被消费，均判失败。
- 容器缺 Docker、镜像摘要、受限挂载或凭证通道时，隔离模式不得退化为宿主执行。
- 旧执行路径删除前，不声称已完成一会话一进程迁移。
