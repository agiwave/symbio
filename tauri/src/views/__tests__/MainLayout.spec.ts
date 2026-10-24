// @vitest-environment happy-dom
/**
 * MainLayout 的**全局键**单测（happy-dom）
 *
 * 外壳承担的是跨页面职责，本文件只钉属于命令面板那一条：`Cmd/Ctrl+K` 在**任何**
 * 页面都该按得出来，且不带 Alt/Shift 修饰（组合键各留各家，误开别人的面板比漏开更难查）。
 *
 * 其余启动接线（转写同步 / 会话挂载登记 / 工作区恢复）不在这里演——那些是被调用的
 * 模块自己的契约，各自有测试；把它们在这里逐个断言一遍，得到的只是「改了实现就要
 * 改测试」的镜像，而不是判据。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'

const hoisted = vi.hoisted(() => ({
  maybeOpen: vi.fn(),
  refreshList: vi.fn(),
  sessionSpace: { value: '' } as { value: string },
}))

vi.mock('vue-router', () => ({
  RouterView: { name: 'RouterView', render: () => null },
  useRouter: () => ({ push: vi.fn() }),
}))
vi.mock('@/stores/sessionTranscriptSync', () => ({
  startSessionTranscriptSync: vi.fn(),
  registerTranscriptMount: vi.fn(),
}))
vi.mock('@/stores/sessionNodeSync', () => ({
  startSessionNodeSync: vi.fn(),
  registerSessionMount: vi.fn(),
}))
vi.mock('@/stores/sessionInboxSync', () => ({
  startSessionInboxSync: vi.fn(),
  registerInboxMount: vi.fn(),
}))
vi.mock('@/services/completionChime', () => ({ setChimeSettingsSource: vi.fn() }))
// 面板打开会取一次清单；本文件断言的是**键**，不是取数（取数由面板自己的单测钉）。
// 不拦这一层，测试会去打真的 Tauri IPC 并喷一屏 `transformCallback` 堆栈。
vi.mock('@/services/vdfs', () => ({ treeVdfs: vi.fn(async () => ({ nodes: [], truncated: false })) }))
vi.mock('@/services/home', () => ({ getWorkspacePath: vi.fn(async () => ({ path: '/tmp/w' })) }))
vi.mock('@/stores/sessions', () => ({
  useSessionsStore: () => ({
    sessionSpace: hoisted.sessionSpace.value,
    refreshList: hoisted.refreshList,
    hasMessage: vi.fn(),
    applyTranscriptMessages: vi.fn(),
    loadMessages: vi.fn(),
    applyInboxItem: vi.fn(),
    removeInboxItem: vi.fn(),
    refreshInbox: vi.fn(),
    setSessionSpace: vi.fn(),
  }),
}))
vi.mock('@/stores/soundSettings', () => ({
  useSoundSettingsStore: () => ({ isKindEnabled: vi.fn(), volume: 0.5 }),
}))
vi.mock('@/stores/onboarding', () => ({
  useOnboardingStore: () => ({ open: false, maybeOpen: hoisted.maybeOpen }),
}))

import MainLayout from '../MainLayout.vue'
import CommandPalette from '@/components/common/CommandPalette.vue'

/** 按一次组合键，并等外壳把新的显隐值传进面板（props 是**渲染后**才更新的） */
async function press(w: ReturnType<typeof mount>, over: Partial<KeyboardEventInit> & { key: string }) {
  window.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...over }))
  await w.vm.$nextTick()
}

function visibleOf(w: ReturnType<typeof mount>) {
  return w.findComponent(CommandPalette).props('visible') as boolean
}

beforeEach(() => {
  hoisted.maybeOpen.mockClear()
})

describe('Cmd/Ctrl+K', () => {
  it('Ctrl+K 打开面板（Windows/Linux 口径）', async () => {
    const w = mount(MainLayout)
    await flushPromises()
    expect(visibleOf(w), '一挂载就该是关着的').toBe(false)

    await press(w, { key: 'k', ctrlKey: true })
    expect(visibleOf(w)).toBe(true)
  })

  it('⌘K 同样打开，且**再按一次关掉**（面板是开关，不是只进不出的门）', async () => {
    const w = mount(MainLayout)
    await flushPromises()

    await press(w, { key: 'K', metaKey: true })
    expect(visibleOf(w), '大写锁定下 `e.key` 是 "K"，判据要按字符不看大小写').toBe(true)

    await press(w, { key: 'k', metaKey: true })
    expect(visibleOf(w)).toBe(false)
  })

  it('裸 k（打字）与 Ctrl+Alt+K 都不开面板', async () => {
    const w = mount(MainLayout)
    await flushPromises()

    await press(w, { key: 'k' })
    await press(w, { key: 'k', ctrlKey: true, altKey: true })
    await press(w, { key: 'k', ctrlKey: true, shiftKey: true })
    expect(visibleOf(w)).toBe(false)
  })

  it('★ 拦下浏览器默认：不 preventDefault 的话，⌘/Ctrl+K 会同时把 webview 的地址栏搜索叫出来', async () => {
    const w = mount(MainLayout)
    await flushPromises()

    const evt = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, key: 'k', ctrlKey: true })
    window.dispatchEvent(evt)
    await w.vm.$nextTick()
    expect(evt.defaultPrevented).toBe(true)
    expect(visibleOf(w)).toBe(true)
  })
})
