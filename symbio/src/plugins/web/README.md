# Web 插件

Web 工具集：提供网络请求与检索能力，经 `traverse` 向会话贡献工具。

## 工具

| 工具 | 说明 |
|------|------|
| `web/http_request` | 通用 HTTP 请求（GET/POST/PUT/DELETE 等，可带 header/body） |
| `web/web_search` | 网络搜索 |
| `web/web_fetch` | 抓取网页内容（HTTP/HTTPS） |

> 清单以 `docs/reference/ROUTES.md` §Web 插件为准（**权威**）；此处仅列机制相关的简表。

## 机制

- 与 local 插件同构：经 `traverse` 注册 Capability，session 收集后进入模型工具定义。
- 结果为纯文本/JSON 回传，超大结果由 session 侧压缩守卫处理。

## 关联

- 工具收集管线：`plugins/session/chat_pipeline.rs`
- 压缩守卫：`../session/README.md` 策略③
