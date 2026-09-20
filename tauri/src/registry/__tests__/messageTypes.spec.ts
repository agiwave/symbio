/**
 * 消息呈现注册表 — 前端纯 UI 映射单测（node 环境）
 *
 * 覆盖三类：
 * 1. **词表覆盖率**——`MESSAGE_TYPES` / `MESSAGE_STATUSES` / `CHAT_ROLES` 的每个取值
 *    都必须在文案表里有映射。漏一个就会把机制词（如 `compression`）原样摆给用户看，
 *    这条断言就是防它。
 * 2. 头部呈现（图标 / 标题 / 状态标签 / 色调 / 修饰类）的判定顺序。
 * 3. 折叠策略与「首次点击」语义。
 *
 * 本文件只验证纯映射；组件的 props → facets 摊平与递归渲染在
 * `components/__tests__/MessageNode.spec.ts` 里断言。
 */

import { describe, expect, it } from 'vitest'
import {
  CHAT_ROLES,
  MESSAGE_STATUSES,
  MESSAGE_STATUS_ABORTED,
  MESSAGE_STATUS_COMPLETED,
  MESSAGE_STATUS_FAILED,
  MESSAGE_STATUS_STREAMING,
  MESSAGE_STATUS_WAITING_USER_ACTION,
  MESSAGE_TYPES,
  MESSAGE_TYPE_COMPRESSION,
  MESSAGE_TYPE_REASONING,
  MESSAGE_TYPE_TEXT,
  MESSAGE_TYPE_TOOL_CALL,
  MESSAGE_TYPE_TURN,
  MESSAGE_TYPE_USER_PROMPT,
  CHAT_ROLE_ASSISTANT,
  CHAT_ROLE_TOOL,
  CHAT_ROLE_USER,
  isUnsettledMessageStatus,
} from '@/schemas/chat_message'
import {
  DEEP_COLLAPSE_LEVEL,
  MESSAGE_ROLE_LABELS,
  MESSAGE_STATUS_LABELS,
  MESSAGE_TYPE_LABELS,
  agentNameOf,
  canRetryCompaction,
  canRetryTool,
  canSupplyToolArgs,
  effectiveOpenOf,
  facetsOf,
  messageDefaultOpen,
  messageHeadModifier,
  messageIcon,
  messageIsHeartbeat,
  messageIsRunningAction,
  messageRendererKey,
  missingResultNoteOf,
  messageRetryTargetOf,
  messageRoleLabel,
  messageShowsLiveBadge,
  messageStartedAt,
  messageStatusLabel,
  messageStatusTag,
  messageStatusTone,
  messageTitle,
  messageTypeLabel,
  nextOpenOf,
  type MessageFacets,
} from '../messageTypes'

/** 构造 facets：只写关心的字段，其余取「普通助手正文」缺省 */
function facets(partial: Partial<MessageFacets> = {}): MessageFacets {
  return {
    role: CHAT_ROLE_ASSISTANT,
    type: MESSAGE_TYPE_TEXT,
    status: MESSAGE_STATUS_COMPLETED,
    subSession: false,
    toolRequest: false,
    toolResult: false,
    hasWaitingChild: false,
    recoverable: false,
    responseText: false,
    ...partial,
  }
}

