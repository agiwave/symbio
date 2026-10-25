// @vitest-environment happy-dom
/**
 * ModelChatPanel 的 **渲染窗口与阅读位置**单测（happy-dom）
 *
 * P1-3a 落到面板上的判据只有四条，它们各自错了都不报错：
 *
 * 1. **模板渲染的是窗口，不是整棵树**——`messageTree` 有 100 条根节点时，DOM 里
 *    只该有首屏那几十张卡。这条一旦写错（漏了 `renderedRoots` 而仍 `v-for="node in messageTree"`），
 *    窗口逻辑全部存在但**一帧都没生效**，长会话照旧整铺。
 * 2. **「显示更早」是显式入口**：按钮上的数字就是还没铺出来的条数，铺到顶即消失。
 *    自动续拉（滚到顶就加载）不是这里的形态——那会把「往上翻」变成一场无限加载。
 * 3. **窗口条数与阅读位置是一对**：条数按会话记进 `sessionUiState`，挂载时读回来；
 *    位置走**离底距离**而不是 `scrollTop`（换算与算术由 `useChatScroll` 自己的测试钉，
 *    本文件把「面板调的是哪一个」钉住——写错成 `scrollTop` 时窗口一起就错位了）。
 * 4. **展开时补的是「新插入的高度」这一笔账**：补少了视线被推走，补两倍（浏览器的
 *    滚动锚定没关时就是这样）往下窜一大截。happy-dom 没有布局，这里按已铺条数造高度。
 *
 * 环境说明：`useChatConnection` 与 `useChatScroll` 都整体替换为可控桩（前者经
 * `services/vdfs` / Tauri 通道出站，后者要真实布局才有意义）；`MessageNode` 换成
 * 只报 id 的替身——本文件判的是**铺了多少条、铺哪几条**，不是节点内容。
 */
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'
import { mount, type DOMWrapper, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { defineComponent, h } from 'vue'
import type { ChatNode } from '@/schemas/chat_node'

interface ChatStub {
  isLoading: { value: boolean }
  isWaitingApproval: { value: boolean }
  isConnected: { value: boolean }
  messageTree: { value: unknown[] }
  send: Mock
  abort: Mock
  removeMessage: Mock
  resume: Mock
  expandSkeleton: Mock
  /** 骨架惰性重取面：面板把它 provide 给下游（本文件不判它，给个不炸的桩） */
  skeletonLazy: { onCollapse: Mock; isReleased: Mock; summaryOf: Mock }
}

interface ScrollStub {
  isUserAtBottom: { value: boolean }
  scrollToBottom: Mock
  scrollToGap: Mock
  gapFromBottom: Mock
  handleScroll: Mock
  smartScroll: Mock
}

const hoisted = vi.hoisted(() => ({
  chat: null as ChatStub | null,
  scroll: null as ScrollStub | null,
}))

vi.mock('@/composables/useChatConnection', async () => {
  // `ref` 只能在工厂里动态取：`vi.hoisted` 跑在所有 import 之前，
  // 在那里用顶层 import 的 `ref` 会炸成「before initialization」。
  const { ref, shallowRef } = await import('vue')
  hoisted.chat = {
    isLoading: ref(false),
    isWaitingApproval: ref(false),
    isConnected: ref(true),
    messageTree: shallowRef([]),
    send: vi.fn(),
    abort: vi.fn(),
    removeMessage: vi.fn(),
    resume: vi.fn(),
    expandSkeleton: vi.fn(),
    skeletonLazy: { onCollapse: vi.fn(), isReleased: vi.fn(() => false), summaryOf: vi.fn(() => undefined) },
  }
  // 三个注入键都要回：面板 provide 它们，缺一个就是「Invalid vnode type」级的挂载失败
  return {
    RESUME_KEY: Symbol('resume'),
    SKELETON_LAZY_KEY: Symbol('chat-skeleton-lazy'),
    ALLOW_FOR_SESSION_KEY: Symbol('chat-allow-for-session'),
    useChatConnection: () => hoisted.chat,
  }
})

vi.mock('@/composables/useChatScroll', async () => {
  const { ref } = await import('vue')
  hoisted.scroll = {
    isUserAtBottom: ref(true),
    scrollToBottom: vi.fn(),
    scrollToGap: vi.fn(),
    // 离底 321px：面板不该知道像素怎么来的，只该把它原样存进按会话的状态
    gapFromBottom: vi.fn(() => 321),
    handleScroll: vi.fn(),
    smartScroll: vi.fn(),
  }
  return { useChatScroll: () => hoisted.scroll }
})

import ModelChatPanel from '../ModelChatPanel.vue'
import { useSessionUiStateStore } from '@/stores/sessionUiState'
import { TRANSCRIPT_WINDOW_INITIAL, TRANSCRIPT_WINDOW_STEP } from '@/composables/useTranscriptWindow'

const SID = 's1'

/** 只报 id 的替身：本文件数的是「铺了哪几条」，不是卡片内容 */
const FakeNode = defineComponent({
  name: 'MessageNode',
  props: { node: { type: Object, required: true }, depth: { type: Number, default: 0 } },
  setup: (props) => () => h('div', { class: 'fake-node', 'data-id': (props.node as ChatNode).id }),
})

const roots = (n: number): ChatNode[] =>
  Array.from({ length: n }, (_, i) => ({ id: `r${i}`, seq: i }) as ChatNode)

let w: VueWrapper | null = null

function panel() {
  w = mount(ModelChatPanel, {
    props: { sessionId: SID, sessionAddr: `@vfs/session/${SID}` },
    global: { stubs: { MessageNode: FakeNode } },
    attachTo: document.body,
  })
  return w
}

/** DOM 里已铺出来的根节点 id（顺序即渲染顺序） */
function renderedIds(): string[] {
  return w!.findAll('.fake-node').map((d) => (d.element as HTMLElement).dataset.id ?? '')
}

function earlierButton(): DOMWrapper<Element> | null {
  const found = w!.find('.earlier-btn')
  return found.exists() ? found : null
}

function chatStub(): ChatStub {
  if (!hoisted.chat) throw new Error('useChatConnection 桩未装配（面板没被导入？）')
  return hoisted.chat
}

function scrollStub(): ScrollStub {
  if (!hoisted.scroll) throw new Error('useChatScroll 桩未装配（面板没被导入？）')
  return hoisted.scroll
}

beforeEach(() => {
  setActivePinia(createPinia())
  chatStub().messageTree.value = []
  for (const fn of [
    scrollStub().scrollToBottom,
    scrollStub().scrollToGap,
    scrollStub().gapFromBottom,
    scrollStub().handleScroll,
  ]) {
    fn.mockClear()
  }
})

afterEach(() => {
  w?.unmount()
  w = null
  document.body.replaceChildren()
})

describe('ModelChatPanel 渲染窗口', () => {
  it('★ 100 条根节点 ⇒ DOM 里只有首屏那几十条（漏了窗口时这里全绿不了，却处处静默）', async () => {
    chatStub().messageTree.value = roots(100)
    panel()
    await w!.vm.$nextTick()

    const ids = renderedIds()
    expect(ids).toHaveLength(TRANSCRIPT_WINDOW_INITIAL)
    expect(ids[0]).toBe(`r${100 - TRANSCRIPT_WINDOW_INITIAL}`)
    expect(ids[ids.length - 1]).toBe('r99')
    expect(earlierButton(), '还有更早的 ⇒ 必须给显式入口').not.toBeNull()
  })

  it('不足首屏条数 ⇒ 全铺且不给入口（空会话与短会话不该看到一个死按钮）', async () => {
    chatStub().messageTree.value = roots(5)
    panel()
    await w!.vm.$nextTick()

    expect(renderedIds()).toEqual(['r0', 'r1', 'r2', 'r3', 'r4'])
    expect(earlierButton()).toBeNull()
  })

  it('★ 点一次「显示更早」多铺一批，并**把条数记进按会话的状态**（它与离底距离是一对）', async () => {
    chatStub().messageTree.value = roots(100)
    panel()
    await w!.vm.$nextTick()
    const ui = useSessionUiStateStore()

    await earlierButton()!.trigger('click')
    await w!.vm.$nextTick()

    expect(renderedIds()).toHaveLength(TRANSCRIPT_WINDOW_INITIAL + TRANSCRIPT_WINDOW_STEP)
    expect(earlierButton()!.text()).toContain(String(100 - TRANSCRIPT_WINDOW_INITIAL - TRANSCRIPT_WINDOW_STEP))
    expect(ui.windowSizeOf(SID)).toBe(TRANSCRIPT_WINDOW_INITIAL + TRANSCRIPT_WINDOW_STEP)
  })

  it('铺到顶 ⇒ 入口消失（不把「没有了」演成「加载失败」）', async () => {
    chatStub().messageTree.value = roots(100)
    panel()
    await w!.vm.$nextTick()

    await earlierButton()!.trigger('click')
    await w!.vm.$nextTick()
    await earlierButton()!.trigger('click')
    await w!.vm.$nextTick()

    expect(renderedIds()).toHaveLength(100)
    expect(earlierButton()).toBeNull()
  })

  it('★ 挂载时读回上次铺出来的条数：切回来还是那一屏，而不是被窗口砍到顶部', async () => {
    useSessionUiStateStore().setWindowSize(SID, 120)
    chatStub().messageTree.value = roots(300)
    panel()
    await w!.vm.$nextTick()

    expect(renderedIds()).toHaveLength(120)
    expect(renderedIds()[0]).toBe('r180')
  })

  /**
   * 这条是窗口**不滑动**的判据（`useTranscriptWindow` 里写得很清楚，但那里测的是
   * 数组切片；「已经铺出来的不掉出去」只有在面板这一层才成立——它是唯一知道
   * 「新帧到了」的地方）。
   */
  it('流式新增一条根节点 ⇒ 已铺的最旧那条仍在 DOM 里（挤掉它就是把用户上方抽走）', async () => {
    chatStub().messageTree.value = roots(60)
    panel()
    await w!.vm.$nextTick()
    const firstBefore = renderedIds()[0]

    chatStub().messageTree.value = roots(61)
    await w!.vm.$nextTick()

    expect(renderedIds()[0]).toBe(firstBefore)
    expect(renderedIds()[renderedIds().length - 1]).toBe('r60')
  })
})

/**
 * happy-dom 没有布局引擎，`scrollHeight` 恒为 0——这里按「已铺出来的条数」造一个
 * 假高度（每条 50px + 底子 1000px），让高度差可断言。测的是**面板补的那一下**：
 * 展开时往上插内容后仍按原 `scrollTop` 显示等于把视线推走，补偿量必须正好是
 * 新插入的高度。浏览器自己的滚动锚定已被 `overflow-anchor: none` 关掉（见
 * `ModelChatPanel.vue` 的 `.column-body`），否则这里会是加**两**倍。
 */
function fakeLayout(el: HTMLElement, perItem = 50, base = 1000): void {
  Object.defineProperty(el, 'scrollHeight', {
    configurable: true,
    get: () => base + renderedIds().length * perItem,
  })
}

describe('ModelChatPanel 展开更早时的视线补偿', () => {
  it('★ 往上铺一批 ⇒ scrollTop 加的正好是新插入的高度（不补=视线被推走，补两倍=往下窜）', async () => {
    chatStub().messageTree.value = roots(100)
    panel()
    await w!.vm.$nextTick()
    const el = w!.find('.column-body').element as HTMLElement
    fakeLayout(el)
    el.scrollTop = el.scrollHeight // 贴着底

    await earlierButton()!.trigger('click')
    await w!.vm.$nextTick()

    expect(el.scrollTop).toBe(1000 + (TRANSCRIPT_WINDOW_INITIAL + TRANSCRIPT_WINDOW_STEP) * 50)
  })

  it('铺到顶 ⇒ 补齐全部条数后入口消失（不给一个点了没反应的死按钮）', async () => {
    chatStub().messageTree.value = roots(100)
    panel()
    await w!.vm.$nextTick()
    const el = w!.find('.column-body').element as HTMLElement
    fakeLayout(el)

    await earlierButton()!.trigger('click')
    await w!.vm.$nextTick()
    await earlierButton()!.trigger('click') // 这批不足额，铺到顶
    await w!.vm.$nextTick()

    expect(renderedIds()).toHaveLength(100)
    expect(earlierButton()).toBeNull()
    // 补偿按**累计新增**高度走：起点 0 + 后来铺出来的那 60 条，与分了几批无关
    expect(el.scrollTop).toBe((100 - TRANSCRIPT_WINDOW_INITIAL) * 50)
  })
})

describe('ModelChatPanel 阅读位置的还原与记录', () => {
  it('无记录 ⇒ 贴底（新会话不该落在上一条会话的位置上）', async () => {
    chatStub().messageTree.value = roots(3)
    panel()
    await w!.vm.$nextTick()

    expect(scrollStub().scrollToBottom).toHaveBeenCalledTimes(1)
    expect(scrollStub().scrollToGap).not.toHaveBeenCalled()
  })

  it('★ 有记录 ⇒ 回到**离底同样远**处，并重算「是否贴底」（否则下一次流式追加把用户拽回底部）', async () => {
    useSessionUiStateStore().setScrollGap(SID, 800)
    chatStub().messageTree.value = roots(3)
    panel()
    await w!.vm.$nextTick()

    expect(scrollStub().scrollToGap).toHaveBeenCalledWith(800)
    expect(scrollStub().scrollToBottom).not.toHaveBeenCalled()
    expect(scrollStub().handleScroll).toHaveBeenCalledTimes(1)
  })

  it('滚动时把**离底距离**写进按会话的状态（不是 scrollTop——窗口起点会变，从顶部量的数下次对不上）', async () => {
    chatStub().messageTree.value = roots(3)
    panel()
    await w!.find('.column-body').trigger('scroll')

    expect(useSessionUiStateStore().scrollGapOf(SID)).toBe(321)
  })
})
