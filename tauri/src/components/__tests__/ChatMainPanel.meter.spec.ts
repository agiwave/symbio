// @vitest-environment happy-dom
/**
 * 会话头的用量读数（`session/stats.usage` → `ReadingMeter`）。
 *
 * 钉三件事，都不是「长得对不对」：
 *
 * 1. **读数从哪来**：头上那一格是后端在 `session/stats` 里排好的（与抽屉面板里的
 *    「用量」段同一次装配）。组件不解析、不换算、不认任何标签文本——它只把这一格
 *    交给方言的紧凑渲染器。
 * 2. **什么时候读**：切会话读一次、一轮跑完读一次。**不轮询**——按时间读会把
 *    一次投影的成本乘上秒数，而两次读之间的数本来就不该变。
 * 3. **缺数不画**：一次实测都没有时后端整个键不出现，头上就什么都不该有
 *    （不是画一条空条、也不是写「0 token」）。
 *
 * `ChatMainPanel` 的正文（转写加载 / ModelChatPanel）与本判据无关，两处子组件
 * 在这里都换成哑壳：本文件关心的是**头部**，正文的任何行为都会把噪音带进来。
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
}))

// 会话 store 只用到八个成员（见组件里的 `store.*`）。这里给一份**响应式**的假对象：
// `isActiveWorking` 要能被 watch 追到，否则「轮末重读」这条判据根本不会触发。
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
vi.mock('../ModelChatPanel.vue', () => ({
  default: { name: 'ModelChatPanel', template: '<div class="fake-chat" />' },
}))
vi.mock('@/components/vdfs/VdfsActions.vue', () => ({
  default: { name: 'VdfsActions', template: '<div class="fake-actions" />' },
}))

import { useSessionsStore } from '@/stores/sessions'
import ChatMainPanel from '../session/ChatMainPanel.vue'

/**
 * 假 store 的句柄。经**被测组件用的同一个入口**（`useSessionsStore`）取，
 * 而不是在 mock 工厂里另存一份到 `hoisted` 上——两处引用同一个对象要靠时序，
 * 而时序是这里最不该被依赖的东西（工厂是异步的）。
 */
const store = useSessionsStore() as unknown as FakeStore

/** 后端下发的一份「会话头那一格」（形状是 `ReadingSection`，内容由产者决定）。 */
function usageSection() {
  return {
    title: '用量',
    rows: [
      { label: '输入', value: '1.5k' },
      { label: '输出', value: '200' },
      { label: '合计', value: '1.7k' },
    ],
  }
}

/** 一份最小合法响应：本文件只消费 `usage`。 */
function statsWith(usage?: ReturnType<typeof usageSection>) {
  return {
    session_id: 's1',
    has_wal: true,
    panel: { title: '会话读数', sections: [] },
    ...(usage ? { usage } : {}),
  }
}

/**
 * 已挂载的面板。
 *
 * **必须逐个卸载**：假 store 是响应式的，上一个用例留下的组件会继续响应
 * `beforeEach` 里的 `activeId` 复位而不停重读——于是「读了几次」这条判据量到的
 * 是残留组件的次数，而它看起来像被测代码多读了一次。
 */
const mounted: VueWrapper[] = []

function mountPanel(): VueWrapper {
  const w = mount(ChatMainPanel)
  mounted.push(w)
  return w
}

beforeEach(() => {
  hoisted.readSessionStats.mockReset()
  hoisted.readSessionStats.mockResolvedValue(statsWith(usageSection()))
  store.activeId = 's1'
  store.isActiveWorking = false
  store.loadMessages.mockClear()
  store.refreshInbox.mockClear()
})

afterEach(() => {
  while (mounted.length) mounted.pop()!.unmount()
})

describe('ChatMainPanel：会话头的用量读数', () => {
  it('挂载即为当前会话读一次，并把那一格渲染在头上', async () => {
    const w = mountPanel()
    await flushPromises()

    expect(hoisted.readSessionStats).toHaveBeenCalledWith('s1')
    const meter = w.find('.chat-header .reading-meter')
    expect(meter.exists()).toBe(true)
    // 标签与值都来自后端那一格，组件一个都不认
    expect(meter.text()).toContain('1.7k')
    expect(meter.text()).toContain('合计')
  })

  it('缺那一格（一次实测都没有）⇒ 头上没有读数条，不是一条空条', async () => {
    hoisted.readSessionStats.mockResolvedValue(statsWith())
    const w = mountPanel()
    await flushPromises()

    expect(hoisted.readSessionStats).toHaveBeenCalledTimes(1)
    expect(w.find('.chat-header .reading-meter').exists()).toBe(false)
  })

  it('切会话 ⇒ 为新会话重读（旧会话的数不留在新会话头上）', async () => {
    const w = mountPanel()
    await flushPromises()
    expect(hoisted.readSessionStats).toHaveBeenCalledTimes(1)
    expect(w.find('.chat-header .reading-meter').text()).toContain('1.7k')

    // 新会话的用量与旧会话不同：留下来的话两边都会「看起来正常」
    hoisted.readSessionStats.mockResolvedValue({
      session_id: 's2',
      has_wal: true,
      panel: { title: '会话读数', sections: [] },
      usage: { title: '用量', rows: [{ label: '合计', value: '900' }] },
    })
    store.activeId = 's2'
    await flushPromises()

    expect(hoisted.readSessionStats).toHaveBeenLastCalledWith('s2')
    expect(hoisted.readSessionStats).toHaveBeenCalledTimes(2)
    const meter = w.find('.chat-header .reading-meter')
    expect(meter.text()).toContain('900')
    expect(meter.text()).not.toContain('1.7k')
  })

  it('一轮跑完（working → 不 working）重读一次；开始跑的时候不读', async () => {
    mountPanel()
    await flushPromises()
    expect(hoisted.readSessionStats).toHaveBeenCalledTimes(1)

    store.isActiveWorking = true
    await flushPromises()
    expect(hoisted.readSessionStats).toHaveBeenCalledTimes(1)

    store.isActiveWorking = false
    await flushPromises()
    expect(hoisted.readSessionStats).toHaveBeenCalledTimes(2)
  })

  it('读失败不画读数：头上留白而不是留上一次的数', async () => {
    hoisted.readSessionStats.mockRejectedValueOnce(new Error('boom'))
    const w = mountPanel()
    await flushPromises()

    expect(w.find('.chat-header .reading-meter').exists()).toBe(false)
  })
})