describe('词表覆盖率（防机制词泄漏给用户）', () => {
  it('每个消息类型都有面向用户的文案', () => {
    for (const t of MESSAGE_TYPES) {
      expect(MESSAGE_TYPE_LABELS[t], `类型 ${t} 缺文案映射`).toBeTruthy()
    }
  })

  it('每个消息状态都有面向用户的文案', () => {
    for (const s of MESSAGE_STATUSES) {
      expect(MESSAGE_STATUS_LABELS[s], `状态 ${s} 缺文案映射`).toBeTruthy()
    }
  })

  it('每个角色都有面向用户的文案', () => {
    for (const r of CHAT_ROLES) {
      expect(MESSAGE_ROLE_LABELS[r], `角色 ${r} 缺文案映射`).toBeTruthy()
    }
  })

  it('未登记的取值原样返回（便于发现新取值，而不是静默显示空白）', () => {
    expect(messageTypeLabel('brand_new_type')).toBe('brand_new_type')
    expect(messageStatusLabel('brand_new_status')).toBe('brand_new_status')
    expect(messageRoleLabel('brand_new_role')).toBe('brand_new_role')
  })

  it('压缩节点有文案，不会把 compression 原样漏给用户', () => {
    expect(messageTypeLabel(MESSAGE_TYPE_COMPRESSION)).toBe('上下文压缩')
  })

  it('中止是独立状态且有文案（不是 failed 的别名）', () => {
    expect(messageStatusLabel(MESSAGE_STATUS_ABORTED)).toBe('已中止')
    expect(isUnsettledMessageStatus(MESSAGE_STATUS_ABORTED)).toBe(true)
    expect(isUnsettledMessageStatus(MESSAGE_STATUS_FAILED)).toBe(true)
    expect(isUnsettledMessageStatus(MESSAGE_STATUS_COMPLETED)).toBe(false)
  })
})

describe('messageIcon：判定顺序', () => {
  it('用户消息优先于类型', () => {
    expect(messageIcon(facets({ role: CHAT_ROLE_USER }))).toBe('👤')
  })

  it('合成请求节点优先于它自身的 type=text', () => {
    expect(messageIcon(facets({ type: MESSAGE_TYPE_TEXT, toolRequest: true }))).toBe('📤')
  })

  it('各类型各自的图标', () => {
    expect(messageIcon(facets({ type: MESSAGE_TYPE_USER_PROMPT }))).toBe('❓')
    expect(messageIcon(facets({ type: MESSAGE_TYPE_COMPRESSION }))).toBe('🗜')
    expect(messageIcon(facets({ type: MESSAGE_TYPE_REASONING }))).toBe('💭')
    expect(messageIcon(facets({ type: MESSAGE_TYPE_TOOL_CALL }))).toBe('🔧')
  })

  it('Turn 按是否子会话区分：子会话用 ↳，根级助手用 AI', () => {
    expect(messageIcon(facets({ type: MESSAGE_TYPE_TURN, subSession: false }))).toBe('AI')
    expect(messageIcon(facets({ type: MESSAGE_TYPE_TURN, subSession: true }))).toBe('↳')
  })

  it('工具返回用 ↩，其余叶子内容用 💬', () => {
    expect(messageIcon(facets({ role: CHAT_ROLE_TOOL, toolResult: true }))).toBe('↩')
    expect(messageIcon(facets({}))).toBe('💬')
  })
})

describe('messageTitle', () => {
  it('用户消息是「你」', () => {
    expect(messageTitle(facets({ role: CHAT_ROLE_USER }), '助手')).toBe('你')
  })

  it('提问按 promptKind 区分提问 / 工具确认', () => {
    expect(messageTitle(facets({ type: MESSAGE_TYPE_USER_PROMPT, promptKind: 'question' }), '助手')).toBe('提问')
    expect(messageTitle(facets({ type: MESSAGE_TYPE_USER_PROMPT, promptKind: 'confirm' }), '助手')).toBe('工具确认')
  })

  it('思考的标题随流式状态变化', () => {
    expect(
      messageTitle(facets({ type: MESSAGE_TYPE_REASONING, status: MESSAGE_STATUS_STREAMING }), '助手'),
    ).toBe('思考中…')
    expect(
      messageTitle(facets({ type: MESSAGE_TYPE_REASONING, status: MESSAGE_STATUS_COMPLETED }), '助手'),
    ).toBe('思考')
  })

  it('工具调用用节点名，缺省回退「工具」', () => {
    expect(messageTitle(facets({ type: MESSAGE_TYPE_TOOL_CALL, name: 'cmd' }), '助手')).toBe('cmd')
    expect(messageTitle(facets({ type: MESSAGE_TYPE_TOOL_CALL }), '助手')).toBe('工具')
  })

  it('Turn 的标题是智能体名（由调用方传入）', () => {
    expect(messageTitle(facets({ type: MESSAGE_TYPE_TURN }), '探索者')).toBe('探索者')
  })

  it('合成请求节点是「请求」，工具返回是「响应」', () => {
    expect(messageTitle(facets({ toolRequest: true }), '助手')).toBe('请求')
    expect(messageTitle(facets({ role: CHAT_ROLE_TOOL, toolResult: true }), '助手')).toBe('响应')
  })
})

