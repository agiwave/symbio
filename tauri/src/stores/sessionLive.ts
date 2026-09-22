/**
 * 会话实时状态的派生规则 —— 纯逻辑（无响应式、无 IPC、无 store）
 *
 * ## 职责
 *
 * 「会话节点 / 清单快照 → 本地实时状态」的映射。与 `sessionTranscript` 并列：
 * 前者管**消息**，本模块管**会话本身**（运行态 / 结局 / 清单条目 / 选项回填）。
 *
 * 这里放的都是**判定**，不是状态：store 持有 ref 并调用它们。
 * 判定抽出来的直接收益是「节点状态 → 运行态」这条最容易出错、又最难肉眼验证的
 * 映射可以逐条断言（见 `__tests__/sessionLive.spec.ts`）。
 */

import {
  VDFS_STATUS_WORKING,
  isWorkingStatus,
  type SessionOutcome,
} from '@/schemas/vdfs'
import { isBlankContentNode, isInProgressMessage } from './sessionTranscript'
import type { ChatMessage } from '@/schemas/chat_message'
import type { SessionListItem } from '@/services/session'

/** 单个 session 的实时状态（用于缩略卡展示） */
export interface SessionLiveStatus {
  /** 会话的运行状态：与 VDFS 节点 `status` **同一词表**（`working` / `active` / …）。
   *
   * 这里不是「第二份真相」，而是**节点状态的本地镜像**：`status` 只在
   * `list` / `stat` 时可见，而 busy / idle 是事件流；两次采样之间（尤其是
   * send 的乐观置位）必须有处落脚。分页后当前会话也可能不在这一页里，
   * 按 list 取节点会落空——所以按 id 索引的这份 map 不能省。
   *
   * 读法一律走 `isWorkingStatus(status)`：**不要**再引入 `is_working` 布尔。 */
  status?: string
  /** 是否有消息处于 waiting_user_action 状态（缩略卡显示"等待审批"角标） */
  is_waiting_approval: boolean
  /**
   * 最近一次"状态写入"的本地时间（毫秒）。
   *
   * 注意：
   * - 写入来源是**节点状态**，不是事件：转写流的两类帧（消息帧与
   *   `transcript_session` 运行态帧，共用 `seq` 空间），以及 send / resume 的乐观置位。
   * - `putStatus` 内部每次都会**自动更新**此字段（避免漏写）；
   *   `applyTranscriptMessage` 只在产生 assistant 文本预览时同步更新。
   * - 若需判断"状态是否过期"，请使用 `getSessionStaleReason()` 而不是直接读此字段。
   */
  last_event_at: number
  /** 当前活动状态文字（如 "正在思考..." / "正在调用工具 ls..."） */
  activity?: string
  /** 最后一条消息预览（assistant 的 text 内容） */
  last_preview?: string
  /**
   * 上一轮的结局（会话节点 `attributes.outcome` 的本地镜像）。
   *
   * 提示音据此选音色——它是**状态**（"上一轮怎么结束的"），不是事件：
   * 不需要靠"谁先到"来区分中止与失败。
   *
   * 注意「上一轮是否失败」**不看这里**，看 `status == 'failed'`
   * （`isFailedStatus`）——结局是过程记录，状态是当前事实，两者职责不同。
   */
  outcome?: SessionOutcome
}

/** 会话运行模式 / 执行风险等级 —— 定义在 `schemas/session_meta`（唯一定义处），
 *  此处 import + re-export：既有消费方一直从 `./sessionLive` 取，不必改路径。 */
import {
  SESSION_MODES,
  SESSION_RISK_LEVELS,
  type SessionMode,
  type SessionRiskLevel,
} from '@/schemas/session_meta'
export type { SessionMode, SessionRiskLevel }

/** 会话节点自述的运行态（`sessionRuntimeOf` 的结果子集） */
export interface SessionRuntime {
  status?: string
  outcome?: SessionOutcome
  error?: string
}

