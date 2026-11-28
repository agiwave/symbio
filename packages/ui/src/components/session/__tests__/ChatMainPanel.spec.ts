// @vitest-environment happy-dom
/**
 * ChatMainPanel —— 「没有工作目录」这道门什么时候该拦
 *
 * 这道门是给**本空间清单里的会话**准备的：用户在会话里干活需要一个工作区，
 * 没有就先挑一个。但子会话不住在清单里（它住在父会话内部的 `subsession` 下，
 * 后端清单永远不会列它），它是一段**已经跑完的转写**——没有工作区可挑。
 * 拿这道门拦住它，用户点开子会话就只能看到「请先选工作目录」，转写明明读到了
 * 也不显示（真实回归）。
 */
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { flushPromises, shallowMount } from '@vue/test-utils'

const mocks = vi.hoisted(() => ({
  store: {
    activeId: 's2',
    activeAddr: '@vfs/session/s2',
    activeTitle: '子智能体 · 检索资料',
    activeWorkdir: undefined as string | undefined,
    activeSessionListed: false,
    isActiveWorking: false,
    loadMessages: vi.fn(async () => []),
    refreshInbox: vi.fn(async () => {}),
    rename: vi.fn(async () => {}),
  },
}))

vi.mock('@/stores/sessions', () => ({ useSessionsStore: () => mocks.store }))
vi.mock('@/services/sessionStats', () => ({ readSessionStats: vi.fn(async () => null) }))
vi.mock('@/services/sessionChanges', () => ({ readSessionChanges: vi.fn(async () => null) }))

import ChatMainPanel from '../ChatMainPanel.vue'

/** 以最小成本判断「哪一块面板被渲染了」：shallowMount 的桩带组件名 */
function mountPanel() {
  return shallowMount(ChatMainPanel, { props: {} })
}

describe('ChatMainPanel：没有工作目录时的分支', () => {
  beforeEach(() => {
    mocks.store.activeWorkdir = undefined
    mocks.store.activeSessionListed = false
    mocks.store.loadMessages.mockClear()
    mocks.store.loadMessages.mockImplementation(async () => [])
  })

  it('不在清单里的会话（子会话）：不给「请先选工作目录」，直接给转写面板', async () => {
    const w = mountPanel()
    await flushPromises()

    expect(w.html()).toContain('model-chat-panel-stub')
    expect(w.html(), '子会话没有工作区可挑，这道门不该拦它').not.toContain(
      'empty-workdir-state-stub',
    )
  })

  it('清单里的会话没有工作目录：仍然是「请先选工作目录」', async () => {
    mocks.store.activeSessionListed = true
    const w = mountPanel()
    await flushPromises()

    expect(w.html()).toContain('empty-workdir-state-stub')
  })

  it('有工作目录的会话：与「在不在清单里」无关，照常给转写面板', async () => {
    mocks.store.activeSessionListed = true
    mocks.store.activeWorkdir = 'D:/work/a'
    const w = mountPanel()
    await flushPromises()

    expect(w.html()).toContain('model-chat-panel-stub')
    expect(w.html()).not.toContain('empty-workdir-state-stub')
  })
})
