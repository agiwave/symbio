---
kind: design
id: symbio/src/plugins/web/README
title: Web 插件
status: active
created: 2026-09-09
impl: done
---

# Web 插件

Web 工具集：提供网络请求与检索能力，经 `traverse` 向会话贡献工具。

## 工具

| 工具 | 说明 |
|------|------|
| `web/http_request` | 通用 HTTP 请求（GET/POST/PUT/DELETE 等，可带 header/body） |
| `web/web_search` | 网络搜索 |
| `web/web_fetch` | 抓取网页内容（HTTP/HTTPS） |

> 清单以 `docs/004-ref/003-ROUTES.md` §Web 插件为准（**权威**）；此处仅列机制相关的简表。

## 机制

- 与 local 插件同构：经 `traverse` 注册 Capability，session 收集后进入模型工具定义。
- 结果为纯文本/JSON 回传，超大结果由 session 侧压缩守卫处理。

## 出站守卫与配置

- **同一套地址判据**：`web_fetch` 与 `http_request` 共用 `net_guard.rs` 拦回环、私有网段、
 链路本地与云元数据地址（按 IP 语义判定，含 IPv4-mapped IPv6）。前者只做地址守卫，
 后者另加 `http_allowed_domains` 白名单；两者都要求 `http` / `https` scheme。
- **超时统一**：三个工具都读 `web_timeout`，单次请求逐次取值——保存配置后立即生效，
 不需要重载插件。配置定义见 [001-CONFIGURATION](../../../../docs/004-ref/001-CONFIGURATION.md "RE-001-CONFIGURATION")。

## 关联

- 工具收集管线：`plugins/session/capabilities.rs`
- 压缩守卫：`../session/README.md` 策略③
