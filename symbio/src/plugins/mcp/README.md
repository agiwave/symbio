# MCP 插件

Model Context Protocol 工具扩展插件。

## 功能特性

### 传输类型
- **stdio**: 本地进程通信
- **HTTP**: REST API
- **SSE**: Server-Sent Events（已支持）

### 接口 (Actions)

| Path | 说明 |
|------|------|
| `servers/list` | 列出已配置的服务器 |
| `servers/get` | 获取单个服务器配置 |
| `servers/set` | 新增/更新服务器 |
| `servers/delete` | 删除服务器 |
| `servers/test` | 测试服务器连接 |

## 配置示例

```json
{
  "name": "filesystem",
  "transport_type": "stdio",
  "command": "mcp-filesystem",
  "args": ["/home/user/projects"]
}
```

## 工具发现

MCP 工具通过 `traverse` 动态发现并注册，而非独立路由：
1. 通过 `servers/set` 注册 MCP 服务器配置。
2. `McpPlugin::traverse(TRAVERSE_AVAILABLE_TOOLS, …)` 把启用的 server 工具注册到 `CapabilityManager`（命名形如 `mcp/<server>/<tool>`）。
3. LLM 调用工具时，由宿主统一经 `traverse` 分发到对应 MCP server 执行（无独立的 `call` 路由）。