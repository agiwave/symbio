/**
 * Agent 管理服务
 *
 * 包装后端 `agent/*` 路由：
 * - list  : 列出所有 agent
 * - get   : 按 id 获取单个 agent
 * - delete: 按 id 删除 agent
 *
 * 对应的后端：symbio/src/plugins/agent/plugin.rs
 * 对应的 schema：tauri/src/schemas/agents.ts
 *
 * **注意**：agent/create 需要 cognition_units（认知单元），不适合 UI 表单。
 * 创建 agent 应使用 seed_agents 脚本。
 */

import { callPlugin } from './plugin'
import type { AgentProfile } from '../schemas/agents'
import { logger } from '@/utils/logger'

const AGENT_PATH = 'agent'

/** 列出所有 agent */
export async function listAgents(): Promise<AgentProfile[]> {
  try {
    const resp = await callPlugin<AgentProfile[]>(
      `${AGENT_PATH}/list`,
      {} as Record<string, never>
    )
    return Array.isArray(resp) ? resp : []
  } catch (err) {
    logger.error('agents-service', 'listAgents failed:', err)
    return []
  }
}

/** 按 id 获取单个 agent */
export async function getAgent(id: string): Promise<AgentProfile | null> {
  try {
    const resp = await callPlugin<AgentProfile>(
      `${AGENT_PATH}/get`,
      { id }
    )
    return resp ?? null
  } catch (err) {
    logger.error('agents-service', 'getAgent failed:', err)
    return null
  }
}

/** 删除一个 agent */
export async function deleteAgent(id: string): Promise<{ deleted: boolean; id: string }> {
  return await callPlugin<{ deleted: boolean; id: string }>(
    `${AGENT_PATH}/delete`,
    { id }
  )
}
