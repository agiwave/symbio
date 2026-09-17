# MCP 插件

Model Context Protocol 工具扩展插件。

## 功能特性

### 传输类型
- **stdio**: 本地进程通信
- **HTTP**: REST API
- **SSE**: Server-Sent Events

### 配置与路由

> **本插件已无自有路由，也不设配置文档**：`mcp/config/*` 已下线，配置就是它的资源树。
> Server 的注册 / 列举 / 启停 / 连通性自检一律经 `.vdfs/mcp`（一个 server = 一个目录，
> 主文件 `server.json`），「测试连接」是节点动作 `vdfs/action { action: "test" }`。
> 见 `docs/reference/ROUTES.md` §MCP 插件（**权威**）。

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
1. 经 `.vdfs/mcp` 写入 server 配置（`vdfs/write`）。
2. `McpPlugin::traverse(TRAVERSE_AVAILABLE_TOOLS, …)` 把启用的 server 工具注册到 `CapabilityVisitor`（三段式命名 `mcp.<server>.<tool>`）。
3. LLM 调用工具时，由宿主统一经 `traverse` 分发到对应 MCP server 执行（无独立的 `call` 路由）。