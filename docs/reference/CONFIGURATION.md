# Symbio 配置参考

> **文档类型：参考** — 配置文件结构与字段说明。

## 配置文件位置

**一个插件 = 一个目录**：配置是该目录下的 `PLUGIN.yml`，与该插件自己的数据 / 资源
同处一个目录，因此整个目录可以直接拷贝移植（机制见
[design/vdfs.md](../design/vdfs.md) §3.4 / §13.2）。

| 文件 | 用途 |
|------|------|
| `~/.symbio/PLUGIN.yml` | 系统级插件（`home`）的配置——工作区与最近记录 |
| `~/.symbio/<插件>/PLUGIN.yml` | 各插件的配置（`session` / `model` / `web` / `local` / `gateway` / `telegram` …） |
| `~/.symbio/<插件>/<id>/<主文件>` | 插件资源条目（`model/<id>/provider.json`、`mcp/<id>/server.json`、`skill/<id>/SKILL.md`…） |
| `~/.symbio/agent/<id>/` | agent 目录（工作区级 + 全局级双层；`AgentDirStore` 自管） |
| `~/.symbio/config.yaml.migrated` | 旧集中式配置的留档（首次迁移后改名，见下） |

`PLUGIN.yml` 是一个 YAML 映射，其中两个**身份字段**——`plugin_provider`（工厂 id）与
`plugin_name`（实例名，缺省 = 目录名）——**不参与配置反序列化**，由 `PluginDir` 读写时
自动剥离 / 补回。加载判据 = 文件存在 + 可解析 + `plugin_provider` 指向已注册的工厂。

**配置的地址**：`<根>/<插件>/PLUGIN.yml`（`ext = form`，`rw`，`schema` = 该插件自己的
`DetailDefinition`）。读写用的就是 `vdfs/read` / `vdfs/write`——与任何其它资源同一条
链路，因此前端与 LLM 用同一种方式改配置。**没有第二条配置协议。**

---

## 系统根配置（`~/.symbio/PLUGIN.yml`）

`home` 的目录就是**系统根**，它不与业务插件并列——否则容器扫描插件根时会把它当
普通插件再构造一次，而那个 `home` 又会构造容器，自举成环。它的配置只有**应用级状态**：

```yaml
# ~/.symbio/PLUGIN.yml
plugin_provider: home
plugin_name: home
work:
  workdir: ~/.symbio
  recent_workspaces: []
```

> **旧 `config.yaml` 已不存在**：首次启动时 `home` 把旧文件里 `symbio.plugins.*` 的每一项
> 逐项写进各插件自己的 `PLUGIN.yml`（目标已存在则跳过），随后把旧文件改名
> `config.yaml.migrated` 留档——天然只生效一次。
>
> 旧文件里的 `plugins:`（插件挂载表）、`embedding:`、`logging:` 三个段**都没有新家**：
> 插件挂载由**扫描插件目录**决定（容器不内置任何清单），嵌入服务与日志级别由代码默认值
> / 环境变量决定，不再有集中配置项。

---

## 资源与数据的落盘位置

**没有全局「存储后端」开关**：配置里不存在 `storage.backend` 之类的键，
也不存在一个统一的存储服务。落盘位置由「谁的数据」决定，各走各的：

| 数据 | 位置 | 由谁决定 |
|------|------|----------|
| **插件配置** | `<homedir>/<插件>/PLUGIN.yml`（系统级插件在 `<homedir>/PLUGIN.yml`） | 配置的**拥有者**自己读写（`PluginConfigFile`）；地址 `<根>/<插件>/PLUGIN.yml`，**没有第二条配置协议** |
| 插件资源（model / mcp / skill 等） | `<homedir>/<类别>/<id>/<主文件>` | 类别段名 = 插件名（如 `model/<id>/provider.json`、`mcp/<id>/server.json`、`skill/<id>/SKILL.md`）；由 `symbio/src/providers/vdfs_service/` 的集中实现读写，**不可配置、无第二种后端** |
| 会话与其消息 | 会话自己的 store（`SessionStore`），非 `plugins/<类别>/<id>/` 资源布局 | 已收为**单一具体类型**：持久会话 = 磁盘 `<根>/<id>/{session.json, messages.json}`，临时会话 = 进程内驻留。`store_kind` / sqlite / memory 后端选型**已删除**（见 ADR-011 及 `session/store/mod.rs` 顶部「它不是什么」）|
| Agent 目录 | agent 目录（工作区级 + 全局级双层，`AgentDirStore` 自管） | 工作区切换，不经 `vdfs_service` |
| 应用级状态 | `<homedir>/PLUGIN.yml` | homedir 由前端「系统目录」切换（`home/reload`） |