describe('messageStatusTag / messageStatusTone', () => {
  it('工具调用的非终态都有标签（折叠态下唯一的运行中信号）', () => {
    const t = (s: MessageFacets['status']) =>
      messageStatusTag(facets({ type: MESSAGE_TYPE_TOOL_CALL, status: s }))
    expect(t(MESSAGE_STATUS_STREAMING)).toBe('运行中')
    expect(t(MESSAGE_STATUS_WAITING_USER_ACTION)).toBe('待确认')
    expect(t(MESSAGE_STATUS_FAILED)).toBe('失败')
    expect(t(MESSAGE_STATUS_ABORTED)).toBe('已中止')
  })

  it('终态 completed 刻意不给标签（否则把真正需要注意的状态淹掉）', () => {
    expect(
      messageStatusTag(facets({ type: MESSAGE_TYPE_TOOL_CALL, status: MESSAGE_STATUS_COMPLETED })),
    ).toBe('')
  })

  it('压缩节点有自己的标签（它同样默认折叠，标签是唯一信号）', () => {
    const c = (s: MessageFacets['status']) =>
      messageStatusTag(facets({ type: MESSAGE_TYPE_COMPRESSION, status: s }))
    expect(c(MESSAGE_STATUS_STREAMING)).toBe('压缩中')
    expect(c(MESSAGE_STATUS_FAILED)).toBe('未完成')
    expect(c(MESSAGE_STATUS_ABORTED)).toBe('已中止')
    expect(c(MESSAGE_STATUS_COMPLETED)).toBe('')
  })

  it('非动作节点没有标签', () => {
    expect(messageStatusTag(facets({ type: MESSAGE_TYPE_TEXT }))).toBe('')
    expect(messageStatusTag(facets({ type: MESSAGE_TYPE_TURN }))).toBe('')
  })

  it('中止不给红色角标（按一次停止不该看到一片失败红）', () => {
    expect(messageStatusTone(facets({ status: MESSAGE_STATUS_ABORTED }))).toBe('sub')
    expect(messageStatusTone(facets({ status: MESSAGE_STATUS_FAILED }))).toBe('err')
    expect(messageStatusTone(facets({ status: MESSAGE_STATUS_WAITING_USER_ACTION }))).toBe('warn')
    expect(messageStatusTone(facets({ status: MESSAGE_STATUS_STREAMING }))).toBe('run')
  })
})

describe('messageHeadModifier', () => {
  it('思考中与工具执行中共用标题呼吸动效', () => {
    expect(
      messageHeadModifier(facets({ type: MESSAGE_TYPE_REASONING, status: MESSAGE_STATUS_STREAMING }))
        .thinking,
    ).toBe(true)
    expect(
      messageHeadModifier(facets({ type: MESSAGE_TYPE_TOOL_CALL, status: MESSAGE_STATUS_STREAMING }))
        .thinking,
    ).toBe(true)
  })

  it('终态的工具调用不带动效', () => {
    expect(
      messageHeadModifier(facets({ type: MESSAGE_TYPE_TOOL_CALL, status: MESSAGE_STATUS_COMPLETED }))
        .thinking,
    ).toBe(false)
  })
})

describe('messageIsRunningAction', () => {
  it('只有工具执行与上下文压缩算「动作」，且必须仍在运行', () => {
    expect(
      messageIsRunningAction(facets({ type: MESSAGE_TYPE_TOOL_CALL, status: MESSAGE_STATUS_STREAMING })),
    ).toBe(true)
    expect(
      messageIsRunningAction(facets({ type: MESSAGE_TYPE_COMPRESSION, status: MESSAGE_STATUS_STREAMING })),
    ).toBe(true)
    expect(
      messageIsRunningAction(facets({ type: MESSAGE_TYPE_TOOL_CALL, status: MESSAGE_STATUS_COMPLETED })),
    ).toBe(false)
    expect(
      messageIsRunningAction(facets({ type: MESSAGE_TYPE_TEXT, status: MESSAGE_STATUS_STREAMING })),
    ).toBe(false)
  })
})

