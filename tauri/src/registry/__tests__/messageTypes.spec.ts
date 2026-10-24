/**
 * 消息呈现注册表 — 前端纯 UI 映射单测（node 环境）
 *
 * 覆盖三类：
 * 1. **词表覆盖率**——`NODE_KINDS` / `NODE_STATUSES` / `CHAT_ROLES` 的每个取值
 *    都必须在文案表里有映射。漏一个就会把机制词原样摆给用户看，这条断言就是防它。
 * 2. 头部呈现（图标 / 标题 / 状态标签 / 色调 / 修饰类）的判定顺序。
 * 3. 折叠策略与「首次点击」语义。
 *
 * 前端分派轴只剩 wire 五值 `NodeKind`。同格内的业务差异（思考 vs 压缩、
 * 整轮 vs 工具）由后端在交互面折成 `title` / `icon` / `actions` **数据**，本表只定义
 * 每格的「多数形态」缺省——不再有业务形态标识，也不读 `meta.style_tag`。
 *
 * 本文件只验证纯映射；组件的 props → facets 摊平与递归渲染在
 * `components/__tests__/MessageNode.spec.ts` 里断言。
 */

import { describe, expect, it } from 'vitest'
import {
  CHAT_ROLES,
  NODE_STATUSES,
  NODE_STATUS_ABORTED,
  NODE_STATUS_COMPLETED,
  NODE_STATUS_FAILED,
  NODE_STATUS_PENDING,
  NODE_STATUS_REMOVED,
  NODE_STATUS_STREAMING,
  NODE_STATUS_WAITING_USER_ACTION,
  NODE_KINDS,
  NODE_KIND_TYPING,
  NODE_KIND_BODY,
  NODE_KIND_SKELETON,
  NODE_KIND_NOTICE,
  NODE_KIND_INTERACTION,
  CHAT_ROLE_ASSISTANT,
  CHAT_ROLE_TOOL,
  CHAT_ROLE_USER,
  isUnsettledNodeStatus,
  nodeKindOf,
  type ChatNode,
  type NodeKind,
} from '@/schemas/chat_node'
import {
  DEEP_COLLAPSE_LEVEL,
  MESSAGE_ROLE_LABELS,
  NODE_STATUS_LABELS,
  NODE_KIND_LABELS,
  agentNameOf,
  messageIsCompacted,
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
  messagePreviewFollowsLiveEdge,
  messageRendererKey,
  messageTypeOf,
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
  nodePrincipal,
  principalNamespace,
  principalPresentationOf,
  synthesizeTypingChildren,
  typingIdOf,
  isTypingHost,
  type MessageFacets,
} from '../messageTypes'

/** 构造 facets：只写关心的字段，其余取「普通助手正文」缺省 */
function facets(partial: Partial<MessageFacets> = {}): MessageFacets {
  return {
    role: CHAT_ROLE_ASSISTANT,
    type: NODE_KIND_BODY,
    status: NODE_STATUS_COMPLETED,
    subSession: false,
    toolRequest: false,
    toolResult: false,
    hasWaitingChild: false,
    recoverable: false,
    compacted: false,
    responseText: false,
    ...partial,
  }
}