> 换 homedir 即换一切：`<homedir>` 由 `HomedirRegistry` 现取，资源类别根每次解析时
> 拼接，因此切换后无需重启即可读到新址的清单（内存镜像随之重建）。

---

## 插件配置

各插件的配置都在**自己目录**的 `PLUGIN.yml` 里；前端「设置」页（`<根>/plugin_manager`）会把
它们一并列出，但条目携带的是**各自的真实地址**——点开读写的还是拥有者那份文件，
设置页只是指路，不代理、不复制。

下表是各插件的配置地址与主要键。**默认值以代码为准**：字段定义由配置的拥有者产出，
默认值从该插件自己的 `Default` 读出，因此面板显示值与实际行为同源。

| 插件 | 配置地址 | 主要键 |
|---|---|---|
| `home` | `<根>/PLUGIN.yml` | `work.workdir` / `work.recent_workspaces` |
| `session` | `<根>/session/PLUGIN.yml` | `max_messages` / `auto_compress` / `context_messages` / `max_tool_rounds` / `tool_context_window` / `fade_activate_rounds` / `fade_keep_recent_turns` / `compress_line_threshold` / `compress_keep_recent` / `enable_compact_tool` / `prune_tool_history` / `memory_max_bytes` / `memory_inject_max_bytes`（字段全表见 `session/config.rs::SessionConfig`；会话存储**无选型项**——已收为单一具体类型，见 ADR-011） |
| `agent` | `<根>/agent/PLUGIN.yml` | `item_max_bytes` / `identity_inject_max_bytes` / `memory_max_bytes` / `memory_inject_max_bytes`（字段全表见 `agent/host/config.rs::AgentConfig`） |
| `work` | `<根>/work/PLUGIN.yml` | `memory_enabled` / `memory_max_bytes` / `memory_inject_max_bytes`（字段全表见 `work/config.rs::WorkConfig`） |
| `web` | `<根>/web/PLUGIN.yml` | `web_enabled` / `web_timeout` / `tavily_api_key` / `serper_api_key` |
| `local` | `<根>/local/PLUGIN.yml` | `shell_enabled` / `file_enabled` / `shell_timeout` |
| `gateway` | `<根>/gateway/PLUGIN.yml` | 见下 |
| `telegram` | `<根>/telegram/PLUGIN.yml` | 见下 |
| `model` | `<根>/model/PLUGIN.yml` | `default_provider_id`（**读宽写窄**：兼容旧形态遗留的 `providers` 明细，迁移后归一） |
| `mcp` | 无配置文档 | 配置就是它的资源树（`<根>/mcp/<id>`） |

### Model 插件