describe('折叠策略', () => {
  it('待审批与用户消息始终展开', () => {
    expect(messageDefaultOpen(facets({ status: MESSAGE_STATUS_WAITING_USER_ACTION }))).toBe(true)
    expect(messageDefaultOpen(facets({ role: CHAT_ROLE_USER }))).toBe(true)
  })

  it('工具调用默认单行；内含待审批或可恢复失败时展开', () => {
    expect(messageDefaultOpen(facets({ type: MESSAGE_TYPE_TOOL_CALL }))).toBe(false)
    expect(messageDefaultOpen(facets({ type: MESSAGE_TYPE_TOOL_CALL, hasWaitingChild: true }))).toBe(true)
    expect(messageDefaultOpen(facets({ type: MESSAGE_TYPE_TOOL_CALL, recoverable: true }))).toBe(true)
  })

  it('思考始终单行', () => {
    expect(messageDefaultOpen(facets({ type: MESSAGE_TYPE_REASONING }))).toBe(false)
  })

  it('正文：助手展开，工具结果收起', () => {
    expect(messageDefaultOpen(facets({ type: MESSAGE_TYPE_TEXT }))).toBe(true)
    expect(messageDefaultOpen(facets({ type: MESSAGE_TYPE_TEXT, role: CHAT_ROLE_TOOL }))).toBe(false)
  })

  it('提问与压缩默认展开', () => {
    expect(messageDefaultOpen(facets({ type: MESSAGE_TYPE_USER_PROMPT }))).toBe(true)
    expect(messageDefaultOpen(facets({ type: MESSAGE_TYPE_COMPRESSION }))).toBe(true)
  })

  it('深层级一律收起，用户手动切换后以其选择为准', () => {
    const f = facets({ type: MESSAGE_TYPE_TEXT })
    expect(effectiveOpenOf(f, DEEP_COLLAPSE_LEVEL, false, true)).toBe(false)
    expect(effectiveOpenOf(f, DEEP_COLLAPSE_LEVEL, true, true)).toBe(true)
    expect(effectiveOpenOf(f, DEEP_COLLAPSE_LEVEL, true, false)).toBe(false)
  })

  it('首次点击默认收起的节点必须真的展开（防「点了没反应」）', () => {
    // 默认收起 + 未手动切换过：userOpen 初值为 true，若按固定初值取反会得到 false，
    // 展示态不变 → 表现为第一次点击无反应。
    const f = facets({ type: MESSAGE_TYPE_REASONING })
    expect(effectiveOpenOf(f, 0, false, true)).toBe(false)
    expect(nextOpenOf(f, 0, false, true)).toBe(true)
  })

  it('首次点击默认展开的节点必须真的收起', () => {
    const f = facets({ type: MESSAGE_TYPE_TEXT })
    expect(effectiveOpenOf(f, 0, false, true)).toBe(true)
    expect(nextOpenOf(f, 0, false, true)).toBe(false)
  })
})

describe('agentNameOf', () => {
  it('优先级：节点名 > meta.agent_name > meta.agent_id > 按子会话兜底', () => {
    expect(agentNameOf({ name: 'n', meta: { agent_name: 'm' } }, false)).toBe('n')
    expect(agentNameOf({ meta: { agent_name: 'm' } }, false)).toBe('m')
    expect(agentNameOf({ meta: { agent_id: 'id1' } }, false)).toBe('id1')
    expect(agentNameOf({}, false)).toBe('助手')
    expect(agentNameOf({}, true)).toBe('子智能体')
  })
})