/**
 * 节点 → 清单条目的**就地补丁**（不整表重拉）。
 *
 * 节点自述的 `message_count` 随载荷下发；缺失则**保留原值**——
 * 把它当 0 会让侧栏计数在一次标题更新后归零。
 * 标题同样只在节点确实带了非空 `title` 时才写进 metadata。
 *
 * 入参按**结构**声明（而不是 `Pick<VdfsNode, …>`）：`message_count` 是 VDFS 节点上的
 * 场景扩展字段（flatten 到顶层），不在 `VdfsNode` 的声明键里，只有索引签名能取到。
 */
export function listItemPatchOf(
  node: { title?: unknown; updated_at?: number; message_count?: unknown },
  cur: SessionListItem,
  status: string | undefined,
): SessionListItem {
  const title = typeof node.title === 'string' ? node.title : ''
  return {
    ...cur,
    status,
    updated_at: node.updated_at ?? cur.updated_at,
    message_count: typeof node.message_count === 'number' ? node.message_count : cur.message_count,
    metadata: title ? { ...(cur.metadata || {}), title } : cur.metadata,
  }
}

/**
 * 运行态镜像补丁（节点状态 / 结局直通）。
 *
 * `activity` 只在 working ↔ 非 working **迁移**时改写：否则一次标题更新就会把消息
 * 节点派生的「正在思考…」顶掉，造成闪动。进入 working 时顺带清掉审批角标
 * （新一轮开始，上一轮的待审批已作废）。
 */
export function liveStatusPatchOf(
  rt: SessionRuntime,
  wasWorking: boolean,
  nowWorking: boolean,
): Partial<SessionLiveStatus> {
  const patch: Partial<SessionLiveStatus> = {
    status: rt.status,
    outcome: rt.outcome,
  }
  if (wasWorking !== nowWorking) {
    if (nowWorking) patch.is_waiting_approval = false
    patch.activity = nowWorking
      ? '处理中…'
      : rt.outcome === 'aborted'
        ? '已中止'
        : rt.outcome === 'failed'
          ? '错误'
          : undefined
  }
  return patch
}

/**
 * 从清单项回填会话级选择（mode / risk_level）。
 *
 * 与 agent_id / provider_id 不同，mode / risk_level 的 UI 状态在 store（不在组件 ref），
 * 故需在清单刷新 / 元数据补丁时回填，使切换会话或改写后下拉态一致。
 * 只回填**合法取值**：非法值不写，避免把脏数据搬进 UI 状态。
 */
export function modeRiskBackfillOf(items: SessionListItem[]): {
  modes: Record<string, SessionMode>
  risks: Record<string, SessionRiskLevel>
} {
  const modes: Record<string, SessionMode> = {}
  const risks: Record<string, SessionRiskLevel> = {}
  for (const it of items) {
    const m = it.metadata
    if (!m) continue
    // 校验走**契约层的词表**而不是此处手写的字面量枚举：`metadata` 是
    // `Record<string, any>`，后端回包没有类型兜底，枚举必须只有一处——
    // 否则「风险等级多一个取值」会在这里被静默丢弃（既不报错也不生效）。
    if (SESSION_MODES.includes(m.mode)) modes[it.id] = m.mode
    if (SESSION_RISK_LEVELS.includes(m.risk_level)) risks[it.id] = m.risk_level
  }
  return { modes, risks }
}

/**
 * 清单快照与本地实时状态的合并。
 *
 * 本地镜像说「运行中」就**保留**：send 的乐观置位不能被一次稍早的 list 快照打回
 * （否则发送按钮会闪回「发送」）。其余一律以服务端 `status` 为准。
 */
export function mergeListWithLive(
  items: SessionListItem[],
  liveStatuses: Record<string, SessionLiveStatus>,
): SessionListItem[] {
  return items.map((it) => {
    const live = liveStatuses[it.id]
    return live && isWorkingStatus(live.status) ? { ...it, status: live.status } : it
  })
}

