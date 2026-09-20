/**
 * 会话实时状态派生规则 —— 纯函数单测（node 环境）
 *
 * 「会话节点 / 清单快照 → 本地实时状态」这条映射最容易出错、又最难肉眼验证：
 * 判错的表现是侧栏计数归零、发送按钮闪回、状态灯不亮/不灭这类**静默退化**。
 * 抽成纯函数后逐条断言。
 */

import { describe, expect, it } from 'vitest'
import {
  listItemPatchOf,
  liveStatusPatchOf,
  mergeListWithLive,
  modeRiskBackfillOf,
  needsTypingRow,
  titleOf,
  workdirOf,
  workingUpgradesOf,
  type SessionLiveStatus,
} from '../sessionLive'
import type { SessionListItem } from '@/services/session'

function item(p: Partial<SessionListItem> & { id: string }): SessionListItem {
  return { message_count: 0, updated_at: 0, metadata: {}, ...p }
}

describe('listItemPatchOf：会话节点 → 清单条目就地补丁', () => {
  it('状态 / 时间 / 计数 / 标题都随节点收敛', () => {
    const out = listItemPatchOf(
      { title: '新标题', updated_at: 999, message_count: 12 },
      item({ id: 's1', message_count: 3, updated_at: 100, metadata: { workdir: '/w' } }),
      'working',
    )
    expect(out).toMatchObject({
      status: 'working',
      updated_at: 999,
      message_count: 12,
      metadata: { workdir: '/w', title: '新标题' },
    })
  })

  it('节点未带 message_count → 保留原值（不能归零）', () => {
    const out = listItemPatchOf({ title: '' }, item({ id: 's1', message_count: 7 }), 'active')
    expect(out.message_count).toBe(7)
  })

  it('节点未带 updated_at → 保留原值', () => {
    const out = listItemPatchOf({ title: '' }, item({ id: 's1', updated_at: 42 }), 'active')
    expect(out.updated_at).toBe(42)
  })

  it('节点未带非空 title → 保留原 metadata（不清空已有标题）', () => {
    const out = listItemPatchOf({ title: '' }, item({ id: 's1', metadata: { title: '旧' } }), 'active')
    expect(out.metadata).toEqual({ title: '旧' })
  })
})

describe('liveStatusPatchOf：运行态镜像', () => {
  it('状态与结局直通（无论是否发生迁移）', () => {
    expect(liveStatusPatchOf({ status: 'active', outcome: 'completed' }, false, false)).toEqual({
      status: 'active',
      outcome: 'completed',
    })
  })

  it('activity 只在迁移时改写（否则一次标题更新会顶掉「正在思考…」造成闪动）', () => {
    const noChange = liveStatusPatchOf({ status: 'active' }, false, false)
    expect('activity' in noChange).toBe(false)
  })

  it('进入运行中：活动文字 + 清掉上一轮的审批角标', () => {
    const patch = liveStatusPatchOf({ status: 'working' }, false, true)
    expect(patch.activity).toBe('处理中…')
    expect(patch.is_waiting_approval).toBe(false)
  })

  it('退出运行中：按结局选活动文字（中止 / 失败 / 正常各不相同）', () => {
    expect(liveStatusPatchOf({ status: 'active', outcome: 'aborted' }, true, false).activity).toBe(
      '已中止',
    )
    expect(liveStatusPatchOf({ status: 'failed', outcome: 'failed' }, true, false).activity).toBe(
      '错误',
    )
    expect(
      liveStatusPatchOf({ status: 'active', outcome: 'completed' }, true, false).activity,
    ).toBeUndefined()
  })
})

describe('modeRiskBackfillOf：只回填合法取值', () => {
  it('合法值回填；非法 / 缺失不写（不把脏数据搬进 UI 状态）', () => {
    const { modes, risks } = modeRiskBackfillOf([
      item({ id: 'a', metadata: { mode: 'auto', risk_level: 'high' } }),
      item({ id: 'b', metadata: { mode: 'interactive', risk_level: 'low' } }),
      item({ id: 'c', metadata: { mode: 'bogus', risk_level: 'extreme' } }),
      item({ id: 'd' }),
      item({ id: 'e', metadata: null as unknown as Record<string, unknown> }),
    ])
    expect(modes).toEqual({ a: 'auto', b: 'interactive' })
    expect(risks).toEqual({ a: 'high', b: 'low' })
  })
})

describe('mergeListWithLive：清单快照与本地镜像', () => {
  const live = (status?: string): SessionLiveStatus => ({ is_waiting_approval: false, last_event_at: 0, status })

  it('本地说「运行中」就保留（乐观置位不能被稍早的快照打回）', () => {
    const out = mergeListWithLive([item({ id: 'a', status: 'active' })], { a: live('working') })
    expect(out[0].status).toBe('working')
  })

  it('本地非运行中 → 以服务端 status 为准', () => {
    const out = mergeListWithLive([item({ id: 'a', status: 'active' })], { a: live('failed') })
    expect(out[0].status).toBe('active')
  })

  it('本地没有该会话 → 原样保留服务端条目', () => {
    const src = item({ id: 'a', status: 'active' })
    expect(mergeListWithLive([src], {})[0]).toEqual(src)
  })
})