describe('facetsOf：props → facets 的唯一实现', () => {
  it('缺省值：无 role/type/status 的节点按「助手正文、已结束」处理', () => {
    const f = facetsOf({ id: 'm1' })
    expect(f.role).toBe(CHAT_ROLE_ASSISTANT)
    expect(f.type).toBe(MESSAGE_TYPE_TEXT)
    expect(f.status).toBe(MESSAGE_STATUS_COMPLETED)
    expect(f.responseText).toBe(false)
  })

  it('子会话 Turn = role=tool 的 turn（工具「过程」段）', () => {
    expect(facetsOf({ id: 't', type: MESSAGE_TYPE_TURN, role: CHAT_ROLE_TOOL }).subSession).toBe(true)
    expect(facetsOf({ id: 't', type: MESSAGE_TYPE_TURN, role: CHAT_ROLE_ASSISTANT }).subSession).toBe(false)
  })

  it('工具返回必须限定父节点是 tool_call（否则子会话内部文本会被误判）', () => {
    const node = { id: 'r', type: MESSAGE_TYPE_TEXT, role: CHAT_ROLE_TOOL } as const
    expect(facetsOf(node, MESSAGE_TYPE_TOOL_CALL).toolResult).toBe(true)
    expect(facetsOf(node, MESSAGE_TYPE_TURN).toolResult).toBe(false)
    expect(facetsOf(node, undefined).toolResult).toBe(false)
  })

  it('正文直排必须限定父节点是 turn 且自身是 text（思考要保留单行行头）', () => {
    expect(facetsOf({ id: 'x', type: MESSAGE_TYPE_TEXT }, MESSAGE_TYPE_TURN).responseText).toBe(true)
    expect(facetsOf({ id: 'r', type: MESSAGE_TYPE_REASONING }, MESSAGE_TYPE_TURN).responseText).toBe(false)
    expect(facetsOf({ id: 'x', type: MESSAGE_TYPE_TEXT }, MESSAGE_TYPE_TOOL_CALL).responseText).toBe(false)
  })

  it('后端 meta 字段只在这一点被解释：recoverable / failure_kind / __toolRequest / heartbeat', () => {
    const f = facetsOf({
      id: 'tc',
      type: MESSAGE_TYPE_TOOL_CALL,
      status: MESSAGE_STATUS_FAILED,
      meta: { recoverable: true, failure_kind: 'error' },
    })
    expect(f.recoverable).toBe(true)
    expect(f.failureKind).toBe('error')
    expect(facetsOf({ id: 'x', meta: { __toolRequest: true } }).toolRequest).toBe(true)
    expect(messageIsHeartbeat({ meta: { heartbeat: true } })).toBe(true)
    expect(messageIsHeartbeat({ meta: {} })).toBe(false)
  })

  it('待审批子节点参与折叠判定（工具卡的展开例外之一）', () => {
    const f = facetsOf({
      id: 'tc',
      type: MESSAGE_TYPE_TOOL_CALL,
      children: [{ id: 'c', status: MESSAGE_STATUS_WAITING_USER_ACTION }],
    })
    expect(f.hasWaitingChild).toBe(true)
  })
})

describe('messageStartedAt：锚点只取后端值', () => {
  it('有效数字才返回，缺失 / 非数字 / 非正数一律 null（不编一个数）', () => {
    expect(messageStartedAt({ meta: { started_at: 1234 } })).toBe(1234)
    expect(messageStartedAt({ meta: {} })).toBeNull()
    expect(messageStartedAt({})).toBeNull()
    expect(messageStartedAt({ meta: { started_at: 0 } })).toBeNull()
    expect(messageStartedAt({ meta: { started_at: '1234' } })).toBeNull()
  })
})

