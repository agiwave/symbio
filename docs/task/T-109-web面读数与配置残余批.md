---
kind: task
id: T-109-web面读数与配置残余批
title: web 面残余批：结果读数回灌（状态码 / 截断 / 超时语义）· `web_enabled` 无读方 · `allowed_domains` 死参数 · `web_search` 上游地址写死
status: todo
created: 2026-10-15
track: web
deps: []
prio: later
exit: 五处各自要么接上**用户可见出口**（模型或界面看得见）、要么能被本地靶子打到，要么改判「明确不做」并写明理由与撤销条件；每一处的判据都必须**能红**（改前改后行为可见地不同），并把 [RE-005](../ref/RE-005-e2e覆盖矩阵.md "RE-005-e2e覆盖矩阵") 的「Web 外取」行补到三格全有归宿：①`web_fetch` 的 `status` / `content_type` / `truncated` / `url` 被 [extract_result](../../symbio/src/plugins/session/tools/tool_executor.rs "tool_executor.rs") 的 `content` 分支整段吃掉 ⇒ 404 页面与正常页面对模型**同形**；②超时与「连不上」在回灌文案里同形（都是 `请求失败: error sending request for url (…)`）⇒ 用户看不出该重试还是该改地址；③`web_enabled` 在 [RE-001](../ref/RE-001-CONFIGURATION.md "RE-001-CONFIGURATION") 有登记、在出厂 `PLUGIN.yml` 里存在，但**全仓无读方**（通用启用键是 `plugin_enabled`）；④`http_request` 的 `allowed_domains` 是 `HttpRequestTool::new` 里写死的 `vec!["*"]`，无任何路径能写进配置 ⇒ `host_matches_allowlist` 恒真、那条「不在 allowed_domains 中」的错误分支**永不触发**；⑤`web_search` 的三个上游地址（Tavily / Serper / DuckDuckGo HTML）**全部写死在代码里** ⇒ 只能打真外网，而 CI 无网。
---

## 内容

本任务由 [T-076](T-076-e2e面缺失补齐批.md "T-076-e2e面缺失补齐批") 的 Web 外取行**顺带查出**：给这一行补 e2e（`e2e/cases/t65-web-fetch.mjs`）时，
判据只能取到「模型实际收到的那段文本」与「本地靶子的请求留痕」，于是每一处
**读不到、也动不了**的东西都被顶到了台面上。五处都属于同一族病——**算出来了，却看不见**。

### ① 状态码与截断标记被丢弃（模型分不清 404 与正常页）

`web_fetch` 的返回体是 `{success, status, content_type, content, truncated, url}`，
而 `tool_executor::extract_result` 的判定顺序是 `content` → `output` → `success` → …
`web_fetch` **有** `content` 字段 ⇒ 第 1 条命中 ⇒ 回灌给模型的只有**正文**。
`status` / `content_type` / `truncated` / `url` 一个都到不了模型，也不在转写的结果节点里
（结果节点的 `content` 就是 `extract_result` 的产物）。

后果是实测到的：404 页面（`{"success": true, "status": 404, "content": "<错误页正文>"}`）
与 200 页面对模型**完全同形**——它会把「没有这个页面」当成页面内容继续推理。
`truncated` 同理：超过 1 MiB 的正文被切掉后，模型不知道自己是拿半截内容在回答。

对照：`http_request` 的返回体**没有** `content` / `output`，落到第 3 条
（`success` 为真 → 整包 JSON）⇒ 它的 `status` / `headers` / `size` **是**回灌的。
两个工具同一族语义、回灌口径却相反，这本身就是漂移的证据。

### ② 超时与连不上同形

`web_fetch` 的错误收敛是 `map_err(|e| PluginError::InternalError(format!("请求失败: {e}")))`，
而 reqwest 对超时与连接失败的 `Display` 都是 `error sending request for url (…)`。
实测回灌文本（T65 证据④）：

```
Error: 内部错误：请求失败: error sending request for url (http://127.0.0.1:18081/slow)
```

用户与模型都看不出这是「对方慢」还是「地址根本不通」——而这两种情况的下一步
（重试 vs 改地址）是相反的。`http_request` 同一形状。

### ③ `web_enabled` 没有读方

`docs/ref/RE-001-CONFIGURATION.md` 的配置总表把 `web` 插件的主要键登记为
`web_enabled` / `web_timeout` / `tavily_api_key` / `serper_api_key`，出厂
`.symbio/web/PLUGIN.yml` 里也确实写着 `web_enabled: true`——但全仓没有任何代码读它：

```
$ grep -rn "web_enabled" --include="*.rs" symbio/src
（无输出）
```

`WebConfig` 的字段只有 `web_timeout` / `tavily_api_key` / `serper_api_key`。
真正的通用启用键是 `plugin_enabled`（`symbio_core/plugin/dir.rs::PLUGIN_KEY_ENABLED`），
由 composite 的注册表读取。所以 `web_enabled` 是一个**登记在册、写在出厂配置里、
谁都不读**的键——正是「配置面板在骗人」那一族，而同一个文件的 `web_timeout`
字段文档里已经为同款病记过一次（「面板上能改，实际从没被任何工具读过」）。

### ④ `allowed_domains` 是死参数

`HttpRequestTool::new` 里 `allowed_domains: vec!["*".to_string()]` 写死，
`host_matches_allowlist` 是它唯一的读方，而**没有任何路径**能把配置写进这个 `Vec`：
`HttpRequestTool` 只有 `new(timeout_secs)` 与 `Default`（也走 `new`），插件侧只传超时。
于是 `*` 恒成立、`host_matches_allowlist` 恒真、`"主机 '{host}' 不在 allowed_domains 中"`
这条错误分支**永不触发**。

这与 T-100 查出的 `rustErrorCodes` 取错正则分组是同一族「**恒真的守卫**」：它看上去在
守域名白名单，实际什么都不守，而且**反向验证也照样绿**（注入什么都能过）。
`host_matches_allowlist` 的通配逻辑（`*.example.com` 必须等价于「裸域名或带点子域」）
本身是对的、也有单测——问题不在它，在**没有生产路径能让它拿到非 `*` 的白名单**。

### ⑤ `web_search` 上游地址写死（本行唯一「动不了」的一处）

`WebSearchTool` 的三个上游地址全部是字面量：

| provider | 地址 | 位置 |
|---|---|---|
| Tavily | `https://api.tavily.com/search` | `search_tavily` |
| Serper | `https://google.serper.dev/search` | `search_serper` |
| DuckDuckGo | `https://html.duckduckgo.com/html/` | `search_duckduckgo` |

`WebConfig` 里只有凭据，没有地址。于是这个工具**无法被指向本地靶子** ⇒ e2e 只能打
真外网，而 CI 无网 ⇒ RE-005 的「Web 外取」行在补了 t65 之后，`web_search` 仍是零覆盖。

补齐的前提是给上游地址加配置——那是**能力新增**，须走配置面登记（`RE-001` +
`scripts/facts/_sets.mjs` 的 `plugin-configs` 封闭集 + 出厂 `PLUGIN.yml`），
不是在本任务里顺手加一个字段。**另一种正当归宿是改判**：写明「搜索服务地址是
供应商契约的一部分、不该可配」并给出撤销条件——但那样 RE-005 的那一格就永久停在
「明确不做」，须在该行写明理由而不是留一个 `缺口`。

## 处理过程

- 2026-10-15 登记：T-076 的 Web 外取行补 e2e 时逐条查出（`e2e/cases/t65-web-fetch.mjs`
  的证据①③④⑤⑥与文件头「一条没有被钉住的读数」段即本任务的取证现场）。
