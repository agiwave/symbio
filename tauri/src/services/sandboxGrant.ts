/**
 * `sandbox/grant` —— 审批卡「永久放行」的唯一写入口。
 *
 * 永久授权按**沙箱档案**隔离存储；档案身份只来自当前审批卡的
 * `prompt.policy.profile_id`。本服务不接受空档案，也不回落到 `default`：
 * 服务端缺失 / 畸形绑定时必须失败，让用户刷新审批卡或重新发起调用。
 *
 * 它不写会话 metadata，也不是 VDFS CRUD。放在 `services/` 而不是 `stores/`：
 * 调用方拿到的响应只用于确认写入结果，会话视图不据此渲染。
 */

import { ROUTE_SANDBOX_GRANT } from '@/constants/routes.gen'
import type { SessionRiskLevel } from '@/schemas/session_meta'
import { callPlugin } from './plugin'

/** 授权清单（后端 `ApprovalGrants` 的线上形态：工具名 → 允许的最高风险档）。 */
export type ApprovalGrantMap = Record<string, SessionRiskLevel>

/** 对齐后端 `symbio-plugin-sandbox/src/plugin.rs::GrantRequest`。 */
export interface SandboxGrantRequest {
  /** 当前审批卡 `prompt.policy.profile_id` 的原值；必填，不能由前端选择。 */
  profile_id: string
  /** 能力名（canonical name）——审批卡的 `tool_name` 原样回传。 */
  tool: string
  /** 这次调用的风险档；上限抬到它，同档及以下从此不问。 */
  risk: SessionRiskLevel
}

/** 对齐后端 `sandbox/grant` 的响应。 */
export interface SandboxGrantResponse {
  profile_id: string
  grants: ApprovalGrantMap
}

/**
 * 把某个档案里该工具的授权上限抬到 `risk`（**只抬不降**，由后端保证）。
 *
 * `sessionId` 仍随控制请求信封发送，用于宿主会话上下文；写入归属由显式
 * `profileId` 决定。返回后端写后的档案与整张清单，不把请求回显冒充结果。
 */
export async function grantSandboxToolForever(
  sessionId: string,
  profileId: string,
  tool: string,
  risk: SessionRiskLevel,
): Promise<SandboxGrantResponse> {
  if (typeof profileId !== 'string' || !profileId.trim()) {
    throw new Error('审批卡缺少有效的沙箱档案绑定，请刷新审批卡或重新发起工具调用后再试')
  }
  const body: SandboxGrantRequest = { profile_id: profileId, tool, risk }
  return callPlugin<SandboxGrantResponse, SandboxGrantRequest>(
    ROUTE_SANDBOX_GRANT,
    body,
    30_000,
    { session_id: sessionId, origin: 'sandbox-grant' },
  )
}