describe('消息级业务规则（纯函数，原先散在渲染组件里）', () => {
  const tool = (p: Partial<MessageFacets>) =>
    facets({ type: MESSAGE_TYPE_TOOL_CALL, status: MESSAGE_STATUS_FAILED, ...p })

  it('只有「工具调用 + 失败 + 后端标记可恢复」才可就地重试', () => {
    expect(canRetryTool(tool({ recoverable: true }))).toBe(true)
    // 运行中的工具失败是信息性的（错误结果已喂给 LLM），给重试按钮只会误导
    expect(canRetryTool(tool({ recoverable: false }))).toBe(false)
    // 非工具调用不在此列：思考/正文/请求的失败归 Turn 级重试
    expect(canRetryTool(facets({ status: MESSAGE_STATUS_FAILED, recoverable: true }))).toBe(false)
    // 非失败态没有重试语义
    expect(canRetryTool(tool({ status: MESSAGE_STATUS_COMPLETED, recoverable: true }))).toBe(false)
  })

  it('补充参数只在 failure_kind=error 时可恢复失败上提供', () => {
    expect(canSupplyToolArgs(tool({ recoverable: true, failureKind: 'error' }))).toBe(true)
    expect(canSupplyToolArgs(tool({ recoverable: true, failureKind: 'timeout' }))).toBe(false)
    expect(canSupplyToolArgs(tool({ recoverable: true }))).toBe(false)
  })
})

describe('missingResultNoteOf：「有请求、无响应」的兑底', () => {
  it('父节点自述「本批跳过」/「中止」→ 给出文案（历史数据真的没有结果子节点）', () => {
    expect(missingResultNoteOf({ meta: { failure_kind: 'not_executed' } })).toContain('未执行')
    expect(missingResultNoteOf({ meta: { failure_kind: 'aborted' } })).toContain('已中止')
  })

  it('无标记 / 其它终态 → 不编话（宁可留白也不替工具伪造一份结果）', () => {
    expect(missingResultNoteOf({ meta: {} })).toBeNull()
    expect(missingResultNoteOf({})).toBeNull()
    // error：结果子节点由后端写入（错误文本就在里面），兑底会让同一句话出现两次
    expect(missingResultNoteOf({ meta: { failure_kind: 'error' } })).toBeNull()
  })
})

describe('messageShowsLiveBadge：「回复中…」只在正文流式时显示', () => {
  it('正文流式显示；思考/工具/压缩各有自己的运行信号，不重复挂', () => {
    const live = (type: MessageFacets['type']) =>
      messageShowsLiveBadge(facets({ type, status: MESSAGE_STATUS_STREAMING }))
    expect(live(MESSAGE_TYPE_TEXT)).toBe(true)
    expect(live(MESSAGE_TYPE_REASONING)).toBe(false)
    expect(live(MESSAGE_TYPE_TOOL_CALL)).toBe(false)
    expect(live(MESSAGE_TYPE_COMPRESSION)).toBe(false)
    expect(messageShowsLiveBadge(facets({ status: MESSAGE_STATUS_COMPLETED }))).toBe(false)
  })
})

