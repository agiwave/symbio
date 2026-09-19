/**
 * HomedirEntry — 系统目录入口单测（happy-dom）
 *
 * 它是侧栏左下角那个按钮：显示当前连接目标（本地 / 远端）、点开切换对话框，
 * 并在切换成功后 `emit('reloaded')` 让宿主重载数据。系统目录切换是**破坏性操作**
 * （整个连接目标都变了），因此这里钉的是「状态显示正确」与「错误不让用户蒙在鼓里」：
 *
 * 1. 按钮标题必须说清**当前目标**（本地 / 远端 + 路径），不能只写「系统目录」。
 * 2. 远端不可达已回退本地时：**按钮带错误态 + 圆点 + 标题注明**，并且要**弹
 *    toast**——静默回退会让用户以为还在用远端。
 * 3. 本地模式下异步拉取真实 homedir 路径，**失败只告警不炸**（不阻塞首屏）。
 * 4. 对话框 `reloaded` → 先同步当前连接目标显示，**再** emit（顺序反了宿主会
 *    拿到旧显示）。
 *
 * 依赖（服务 / 切换对话框 / logger）全部用桩：本组件只做显示与转发的编排。
 */

// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'

const hoisted = vi.hoisted(() => ({
  getHomedirInfo: vi.fn(() => Promise.resolve({ homedir: '/home/x/.symbio' })),
  loadSystemLocation: vi.fn(() => ({ kind: 'local' as const, localPath: '/ persisted' })),
}))

vi.mock('@/services/home', () => ({ getHomedirInfo: hoisted.getHomedirInfo }))
// 只桩掉「从持久化读连接目标」（它会读本地存储），其余（currentLocation /
// locationError 这两个模块级 ref、formatLocation）用真的——本用例正是靠改这两个
// ref 来驱动显示。
vi.mock('@/services/systemLocation', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/services/systemLocation')>()
  return { ...actual, loadSystemLocation: hoisted.loadSystemLocation }
})
// 切换对话框是另一个 300 行的组件（另有其用例），这里只要能触发 reloaded
vi.mock('@/components/common/HomedirSwitcher.vue', () => ({
  default: {
    name: 'HomedirSwitcherStub',
    props: ['open'],
    emits: ['update:open', 'reloaded'],
    render: () => null,
  },
}))
vi.mock('@/utils/logger', () => ({
  logger: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() },
}))

import HomedirEntry from '../HomedirEntry.vue'
import { currentLocation, locationError } from '@/services/systemLocation'
import { useToast } from '@/composables/useToast'

function clearToasts() {
  const { toasts } = useToast()
  toasts.value.splice(0, toasts.value.length)
}

beforeEach(() => {
  hoisted.getHomedirInfo.mockClear()
  hoisted.getHomedirInfo.mockResolvedValue({ homedir: '/home/x/.symbio' })
  hoisted.loadSystemLocation.mockClear()
  currentLocation.value = { kind: 'local', localPath: '' }
  locationError.value = null
  clearToasts()
})

describe('HomedirEntry — 显示', () => {
  it('标题说清当前目标：本地 + 路径', async () => {
    currentLocation.value = { kind: 'local', localPath: '/home/x/.symbio' }
    const w = mount(HomedirEntry)
    await new Promise((r) => setTimeout(r, 0))
    expect(w.find('.nav-btn').attributes('title')).toContain('本地')
    expect(w.find('.nav-btn').attributes('title')).toContain('/home/x/.symbio')
  })

  it('远端时标注「远端」', async () => {
    currentLocation.value = { kind: 'remote', url: 'https://gw.example', key: 'k' } as never
    const w = mount(HomedirEntry)
    await new Promise((r) => setTimeout(r, 0))
    expect(w.find('.nav-btn').attributes('title')).toContain('远端')
  })

  it('aria-label 是稳定的「系统目录」（不随路径变）', () => {
    const w = mount(HomedirEntry)
    expect(w.find('.nav-btn').attributes('aria-label')).toBe('系统目录')
  })
})

describe('HomedirEntry — 错误可见', () => {
  it('连接异常：按钮带错误态 + 圆点，标题注明已回退本地', () => {
    locationError.value = '远端不可达'
    const w = mount(HomedirEntry)
    const btn = w.find('.nav-btn')
    expect(btn.classes()).toContain('nav-btn--error')
    expect(w.find('.nav-dot').exists()).toBe(true)
    expect(btn.attributes('title')).toContain('连接异常')
  })

  it('无异常时不带错误态、无圆点', () => {
    const w = mount(HomedirEntry)
    expect(w.find('.nav-btn').classes()).not.toContain('nav-btn--error')
    expect(w.find('.nav-dot').exists()).toBe(false)
  })

  it('启动期已有错误 → 弹 toast（不静默回退）', async () => {
    locationError.value = '远端不可达'
    mount(HomedirEntry)
    await new Promise((r) => setTimeout(r, 0))
    const { toasts } = useToast()
    expect(toasts.value.some((t) => t.type === 'error' && t.text.includes('远端不可达'))).toBe(true)
  })

  it('运行中错误出现 → 也弹 toast', async () => {
    const w = mount(HomedirEntry)
    await new Promise((r) => setTimeout(r, 0))
    locationError.value = '连接断开'
    await w.vm.$nextTick()
    const { toasts } = useToast()
    expect(toasts.value.some((t) => t.text.includes('连接断开'))).toBe(true)
  })
})

describe('HomedirEntry — 加载 homedir 显示', () => {
  it('本地模式：异步拉真实路径并写入显示', async () => {
    currentLocation.value = { kind: 'local', localPath: '' }
    const w = mount(HomedirEntry)
    await new Promise((r) => setTimeout(r, 0))
    expect(hoisted.getHomedirInfo).toHaveBeenCalled()
    expect(currentLocation.value.localPath).toBe('/home/x/.symbio')
    expect(w.find('.nav-btn').attributes('title')).toContain('/home/x/.symbio')
  })

  it('远端模式：不去拉本地 homedir', async () => {
    currentLocation.value = { kind: 'remote', url: 'https://gw', key: 'k' } as never
    mount(HomedirEntry)
    await new Promise((r) => setTimeout(r, 0))
    expect(hoisted.getHomedirInfo).not.toHaveBeenCalled()
  })

  it('拉取失败只告警、不炸（不阻塞首屏）', async () => {
    hoisted.getHomedirInfo.mockRejectedValueOnce(new Error('IPC 断了'))
    const w = mount(HomedirEntry)
    await expect(
      new Promise((r) => setTimeout(r, 0)),
    ).resolves.toBeUndefined()
    expect(w.find('.nav-btn').exists()).toBe(true)
  })
})

describe('HomedirEntry — 打开与切换完成', () => {
  it('点按钮 → 打开对话框', async () => {
    const w = mount(HomedirEntry)
    await w.find('.nav-btn').trigger('click')
    const stub = w.findComponent({ name: 'HomedirSwitcherStub' })
    expect(stub.props('open')).toBe(true)
  })

  it('切换完成 → 先同步显示，再 emit reloaded', async () => {
    const w = mount(HomedirEntry)
    await new Promise((r) => setTimeout(r, 0))
    w.findComponent({ name: 'HomedirSwitcherStub' }).vm.$emit('reloaded')
    await new Promise((r) => setTimeout(r, 0))
    expect(hoisted.loadSystemLocation).toHaveBeenCalled()
    expect(w.emitted('reloaded')).toHaveLength(1)
  })
})