describe('workingUpgradesOf：只做 false→true 的升级', () => {
  const live = (status?: string): SessionLiveStatus => ({ is_waiting_approval: false, last_event_at: 0, status })

  it('服务端说运行中、本地未知 → 需要升级（否则停止按钮失效）', () => {
    expect(workingUpgradesOf([item({ id: 'a', status: 'working' })], {})).toEqual(['a'])
  })

  it('本地已是运行中 → 不重复写；本地非运行中也不降级', () => {
    expect(workingUpgradesOf([item({ id: 'a', status: 'working' })], { a: live('working') })).toEqual([])
    expect(workingUpgradesOf([item({ id: 'a', status: 'working' })], { a: live('active') })).toEqual(['a'])
  })

  it('服务端非运行中 → 不升级（true→false 的收敛交给节点状态变更）', () => {
    expect(workingUpgradesOf([item({ id: 'a', status: 'active' })], {})).toEqual([])
  })
})

describe('titleOf / workdirOf：清单项取值', () => {
  it('标题：metadata.title 优先，回退节点名；都没有给空串（调用方据此跳过）', () => {
    expect(titleOf(item({ id: 'a', name: '节点名', metadata: { title: '元数据标题' } }))).toBe(
      '元数据标题',
    )
    expect(titleOf(item({ id: 'a', name: '节点名' }))).toBe('节点名')
    expect(titleOf(item({ id: 'a' }))).toBe('')
    expect(titleOf(item({ id: 'a', metadata: { title: '' } }))).toBe('')
  })

  it('工作目录：只认非空字符串', () => {
    expect(workdirOf(item({ id: 'a', metadata: { workdir: '/w' } }))).toBe('/w')
    expect(workdirOf(item({ id: 'a', metadata: { workdir: '' } }))).toBeUndefined()
    expect(workdirOf(item({ id: 'a', metadata: { workdir: 123 } }))).toBeUndefined()
    expect(workdirOf(item({ id: 'a' }))).toBeUndefined()
  })
})

/**
 * 流尾等待骨架的**兜底判据**。
 *
 * 它回答的是「会话在跑，但屏幕上什么都没有」——正是「点了发送却半天没反应」
 * 那类反馈缺失的判据。因此这里成组钉住两件事：
 *
 * 1. 「在跑」只看**会话节点**，与「Turn 节点到没到」无关（变更不重放，
 *    Turn 的 `created` 可能丢、也可能最后到）；
 * 2. 「有东西可看」必须按**渲染结果**算，而不是按 `status`：流式占位节点
 *    （`text` / `reasoning`，内容为空）也是 `streaming`，却什么都画不出来——
 *    把它当内容，就正好在需要骨架的那一刻判成「不用补」。
 */
describe('needsTypingRow：流尾等待骨架（Turn 节点丢失时的兜底来源）', () => {
  const msg = (p: Record<string, unknown>) => p as never

  it('会话没在跑 → 不补（哪怕流里一条消息都没有）', () => {
    expect(needsTypingRow(false, [])).toBe(false)
    expect(needsTypingRow(false, [msg({ status: 'streaming', type: 'text', content: '半截' })])).toBe(
      false,
    )
  })

  it('会话在跑、流里什么都没有 → 补（Turn 的 created 还没到 / 丢了一次）', () => {
    expect(needsTypingRow(true, [])).toBe(true)
    expect(needsTypingRow(true, [msg({ status: 'completed', type: 'text', content: '上一轮' })])).toBe(
      true,
    )
  })

  it('会话在跑、有在途正文 → 不补（正文本身在说话，Turn 骨架也已在位）', () => {
    expect(needsTypingRow(true, [msg({ status: 'streaming', type: 'text', content: '正在写' })])).toBe(
      false,
    )
    expect(
      needsTypingRow(true, [msg({ status: 'waiting_user_action', type: 'text', content: '选一个' })]),
    ).toBe(false)
  })

  it('会话在跑、只有**空壳**流式节点 → 补（空壳渲染不出任何东西）', () => {
    expect(needsTypingRow(true, [msg({ status: 'streaming', type: 'reasoning', content: '' })])).toBe(
      true,
    )
    expect(needsTypingRow(true, [msg({ status: 'streaming', type: 'text', content: '\n\n' })])).toBe(
      true,
    )
    // 多模态形状的空壳同样算空壳（取值走契约层的 messageTextOf）
    expect(
      needsTypingRow(true, [
        msg({ status: 'streaming', type: 'text', content: [{ type: 'text', text: '  ' }] }),
      ]),
    ).toBe(true)
  })

  it('空壳判据只对文字类节点生效：streaming 的工具调用即便无参数也占一张卡片', () => {
    expect(
      needsTypingRow(true, [msg({ status: 'streaming', type: 'tool_call', content: '' })]),
    ).toBe(false)
  })

  it('空壳 + 真内容并存 → 不补（真内容已经把这一轮的存在说清楚了）', () => {
    expect(
      needsTypingRow(true, [
        msg({ status: 'streaming', type: 'reasoning', content: '' }),
        msg({ status: 'streaming', type: 'text', content: '答' }),
      ]),
    ).toBe(false)
  })
})
