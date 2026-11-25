---
kind: design
id: crates/symbio-runtime/README
title: Server 与共享运行时
status: active
created: 2026-11-24
impl: partial
---

# Server 与共享运行时

`symbio-runtime` 是 Symbio 的共享运行时：传输与会话执行（`server`）加上每会话驱动
（`host`）。CLI、Tauri 和 `symbio-cli serve` 都经 `RuntimeHost` 创建同一棵插件树；
Server 额外提供静态 Vue 资源与宿主目录浏览 API。本 crate 不发布独立二进制。

## 边界

- `symbio-http` 拥有传输、鉴权与连接机制；本 crate 的 `Ingress` 决定 Server 的公开面。
- 宿主目录 API 只列目录，不能读取文件，也不替代物理 VDFS。
- 浏览器只走 HTTP/WS；Tauri IPC 只存在于 embedded 环境。
- homedir / rootdir 是系统数据根，workdir 是会话元数据，二者不互换。

## 运行

```text
symbio-cli serve --bind 127.0.0.1 --port 9231 --homedir <系统根> --frontend-dist tauri/dist
```

非回环监听必须提供令牌；浏览器 WebSocket 通过 `bearer.<token>` 子协议携带同一个令牌。

## 浏览器接入安全

- 默认只允许同源 HTTP/WS。回环免令牌模式仍校验 `Host` 与 `Origin`，拒绝跨站页面和
  Host 重绑定；非回环监听必须设置 `--token`。
- 显式跨源访问用 `--allowed-origin <精确 Origin>`（或 `SYMBIO_SERVER_ALLOWED_ORIGIN`），
  必须同时设置令牌；不接受通配 Origin。普通 HTTP 使用 `Authorization: Bearer <token>`。
- `POST /api/v1/invoke` 只接受 `Content-Type: application/json`；静态资源、宿主目录与
  `/api/v1/events`、`/api/v1/ws` 共用这套传输鉴权。
- 请求读取和单次响应写入分别限制为 15 秒；停机给连接 20 秒退出，超时后才强制结束。
- `home/reload` 持运行时写锁，取消并等待其他 SSE/WS 连接退出后再重载。WebSocket 发起
  重载时保留自身连接以避免自等待，重载响应发送后即关闭。

宿主目录入口是 `GET /api/v1/host/runtime` 与
`GET /api/v1/host/directories?path=<Server 路径>`；两者都要求鉴权，且只返回目录信息。

一会话一执行体的生产绑定已落到 `RuntimeHost`：会话作用域请求经 durable inbox 派发到
`SessionExecutor` 端口的当前落点——host 为进程内执行体，container 为独立容器进程；顶层
与子会话使用同一 Supervisor；host 插件的事件直接投递进程总线，container 事件由监督者
跨进程回灌，监督者只把事件中继给其他容器落点。两种落点都不宣称隔离；授权、
审批、审计与事实语义不随落点改变，目标拓扑见
[DE-060](../../docs/003-design/060-Server运行时与会话执行.md)。

## 容器 worker

宿主以 `SYMBIO_WORKER_MODE=container` 显式选择容器落点，配置项见
[RE-001](../../docs/004-ref/001-CONFIGURATION.md)。镜像必须固定到 registry manifest digest，
且镜像内的 `symbio` 与宿主 `symbio-cli` 二进制逐字节相同：

```sh
cargo build --release -p symbio-cli
mkdir -p ../../dist
cp ../../target/release/symbio-cli ../../dist/symbio
build_id=$(sha256sum ../../target/release/symbio-cli | cut -d' ' -f1)
docker build --build-arg SYMBIO_BUILD_ID="$build_id" -f ../../Dockerfile -t <registry>/symbio-worker:<tag> ../..
docker push <registry>/symbio-worker:<tag>
docker image inspect --format '{{index .RepoDigests 0}}' <registry>/symbio-worker:<tag>
```

将最后输出的 `repository@sha256:...` 设为 `SYMBIO_WORKER_IMAGE`。镜像 OS 必须与宿主一致；
Windows 宿主或 Linux Docker Desktop 混用会在启动前失败。`Dockerfile` 只封装已构建的宿主
二进制，不在镜像内重新编译，从而保留 build-id 校验的意义。容器使用会话所属非 root
UID/GID；会话与 workdir 必须由当前用户所有且可写，rootless Docker 因 UID 映射不同而失败。
workdir 树内不得含凭证目录，Docker 检查与清理超过 15 秒即失败并告警。

容器以 `network none` 启动，模型与 HTTP MCP 请求由 worker 的 `127.0.0.1:18791` shim 转为
受 epoch fencing 的 stdio broker 流；Supervisor 仅访问启动投影登记的目标，剥离 worker
提供的凭证头并注入宿主可信配置。SSE 分片转发，不整缓冲；取消、EOF、停机和 epoch 替换都会
取消上游请求。该机制只提供受控凭证出口，不授予工具、审批或沙箱策略权限。
