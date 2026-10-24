// @vitest-environment happy-dom
/**
 * MainLayout 的**全局快捷键**单测（happy-dom）
 *
 * 外壳承担的是跨页面职责，本文件只钉属于「外壳这张表」的判据：
 *
 * - 键在任何页面都该按得出来，且只认恰好的那组修饰（`Ctrl+Shift+K` 不是 `Ctrl+K`）；
 * - **必须 `preventDefault`**——不拦的话 ⌘/Ctrl+K 会同时把 webview 的地址栏搜索叫出来；
 * - **模态让路**：面板开着时，全局绑定一律不参与（否则一条 `Cmd+/` 会同时
 *   「关掉确认框」和「打开速查表」，用户看到的是焦点乱跳）；
 * - **`when` 不成立 ⇒ 不吞键**：会话挂载点没认出来时 `Cmd+N` 必须原样还给浏览器，
 *   而不是变成一条什么都不做的死键；
 * - 新建会话的目标地址是**认出来的**（`ensureSessionMountDir`），不是写死的
 *   `/vdfs/session`——段名由后端 provider 决定，写死会在改名时静默跳空目录。
 *
 * ⚠️ 每个用例都必须 `unmount`：登记表是模块级单例，上一用例留下的绑定会
 * 在本用例里先命中（注册序即优先级），于是「第一条命中」永远属于上一个组件实例。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'

const hoisted = vi.hoisted(() => ({
  push: vi.fn(),
  maybeOpen: vi.fn(),
  ensureSessionMountDir: vi.fn(),
  sessionDir: '@vfs/session' as string | null,
}))

vi.mock('vue-router', () => ({
  RouterView: { name: 'RouterView', render: () => null },
  useRouter: () => ({ push: hoisted.push }),
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
vi.mock('@/services/vdfs', () => ({
  treeVdfs: vi.fn(async () => ({ nodes: [], truncated: false })),
  listVdfs: vi.fn(async () => ({ items: [] })),
}))
// 会话挂载点：Cmd+N 的前提。让它由用例决定认不认得出来。
vi.mock('@/services/vdfsScheme', () => ({
  ensureSessionMountDir: () =>
    hoisted.sessionDir === null
      ? Promise.reject(new Error('会话挂载点未找到'))
      : Promise.resolve(hoisted.sessionDir),
}))
vi.mock('@/services/home', () => ({ getWorkspacePath: vi.fn(async () => ({ path: '/tmp/w' })) }))
vi.mock('@/stores/sessions', () => ({
  useSessionsStore: () => ({
    sessionSpace: '',
    refreshList: vi.fn(),
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
import HotkeySheet from '@/components/common/HotkeySheet.vue'
import { setVdfsRoot } from '@/schemas/vdfsRoot'

setVdfsRoot('@vfs')

/** 按一次组合键，并等外壳把新的显隐值传进面板（props 是**渲染后**才更新的） */
async function press(
  w: VueWrapper,
  over: Partial<KeyboardEventInit> & { key: string },
  target?: Element
) {
  const evt = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...over })
  // 有目标 ⇒ 从那个元素冒泡（模态内/输入框内的让路判据靠的就是 target 是谁）；
  // 无目标 ⇒ 打在 window 上（真实页面里焦点在 `body` 时事件目标就是它）。
  ;(target ?? window).dispatchEvent(evt)
  await w.vm.$nextTick()
  return evt
}

const paletteVisible = (w: VueWrapper) => w.findComponent(CommandPalette).props('visible') as boolean
const sheetVisible = (w: VueWrapper) => w.findComponent(HotkeySheet).props('visible') as boolean

let mounted: VueWrapper | null = null

async function bench() {
  const w = mount(MainLayout)
  mounted = w
  await flushPromises()
  return w
}

beforeEach(() => {
  hoisted.push.mockClear()
  hoisted.maybeOpen.mockClear()
})

afterEach(() => {
  mounted?.unmount()
  mounted = null
})

describe('全局键（外壳这张表）', () => {
  it('Ctrl+K 打开命令面板（Windows/Linux 口径）', async () => {
    const w = await bench()
    expect(paletteVisible(w), '一挂载就该是关着的').toBe(false)

    await press(w, { key: 'k', ctrlKey: true })
    expect(paletteVisible(w)).toBe(true)
  })

  it('⌘K 同样打开，且大写锁定（`e.key` 是 "K"）不算另一个键', async () => {
    const w = await bench()

    await press(w, { key: 'K', metaKey: true })
    expect(paletteVisible(w)).toBe(true)
  })

  it('裸 k（打字）与多按一个修饰键（Ctrl+Shift+K / Ctrl+Alt+K）都不开面板', async () => {
    const w = await bench()

    await press(w, { key: 'k' })
    await press(w, { key: 'k', ctrlKey: true, shiftKey: true })
    await press(w, { key: 'k', ctrlKey: true, altKey: true })
    expect(paletteVisible(w)).toBe(false)
  })

  it('★ 拦下浏览器默认：不 preventDefault 的话，⌘/Ctrl+K 会同时把 webview 的地址栏搜索叫出来', async () => {
    const w = await bench()

    const evt = await press(w, { key: 'k', ctrlKey: true })
    expect(evt.defaultPrevented).toBe(true)
    expect(paletteVisible(w)).toBe(true)
  })

  it('★ 模态让路：面板开着时按 Cmd+/，速查表**不**在后头展开（焦点陷阱归 BaseModal）', async () => {
    const w = await bench()
    await press(w, { key: 'k', ctrlKey: true })
    expect(paletteVisible(w)).toBe(true)

    // 事件目标落在遮罩里（真实按键时焦点就在面板输入框上）
    const insideMask = w.find('.modal-panel').element
    await press(w, { key: '/', ctrlKey: true }, insideMask)

    expect(sheetVisible(w), '模态里的事件不该被全局绑定应答').toBe(false)
  })

  it('Cmd+/ 打开速查表（内容与真实绑定同源，见 useHotkeys.spec）', async () => {
    const w = await bench()

    await press(w, { key: '/', ctrlKey: true })
    expect(sheetVisible(w)).toBe(true)
  })

  it('★ Cmd+N 推**认出来的**会话目录；目的地就是那一类目录（工作台自动铺草稿）', async () => {
    const w = await bench()

    await press(w, { key: 'n', ctrlKey: true })
    expect(hoisted.push).toHaveBeenCalledWith('/vdfs/session')
  })

  it('★ 会话挂载点认不出 ⇒ Cmd+N 不吞键（那个组合还给浏览器，而不是变成死键）', async () => {
    hoisted.sessionDir = null
    const w = await bench()

    const evt = await press(w, { key: 'n', ctrlKey: true })
    expect(evt.defaultPrevented).toBe(false)
    expect(hoisted.push).not.toHaveBeenCalled()

    hoisted.sessionDir = '@vfs/session'
  })
})