/**
 * 需要**升级**为运行中的会话 id。
 *
 * 仅做 false→true 的升级：true→false 的收敛交给节点状态变更，
 * 避免 list 快照与实时变更竞争时误降级。
 * 缺了这一步，页面重载/视图挂载后运行中的会话在输入框显示为禁用的「发送」按钮，
 * 用户无法点击停止（stop 按钮失效 bug）。
 */
export function workingUpgradesOf(
  items: SessionListItem[],
  liveStatuses: Record<string, SessionLiveStatus>,
): string[] {
  return items
    .filter((it) => isWorkingStatus(it.status) && !isWorkingStatus(liveStatuses[it.id]?.status))
    .map((it) => it.id)
}

/** 清单项 → 标题（`metadata.title` 优先，回退节点名；都没有返回空串） */
export function titleOf(it: SessionListItem): string {
  const t = it.metadata?.title
  return (typeof t === 'string' && t) || it.name || ''
}

/** 清单项 → 工作目录（非空字符串才算；否则 undefined） */
export function workdirOf(it: SessionListItem): string | undefined {
  const wd = it.metadata?.workdir
  return typeof wd === 'string' && wd ? wd : undefined
}

/**
 * 是否需要在会话流末尾补一条等待提示（三点脉动 +「正在思考…」）。
 *
 * ## 它补的是哪个窟窿
 *
 * 消息流里的等待提示原先只有**一个**触发点：`TurnGroupNode` 的「Turn 节点已到、
 * 尚无子节点」。这要求 Turn 节点**先于**任何内容到达前端——而两者是同一会话里
 * 相邻的两帧（转写流，批次 E 起还共用同一个 `seq` 空间），首帧丢失只会靠
 * **跳号 ⇒ 整份重读**补回来，中间那段空窗期没有任何东西可看。
 * 于是用户看到的就是：点了发送之后**什么都没有发生**，直到重读落地或第一个 token 到达。
 *
 * 「会话在跑」是**会话节点**的属性（`status === working`），与「Turn 节点到没到」
 * 是两件事。于是这里给出第二个来源：**会话在跑，而流里没有任何在途节点**。
 * 两个来源天然互斥——有在途节点时 Turn 骨架必然已显示，于是同一时刻只会出现一条。
 * （它是**兜底**，不是主路径：主路径是 Turn 节点自己把骨架画出来。）
 *
 * ## 判据为什么要排除「空壳节点」
 *
 * 后半句「没有在途节点」不能按 `status` 一眼看过去：流式占位节点（`text` /
 * `reasoning`，`content` 为空）也是 `streaming`，却**渲染不出任何东西**。把它算作
 * 「已有内容」，正好在最需要骨架的那个瞬间（Turn 的 `created` 丢了、只到了一个
 * 空壳）判成「不用补」——用户看到的就是一片空白。因此可见性判据走
 * `sessionTranscript.isBlankContentNode`，与树构建**同源**（两处只差「是否有子节点」）。
 *
 * 纯函数（无响应式、无 store）：这条判定最容易写成「看谁先到」的竞态，
 * 抽出来才能逐条断言（见 `__tests__/sessionLive.spec.ts`）。
 *
 * @param working  会话节点自述是否在处理中
 * @param messages 该会话本地全部消息（含被渲染层过滤的空内容叶子）
 */
export function needsTypingRow(
  working: boolean,
  messages: Array<Pick<ChatMessage, 'status' | 'type' | 'content'>>,
): boolean {
  if (!working) return false
  // 「有东西可看」= 有节点在跑**且它能渲染出内容**（空壳不算，见上）
  return !messages.some((m) => isInProgressMessage(m) && !isBlankContentNode(m))
}

/** 状态字面量：进入运行中时的活动文字（与 `liveStatusPatchOf` 保持一致） */
export const ACTIVITY_WORKING = '处理中…'
/** 状态字面量：运行中的节点状态值 */
export const STATUS_WORKING = VDFS_STATUS_WORKING
