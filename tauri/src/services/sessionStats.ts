/**
 * SessionStats 服务 —— 只读路由 `session/stats` 的受控包装
 *
 * ## 为什么单独一个文件而不是并进 `session.ts`
 *
 * 那个文件是**纯 VDFS 门面**（会话域的一切操作都走 VDFS 的 list/read/write/delete，
 * 模块头写明「这里已经没有任何会话专用路由」）。`session/stats` 是一条**只读插件
 * 路由**，性质不同：它不碰 VDFS 节点，读的是该会话的事实源（v2-events.wal）。
 *
 * 并进去会让「session.ts 无专用路由」这条自述失效，也会让一个纯函数式的门面文件
 * 长出第二种调用形态。故独立成文件，与 `home.ts`（控制面路由 → service wrapper）
 * 同族。
 *
 * ## 前端只认描述，不认数
 *
 * 返回值里的 `panel` 是后端下发的呈现描述（`schemas/reading.ts`），组件按它排版；
 * 其余各列是机器可读的读数，供诊断脚本与 `slo-exit-audit` 用。本文件**两者原样
 * 透传**，不做任何字段加工——加工就是把口径抄进第二处。
 */

import { callPlugin } from './plugin'
import { ROUTE_SESSION_STATS } from '@/constants/routes.gen'
import type { ReadingPanel, ReadingSection } from '@/schemas/reading'

/** 读方身份声明：不传 = 本机默认（署主全量读数）。 */
export interface SessionStatsRequest {
  principal?: string
}

/** `session/stats` 的响应（后端 `SessionStats` 的镜像）。
 *
 *  只读路由，故这里**只声明前端会用的三族字段**：`panel`（人读的全部描述）、
 *  `usage`（会话头那一格，与 `panel` 里「用量」那段同一次装配）与 `has_wal`
 *  （「这个会话跑过没有」——它决定空态文案，而空态是组件层的事）。
 *  其余各列（tiers / cost / fleet …）不在本文件镜像：没有消费方就不写第二份
 *  形状（schema-audit 的「未被使用」判据盯的正是这个）。 */
export interface SessionStats {
  session_id: string
  has_wal: boolean
  panel: ReadingPanel
  /**
   * 会话头那一格读数（后端 `stats.rs` 的 `usage`）：一次实测都没有时**整个键不出现**。
   *
   * 为什么不从 `panel.sections` 里按标题挑「用量」那一节：方言里的 `ReadingSection`
   * 只有 `title` 没有身份字段，按中文标签定位等于把标签变成跨栈的键——改一个字，
   * 头上就静默少一格。让产者把要的那一格单独交出来，前端一个字符串都不用认。
   */
  usage?: ReadingSection
}

/**
 * 读一个会话的读数。
 *
 * 走 `callReadback` 同一族口径（回读类调用：明确 origin、限定超时）——
 * 与 `home.ts` 的 `getHomedirInfo` 同形。
 */
export async function readSessionStats(
  sessionId: string,
  principal?: string,
): Promise<SessionStats> {
  const body: SessionStatsRequest = principal ? { principal } : {}
  return callPlugin<SessionStats, SessionStatsRequest>(ROUTE_SESSION_STATS, body, 30_000, {
    session_id: sessionId,
    origin: 'session-stats',
  })
}
