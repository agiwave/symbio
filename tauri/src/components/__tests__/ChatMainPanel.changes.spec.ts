// @vitest-environment happy-dom
/**
 * 会话头的「变更」抽屉（`session/changes` → `SessionChangesList`）。
 *
 * 钉的是**装配**，不是排版（排版由 `SessionChangesList.spec.ts` 单独钉）：
 *
 * 1. **读的时刻**：面板关着不读；打开读一次；切会话作废重读；一轮跑完重读。
 *    变更清单是一次投影（要扫 `artifact.added`），按时间轮询等于把成本乘上秒数。
 * 2. **点行不导航**：渲染器把「相对地址」拼成完整地址后上抛 `open-node`，怎么打开
 *    由页面层决定（三栏父目录 + 选中项）。渲染器里出现 router 就是把导航权搬进了
 *    详情层，正是本判据要防的事。
 * 3. **两个抽屉同一个位置**：读数与变更不能同时滑出（会互相盖住，而界面看起来
 *    仍然正常）。
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'

interface FakeStore {
  activeId: string
  activeAddr: string
  activeTitle: string
  activeWorkdir: string
  isActiveWorking: boolean
  loadMessages: ReturnType<typeof vi.fn>
  refreshInbox: ReturnType<typeof vi.fn>
  rename: ReturnType<typeof vi.fn>
}

const hoisted = vi.hoisted(() => ({
  readSessionStats: vi.fn(),
  readSessionChanges: vi.fn(),
}))

// 假 store 必须**响应式**：`isActiveWorking` 的轮末重读靠 watch 追。
vi.mock('@/stores/sessions', async () => {
  const { reactive } = await import('vue')
  const store = reactive({
    activeId: 's1',
    activeAddr: '@vfs/session/s1',
    activeTitle: '一个会话',
    activeWorkdir: 'D:/work',
    isActiveWorking: false,
    loadMessages: vi.fn(async () => {}),
    refreshInbox: vi.fn(async () => {}),
    rename: vi.fn(async () => {}),
  })
  return { useSessionsStore: () => store }
})

vi.mock('@/services/sessionStats', () => ({ readSessionStats: hoisted.readSessionStats }))
vi.mock('@/services/sessionChanges', () => ({ readSessionChanges: hoisted.readSessionChanges }))
vi.mock('../ModelChatPanel.vue', () => ({
  default: { name: 'ModelChatPanel', template: '<div class="fake-chat" />' },
}))
vi.mock('@/components/vdfs/VdfsActions.vue', () => ({
  default: { name: 'VdfsActions', template: '<div class="fake-actions" />' },
}))

import { useSessionsStore } from '@/stores/sessions'
import ChatMainPanel from '../session/ChatMainPanel.vue'
import type { ChangeRow } from '@/services/sessionChanges'

const store = useSessionsStore() as unknown as FakeStore

function row(over: Partial<ChangeRow> = {}): ChangeRow {
  return {
    path: 'docs/a.md',
    address: 'workdir/docs/a.md',
    op: 'write',
    label: '写入',
    turn: 3,
    changes: 1,
    ...over,
  }
}

function ledger(rows: ChangeRow[] = [row()], sessionId = 's1') {
  return { session_id: sessionId, has_wal: true, readable: true, rows }
}

function stats() {
  return {
    session_id: 's1',
    has_wal: true,
    panel: { title: '会话读数', sections: [] },
  }
}

const mounted: VueWrapper[] = []

function mountPanel(): VueWrapper {
  const w = mount(ChatMainPanel)
  mounted.push(w)
  return w
}

/** 打开变更抽屉（面板挂载 + 首次读数也要跑完，否则读到的是残留状态） */
async function openChanges(w: VueWrapper) {
  await w.find('button[title="变更"]').trigger('click')
  await flushPromises()
}

beforeEach(() => {
  hoisted.readSessionStats.mockReset()
  hoisted.readSessionStats.mockResolvedValue(stats())
  hoisted.readSessionChanges.mockReset()
  hoisted.readSessionChanges.mockResolvedValue(ledger())
  store.activeId = 's1'
  store.activeAddr = '@vfs/session/s1'
  store.isActiveWorking = false
  store.loadMessages.mockClear()
  store.refreshInbox.mockClear()
})

afterEach(() => {
  while (mounted.length) mounted.pop()!.unmount()
})

describe('ChatMainPanel：变更抽屉', () => {
  it('关着不读（没有消费方）；打开读一次，行由后端字段直接排版', async () => {
    const w = mountPanel()
    await flushPromises()
    expect(hoisted.readSessionChanges, '面板关着就不该花一次投影').not.toHaveBeenCalled()

    await openChanges(w)

    expect(hoisted.readSessionChanges).toHaveBeenCalledWith('s1')
    expect(w.find('.reading-drawer .drawer-title').text()).toBe('变更')
    expect(w.find('.change-row').text()).toContain('docs/a.md')
    expect(w.find('.change-row').text()).toContain('写入')
  })

  it('★ 点行只上抛 `open-node`：会话地址 + 后端给的相对地址，渲染器不导航', async () => {
    const w = mountPanel()
    await flushPromises()
    await openChanges(w)

    await w.find('.change-row').trigger('click')

    expect(w.emitted('open-node')?.[0]).toEqual(['@vfs/session/s1/workdir/docs/a.md'])
  })

  it('切会话且抽屉开着 ⇒ 旧行作废，为新会话重读', async () => {
    const w = mountPanel()
    await flushPromises()
    await openChanges(w)
    expect(w.find('.change-row').text()).toContain('docs/a.md')

    hoisted.readSessionChanges.mockResolvedValue(ledger([row({ path: 'src/b.rs', address: 'workdir/src/b.rs' })], 's2'))
    store.activeId = 's2'
    await flushPromises()

    expect(hoisted.readSessionChanges).toHaveBeenLastCalledWith('s2')
    const text = w.find('.changes-list').text()
    expect(text).toContain('src/b.rs')
    expect(text, '旧会话的行不能留在新会话的抽屉里').not.toContain('docs/a.md')
  })

  it('两个抽屉同一个位置：打开读数 ⇒ 变更收起（不会两层叠着）', async () => {
    const w = mountPanel()
    await flushPromises()
    await openChanges(w)
    expect(w.find('.drawer-title').text()).toBe('变更')

    await w.find('button[title="读数"]').trigger('click')
    await flushPromises()

    const drawers = w.findAll('.reading-drawer')
    expect(drawers).toHaveLength(1)
    expect(drawers[0].find('.drawer-title').text()).toBe('读数')
  })

  it('一轮跑完（working → 不 working）重读；开始跑的时候不读', async () => {
    const w = mountPanel()
    await flushPromises()
    await openChanges(w)
    expect(hoisted.readSessionChanges).toHaveBeenCalledTimes(1)

    store.isActiveWorking = true
    await flushPromises()
    expect(hoisted.readSessionChanges).toHaveBeenCalledTimes(1)

    store.isActiveWorking = false
    await flushPromises()
    expect(hoisted.readSessionChanges).toHaveBeenCalledTimes(2)
  })
})