describe('词表覆盖率（防机制词泄漏给用户）', () => {
  it('每个 wire 五值都有面向用户的文案', () => {
    for (const t of NODE_KINDS) {
      expect(NODE_KIND_LABELS[t], `形态 ${t} 缺文案映射`).toBeTruthy()
    }
  })

  it('每个消息状态都有面向用户的文案', () => {
    for (const s of NODE_STATUSES) {
      expect(NODE_STATUS_LABELS[s], `状态 ${s} 缺文案映射`).toBeTruthy()
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

  it('五值格的业务差异不落在文案表——后端 title 才是差异来源', () => {
    // 折叠叶这一格既容纳「思考」也容纳「压缩」，前端表只给多数文案「系统提示」；
    // 「上下文压缩」这类专名由后端 title 数据给出（见下 messageTitle 的 title 优先）。
    expect(messageTypeLabel(NODE_KIND_NOTICE)).toBe('系统提示')
    expect(messageTitle(facets({ type: NODE_KIND_NOTICE, title: '上下文压缩' }), '助手')).toBe(
      '上下文压缩',
    )
  })

  it('中止是独立状态且有文案（不是 failed 的别名）', () => {
    expect(messageStatusLabel(NODE_STATUS_ABORTED)).toBe('已中止')
    expect(isUnsettledNodeStatus(NODE_STATUS_ABORTED)).toBe(true)
    expect(isUnsettledNodeStatus(NODE_STATUS_FAILED)).toBe(true)
    expect(isUnsettledNodeStatus(NODE_STATUS_COMPLETED)).toBe(false)
  })
})

describe('messageIcon：判定顺序（数据优先，退多数形态表）', () => {
  it('后端 title/icon 数据最优先于一切', () => {
    expect(messageIcon(facets({ icon: '🛠️' }))).toBe('🛠️')
  })

  it('用户消息优先于类型', () => {
    expect(messageIcon(facets({ role: CHAT_ROLE_USER }))).toBe('👤')
  })

  it('合成请求节点优先于它自身的 body 形态', () => {
    expect(messageIcon(facets({ type: NODE_KIND_BODY, toolRequest: true }))).toBe('📤')
  })

  it('折叠叶 / 容器 / 交互各取自己格的缺省图标', () => {
    expect(messageIcon(facets({ type: NODE_KIND_NOTICE }))).toBe('💭')
    expect(messageIcon(facets({ type: NODE_KIND_INTERACTION }))).toBe('❓')
  })

  it('容器按是否子会话区分：子会话用 ↳，根级用 AI', () => {
    expect(messageIcon(facets({ type: NODE_KIND_SKELETON, subSession: false }))).toBe('AI')
    expect(messageIcon(facets({ type: NODE_KIND_SKELETON, subSession: true }))).toBe('↳')
  })

  it('工具返回用 ↩，其余叶子内容用 💬', () => {
    expect(messageIcon(facets({ role: CHAT_ROLE_TOOL, toolResult: true }))).toBe('↩')
    expect(messageIcon(facets({}))).toBe('💬')
  })
})

describe('messageTitle', () => {
  it('后端 title 数据优先', () => {
    expect(messageTitle(facets({ title: 'vdfs_list' }), '助手')).toBe('vdfs_list')
  })

  it('用户消息是「你」', () => {
    expect(messageTitle(facets({ role: CHAT_ROLE_USER }), '助手')).toBe('你')
  })

  it('交互按 promptKind 区分提问 / 工具确认', () => {
    expect(messageTitle(facets({ type: NODE_KIND_INTERACTION, promptKind: 'question' }), '助手')).toBe('提问')
    expect(messageTitle(facets({ type: NODE_KIND_INTERACTION, promptKind: 'confirm' }), '助手')).toBe('工具确认')
  })

  it('折叠叶的标题随流式状态变化', () => {
    expect(
      messageTitle(facets({ type: NODE_KIND_NOTICE, status: NODE_STATUS_STREAMING }), '助手'),
    ).toBe('处理中…')
    expect(
      messageTitle(facets({ type: NODE_KIND_NOTICE, status: NODE_STATUS_COMPLETED }), '助手'),
    ).toBe('提示')
  })

  it('容器的标题是智能体名（由调用方传入，专名靠后端 title）', () => {
    expect(messageTitle(facets({ type: NODE_KIND_SKELETON }), '探索者')).toBe('探索者')
  })

  it('合成请求节点是「请求」，工具返回是「响应」', () => {
    expect(messageTitle(facets({ toolRequest: true }), '助手')).toBe('请求')
    expect(messageTitle(facets({ role: CHAT_ROLE_TOOL, toolResult: true }), '助手')).toBe('响应')
  })
})

describe('messageStatusTag / messageStatusTone', () => {
  it('容器骨架的非终态都有标签（折叠态下唯一的运行中信号）', () => {
    const t = (s: MessageFacets['status']) =>
      messageStatusTag(facets({ type: NODE_KIND_SKELETON, status: s }))
    expect(t(NODE_STATUS_STREAMING)).toBe('运行中')
    expect(t(NODE_STATUS_WAITING_USER_ACTION)).toBe('待确认')
    expect(t(NODE_STATUS_FAILED)).toBe('失败')
    expect(t(NODE_STATUS_ABORTED)).toBe('已中止')
  })

  it('容器终态 completed 刻意不给标签（否则把真正需要注意的状态淹掉）', () => {
    expect(
      messageStatusTag(facets({ type: NODE_KIND_SKELETON, status: NODE_STATUS_COMPLETED })),
    ).toBe('')
  })

  it('折叠叶 / 交互 / 正文无专属标签（业务专名靠 title 数据，不靠标签）', () => {
    expect(messageStatusTag(facets({ type: NODE_KIND_NOTICE, status: NODE_STATUS_STREAMING }))).toBe('')
    expect(messageStatusTag(facets({ type: NODE_KIND_BODY }))).toBe('')
    expect(messageStatusTag(facets({ type: NODE_KIND_INTERACTION }))).toBe('')
  })

  it('中止不给红色角标（按一次停止不该看到一片失败红）', () => {
    expect(messageStatusTone(facets({ status: NODE_STATUS_ABORTED }))).toBe('sub')
    expect(messageStatusTone(facets({ status: NODE_STATUS_FAILED }))).toBe('err')
    expect(messageStatusTone(facets({ status: NODE_STATUS_WAITING_USER_ACTION }))).toBe('warn')
    expect(messageStatusTone(facets({ status: NODE_STATUS_STREAMING }))).toBe('run')
  })
})

describe('messageHeadModifier', () => {
  it('折叠叶与容器在流式中共用标题呼吸动效', () => {
    expect(
      messageHeadModifier(facets({ type: NODE_KIND_NOTICE, status: NODE_STATUS_STREAMING }))
        .thinking,
    ).toBe(true)
    expect(
      messageHeadModifier(facets({ type: NODE_KIND_SKELETON, status: NODE_STATUS_STREAMING }))
        .thinking,
    ).toBe(true)
  })

  it('终态的容器不带动效', () => {
    expect(
      messageHeadModifier(facets({ type: NODE_KIND_SKELETON, status: NODE_STATUS_COMPLETED }))
        .thinking,
    ).toBe(false)
  })

  it('色调钩子按形状给：容器格 skeleton、折叠叶 notice', () => {
    expect(messageHeadModifier(facets({ type: NODE_KIND_SKELETON })).skeleton).toBe(true)
    expect(messageHeadModifier(facets({ type: NODE_KIND_NOTICE })).notice).toBe(true)
    expect(messageHeadModifier(facets({ type: NODE_KIND_BODY })).skeleton).toBe(false)
  })
})

describe('messageIsRunningAction', () => {
  it('只有容器骨架算「动作」，且必须仍在运行', () => {
    expect(
      messageIsRunningAction(facets({ type: NODE_KIND_SKELETON, status: NODE_STATUS_STREAMING })),
    ).toBe(true)
    expect(
      messageIsRunningAction(facets({ type: NODE_KIND_SKELETON, status: NODE_STATUS_COMPLETED })),
    ).toBe(false)
    expect(
      messageIsRunningAction(facets({ type: NODE_KIND_BODY, status: NODE_STATUS_STREAMING })),
    ).toBe(false)
    expect(
      messageIsRunningAction(facets({ type: NODE_KIND_NOTICE, status: NODE_STATUS_STREAMING })),
    ).toBe(false)
  })
})

describe('折叠策略', () => {
  it('待审批与用户消息始终展开', () => {
    expect(messageDefaultOpen(facets({ status: NODE_STATUS_WAITING_USER_ACTION }))).toBe(true)
    expect(messageDefaultOpen(facets({ role: CHAT_ROLE_USER }))).toBe(true)
  })

  it('容器默认展开（整轮是对话主体；工具与整轮同构）', () => {
    expect(messageDefaultOpen(facets({ type: NODE_KIND_SKELETON }))).toBe(true)
  })

  it('折叠叶始终单行', () => {
    expect(messageDefaultOpen(facets({ type: NODE_KIND_NOTICE }))).toBe(false)
  })

  it('正文：助手展开，工具结果收起', () => {
    expect(messageDefaultOpen(facets({ type: NODE_KIND_BODY }))).toBe(true)
    expect(messageDefaultOpen(facets({ type: NODE_KIND_BODY, role: CHAT_ROLE_TOOL }))).toBe(false)
  })

  it('交互默认展开', () => {
    expect(messageDefaultOpen(facets({ type: NODE_KIND_INTERACTION }))).toBe(true)
  })

  it('深层级一律收起，用户手动切换后以其选择为准', () => {
    const f = facets({ type: NODE_KIND_BODY })
    expect(effectiveOpenOf(f, DEEP_COLLAPSE_LEVEL, false, true)).toBe(false)
    expect(effectiveOpenOf(f, DEEP_COLLAPSE_LEVEL, true, true)).toBe(true)
    expect(effectiveOpenOf(f, DEEP_COLLAPSE_LEVEL, true, false)).toBe(false)
  })

  it('首次点击默认收起的节点必须真的展开（防「点了没反应」）', () => {
    // 默认收起 + 未手动切换过：userOpen 初值为 true，若按固定初值取反会得到 false，
    // 展示态不变 → 表现为第一次点击无反应。
    const f = facets({ type: NODE_KIND_NOTICE })
    expect(effectiveOpenOf(f, 0, false, true)).toBe(false)
    expect(nextOpenOf(f, 0, false, true)).toBe(true)
  })

  it('首次点击默认展开的节点必须真的收起', () => {
    const f = facets({ type: NODE_KIND_BODY })
    expect(effectiveOpenOf(f, 0, false, true)).toBe(true)
    expect(nextOpenOf(f, 0, false, true)).toBe(false)
  })
})

describe('messagePreviewFollowsLiveEdge：收起态摘要取哪一端', () => {
  it('只有「正在产生内容」才跟随末端；其余状态（含排队 / 等待响应 / 终态）取开头', () => {
    expect(messagePreviewFollowsLiveEdge(facets({ status: NODE_STATUS_STREAMING }))).toBe(true)
    // 排队中还没有内容、等待响应时内容不会自己长、终态内容已不变——都不该跟着走
    const nonLive = [
      NODE_STATUS_PENDING,
      NODE_STATUS_WAITING_USER_ACTION,
      NODE_STATUS_COMPLETED,
      NODE_STATUS_ABORTED,
      NODE_STATUS_FAILED,
      NODE_STATUS_REMOVED,
    ] as const
    for (const status of nonLive) {
      expect(messagePreviewFollowsLiveEdge(facets({ status })), `状态 ${status} 不应取末端`).toBe(
        false,
      )
    }
  })

  it('判定只看状态，与节点形态无关（折叠叶 / 容器 / 正文同样适用）', () => {
    const types = [NODE_KIND_NOTICE, NODE_KIND_SKELETON, NODE_KIND_BODY] as const
    for (const type of types) {
      expect(messagePreviewFollowsLiveEdge(facets({ type, status: NODE_STATUS_STREAMING }))).toBe(
        true,
      )
      expect(messagePreviewFollowsLiveEdge(facets({ type, status: NODE_STATUS_COMPLETED }))).toBe(
        false,
      )
    }
  })
})

describe('agentNameOf', () => {
  it('优先级：meta.agent_id > 按子会话兜底', () => {
    expect(agentNameOf({ meta: { agent_id: 'id1' } }, false)).toBe('id1')
    expect(agentNameOf({}, false)).toBe('助手')
    expect(agentNameOf({}, true)).toBe('子智能体')
  })
})

describe('facetsOf：props → facets 的唯一实现', () => {
  it('缺省值：无 role/type/status 的节点按「助手正文、已结束」处理', () => {
    const f = facetsOf({ id: 'm1' })
    expect(f.role).toBe(CHAT_ROLE_ASSISTANT)
    expect(f.type).toBe(NODE_KIND_BODY)
    expect(f.status).toBe(NODE_STATUS_COMPLETED)
    expect(f.responseText).toBe(false)
  })

  it('子会话过程段 = role=tool 的骨架', () => {
    expect(facetsOf({ id: 't', type: NODE_KIND_SKELETON, role: CHAT_ROLE_TOOL }).subSession).toBe(true)
    expect(facetsOf({ id: 't', type: NODE_KIND_SKELETON, role: CHAT_ROLE_ASSISTANT }).subSession).toBe(false)
  })

  it('工具返回必须限定父节点是骨架（否则子会话内部文本会被误判）', () => {
    const node = { id: 'r', type: NODE_KIND_BODY, role: CHAT_ROLE_TOOL } as const
    expect(facetsOf(node, NODE_KIND_SKELETON).toolResult).toBe(true)
    expect(facetsOf(node, NODE_KIND_BODY).toolResult).toBe(false)
    expect(facetsOf(node, undefined).toolResult).toBe(false)
  })

  it('正文直排必须限定父节点是骨架且自身是正文（折叠叶要保留单行行头）', () => {
    expect(facetsOf({ id: 'x', type: NODE_KIND_BODY }, NODE_KIND_SKELETON).responseText).toBe(true)
    expect(facetsOf({ id: 'r', type: NODE_KIND_NOTICE }, NODE_KIND_SKELETON).responseText).toBe(false)
    expect(facetsOf({ id: 'x', type: NODE_KIND_BODY }, NODE_KIND_BODY).responseText).toBe(false)
  })

  it('工具名取自 meta.tool_name（[T-147d-ii]：顶层 name 已退役）', () => {
    const f = facetsOf({
      id: 'tc',
      type: NODE_KIND_SKELETON,
      meta: { tool_name: 'vdfs_list' },
    })
    expect(f.name).toBe('vdfs_list')
    expect(facetsOf({ id: 'tc', type: NODE_KIND_SKELETON }).name).toBeUndefined()
  })

  it('呈现数据 title / icon / actions 从节点顶层直读（[T-155]）', () => {
    const f = facetsOf({
      id: 'x',
      type: NODE_KIND_NOTICE,
      title: '上下文压缩',
      icon: '🗜',
      actions: ['retry_compaction'] as ChatNode['actions'],
    })
    expect(f.title).toBe('上下文压缩')
    expect(f.icon).toBe('🗜')
    expect(f.actions).toEqual(['retry_compaction'])
  })

  it('后端 meta 字段只在这一点被解释：recoverable / failure_kind / __toolRequest / heartbeat', () => {
    const f = facetsOf({
      id: 'tc',
      type: NODE_KIND_SKELETON,
      status: NODE_STATUS_FAILED,
      meta: { recoverable: true, failure_kind: 'error' },
    })
    expect(f.recoverable).toBe(true)
    expect(f.failureKind).toBe('error')
    expect(facetsOf({ id: 'x', meta: { __toolRequest: true } }).toolRequest).toBe(true)
    expect(messageIsHeartbeat({ meta: { heartbeat: true } })).toBe(true)
    expect(messageIsHeartbeat({ meta: {} })).toBe(false)
  })

  it('待审批子节点参与折叠判定（容器的展开例外之一）', () => {
    const f = facetsOf({
      id: 'tc',
      type: NODE_KIND_SKELETON,
      children: [{ id: 'c', status: NODE_STATUS_WAITING_USER_ACTION }],
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

describe('消息级业务规则（纯函数，[T-155 B2a] 只读后端下发的 actions）', () => {
  const tool = (p: Partial<MessageFacets>) =>
    facets({ type: NODE_KIND_SKELETON, status: NODE_STATUS_FAILED, ...p })

  it('工具失败能否重试 = 后端是否下发 retry（不再看业务类型 / recoverable）', () => {
    expect(canRetryTool(tool({ actions: ['retry'] as MessageFacets['actions'] }))).toBe(true)
    // 后端未声明 retry → 不画（会话忙碌 / auto 模式信息性失败都由后端收敛掉）
    expect(canRetryTool(tool({ actions: [] as MessageFacets['actions'] }))).toBe(false)
    expect(canRetryTool(tool({}))).toBe(false)
    // 非失败态即便带 retry 也不给入口（后端不会给，但判据本身把住状态）
    expect(
      canRetryTool(
        tool({ status: NODE_STATUS_COMPLETED, actions: ['retry'] as MessageFacets['actions'] }),
      ),
    ).toBe(false)
  })

  it('补充参数 = 后端是否下发 supply（supply 的出现已由后端保证 failure_kind=error）', () => {
    expect(canSupplyToolArgs(tool({ actions: ['supply'] as MessageFacets['actions'] }))).toBe(true)
    expect(
      canSupplyToolArgs(tool({ actions: ['retry'] as MessageFacets['actions'] })),
    ).toBe(false)
    expect(canSupplyToolArgs(tool({}))).toBe(false)
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
  it('正文流式显示；折叠叶 / 容器 / typing 各有自己的运行信号，不重复挂', () => {
    const live = (type: MessageFacets['type']) =>
      messageShowsLiveBadge(facets({ type, status: NODE_STATUS_STREAMING }))
    expect(live(NODE_KIND_BODY)).toBe(true)
    expect(live(NODE_KIND_NOTICE)).toBe(false)
    expect(live(NODE_KIND_SKELETON)).toBe(false)
    expect(live(NODE_KIND_TYPING)).toBe(false)
    expect(messageShowsLiveBadge(facets({ status: NODE_STATUS_COMPLETED }))).toBe(false)
  })
})

describe('messageRetryTargetOf：重试分派（[T-155 B2a] 动作由后端 actions 决定，无 actions 走状态兜底）', () => {
  it('后端下发 retry → 单工具重试，锚点是它自己', () => {
    expect(
      messageRetryTargetOf({ id: 'tc1', status: NODE_STATUS_FAILED, actions: ['retry'] as ChatNode['actions'] }),
    ).toEqual({ action: 'retry', targetId: 'tc1' })
  })

  it('后端下发 retry_subtree → 整轮重试，锚点是骨架自己', () => {
    expect(
      messageRetryTargetOf({ id: 't1', status: NODE_STATUS_FAILED, actions: ['retry_subtree'] as ChatNode['actions'] }),
    ).toEqual({ action: 'retry_subtree', targetId: 't1' })
  })

  it('后端下发 retry_compaction → 压缩粒度，锚点是压缩节点自己', () => {
    expect(
      messageRetryTargetOf({ id: 'cp1', status: NODE_STATUS_FAILED, actions: ['retry_compaction'] as ChatNode['actions'] }),
    ).toEqual({ action: 'retry_compaction', targetId: 'cp1' })
  })

  it('兜底：无 actions 的失败叶子 → 整轮重试并回溯到父骨架', () => {
    expect(
      messageRetryTargetOf({ id: 'x1', status: NODE_STATUS_FAILED, parent_id: 't1' }),
    ).toEqual({ action: 'retry_subtree', targetId: 't1' })
    expect(
      messageRetryTargetOf({ id: 'r1', status: NODE_STATUS_FAILED, parent_id: 't1' }),
    ).toEqual({ action: 'retry_subtree', targetId: 't1' })
  })

  it('兜底：无父可回溯时退回自身（不发出空 targetId）', () => {
    expect(messageRetryTargetOf({ id: 'x1', status: NODE_STATUS_FAILED })).toEqual({
      action: 'retry_subtree',
      targetId: 'x1',
    })
  })

  it('兜底：中止态同样回溯到父骨架', () => {
    expect(
      messageRetryTargetOf({ id: 'tc1', status: NODE_STATUS_ABORTED, parent_id: 't1' }),
    ).toEqual({ action: 'retry_subtree', targetId: 't1' })
  })
})

/**
 * 压缩失败能否重试 —— 纯数据判据：后端下发 `retry_compaction` 才给入口。
 *
 * 「只要失败就给入口、不按 `failure_kind` 分档」这条业务决定已上移到后端 `derive_actions`
 * （[D-070 §决策 3]），前端只认动词是否存在。
 */
describe('canRetryCompaction：压缩失败的重试入口（读后端 actions）', () => {
  it('后端下发 retry_compaction + 失败 → 给入口', () => {
    expect(
      canRetryCompaction(
        facets({ status: NODE_STATUS_FAILED, actions: ['retry_compaction'] as MessageFacets['actions'] }),
      ),
    ).toBe(true)
  })

  it('非失败态即便带 retry_compaction 也不给入口（判据把住状态）', () => {
    for (const status of [
      NODE_STATUS_STREAMING,
      NODE_STATUS_COMPLETED,
      NODE_STATUS_ABORTED,
      NODE_STATUS_WAITING_USER_ACTION,
    ] as const) {
      expect(
        canRetryCompaction(
          facets({ status, actions: ['retry_compaction'] as MessageFacets['actions'] }),
        ),
      ).toBe(false)
    }
  })

  it('无 retry_compaction 动词 → 不给入口（不再按业务类型反推）', () => {
    for (const actions of [
      [] as MessageFacets['actions'],
      ['retry'] as MessageFacets['actions'],
    ]) {
      expect(canRetryCompaction(facets({ status: NODE_STATUS_FAILED, actions }))).toBe(false)
    }
  })
})

describe('messageRendererKey：渲染形态分派', () => {
  const key = (p: Partial<MessageFacets>) => messageRendererKey(facets(p))

  it('骨架最先判定：任何容器都走分组形态', () => {
    expect(key({ type: NODE_KIND_SKELETON })).toBe('skeleton')
    expect(key({ type: NODE_KIND_SKELETON, subSession: true })).toBe('skeleton')
  })

  it('压缩记忆先于角色：它是 role=user 的系统记忆，走折叠叶而非用户气泡', () => {
    expect(key({ role: CHAT_ROLE_USER, type: NODE_KIND_BODY, compacted: true })).toBe('notice')
  })

  it('角色优先于类型：用户消息（非记忆）一律走气泡', () => {
    expect(key({ role: CHAT_ROLE_USER, type: NODE_KIND_BODY })).toBe('body')
  })

  it('其余按形态分派', () => {
    expect(key({ type: NODE_KIND_BODY })).toBe('body')
    expect(key({ type: NODE_KIND_NOTICE })).toBe('notice')
    expect(key({ type: NODE_KIND_INTERACTION })).toBe('interaction')
  })

  it('typing 占位走 fallback（[T-151]：占位形态无专属组件，是有意的兜底）', () => {
    expect(key({ type: NODE_KIND_TYPING })).toBe('fallback')
  })

  it('未登记取值 → fallback（后端先行上线新 type 时旧前端仍能显示内容）', () => {
    expect(key({ type: 'brand_new_type' as MessageFacets['type'] })).toBe('fallback')
  })

  it('每个 wire 五值都分派到渲染器（防漏登记）', () => {
    for (const t of NODE_KINDS) {
      const resolved = messageRendererKey(facets({ type: t }))
      // typing 是唯一有意的 fallback（占位形态），其余四值必须有专属渲染器
      if (t === NODE_KIND_TYPING) {
        expect(resolved).toBe('fallback')
        continue
      }
      expect(resolved, `形态 ${t} 未分派到专属渲染器`).not.toBe('fallback')
    }
  })

  // 既有的**有意分歧**：图标/标题是「角色优先于类型」，渲染器是「骨架最先」。
  // 两者对 `role=user + type=skeleton` 这种（实际不可达的）组合给出不同答案。
  // 之所以钉住而不是统一：骨架节点带 children，必须走分组渲染器，否则骨架内容
  // 会塌成一条用户气泡；而图标侧让「用户」胜出更符合直觉。统一任何一边都会
  // 改掉一处既有语义，故保留分歧并在此显式记录——改动它必须是有意的。
  it('role=user + type=skeleton：图标按角色、渲染器按骨架（有意分歧，勿静默统一）', () => {
    const f = facets({ role: CHAT_ROLE_USER, type: NODE_KIND_SKELETON })
    expect(messageIcon(f)).toBe('👤')
    expect(messageTitle(f, '助手')).toBe('你')
    expect(messageRendererKey(f)).toBe('skeleton')
  })
})

describe('synthesizeTypingChildren：Typing 占位合成（[T-151] 三态）', () => {
  // 一个根级骨架（skeleton + 无父），是 typing 的宿主
  const runHost = (status: ChatNode['status'], id = 'run1'): ChatNode => ({
    id,
    type: NODE_KIND_SKELETON,
    status,
    seq: 1,
  })

  it('合成：骨架容器 streaming 且无子 ⇒ 产一条 typing 子', () => {
    const out = synthesizeTypingChildren([runHost(NODE_STATUS_STREAMING)])
    expect(out).toHaveLength(1)
    expect(out[0].id).toBe(typingIdOf('run1'))
    expect(out[0].parent_id).toBe('run1')
    expect(out[0].type).toBe(NODE_KIND_TYPING)
    expect(out[0].status).toBe(NODE_STATUS_STREAMING)
  })

  it('替换：同父第一个真实子到达 ⇒ 该父不再「无子」，不产 typing', () => {
    const out = synthesizeTypingChildren([
      runHost(NODE_STATUS_STREAMING),
      { id: 'txt', parent_id: 'run1', type: NODE_KIND_BODY, status: NODE_STATUS_STREAMING, seq: 2 },
    ])
    expect(out).toEqual([])
  })

  it('父收束：容器不再 streaming ⇒ 谓词不成立，typing 消失', () => {
    const out = synthesizeTypingChildren([runHost(NODE_STATUS_COMPLETED)])
    expect(out).toEqual([])
  })

  it('非容器（正文叶子）不合成——即便 streaming 且无子', () => {
    const out = synthesizeTypingChildren([
      { id: 'txt', type: NODE_KIND_BODY, status: NODE_STATUS_STREAMING, seq: 1 },
    ])
    expect(out).toEqual([])
  })

  it('子级骨架不合成——工具有自己的在途信号（「运行中 / 秒数」），挂「生成中」是错位', () => {
    // 宿主判据是「无父 = 根级骨架」：带 parent_id 的骨架（工具子骨架）不当 typing 宿主。
    const out = synthesizeTypingChildren([
      {
        id: 'tool1',
        parent_id: 'run1',
        type: NODE_KIND_SKELETON,
        status: NODE_STATUS_STREAMING,
        seq: 1,
      },
    ])
    expect(out).toEqual([])
  })

  it('已有的 typing 派生子不算「真实子」——宿主仍视为空、继续合成', () => {
    const out = synthesizeTypingChildren([
      runHost(NODE_STATUS_STREAMING),
      { id: typingIdOf('run1'), parent_id: 'run1', type: NODE_KIND_TYPING, status: NODE_STATUS_STREAMING },
    ])
    expect(out).toHaveLength(1)
    expect(out[0].parent_id).toBe('run1')
  })

  it('typingIdOf 以宿主为键、跨帧稳定；isTypingHost 只认 streaming 的根级骨架', () => {
    expect(typingIdOf('run1')).toBe(typingIdOf('run1'))
    expect(isTypingHost(runHost(NODE_STATUS_STREAMING))).toBe(true)
    expect(isTypingHost(runHost(NODE_STATUS_COMPLETED))).toBe(false)
    expect(isTypingHost({ id: 'x', type: NODE_KIND_BODY, status: NODE_STATUS_STREAMING })).toBe(false)
  })
})

describe('messageTypeOf：wire 五值归一（读边界唯一折叠点）', () => {
  it('五值直通，不读 meta.style_tag（同格业务差异靠数据分呈现）', () => {
    expect(messageTypeOf({ type: NODE_KIND_BODY })).toBe(NODE_KIND_BODY)
    expect(messageTypeOf({ type: NODE_KIND_SKELETON })).toBe(NODE_KIND_SKELETON)
    expect(messageTypeOf({ type: NODE_KIND_NOTICE })).toBe(NODE_KIND_NOTICE)
    expect(messageTypeOf({ type: NODE_KIND_INTERACTION })).toBe(NODE_KIND_INTERACTION)
    expect(messageTypeOf({ type: NODE_KIND_TYPING })).toBe(NODE_KIND_TYPING)
  })

  it('旧业务词一次性折叠到五值——reasoning/compression→notice，turn/tool_call→skeleton', () => {
    expect(messageTypeOf({ type: 'reasoning' as NodeKind })).toBe(NODE_KIND_NOTICE)
    expect(messageTypeOf({ type: 'compression' as NodeKind })).toBe(NODE_KIND_NOTICE)
    expect(messageTypeOf({ type: 'turn' as NodeKind })).toBe(NODE_KIND_SKELETON)
    expect(messageTypeOf({ type: 'tool_call' as NodeKind })).toBe(NODE_KIND_SKELETON)
    expect(messageTypeOf({ type: 'text' as NodeKind })).toBe(NODE_KIND_BODY)
    expect(messageTypeOf({ type: 'user_prompt' as NodeKind })).toBe(NODE_KIND_INTERACTION)
  })

  it('缺省 / 未知取值 → 正文（未标形态的帧按正文处理）', () => {
    expect(messageTypeOf({ type: undefined })).toBe(NODE_KIND_BODY)
    expect(messageTypeOf({ type: 'brand_new' as NodeKind })).toBe(NODE_KIND_BODY)
  })

  it('折叠幂等：五值再折一次仍是五值（落地口重复折叠不漂移）', () => {
    for (const t of NODE_KINDS) {
      expect(nodeKindOf(nodeKindOf(t))).toBe(t)
    }
  })

  it('折叠结果可直接进渲染分派', () => {
    expect(messageRendererKey(facets({ type: messageTypeOf({ type: 'turn' as NodeKind }) }))).toBe(
      'skeleton',
    )
    expect(messageRendererKey(facets({ type: messageTypeOf({ type: 'reasoning' as NodeKind }) }))).toBe(
      'notice',
    )
  })
})

/**
 * [] 判据 2：principal 前缀 → 图标/颜色/标签**一张独立表**。
 *
 * 主键是可路由地址的**命名空间**（`<ns>/<name>`），`kind` 只是兜底——比 role 折叠更细
 * （同为 tool，`vdfs/write` 与 `web/fetch` 各有身份徽标）。钉四件事：
 * 命名空间精确命中、未登记命名空间退回 kind、缺主体返回 undefined（不替它编徽标）、
 * 历史裸字符串 principal 一并归一后进表。
 */
describe('principal 前缀 → 身份呈现（[T-148] 独立表）', () => {
  it('各命名空间有自己的图标 / 色调 / 标签', () => {
    expect(principalPresentationOf('person/local')).toEqual({ icon: '👤', tone: 'person', label: '用户' })
    expect(principalPresentationOf('session/main')?.tone).toBe('agent')
    expect(principalPresentationOf('system/compactor')?.label).toBe('系统')
    // 工具族：三个命名空间标签互不相同（这正是「命名空间比 kind 细」的意义）
    expect(principalPresentationOf('vdfs/write')?.label).toBe('文件工具')
    expect(principalPresentationOf('web/fetch')?.label).toBe('联网工具')
    expect(principalPresentationOf('local/content_search')?.label).toBe('本地工具')
  })

  it('未登记命名空间退回 kind 表（新主体上线不判红）', () => {
    // `decider/reflex` 尚未进命名空间表 → 按 kind=agent 兜底
    expect(
      principalPresentationOf({ kind: 'agent', id: 'decider/reflex' } as const),
    ).toEqual({
      icon: '🤖',
      tone: 'agent',
      label: '智能体',
    })
    // 无斜杠地址（无命名空间）也退回 kind
    expect(principalPresentationOf({ kind: 'tool', id: 'bare_tool' } as const)?.tone).toBe('tool')
  })

  it('缺主体 → undefined（调用方不画徽标，而不是替它编一个）', () => {
    expect(principalPresentationOf(undefined)).toBeUndefined()
    expect(principalPresentationOf(null)).toBeUndefined()
  })

  it('地址字符串进表（"person/local"→用户，"session/main"→agent）', () => {
    expect(principalNamespace('person/local')).toBe('person')
    expect(principalNamespace('session/main')).toBe('session')
    expect(principalPresentationOf('person/local')?.label).toBe('用户')
    expect(principalPresentationOf('session/main')?.tone).toBe('agent')
  })

  it('facetsOf 带上 principal 呈现（表被消费，非死数据）', () => {
    const f = facetsOf({
      id: 'tc',
      type: NODE_KIND_SKELETON,
      principal: { kind: 'tool', id: 'vdfs/write' },
    })
    expect(f.principal?.label).toBe('文件工具')
    // 无 principal 的节点（存量 / 未接线）：呈现缺席，不是报错
    expect(facetsOf({ id: 'x' }).principal).toBeUndefined()
  })

  it('nodePrincipal 归一地址字符串为结构体，旧形态不再归一', () => {
    expect(nodePrincipal({ principal: { kind: 'agent', id: 'session/main' } })).toEqual({
      kind: 'agent',
      id: 'session/main',
    })
    expect(nodePrincipal({ principal: undefined })).toBeUndefined()
    // 旧形态没有任何生产来源（SP-008）：解不出族就当「尚未标身份」
    expect(
      nodePrincipal({ principal: 'agent:main' as unknown as { kind: 'agent'; id: string } }),
    ).toBeUndefined()
  })
})

/**
 * 压缩后的**历史记忆**（`meta.compacted`）。
 *
 * 它是一条 `role = user` 的消息——后端为了让对话以 user 开头而这么写。因此这段的
 * 每一条断言防的都是同一件事：**它不许被当成用户说的话**（右对齐气泡、「你」、
 * 悬停可编辑/删除）。判据是后端打的 `meta.compacted`，不是正文长什么样，也不是业务名。
 */
describe('压缩后的历史记忆：不是「你说的那句话」', () => {
  /** 后端那条记忆节点的形状：role=user、正文以内部标记开头、带 meta 标记，形态是折叠叶 */
  const memory = {
    id: 'm1',
    role: CHAT_ROLE_USER,
    type: NODE_KIND_NOTICE,
    status: NODE_STATUS_COMPLETED,
    content: '[CONTEXT SNAPSHOT — 压缩的历史记忆]\n…',
    meta: { compacted: true, post_tokens: 9000, transcript_path: '/s/t/a.json' },
  } as const

  it('facets 认后端标记（判定只有一处：`messageIsCompacted`）', () => {
    expect(facetsOf(memory).compacted).toBe(true)
    expect(messageIsCompacted(memory)).toBe(true)
    // 缺失 / 非 true 一律不算（不能靠正文猜）
    expect(messageIsCompacted({ meta: {} })).toBe(false)
    expect(messageIsCompacted({})).toBe(false)
  })

  it('渲染器优先于角色判定 → `notice`（折叠叶），而不是用户气泡 `body`', () => {
    expect(messageRendererKey(facetsOf(memory))).toBe('notice')
  })

  it('图标与标题自述「历史记忆」，不叫「你」', () => {
    const f = facetsOf(memory)
    expect(messageIcon(f)).toBe('🧠')
    expect(messageTitle(f, '助手')).toBe('历史记忆')
  })

  it('默认收起（长摘要不得盖过对话本体）——连「user 一律展开」也要让它', () => {
    expect(messageDefaultOpen(facetsOf(memory))).toBe(false)
  })

  it('不是运行中动作、不挂「回复中…」小字（它是终态产物）', () => {
    const f = facetsOf(memory)
    expect(messageIsRunningAction(f)).toBe(false)
    expect(messageShowsLiveBadge(f)).toBe(false)
  })
})