describe('messageRetryTargetOf：重试分派（粒度由节点类型决定）', () => {
  it('工具调用失败 → 单工具重试，锚点是它自己', () => {
    expect(
      messageRetryTargetOf({ id: 'tc1', type: MESSAGE_TYPE_TOOL_CALL, status: MESSAGE_STATUS_FAILED }),
    ).toEqual({ action: 'retry', targetId: 'tc1' })
  })

  it('Turn 失败 → 整轮重试，锚点是 Turn 自己', () => {
    expect(
      messageRetryTargetOf({ id: 't1', type: MESSAGE_TYPE_TURN, status: MESSAGE_STATUS_FAILED }),
    ).toEqual({ action: 'retry_turn', targetId: 't1' })
  })

  it('Turn 下的叶子失败 → 回溯到父 Turn（后端要求 target_id 指向 Failed Turn）', () => {
    expect(
      messageRetryTargetOf({
        id: 'x1',
        type: MESSAGE_TYPE_TEXT,
        status: MESSAGE_STATUS_FAILED,
        parent_id: 't1',
      }),
    ).toEqual({ action: 'retry_turn', targetId: 't1' })
    expect(
      messageRetryTargetOf({
        id: 'r1',
        type: MESSAGE_TYPE_REASONING,
        status: MESSAGE_STATUS_FAILED,
        parent_id: 't1',
      }),
    ).toEqual({ action: 'retry_turn', targetId: 't1' })
  })

  it('无父节点可回溯时退回自身（不发出空 targetId）', () => {
    expect(
      messageRetryTargetOf({ id: 'x1', type: MESSAGE_TYPE_TEXT, status: MESSAGE_STATUS_FAILED }),
    ).toEqual({ action: 'retry_turn', targetId: 'x1' })
  })

  it('中止的 Turn 同样走整轮重试（这一轮只跑了一半）', () => {
    expect(
      messageRetryTargetOf({ id: 't1', type: MESSAGE_TYPE_TURN, status: MESSAGE_STATUS_ABORTED }),
    ).toEqual({ action: 'retry_turn', targetId: 't1' })
  })

  it('中止的工具调用不按「单工具」处理（与改造前的行为一致）', () => {
    expect(
      messageRetryTargetOf({
        id: 'tc1',
        type: MESSAGE_TYPE_TOOL_CALL,
        status: MESSAGE_STATUS_ABORTED,
        parent_id: 't1',
      }),
    ).toEqual({ action: 'retry_turn', targetId: 't1' })
  })

  it('缺省 type 按正文处理 → 整轮重试', () => {
    expect(messageRetryTargetOf({ id: 'x1', status: MESSAGE_STATUS_FAILED, parent_id: 't1' })).toEqual(
      { action: 'retry_turn', targetId: 't1' },
    )
  })

  /**
   * 压缩失败是**第三种粒度**：删的是压缩节点本身（系统动作），不是响应子树。
   *
   * 回归动机：压缩节点是根级节点（无 `parent_id`），若落到"回溯到父 Turn"的兜底
   * 分支，`target_id` 会是它自己而类型是 `Compression` —— 后端 `process_retry_turn`
   * 要求 Failed **Turn**，会直接 NotFound 拒绝。所以必须在工具分支之前单独分派。
   */
  it('压缩失败 → 压缩粒度（锚点是压缩节点自己，不回退到父 Turn）', () => {
    expect(
      messageRetryTargetOf({
        id: 'cp1',
        type: MESSAGE_TYPE_COMPRESSION,
        status: MESSAGE_STATUS_FAILED,
      }),
    ).toEqual({ action: 'retry_compaction', targetId: 'cp1' })
  })

  it('压缩失败即便带了 parent_id 也不回溯（它不是对话轮次的一部分）', () => {
    expect(
      messageRetryTargetOf({
        id: 'cp1',
        type: MESSAGE_TYPE_COMPRESSION,
        status: MESSAGE_STATUS_FAILED,
        parent_id: 't1',
      }),
    ).toEqual({ action: 'retry_compaction', targetId: 'cp1' })
  })

  it('未失败的压缩节点不发重试（成功 / 压缩中都没有入口）', () => {
    // `as const` 不可省：数组字面量里的字面量类型会被拓宽成 `string`，
    // 于是 `status` 无法赋给 `MessageStatus`（与下面几处同理）
    for (const status of [MESSAGE_STATUS_COMPLETED, MESSAGE_STATUS_STREAMING] as const) {
      expect(
        messageRetryTargetOf({ id: 'cp1', type: MESSAGE_TYPE_COMPRESSION, status }),
      ).toEqual({ action: 'retry_turn', targetId: 'cp1' })
    }
  })
})

/**
 * 压缩失败能否重试 —— 「只要失败就给入口」，不按 `failure_kind` 分档。
 *
 * 分档看起来更"聪明"（`input_over_limit` 重试必然再失败），但会掐掉唯一的出路：
 * 用户换一个上下文更大的模型后，同一次压缩就能成功。失败原因已写在节点正文里，
 * 由用户判断，不由前端替他决定"别试了"。
 */
