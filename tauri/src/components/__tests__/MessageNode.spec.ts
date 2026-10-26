// @vitest-environment happy-dom
/**
 * MessageNode 组件级测试 —— 渲染只按 wire `NodeKind` 五值分派（[D-070 §后果 2]）。
 *
 * 分派轴只有五格（body / notice / skeleton / interaction / typing）：每格只按**形状 +
 * 呈现数据**（title / icon / actions）渲染，不含任何业务分支。类名钩子来自 `NodeShell`
 * 的 `type-${facets.type}`，故 DOM 里只会出现五值（`type-skeleton` 等）。
 *
 * 「存量业务旧词（turn / tool_call / reasoning / compression / user_prompt）经
 * `nodeKindOf` 折到哪一格」这条读边界由 `messageTypes.spec` / `chat_node.spec` 专钉；
 * 本文件因此直接用**折后的五值**构造节点，逐格锁定可见渲染结果（含未登记类型兜底到 body）。
 */
import { describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import MessageNode from '../MessageNode.vue'
import { useRunningClock } from '@/composables/useRunningClock'
import {
  ALLOW_FOR_SESSION_KEY,
  ALLOW_FOREVER_KEY,
  RESUME_KEY,
  SKELETON_LAZY_KEY,
  type ResumePayload,
  type SkeletonLazySurface,
} from '@/composables/useChatConnection'
import type { ReleasedSummary } from '@/stores/skeletonLazy'
import type { ChatNode } from '@/schemas/chat_node'

/**
 * 构造合法 ChatNode（缺省：assistant / body / completed）。
 *
 * 分派轴即 `NodeKind` 五值，故 `type` 直接取折后形态；入参容忍 `string` 只为
 * 「未登记类型」那一条用例（`brand_new_type`）保留其兜底验证。
 */
const AS_PERSON = { kind: 'person', id: 'person/local' } as const
const AS_AGENT = { kind: 'agent', id: 'session/main' } as const
const AS_TOOL = { kind: 'tool', id: 'vdfs/write' } as const

function msg(p: Partial<Omit<ChatNode, 'type'>> & { id: string; type?: string }): ChatNode {
  return { principal: AS_AGENT, type: 'body', content: '', status: 'completed', ...p } as unknown as ChatNode
}

function mountNode(node: ChatNode, extraProps: Record<string, unknown> = {}) {
  return mount(MessageNode, {
    props: { node, ...extraProps },
    global: { provide: { resume: vi.fn() } },
  })
}

describe('MessageNode：根级骨架（skeleton）容器三形态', () => {
  it('形态①：无子节点且流式中 → 等待骨架 +「运行中」标签', () => {
    const w = mountNode(msg({ id: 't1', type: 'skeleton', status: 'streaming', children: [] }))
    expect(w.find('.skeleton-pending').exists()).toBe(true)
    expect(w.text()).toContain('正在思考')
    expect(w.find('.node-tag.run').text()).toContain('运行中')
  })

  it('形态②：子节点出现 → 等待骨架消失，折叠叶与正文作为子流直排', () => {
    const w = mountNode(
      msg({
        id: 't1',
        type: 'skeleton',
        status: 'streaming',
        children: [
          msg({ id: 'r1', type: 'notice', status: 'completed', content: '想了一下', parent_id: 't1' }),
          msg({ id: 'x1', type: 'body', status: 'streaming', content: '你好世界', parent_id: 't1' }),
        ],
      }),
    )
    expect(w.find('.skeleton-pending').exists()).toBe(false)
    // 折叠叶与正文各是一格（分派钩子即 NodeKind 五值）
    expect(w.findAll('.type-notice').length).toBe(1)
    expect(w.findAll('.type-body').length).toBe(1)
    expect(w.text()).toContain('你好世界')
  })

  it('形态③：失败 → 交代条 + 重试（发射骨架 id，供 retry_subtree）', async () => {
    const w = mountNode(
      msg({
        id: 't1',
        type: 'skeleton',
        status: 'failed',
        error: '请求失败',
        children: [msg({ id: 'x1', type: 'body', status: 'failed', content: '', error: '请求失败', parent_id: 't1' })],
      }),
    )
    // 骨架级错误条存在且携带错误文本；叶子级错误由父失败抑制（不重复报错）
    const boxes = w.findAll('.error-box')
    expect(boxes.length).toBe(1)
    expect(boxes[0].text()).toContain('请求失败')
    await boxes[0].find('button.retry').trigger('click')
    expect(w.emitted('retry')?.[0]).toEqual(['t1'])
  })

  it('形态③变体：失败且无子节点 → 交代条而非等待骨架（请求即败场景）', () => {
    const w = mountNode(msg({ id: 't1', type: 'skeleton', status: 'failed', error: 'bad', children: [] }))
    expect(w.find('.skeleton-pending').exists()).toBe(false)
    expect(w.find('.error-box').exists()).toBe(true)
  })

  it('中止的骨架同样给交代条 + 重试（中止不是故障）', async () => {
    const w = mountNode(
      msg({
        id: 't1',
        type: 'skeleton',
        status: 'aborted',
        children: [msg({ id: 'x1', type: 'body', content: '半截', parent_id: 't1' })],
      }),
    )
    const boxes = w.findAll('.error-box')
    expect(boxes.length, '中止必须有交代，否则这一轮看起来像正常结束').toBe(1)
    expect(boxes[0].text()).toContain('已中止')
    expect(boxes[0].text()).not.toContain('⚠')
    expect(boxes[0].classes(), '不渲染成故障红').toContain('aborted')
    await boxes[0].find('button.retry').trigger('click')
    expect(w.emitted('retry')?.[0]).toEqual(['t1'])
  })

  it('已完成 → 既无交代条也无重试（正常结束不该有多余角标）', () => {
    const w = mountNode(
      msg({
        id: 't1',
        type: 'skeleton',
        status: 'completed',
        children: [msg({ id: 'x1', type: 'body', content: '完整', parent_id: 't1' })],
      }),
    )
    expect(w.findAll('.error-box').length).toBe(0)
    expect(w.findAll('button.retry').length).toBe(0)
  })
})

describe('MessageNode：折叠叶（notice）始终单行', () => {
  it('流式中：行头呼吸动效 +「处理中…」，收起态不渲染正文', () => {
    const w = mountNode(msg({ id: 'r1', type: 'notice', status: 'streaming', content: '生成中的思考' }))
    const head = w.find('.node-head')
    expect(head.exists()).toBe(true)
    expect(head.classes()).toContain('thinking')
    expect(head.text()).toContain('处理中')
    // 折叠叶默认收起（`v-if`）：折叠内容不占 DOM
    expect(w.find('.node-body').exists()).toBe(false)
  })

  it('完成后：标题「提示」，仍保持折叠', () => {
    const w = mountNode(msg({ id: 'r1', type: 'notice', status: 'completed', content: '已完成的思考' }))
    const head = w.find('.node-head')
    expect(head.text()).toContain('提示')
    expect(head.classes()).not.toContain('thinking')
    expect(w.find('.node-body').exists()).toBe(false)
  })
})

describe('MessageNode：工具子骨架（skeleton 容器，参数作首条正文）', () => {
  it('运行中：单行「运行中」标签 + 动效点；有 started_at 才给时长', () => {
    const { nowMs } = useRunningClock()
    const w = mountNode(
      msg({
        id: 'tc1',
        type: 'skeleton',
        title: 'shell',
        status: 'streaming',
        meta: { started_at: nowMs.value - 47_000 },
        content: '{"cmd":"ls"}',
        children: [],
      }),
    )
    const tag = w.find('.node-tag')
    expect(tag.classes()).toContain('run')
    expect(tag.find('.tag-dots').exists()).toBe(true)
    expect(tag.text()).toContain('运行中')
    expect(w.find('.tag-elapsed').text()).toBe('47s')
    // 已有参数正文 → 不作空壳等待脉动：参数作首条正文（`pre.json.req`）直显，
    // 「等待骨架」只属于「什么都没到」的骨架（否则运行中参数被脉动盖掉，观感=跑完才显示）。
    expect(w.find('.skeleton-pending').exists()).toBe(false)
    expect(w.find('pre.json.req').text()).toContain('"cmd":"ls"')
  })

  it('无 started_at（旧数据）→ 不显示时长，也不编一个数', () => {
    const w = mountNode(
      msg({ id: 'tc1', type: 'skeleton', title: 'shell', status: 'streaming', content: '{}', children: [] }),
    )
    expect(w.find('.node-tag.run').exists()).toBe(true)
    expect(w.find('.tag-elapsed').exists()).toBe(false)
  })

  it('待用户确认 → 「待确认」标签（warn 变体）；终态不给标签', () => {
    const waiting = mountNode(
      msg({ id: 'tc1', type: 'skeleton', title: 'shell', status: 'waiting_user_action', content: '{}', children: [] }),
    )
    expect(waiting.find('.node-tag').text()).toContain('待确认')
    expect(waiting.find('.node-tag').classes()).toContain('warn')

    const done = mountNode(
      msg({ id: 'tc2', type: 'skeleton', title: 'shell', status: 'completed', content: '{}', children: [] }),
    )
    expect(done.find('.node-tag').exists()).toBe(false)
  })

  it('参数住在骨架自身 content → 收起首条正文渲染成 JSON 代码块（带请求强调边）', () => {
    const w = mountNode(
      msg({
        id: 'tc1',
        type: 'skeleton',
        title: 'shell',
        status: 'completed',
        content: '{"cmd":"ls"}',
        children: [],
      }),
    )
    const pre = w.find('pre.json')
    expect(pre.exists()).toBe(true)
    expect(pre.classes()).toContain('req')
    expect(pre.text()).toContain('"cmd":"ls"')
  })

  it('工具结果是 role=tool 的正文子节点，随子流递归渲染', () => {
    const w = mountNode(
      msg({
        id: 'tc1',
        type: 'skeleton',
        title: 'shell',
        status: 'completed',
        content: '{"cmd":"ls"}',
        children: [msg({ id: 'res1', type: 'body', principal: AS_TOOL, status: 'completed', content: '"ok"', parent_id: 'tc1' })],
      }),
    )
    expect(w.find('.skeleton-pending').exists()).toBe(false)
    // 结果子节点是工具一族的 body：分派钩子只会出现五值
    expect(w.findAll('.type-body').length).toBeGreaterThan(0)
    expect(w.text()).toContain('"ok"')
  })

  it('内含待审批提问子节点 → 提问格出现审批入口（批准执行）', () => {
    const w = mountNode(
      msg({
        id: 'tc1',
        type: 'skeleton',
        title: 'shell',
        status: 'streaming',
        content: '{}',
        children: [
          msg({
            id: 'up1',
            type: 'interaction',
            status: 'waiting_user_action',
            meta: { prompt: { kind: 'confirm', tool_name: 'shell', risk_level: 'high', description: '删除文件' } },
            parent_id: 'tc1',
          }),
        ],
      }),
    )
    expect(w.find('.type-interaction').exists()).toBe(true)
    expect(w.text()).toContain('批准执行')
  })

  it('可补充参数的可恢复失败（后端下发 supply）→ 出现补充参数入口', () => {
    const w = mountNode(
      msg({
        id: 'tc1',
        type: 'skeleton',
        title: 'shell',
        status: 'failed',
        error: '参数缺失',
        meta: { failure_kind: 'error', recoverable: true },
        actions: ['retry', 'supply'],
        content: '{}',
        children: [],
      }),
    )
    expect(w.find('.skeleton-supply').exists()).toBe(true)
  })

  it('可恢复失败（后端下发 retry）→ 交代条重试发射骨架 id', async () => {
    const w = mountNode(
      msg({
        id: 'tc1',
        type: 'skeleton',
        title: 'shell',
        status: 'failed',
        error: '工具执行失败',
        meta: { failure_kind: 'error', recoverable: true },
        actions: ['retry'],
        content: '{}',
        children: [],
      }),
    )
    const btn = w.find('.error-box button.retry')
    expect(btn.exists()).toBe(true)
    await btn.trigger('click')
    expect(w.emitted('retry')?.[0]).toEqual(['tc1'])
  })
})

/**
 * 分派器路由与内容形态 —— 分派器只做「按 NodeKind 路由」，渲染形态由各渲染器负责。
 *
 * 这一组同时是压缩形态的回归锚：压缩与思考同在 `notice` 一格，靠**是否携带读数行**
 * （`node.stats`，[D-111](../../../../crates/symbio-plugin-session/README.md#d-111-消息读数由交付出口装配压缩与记忆的事实行离开前端 "crates/symbio-plugin-session/src/README")）分道——有行走「压缩动作」样式，
 * 无行走 Markdown。夹具因此只给 `stats` 而不给 `meta` 数字：**本组件不认识那些字段名**，
 * 数字→文案的口径在后端（判据在 `plugins/session/stats_reading.test.rs`）。
 */
describe('MessageNode：折叠叶内的压缩动作（notice + 读数行）', () => {
  it('带读数行的压缩 → 「系统动作」样式（notice-note），不是普通 Markdown 正文', async () => {
    const w = mountNode(
      msg({
        id: 'cp1',
        type: 'notice',
        status: 'streaming',
        content: '正在压缩上下文…',
        stats: { rows: [{ label: '当前水位', value: '150k' }] },
      }),
    )
    // 折叠叶默认收起，展开后看正文分道
    await w.find('.node-head').trigger('click')
    expect(w.find('.notice-note').exists()).toBe(true)
    expect(w.find('.notice-note').text()).toContain('正在压缩上下文')
    expect(w.find('.markdown-body').exists()).toBe(false)
  })

  it('压缩失败 → 错误条 + 重试入口（发射压缩节点自己的 id，供 retry_compaction）', async () => {
    const w = mountNode(
      msg({
        id: 'cp1',
        type: 'notice',
        status: 'failed',
        actions: ['retry_compaction'],
        content: '压缩未完成（模型请求失败：429 Too Many Requests），已保留完整历史',
        stats: { rows: [{ label: '当前水位', value: '150k' }] },
      }),
    )
    await w.find('.node-head').trigger('click')
    expect(w.find('.error-box').exists()).toBe(true)
    expect(w.find('.error-box').text()).toContain('429 Too Many Requests')
    expect(w.find('.notice-note').exists()).toBe(false)
    const retry = w.find('.error-box button.retry')
    expect(retry.exists()).toBe(true)
    await retry.trigger('click')
    expect(w.emitted('retry')?.[0]).toEqual(['cp1'])
  })

  it('后端排好的行逐字显示（水位 / 上限 / 触发来源 / 丢了几条），前端不做数值格式化', async () => {
    const w = mountNode(
      msg({
        id: 'cp1',
        type: 'notice',
        status: 'completed',
        content: '已压缩上下文（12 → 3 条）',
        stats: {
          rows: [
            { label: '压缩后水位', value: '30k' },
            { label: '上限', value: '200k' },
            { label: '触发', value: '自动整理' },
            { label: '压掉', value: '9 条' },
          ],
        },
      }),
    )
    await w.find('.node-head').trigger('click')
    const facts = w.find('.notice-facts').text()
    expect(facts).toContain('压缩后水位 30k')
    expect(facts).toContain('上限 200k')
    expect(facts).toContain('触发 自动整理')
    expect(facts).toContain('压掉 9 条')
    expect(w.find('.notice-note').text()).toContain('12 → 3 条')
  })

  it('旧后端（无读数行）⇒ 不算压缩动作，落 Markdown；不编事实行', async () => {
    const w = mountNode(msg({ id: 'cp1', type: 'notice', status: 'completed', content: '压缩上下文' }))
    await w.find('.node-head').trigger('click')
    expect(w.find('.notice-note').exists()).toBe(false)
    expect(w.find('.notice-facts').exists()).toBe(false)
    expect(w.find('.markdown-body').exists()).toBe(true)
  })
})

describe('MessageNode：正文格（body）与兜底', () => {
  it('用户消息走右对齐气泡，头部是「你」', () => {
    const w = mountNode(msg({ id: 'u1', principal: AS_PERSON, type: 'body', content: '你好' }))
    expect(w.find('.user-bubble').exists()).toBe(true)
    expect(w.find('.node-head').text()).toContain('你')
    expect(w.find('.user-bubble').text()).toContain('你好')
  })

  it('未登记的 wire 类型折叠到 body：内容照常显示，会话流不空白', () => {
    const w = mountNode(
      msg({ id: 'new1', type: 'brand_new_type', content: '未来类型的内容' }),
    )
    expect(w.text()).toContain('未来类型的内容')
    expect(w.find('.node-head').exists()).toBe(true)
  })
})

/**
 * 压缩后的**历史记忆**（`meta.compacted`）：它挂在人一族的主体上（后端为让对话以
 * user 开头），但它是系统产出的记忆，不是用户说的话——分派器把它路由到折叠叶（notice），
 * 而非右对齐的用户气泡。这里钉住四个可见后果：标题 / 对齐 / 默认收起 / 不给悬停操作。
 */
describe('MessageNode：压缩后的历史记忆（折叠叶，不是「你说的那句话」）', () => {
  const memory = (extra: Partial<ChatNode> = {}) =>
    msg({
      id: 'mem1',
      principal: AS_PERSON,
      status: 'completed',
      content: '[CONTEXT SNAPSHOT — 压缩的历史记忆，基于它继续任务]\n要点一\n要点二',
      meta: { compacted: true },
      stats: {
        rows: [
          { label: '压缩后水位', value: '9k' },
          { label: '完整历史', value: '已转存' },
        ],
      },
      ...extra,
    })

  it('标题自述「历史记忆」，且不是用户消息的右对齐形态', () => {
    const w = mountNode(memory())
    expect(w.find('.node-title').text()).toBe('历史记忆')
    expect(w.find('.msg').classes()).not.toContain('user')
  })

  it('默认收起（记忆体很长，不得盖过对话本体）', () => {
    const w = mountNode(memory())
    expect(w.find('.node-head').attributes('aria-expanded')).toBe('false')
    expect(w.find('.notice-memory').exists()).toBe(false)
  })

  it('展开后：记忆原文原样可读 + 事实行（水位 / 已转存）', async () => {
    const w = mountNode(memory())
    await w.find('.node-head').trigger('click')
    expect(w.find('.notice-pre').text()).toContain('CONTEXT SNAPSHOT')
    const facts = w.find('.notice-facts').text()
    expect(facts).toContain('压缩后水位 9k')
    expect(facts).toContain('完整历史 已转存')
  })

  it('不给悬停操作（系统记忆的编辑 / 删除不走悬停）', () => {
    const w = mountNode(memory())
    expect(w.findAll('.node-act')).toHaveLength(0)
  })

  it('旧会话（后端没给读数行）⇒ 事实行不出现，不编数字', () => {
    const w = mountNode(memory({ stats: undefined }))
    expect(w.find('.notice-facts').exists()).toBe(false)
  })
})

/**
 * 节点头的键盘可达性。此前节点头是裸 `div` + `@click`：Tab 键永远走不到它。
 * 这里锁住三件事：可聚焦（tabindex）、开关语义（role / aria-expanded）、键盘能真的触发折叠。
 */
describe('MessageNode：节点头键盘可达性', () => {
  // 折叠叶（notice）默认收起，是验证折叠开关语义最干净的载体
  const foldMsg = () => msg({ id: 'r1', type: 'notice', status: 'completed', content: '一段思考' })

  it('节点头可聚焦且带开关语义（role / tabindex / aria-expanded）', () => {
    const w = mountNode(foldMsg())
    const head = w.find('.node-head')
    expect(head.attributes('role')).toBe('button')
    expect(head.attributes('tabindex')).toBe('0')
    expect(head.attributes('aria-expanded')).toBe('false')
    expect(head.attributes('aria-label')).toContain('展开')
  })

  it('Enter 与 Space 都能切换折叠，且 aria-expanded 随之更新', async () => {
    const w = mountNode(foldMsg())
    const head = w.find('.node-head')
    expect(w.find('.node-body').exists()).toBe(false)

    await head.trigger('keydown', { key: 'Enter' })
    expect(w.find('.node-body').exists()).toBe(true)
    expect(w.find('.node-head').attributes('aria-expanded')).toBe('true')

    await w.find('.node-head').trigger('keydown', { key: ' ' })
    expect(w.find('.node-body').exists()).toBe(false)
    expect(w.find('.node-head').attributes('aria-expanded')).toBe('false')
  })

  it('根级骨架的悬停操作按钮带可读名称（键盘与读屏都能识别）', () => {
    const w = mountNode(msg({ id: 't1', type: 'skeleton', status: 'completed', children: [] }))
    const acts = w.findAll('.node-act')
    expect(acts.length).toBeGreaterThan(0)
    for (const b of acts) {
      expect(b.attributes('aria-label'), '图标按钮必须有无障碍名称').toBeTruthy()
    }
  })

  it('操作按钮的点击不冒泡成折叠切换（否则点删除会顺手展开卡片）', async () => {
    const w = mountNode(msg({ id: 'm1', type: 'body', status: 'completed', content: 'hi' }))
    const before = w.find('.node-body').exists()
    await w.find('.node-act').trigger('click')
    expect(w.find('.node-body').exists()).toBe(before)
  })
})

/**
 * 已折叠释放的骨架（[] 惰性重取 · 判据 4）：后代被本地摘掉后，界面必须
 * 1. 给一行**摘要 chip**（多少条 / 什么类型 / 还在跑几个）——否则折叠态看起来
 *    像「这一轮空了」，用户无法判断里面有过什么、还能不能等；
 * 2. **抑制**「无响应兜底」——子节点是被本地释放的，不是「这次没跑」；
 * 3. 把**折叠态变化**经注入面转给 `useChatConnection`（释放 / 重取的实现在那边）。
 *
 * 夹具取一个「有请求、无响应」自述的骨架（`meta.failure_kind = not_executed`）：
 * 未释放时它按兜底渲染，释放后兜底必须让位给 chip——同一份节点数据，两种可见形态。
 */
describe('MessageNode：已折叠释放的骨架（[] 惰性重取 · 判据 4）', () => {
  /** 「有请求、无响应」自述 + 无子节点：未释放时兜底该出，释放后必须让位 */
  const noResultSkeleton = () =>
    msg({ id: 'skel', type: 'skeleton', status: 'completed', meta: { failure_kind: 'not_executed' }, children: [] })

  function lazyStub(over: { isReleased?: boolean; summary?: ReleasedSummary } = {}) {
    return {
      onCollapse: vi.fn(),
      isReleased: vi.fn(() => over.isReleased ?? false),
      summaryOf: vi.fn(() => over.summary),
    } satisfies SkeletonLazySurface
  }

  function mountLazy(node: ChatNode, lazy: ReturnType<typeof lazyStub>) {
    return mount(MessageNode, {
      props: { node },
      global: { provide: { resume: vi.fn(), [SKELETON_LAZY_KEY]: lazy } },
    })
  }

  it('已释放 → 折叠释放摘要 chip：多少条 / 什么类型 / 还在跑几个', () => {
    const w = mountLazy(
      noResultSkeleton(),
      lazyStub({ isReleased: true, summary: { count: 3, byType: { body: 2, skeleton: 1 }, running: 1 } }),
    )
    const chip = w.find('.skeleton-released')
    expect(chip.exists()).toBe(true)
    expect(chip.text()).toContain('已折叠释放 3 项')
    // 类型分布经 messageTypeLabel 翻成人话（body ⇒ 正文、skeleton ⇒ 组合），不摆线格式词
    expect(chip.text()).toContain('正文 2')
    expect(chip.text()).toContain('组合 1')
    expect(chip.text()).toContain('还在跑 1 个')
  })

  it('已释放 ⇒ 抑制「无响应兜底」（子节点是被本地摘掉的，不是「这次没跑」）', () => {
    const w = mountLazy(
      noResultSkeleton(),
      lazyStub({ isReleased: true, summary: { count: 1, byType: { body: 1 }, running: 0 } }),
    )
    expect(w.find('.skeleton-missing').exists()).toBe(false)
  })

  it('对照：未释放 ⇒ 兜底照出（「未执行」自述），且不出现释放 chip', () => {
    const w = mountLazy(noResultSkeleton(), lazyStub())
    expect(w.find('.skeleton-missing').exists()).toBe(true)
    expect(w.find('.skeleton-missing').text()).toContain('未执行')
    expect(w.find('.skeleton-released').exists()).toBe(false)
  })

  it('释放但无摘要（count=0 的空子树）⇒ 既不出 chip 也不编兜底', () => {
    const w = mountLazy(noResultSkeleton(), lazyStub({ isReleased: true, summary: undefined }))
    expect(w.find('.skeleton-released').exists()).toBe(false)
    expect(w.find('.skeleton-missing').exists()).toBe(false)
  })

  it('折叠态变化经注入面转发（收起 ⇒ onCollapse(mid, false)，供释放记账）', async () => {
    const lazy = lazyStub({ isReleased: true, summary: { count: 2, byType: { body: 2 }, running: 0 } })
    const w = mountLazy(noResultSkeleton(), lazy)
    // 骨架默认展开（整轮 = 对话主体）；点头收起 ⇒ 折叠态变化要送到惰性重取面
    await w.find('.node-head').trigger('click')
    expect(lazy.onCollapse).toHaveBeenCalledWith('skel', false)
  })

  it('缺注入面（单测直挂 / 旧面板）⇒ 界面照常：无 chip、无兜底抑制、折叠不炸', () => {
    const w = mountNode(noResultSkeleton())
    // 无 SKELETON_LAZY_KEY：isReleased 恒 false ⇒ 走未释放路径（兜底照出）
    expect(w.find('.skeleton-missing').exists()).toBe(true)
    expect(w.find('.skeleton-released').exists()).toBe(false)
  })
})

/**
 * 工具确认卡的四档 —— 批准本次 / 本会话放行 / 永久放行 / 拒绝。
 *
 * 三档「放行」写的是**三笔不同的事实**：批准只批这一次（`authz.granted`）；
 * 本会话放行改会话的执行阈值（同档及以下在本会话里从此不问）；永久放行改
 * 该工具的授权上限（全局配置 `approval_grants`，跨会话生效）。所以这里钉的不是
 * 「按钮在哪」，而是三件事：点错按钮不会顺手改另一件事实，点两档放行时两笔的
 * **顺序**也对（先改策略、再批准本次）。
 */
describe('MessageNode：工具确认的四档', () => {
  /** 一张带风险档的工具确认卡（父骨架 + 一个 waiting 的 interaction 子节点） */
  function confirmTree(risk?: string): ChatNode {
    return msg({
      id: 'tc1',
      type: 'skeleton',
      title: 'shell',
      status: 'waiting_user_action',
      content: '{}',
      children: [
        msg({
          id: 'up1',
          type: 'interaction',
          status: 'waiting_user_action',
          meta: {
            prompt: {
              kind: 'confirm',
              tool_name: 'shell',
              description: '删除文件',
              ...(risk ? { risk_level: risk } : {}),
            },
          },
          parent_id: 'tc1',
        }),
      ],
    })
  }

  /**
   * 挂载并接上三个注入口，返回「谁先被调到」的调用序列。
   *
   * 序列是判据的一部分：策略（阈值 / 工具上限）必须先落定，恢复载荷里的
   * `risk_level` / 授权清单才读得到新值——反过来的话本次恢复仍按旧策略判，
   * 用户会看到「刚点了放行又弹一次」。
   */
  function mountWithPorts(node: ChatNode) {
    const calls: string[] = []
    const resumed: ResumePayload[] = []
    const allowed: string[] = []
    const granted: string[] = []
    const w = mount(MessageNode, {
      props: { node },
      global: {
        provide: {
          [RESUME_KEY]: (p: ResumePayload) => {
            calls.push(`resume:${p.action}`)
            resumed.push(p)
          },
          [ALLOW_FOR_SESSION_KEY]: async (risk: string) => {
            // 真实实现是一次后端写（必然跨微任务）。这里显式让出一拍，
            // 否则「先抬阈值再批准」这条顺序在没有 await 时也会碰巧成立。
            await Promise.resolve()
            calls.push(`allow:${risk}`)
            allowed.push(risk)
          },
          [ALLOW_FOREVER_KEY]: async (tool: string, risk: string) => {
            await Promise.resolve()
            calls.push(`grant:${tool}:${risk}`)
            granted.push(`${tool}:${risk}`)
          },
        },
      },
    })
    return { w, calls, resumed, allowed, granted }
  }

  it('带风险档的确认卡给出两档放行；不带档位时两键都不出现', () => {
    const withRisk = mountWithPorts(confirmTree('high')).w
    expect(withRisk.find('button.up-allow-session').text()).toBe('本会话放行')
    expect(withRisk.find('button.up-allow-forever').text()).toBe('永久放行')

    const bare = mountWithPorts(confirmTree()).w
    expect(bare.find('button.up-allow-session').exists()).toBe(false)
    expect(bare.find('button.up-allow-forever').exists()).toBe(false)
    // 另外两键照旧
    expect(bare.find('button.up-approve').exists()).toBe(true)
    expect(bare.find('button.up-reject').exists()).toBe(true)
  })

  it('「本会话放行」= 先抬到这一档，再批准本次', async () => {
    const { w, calls, resumed, allowed } = mountWithPorts(confirmTree('high'))

    await w.find('button.up-allow-session').trigger('click')
    await flushPromises()

    expect(allowed).toEqual(['high'])
    expect(resumed.map((p) => p.action)).toEqual(['approve'])
    expect(calls).toEqual(['allow:high', 'resume:approve'])
    // 恢复锚点仍是父骨架（批准打在工具调用上，不是 interactive 节点自己）
    expect(resumed[0].targetId).toBe('tc1')
  })

  it('「永久放行」= 先把该工具的上限抬到这一档，再批准本次', async () => {
    const { w, calls, resumed, granted } = mountWithPorts(confirmTree('high'))

    await w.find('button.up-allow-forever').trigger('click')
    await flushPromises()

    expect(granted).toEqual(['shell:high'])
    expect(resumed.map((p) => p.action)).toEqual(['approve'])
    expect(calls).toEqual(['grant:shell:high', 'resume:approve'])
    expect(resumed[0].targetId).toBe('tc1')
  })

  it('「批准执行」不顺手抬阈值或写授权：三笔事实各走各的入口', async () => {
    const { w, calls, allowed, granted } = mountWithPorts(confirmTree('high'))

    await w.find('button.up-approve').trigger('click')
    await flushPromises()

    expect(allowed).toEqual([])
    expect(granted).toEqual([])
    expect(calls).toEqual(['resume:approve'])
  })

  it('抬阈值失败仍批准本次：一次写失败不该把用户挡在门外', async () => {
    const resumed: ResumePayload[] = []
    const w = mount(MessageNode, {
      props: { node: confirmTree('high') },
      global: {
        provide: {
          [RESUME_KEY]: (p: ResumePayload) => resumed.push(p),
          [ALLOW_FOR_SESSION_KEY]: async () => {
            throw new Error('boom')
          },
        },
      },
    })

    await w.find('button.up-allow-session').trigger('click')
    await flushPromises()

    expect(resumed.map((p) => p.action)).toEqual(['approve'])
  })

  it('写授权失败仍批准本次：同一纪律，永久放行不是批准的闸门', async () => {
    const resumed: ResumePayload[] = []
    const w = mount(MessageNode, {
      props: { node: confirmTree('high') },
      global: {
        provide: {
          [RESUME_KEY]: (p: ResumePayload) => resumed.push(p),
          [ALLOW_FOREVER_KEY]: async () => {
            throw new Error('boom')
          },
        },
      },
    })

    await w.find('button.up-allow-forever').trigger('click')
    await flushPromises()

    expect(resumed.map((p) => p.action)).toEqual(['approve'])
  })
})
