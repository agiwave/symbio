/**
 * SessionChanges 服务 —— 只读路由 `session/changes` 的受控包装
 *
 * 与 `sessionStats.ts` 同族：读的是该会话的事实源（v2-events.wal），不碰 VDFS
 * 节点，因此不并进纯 VDFS 门面 `session.ts`。两条只读出口各占一个文件，是因为
 * 它们回答的是两个问题（花了多少 vs 改过哪些），响应形状也不同。
 *
 * ## 前端只排版，不判读
 *
 * 后端已经把行排成呈现就绪的 `{path, address, op, label, turn, changes}`：
 * 本文件原样透传，不解析工具参数、不把 `op` 翻译成文案、不拼地址。地址是
 * **相对会话容器**的（`workdir/<path>`），与会话地址拼合由调用方经地址代数完成。
 */

import { callPlugin } from './plugin'
import { ROUTE_SESSION_CHANGES } from '@/constants/routes.gen'

/** 读方身份声明：不传 = 本机默认（署主全量读数）。 */
export interface SessionChangesRequest {
  principal?: string
}

/** 一行变更（后端 `ChangeRow` 的镜像）。 */
export interface ChangeRow {
  /** 工作目录内的相对路径（`/` 分隔）。 */
  path: string
  /** **相对会话容器**的 VDFS 地址（`workdir/<path>`）。 */
  address: string
  /** 判别词（给样式 / 测试用；展示文案在 `label`）。 */
  op: string
  /** 展示文案（「写入」「编辑」…）——前端不认 `op`。 */
  label: string
  /** 最后一次改它的轮号。 */
  turn: number
  /** 本会话对它成功写入的次数。 */
  changes: number
}

/** `session/changes` 的响应（后端 `SessionChanges` 的镜像）。 */
export interface SessionChanges {
  session_id: string
  /** 事实源在不在；`false` = 这个会话还没跑过一轮。 */
  has_wal: boolean
  /** 读方有权看这份清单；`false` + 空行 = 有源但不给看。 */
  readable: boolean
  /** 最近改过的在前。 */
  rows: ChangeRow[]
}

/** 读一个会话的变更清单。与 `readSessionStats` 同一族口径（回读类调用）。 */
export async function readSessionChanges(
  sessionId: string,
  principal?: string,
): Promise<SessionChanges> {
  const body: SessionChangesRequest = principal ? { principal } : {}
  return callPlugin<SessionChanges, SessionChangesRequest>(ROUTE_SESSION_CHANGES, body, 30_000, {
    session_id: sessionId,
    origin: 'session-changes',
  })
}