describe('canRetryCompaction：压缩失败的重试入口', () => {
  it('压缩 + 失败 → 给入口', () => {
    expect(
      canRetryCompaction(
        facets({ type: MESSAGE_TYPE_COMPRESSION, status: MESSAGE_STATUS_FAILED }),
      ),
    ).toBe(true)
  })

  it('不按 failure_kind 分档：任何原因码都给入口', () => {
    for (const failureKind of ['llm_error', 'invalid_snapshot', 'input_over_limit']) {
      expect(
        canRetryCompaction(
          facets({ type: MESSAGE_TYPE_COMPRESSION, status: MESSAGE_STATUS_FAILED, failureKind }),
        ),
      ).toBe(true)
    }
  })

  it('非失败态（压缩中 / 已完成 / 已中止）不给入口', () => {
    for (const status of [
      MESSAGE_STATUS_STREAMING,
      MESSAGE_STATUS_COMPLETED,
      MESSAGE_STATUS_ABORTED,
      MESSAGE_STATUS_WAITING_USER_ACTION,
    ] as const) {
      expect(canRetryCompaction(facets({ type: MESSAGE_TYPE_COMPRESSION, status }))).toBe(false)
    }
  })

  it('非压缩类型不给入口（工具 / Turn 各有自己的重试语义）', () => {
    for (const type of [MESSAGE_TYPE_TURN, MESSAGE_TYPE_TOOL_CALL, MESSAGE_TYPE_TEXT] as const) {
      expect(canRetryCompaction(facets({ type, status: MESSAGE_STATUS_FAILED }))).toBe(false)
    }
  })
})

describe('messageRendererKey：渲染形态分派', () => {
  const key = (p: Partial<MessageFacets>) => messageRendererKey(facets(p))

  it('Turn 最先判定：任何 Turn 都走分组形态', () => {
    expect(key({ type: MESSAGE_TYPE_TURN })).toBe('turn')
    expect(key({ type: MESSAGE_TYPE_TURN, subSession: true })).toBe('turn')
  })

  it('角色优先于类型：用户消息一律走气泡', () => {
    expect(key({ role: CHAT_ROLE_USER, type: MESSAGE_TYPE_TEXT })).toBe('text')
    expect(key({ role: CHAT_ROLE_USER, type: MESSAGE_TYPE_TOOL_CALL })).toBe('text')
  })

  it('其余按类型分派', () => {
    expect(key({ type: MESSAGE_TYPE_TEXT })).toBe('text')
    expect(key({ type: MESSAGE_TYPE_REASONING })).toBe('text')
    expect(key({ type: MESSAGE_TYPE_TOOL_CALL })).toBe('tool_call')
    expect(key({ type: MESSAGE_TYPE_USER_PROMPT })).toBe('user_prompt')
    expect(key({ type: MESSAGE_TYPE_COMPRESSION })).toBe('compression')
  })

  it('未登记取值 → fallback（后端先行上线新 type 时旧前端仍能显示内容）', () => {
    expect(key({ type: 'brand_new_type' as MessageFacets['type'] })).toBe('fallback')
  })

  it('每个已登记的消息类型都分派到非 fallback 的渲染器（防漏登记）', () => {
    for (const t of MESSAGE_TYPES) {
      expect(messageRendererKey(facets({ type: t })), `类型 ${t} 未分派到专属渲染器`).not.toBe(
        'fallback',
      )
    }
  })

  // 既有的**有意分歧**：图标/标题是「角色优先于类型」，渲染器是「Turn 最先」。
  // 两者对 `role=user + type=turn` 这种（实际不可达的）组合给出不同答案。
  // 之所以钉住而不是统一：Turn 节点带 children，必须走分组渲染器，否则整轮内容
  // 会塌成一条用户气泡；而图标侧让「用户」胜出更符合直觉。统一任何一边都会
  // 改掉一处既有语义，故保留分歧并在此显式记录——改动它必须是有意的。
  it('role=user + type=turn：图标按角色、渲染器按 Turn（有意分歧，勿静默统一）', () => {
    const f = facets({ role: CHAT_ROLE_USER, type: MESSAGE_TYPE_TURN })
    expect(messageIcon(f)).toBe('👤')
    expect(messageTitle(f, '助手')).toBe('你')
    expect(messageRendererKey(f)).toBe('turn')
  })
})
