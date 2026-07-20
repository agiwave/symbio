/**
 * MCP Server 管理服务
 *
 * 包装后端 `mcp/servers/*` 路由：
 * - list  : 列出全部 MCP 服务器
 * - get   : 按 name 读取单个服务器
 * - set   : 按 name upsert（创建或更新）
 * - delete: 按 name 删除
 * - test  : 测试服务器连接（不修改任何状态）
 *
 * 对应的后端：symbio/src/plugins/mcp/plugin.rs
 * 对应的 schema：tauri/src/schemas/mcp_servers.ts
 */

import { callPlugin } from './plugin'
import {
  McpServersList,
  McpServersGet,
  McpServersSet,
  McpServersDelete,
  McpServersTest
} from '../schemas/mcp_servers'
import type { McpConfig, McpServerConfig } from '../schemas/mcp_config'
import { logger } from '@/utils/logger'

const MCP_SERVERS_PATH = 'mcp/servers'

/** 列出全部 MCP 服务器（含 disabled）
 *
 * 错误时返回空配置（不抛出），UI 层据此展示空列表；
 * 错误会通过 logger 记录。
 */
export async function listMcpServers(): Promise<McpConfig> {
  try {
    const resp = await callPlugin<McpServersList.Response>(
      `${MCP_SERVERS_PATH}/list`,
      {} satisfies McpServersList.Request
    )
    return resp?.config ?? { servers: {} }
  } catch (err) {
    logger.error('mcp-servers-service', 'listMcpServers failed:', err)
    return { servers: {} }
  }
}

/** 按 name 获取单个 MCP 服务器
 *
 * 找不到时后端返回 NotFound 错误，本函数把错误原样抛出，
 * 调用方应通过 try/catch 处理。错误会通过 logger 记录。
 */
export async function getMcpServer(name: string): Promise<McpServerConfig> {
  try {
    const resp = await callPlugin<McpServersGet.Response>(
      `${MCP_SERVERS_PATH}/get`,
      { name } satisfies McpServersGet.Request
    )
    return resp.server
  } catch (err) {
    logger.error('mcp-servers-service', `getMcpServer(${name}) failed:`, err)
    throw err
  }
}

/** 创建或更新一个 MCP 服务器
 *
 * 名称为空 / 必填字段缺失时后端返回 ValidationError；
 * 本函数把错误原样抛出供调用方处理。错误会通过 logger 记录。
 */
export async function setMcpServer(
  name: string,
  server: McpServerConfig
): Promise<McpConfig> {
  try {
    const resp = await callPlugin<McpServersSet.Response>(
      `${MCP_SERVERS_PATH}/set`,
      { name, server } satisfies McpServersSet.Request
    )
    return resp.config
  } catch (err) {
    logger.error('mcp-servers-service', `setMcpServer(${name}) failed:`, err)
    throw err
  }
}

/** 删除一个 MCP 服务器
 *
 * 找不到时后端返回 NotFound；
 * 本函数把错误原样抛出供调用方处理。错误会通过 logger 记录。
 */
export async function deleteMcpServer(name: string): Promise<McpConfig> {
  try {
    const resp = await callPlugin<McpServersDelete.Response>(
      `${MCP_SERVERS_PATH}/delete`,
      { name } satisfies McpServersDelete.Request
    )
    return resp.config
  } catch (err) {
    logger.error('mcp-servers-service', `deleteMcpServer(${name}) failed:`, err)
    throw err
  }
}

/** 把 McpConfig 拍平为数组（按 name 排序） */
export function flattenMcpServers(cfg: McpConfig): Array<{ name: string; server: McpServerConfig }> {
  return Object.entries(cfg.servers ?? {})
    .map(([name, server]) => ({ name, server }))
    .sort((a, b) => a.name.localeCompare(b.name))
}

/** 测试一个 MCP server 的连接（不修改任何状态）
 *
 * 走后端 `mcp/servers/test` 路由，执行完整握手 + tools/list。
 * 失败时不抛出，返回 `{ ok: false, error: ... }`，调用方可直接展示。
 */
export async function testMcpServer(name: string): Promise<McpServersTest.Response> {
  try {
    const resp = await callPlugin<McpServersTest.Response>(
      `${MCP_SERVERS_PATH}/test`,
      { name } satisfies McpServersTest.Request
    )
    return resp
  } catch (err) {
    logger.error('mcp-servers-service', `testMcpServer(${name}) failed:`, err)
    // 后端异常时包装成 Response 形式
    return {
      name,
      ok: false,
      tool_count: 0,
      protocol_version: 'unknown',
      server_name: null,
      server_version: null,
      instructions: null,
      error: String(err),
      elapsed_ms: 0,
    }
  }
}
