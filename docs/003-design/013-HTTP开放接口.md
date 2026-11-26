---
kind: design
id: DE-013-HTTP开放接口
title: HTTP 开放接口
status: active
created: 2026-09-06
impl: done
---

# HTTP 开放接口

> 目标：在 App 运行期间对外提供一套纯 HTTP/WebSocket 接口，让第三方应用像前端一样访问和控制整个应用。
> 接口服务本身作为**一个普通插件**纳入现有插件体系，配置/开启/关闭**复用现有设置页机制**，前端零改动。

- 现状：已实现（入站网关插件，**可选装配**，按需从插件市场安装）。端点清单见 [路由参考](../004-ref/003-ROUTES.md "RE-003-ROUTES")，配置键见 [配置参考](../004-ref/001-CONFIGURATION.md "RE-001-CONFIGURATION")，调用链见 [数据流与调用链](003-数据流与调用链.md "DE-003-数据流与调用链")。
- 实现与专题设计：**归插件自述文档**——[入站网关插件专题设计](../../crates/symbio-plugin-gateway/src/docs/114-HTTP入站网关.md "DE-114-HTTP入站网关")。本文只保留系统级机制定位；端点、鉴权、帧协议、重放防护等实现细节都在那篇文档里，不在此复述以免漂移。
- 关联：[协议规范](005-协议规范.md "DE-005-协议规范")（`PluginMessageWire` / `PluginPayloadWire` / `PluginFrame` 线上类型）

---

## 1. 系统级机制定位

入站开放接口在系统架构里占的位置，与具体插件身份无关，由以下不变量界定：

1. **做成插件**：接口服务是插件树里的一个普通节点，不做成宿主内的旁路；配置复用插件目录的 `PLUGIN.yml` 与可配置收集通道，前端零改动。
2. **同构转发**：入站请求翻译成 `PluginMessageWire` 后经 `parent.route` 进入插件树，载荷与 `route_v2` 一字不差——不引入第二套调用协议。
3. **通道与信源分离**：WebSocket 解决「通道」，AI 增量信源是 `vdfs/watch` 上的会话子树；连上 WS 后必须再开一条连接订阅总线 + 登记 `vdfs/watch`，两步缺一不可（详见[专题设计](../../crates/symbio-plugin-gateway/src/docs/114-HTTP入站网关.md "DE-114-HTTP入站网关") §5.3）。
4. **受控暴露**：默认关闭、回环绑定、令牌只走 `Authorization` 头或 WS 子协议（不走查询串）、只读模式与连接上限——安全取舍详见[专题设计](../../crates/symbio-plugin-gateway/src/docs/114-HTTP入站网关.md "DE-114-HTTP入站网关") §6。

> 上述四条是**机制约束**，不随实现插件的增删或改名改变。任何入站插件都要满足它们；
> 具体端点表、配置键名、代码锚点属于实现细节，归插件自述文档。

### 关注点分离的现行边界

入站由三个关注点组成，各有单一 owner，边界按职责而非按行数划分：

| 关注点 | owner | 不越界的含义 |
|---|---|---|
| 线路机制 | `symbio-http` | HTTP/1.1、WebSocket、SSE、鉴权、幂等键、只读白名单——两个 ingress 必须同形的部分只此一份 |
| 路由与策略 | 各 ingress 实现 | 把 `PluginMessageWire` 交给插件树的分发、只读判定后的调用上下文；宿主 Server 形态在 `symbio-runtime`，桌面入站形态在网关插件 |
| 配置面 | 网关插件 | `DetailDefinition` 表单、`<根>/gateway/PLUGIN.yml`、写后热生效——前端设置页靠插件的可配置收集通道渲染 |

**配置面留在插件是硬约束**：前端「设置」页经插件的可配置通道发现并渲染入站开关与表单；把配置挪到宿主目录会让前端失去唯一的配置入口，属于功能倒退。因此**网关插件不退场**。

**两种 ingress 不是重复、不合并**：宿主 Server 形态（挂 `RuntimeHost`、含静态前端与 `host/*` 状态面、reload 需 drain 浏览器连接）与桌面入站形态（挂 worker parent、可达面受 composite 限制、`dispatch_session_invoke` 走会话分发桥）行为刻意不同；合并会改变可达面语义并波及依赖它的 e2e 判据。两者共享的部分已经收敛在 `symbio-http`。

---

## 2. 关联

- 专题设计与实现锚点：[入站网关插件专题设计](../../crates/symbio-plugin-gateway/src/docs/114-HTTP入站网关.md "DE-114-HTTP入站网关")
- 调用链：[数据流与调用链](003-数据流与调用链.md "DE-003-数据流与调用链") 链路三
- 线上类型：[协议规范](005-协议规范.md "DE-005-协议规范")
- 端点权威表：[路由参考](../004-ref/003-ROUTES.md "RE-003-ROUTES")
