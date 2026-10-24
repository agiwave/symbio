// @vitest-environment happy-dom
/**
 * ModelChatPanel 的 **Esc 停止生成** 单测（happy-dom）
 *
 * 只钉这一条绑定，因为它的三个判据各自都静默：
 *
 * 1. **声明在本组件而不是外壳里**——「这一轮还在跑吗」是本组件经
 *    `useChatConnection` 才有的状态；外壳只负责判键与让路。
 * 2. **`when` 取的是运行态**：没有东西可停时 Esc 不该被吃掉（否则确认框、
 *    图片预览那一类「Esc 关掉」的期望会莫名失灵）。
 * 3. **`allowInInput` 是必需的例外**：真要按停止的时候，焦点正落在输入框里
 *    （刚发完言）。少了这个标志，这条键在它唯一的使用场景里失灵。
 *
 * 机制本身（判键、三条守卫、派发序）由 `composables/__tests__/useHotkeys.spec.ts`
 * 钉；这里只把「这条绑定登记了什么」钉住。桩只打在 `useChatConnection` 一层
 * （它是唯一出站的），store 用真的——本组件对 store 只做按会话 id 的读写。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import type { Mock } from 'vitest'

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
}

const hoisted = vi.hoisted(() => ({ chat: null as ChatStub | null }))

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
  }
  return { RESUME_KEY: Symbol('resume'), useChatConnection: () => hoisted.chat }
})

import ModelChatPanel from '../ModelChatPanel.vue'
import { dispatchHotkeys } from '@/composables/useHotkeys'

const chatStub = (): ChatStub => {
  if (!hoisted.chat) throw new Error('useChatConnection 桩未装配（面板没被导入？）')
  return hoisted.chat
}

let w: VueWrapper | null = null

function panel() {
  w = mount(ModelChatPanel, { props: { sessionId: 's1', sessionAddr: '@vfs/session/s1' }, attachTo: document.body })
  return w
}

/**
 * 在输入框里按一次 Esc：真实场景是「刚发完言，光标还停在输入框」。
 *
 * 输入框由 `ChatComposer` 承担，本文件把它连同消息节点一起浅桩掉了，于是这里
 * 放一个真实的 `textarea` 当事件目标——打字优先守卫读的就是 `e.target`，
 * 目标在不在本组件的树里不参与判定。
 */
function pressEscInTextarea() {
  const area = document.createElement('textarea')
  document.body.appendChild(area)
  area.focus()
  const evt = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
  area.dispatchEvent(evt)
  return { consumed: dispatchHotkeys(evt), prevented: evt.defaultPrevented }
}

beforeEach(() => {
  setActivePinia(createPinia())
  const chat = chatStub()
  for (const fn of [chat.send, chat.abort, chat.removeMessage, chat.resume, chat.expandSkeleton]) fn.mockClear()
  chat.isLoading.value = false
})

afterEach(() => {
  w?.unmount()
  w = null
  document.body.replaceChildren()
})

describe('ModelChatPanel 的 Esc（停止生成）', () => {
  it('★ 正在生成 ⇒ 在输入框里按 Esc 也能中止（allowInInput 是那条守卫的必需例外）', () => {
    chatStub().isLoading.value = true
    panel()

    expect(pressEscInTextarea()).toEqual({ consumed: true, prevented: true })
    expect(chatStub().abort).toHaveBeenCalledTimes(1)
  })

  it('没有在生成 ⇒ Esc 不吞键（关确认框、关预览那一类期望不该被这条键顶掉）', () => {
    panel()

    expect(pressEscInTextarea()).toEqual({ consumed: false, prevented: false })
    expect(chatStub().abort).not.toHaveBeenCalled()
  })

  /**
   * 卸载即注销：面板按会话地址**重挂载**（`:key`），漏注销会让上一个会话的绑定
   * 继续抢键——表现为「切了会话，Esc 停的是上一轮」。
   */
  it('unmount 后这条绑定不再命中', () => {
    chatStub().isLoading.value = true
    panel().unmount()
    w = null
    chatStub().abort.mockClear()

    expect(pressEscInTextarea()).toEqual({ consumed: false, prevented: false })
    expect(chatStub().abort).not.toHaveBeenCalled()
  })
})
