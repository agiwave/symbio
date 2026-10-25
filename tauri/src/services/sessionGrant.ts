/**
 * `session/grant` —— 审批卡「永久放行」的唯一写入口（D-170）。
 *
 * 它写的是**会话插件自己的全局配置文档**（`approval_grants`：工具名 → 授权上限），
 * 不是某个会话的 metadata：因此一次点击对所有会话生效，撤销在设置页删掉那一行。
 *
 * ## 与读路由分家
 *
 * 本文件只做写（且是控制动作，不是数据 CRUD——真正的配置读写仍走 VDFS 的
 * `PLUGIN.yml` 表单）。放在 `services/` 而不是 `stores/`：它不持有任何状态，
 * 调用方拿到的新清单只用于确认写入结果，会话视图不据此渲染。
 */

import { callPlugin } from './plugin'
import { ROUTE_SESSION_GRANT } from '@/constants/routes.gen'
import type { SessionRiskLevel } from '@/schemas/session_meta'

/** 授权清单（后端 `ApprovalGrants` 的线上形态：工具名 → 允许的最高风险档）。 */
export type ApprovalGrantMap = Record<string, SessionRiskLevel>

interface SessionGrantRequest {
  /** 能力名（canonical name）——审批卡的 `tool_name` 原样回传。 */
  tool: string
  /** 这次调用的风险档；上限抬到它，同档及以下从此不问。 */
  risk: SessionRiskLevel
}

/**
 * 把某个工具的授权上限抬到 `risk`（**只抬不降**，由后端保证）。
 *
 * 返回写后的整张清单：后端只抬不降，回显请求会掩盖「已经更高」的事实，
 * 也会让前端误以为一次点击真的改变了什么。
 */
export async function grantToolForever(
  sessionId: string,
  tool: string,
  risk: SessionRiskLevel,
): Promise<ApprovalGrantMap> {
  const body: SessionGrantRequest = { tool, risk }
  return callPlugin<ApprovalGrantMap, SessionGrantRequest>(
    ROUTE_SESSION_GRANT,
    body,
    30_000,
    { session_id: sessionId, origin: 'session-grant' },
  )
}