```json
// ~/.symbio/model/<id>/provider.json
{
  "default_provider_id": "openai_main",
  "providers": {
    "openai_main": {
      "provider_type": "openai_chat",
      "api_key": "sk-...",
      "model": "gpt-4-turbo",
      "base_url": "https://api.openai.com/v1",
      "temperature": 0.7,
      "max_tokens": 4096,
      "min_interval_ms": 1000
    },
    "anthropic_backup": {
      "provider_type": "anthropic_messages",
      "api_key": "sk-ant-...",
      "model": "claude-3-opus-20240229",
      "base_url": "https://api.anthropic.com/v1"
    }
  }
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `provider_type` | String | ✅ | `openai_chat` / `openai_responses` / `anthropic_messages` / `gemini_api` |
| `api_key` | String | ✅ | API 密钥 |
| `model` | String | ✅ | 模型名称 |
| `base_url` | String | ❌ | 自定义 API 端点 (用于代理) |
| `temperature` | Float | ❌ | 采样温度 |
| `max_tokens` | Int | ❌ | 最大输出 token 数 |
| `min_interval_ms` | Int | ❌ | 最小请求间隔 (限流) |

### MCP 插件

**本插件不设配置文档**——配置就是它的资源树：一个 server 一个目录，主文件
`server.json`（`<根>/mcp/<id>`）。

```json
// ~/.symbio/mcp/filesystem/server.json
{
  "type": "stdio",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path"]
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `type` | String | ✅ | `stdio` / `http` / `sse`（默认 `stdio`） |
| `command` | String | stdio 时 | 可执行文件 |
| `args` | List[String] | ❌ | 命令行参数 |
| `env` | Map[String,String] | ❌ | stdio 环境变量 |
| `url` | String | http / sse 时 | 端点 |
| `enabled` | Bool | ❌ | 运行时是否启用（激活标记） |

### Telegram 插件

```yaml
# ~/.symbio/telegram/PLUGIN.yml
plugin_provider: telegram
bot_token: "123456:ABC-DEF..."
chat_id: ""
streaming_enabled: true
poll_enabled: true
allowed_users: [123456789]
```

| 键 | 类型 | 默认值 | 说明 |
|----|------|--------|------|
| `bot_token` | String | 空 | BotFather 下发的令牌 |
| `chat_id` | String | 空 | 默认会话（留空 = 由用户消息决定） |
| `streaming_enabled` | Bool | `true` | 是否流式回包 |
| `poll_enabled` | Bool | `true` | 是否轮询接收更新 |
| `allowed_users` | List[Int] | `[]` | 允许使用的 Telegram 用户 ID（空 = 不限制） |

> `allowed_users` 的落盘形态是**数字数组**，而配置文件的 `list` 控件提交的是
> **每行一项的字符串数组**——两者都是「用户 ID 列表」，转换在插件边界完成
> （宽松反序列化），磁盘形态不受前端控件形态影响。

### Gateway 插件

```yaml
# ~/.symbio/gateway/PLUGIN.yml
plugin_provider: gateway
inbound_enabled: false
inbound_protocol: native
inbound_bind: 127.0.0.1
inbound_port: 9231
inbound_token: ""
inbound_readonly: false
```

扁平键配置，**整体读写于插件自己的文件**（前端「设置」页点开 `<根>/gateway/PLUGIN.yml`
就是这份表单；`vdfs/write` 是整体替换，与其它资源同一条链路）：

| 键 | 默认值 | 说明 |
|----|--------|------|
| `inbound_enabled` | `false` | 是否启动 HTTP/WS 入站服务（默认仅 Tauri IPC，不监听端口） |
| `inbound_protocol` | `native` | `native`（仅 IPC）或 `http`（监听端口） |
| `inbound_bind` | `127.0.0.1` | 监听地址 |
| `inbound_port` | `9231` | 监听端口 |
| `inbound_token` | 空 | Bearer 令牌；**为空仅允许回环地址**，非回环监听必须设置 |
| `inbound_readonly` | `false` | 只读模式：仅放行查询类路径（白名单见 `gateway/config.rs::is_readonly_allowed`） |

> 出站配置（前端连向何处）不在这里——连接目标由前端"系统目录"切换器统一管理（localStorage 为权威），经 `initGatewayTransport` 决定 native / http 出站。
>
> 只读白名单（精确匹配）：`vdfs/list`、`vdfs/tree`、`vdfs/stat`、`vdfs/read`、`vdfs/search`、`home/get_homedir`、`work/get_workspace`。资源一律经 VDFS，故放行的是它的**读操作**——`vdfs/write` / `delete` / `mkdir` / `move` / `edit` 与节点动作 `vdfs/action` 都不在列；早已下线的 `entities/*` 也不再放行。设计定位是**兜底而非完整安全边界**：即便令牌泄露到可信内网，也只能读取而无法触发写操作与命令执行。
>
> **配置文件本身不在白名单内**：`vdfs/read` 只要地址落在任一插件的 `PLUGIN.yml` 上就一律拒绝（`reads_config_document`）——配置可能含凭据（网关访问令牌、搜索服务 API Key），放行等于只读模式下就能把它们读走。网关自身的 `gateway/*` 接口也恒走 native（前端不经 HTTP 访问本插件）。

---

## 环境变量

| 变量 | 用途 | 示例 |
|------|------|------|
| `RUST_LOG` | Rust 日志级别（`symbio_core::logger` 读取） | `debug`, `symbio=trace` |
| `VITE_LOG_LEVEL` | 前端日志级别（`tauri/src/utils/logger.ts` 读取） | `debug`, `info` |

> 工作区路径**不是**环境变量：它由前端「系统目录」切换器决定，落在
> `<homedir>/PLUGIN.yml` 的 `work.workdir`。API Key 也不是环境变量——它属于**资源**，
> 在模型条目自己的 `provider.json` 里。

---

## 配置热加载

部分配置支持热加载 (无需重启)：

| 配置 | 热加载方式 |
|------|-----------|
| 插件配置（`PLUGIN.yml`） | 一次 `vdfs/write` → `PluginConfigFile::apply` = **校验 → 落内存 → 落自己的文件 → 广播**，订阅方（前端 / LLM）随事件收敛；需要副作用的插件（如网关重建监听）在自己的 `write` 返回后执行 |
| 插件集合 | 容器**不缓存**子插件清单——每次现取（`children_of`），因此新增 / 移除插件目录即时可见 |
| 资源条目 | 走 VDFS 的 `watch` / `unwatch` 事件，前端按路径防抖刷新（**禁止轮询**） |

---

## 配置验证

启动时验证：

1. **必填字段**：缺失则启动失败
2. **插件注册**：`plugin_provider` 必须在 `ObjectCreatorRegistry` 中存在
3. **路径合法性**：`workdir` 必须存在且可写

---

> **维护原则**：新增配置项必须在此文档登记，包括类型、默认值、验证规则。
