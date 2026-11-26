// @vitest-environment happy-dom
/**
 * ModelChatPanel 附件生命周期：真实 ChatComposer / ChatInputArea + 可控发送桩。
 *
 * 钉住所有权的四条路径：成功释放一次、失败 / 取消接回、在途卸载收尾、
 * 切换会话丢弃未发送图片。懒创建邮箱传入的 URL 不属于输入区，也由面板负责；
 * 新建草稿发送方则负责到「成功转入邮箱」为止的交接。
 */
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import type { ImageAttachment } from '@/types'

interface ChatStub {
  isLoading: { value: boolean }
  isStopping: { value: boolean }
  stopNotice: { value: { tone: 'info' | 'success' | 'error'; text: string } | null }
  isWaitingApproval: { value: boolean }
  isConnected: { value: boolean }
  messageTree: { value: unknown[] }
  send: Mock
  abort: Mock
  resume: Mock
  switchBranch: Mock
  skeletonLazy: { onCollapse: Mock; isReleased: Mock; summaryOf: Mock }
}

const hoisted = vi.hoisted(() => ({
  chat: null as ChatStub | null,
  showToast: vi.fn(),
}))

vi.mock('@/composables/useChatConnection', async () => {
  const { ref, shallowRef } = await import('vue')
  hoisted.chat = {
    isLoading: ref(false),
    isStopping: ref(false),
    stopNotice: ref(null),
    isWaitingApproval: ref(false),
    isConnected: ref(true),
    messageTree: shallowRef([]),
    send: vi.fn(),
    abort: vi.fn(),
    resume: vi.fn(),
    switchBranch: vi.fn(),
    skeletonLazy: { onCollapse: vi.fn(), isReleased: vi.fn(() => false), summaryOf: vi.fn(() => undefined) },
  }
  return {
    RESUME_KEY: Symbol('resume'),
    SKELETON_LAZY_KEY: Symbol('chat-skeleton-lazy'),
    SWITCH_BRANCH_KEY: Symbol('chat-switch-branch'),
    ALLOW_FOR_SESSION_KEY: Symbol('chat-allow-for-session'),
    ALLOW_FOREVER_KEY: Symbol('chat-allow-forever'),
    useChatConnection: () => hoisted.chat,
  }
})

vi.mock('@/composables/useChatScroll', async () => {
  const { ref } = await import('vue')
  return {
    useChatScroll: () => ({
      isUserAtBottom: ref(true),
      scrollToBottom: vi.fn(),
      scrollToGap: vi.fn(),
      gapFromBottom: vi.fn(() => null),
      handleScroll: vi.fn(),
      smartScroll: vi.fn(),
    }),
  }
})

vi.mock('@/composables/useToast', () => ({ useToast: () => ({ showToast: hoisted.showToast }) }))
vi.mock('@/utils/logger', () => ({ logger: { info: vi.fn(), warn: vi.fn(), error: vi.fn() } }))
vi.mock('../chat/ChatOptionBar.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return {
    default: defineComponent({
      name: 'ChatOptionBarStub',
      props: { sessionId: String, definition: Object, values: Object, scope: Object },
      setup: (_props, { expose }) => {
        expose({ getDraftMetadata: () => ({}) })
        return () => h('div')
      },
    }),
  }
})

import ModelChatPanel from '../ModelChatPanel.vue'
import Session from '@/components/vdfs/Session.vue'
import { useSessionsStore } from '@/stores/sessions'

class ControlledFileReader {
  static instances: ControlledFileReader[] = []
  result: string | null = null
  error: DOMException | null = null
  onload: (() => void) | null = null
  onerror: (() => void) | null = null
  onabort: (() => void) | null = null
  file: File | null = null

  constructor() {
    ControlledFileReader.instances.push(this)
  }

  readAsDataURL(file: File): void {
    this.file = file
  }

  succeed(): void {
    this.result = `data:${this.file?.type ?? 'image/png'};base64,IMAGE`
    this.onload?.()
  }

  abort(): void {
    this.onabort?.()
  }
}

let w: VueWrapper | null = null
let blobSeq = 0
let revokeSpy: ReturnType<typeof vi.spyOn>

function chatStub(): ChatStub {
  if (!hoisted.chat) throw new Error('useChatConnection 桩未装配')
  return hoisted.chat
}

function panel(sessionId = 's1'): VueWrapper {
  w = mount(ModelChatPanel, {
    props: { sessionId, sessionAddr: `@vfs/session/${sessionId}` },
    attachTo: document.body,
  })
  return w
}

function draft(): VueWrapper {
  w = mount(Session, { props: { node: null }, attachTo: document.body })
  return w
}

async function attachImage(name = 'draft.png'): Promise<void> {
  const wrapper = w!
  const input = wrapper.get('input[type="file"]').element as HTMLInputElement
  const file = new File(['image'], name, { type: 'image/png' })
  Object.defineProperty(input, 'files', { configurable: true, value: [file] })
  input.dispatchEvent(new Event('change', { bubbles: true }))
  await flushPromises()
  const readers = ControlledFileReader.instances
  readers[readers.length - 1].succeed()
  await flushPromises()
  expect(wrapper.findAll('.image-item')).toHaveLength(1)
}

beforeEach(() => {
  setActivePinia(createPinia())
  ControlledFileReader.instances = []
  blobSeq = 0
  hoisted.showToast.mockClear()
  const chat = chatStub()
  chat.send.mockReset()
  chat.abort.mockClear()
  chat.isLoading.value = false
  chat.isStopping.value = false
  chat.stopNotice.value = null
  vi.stubGlobal('FileReader', ControlledFileReader)
  vi.spyOn(URL, 'createObjectURL').mockImplementation(() => `blob:panel-${++blobSeq}`)
  revokeSpy = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
})

afterEach(() => {
  w?.unmount()
  w = null
  document.body.replaceChildren()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('ModelChatPanel 附件生命周期', () => {
  it('发送成功释放一次，随后卸载不重复 revoke', async () => {
    chatStub().send.mockResolvedValue(true)
    panel()
    await attachImage()

    await w!.find('.send-btn').trigger('click')
    await flushPromises()
    expect(w!.find('.image-item').exists()).toBe(false)
    expect(revokeSpy).toHaveBeenCalledTimes(1)
    expect(revokeSpy).toHaveBeenCalledWith('blob:panel-1')

    w!.unmount()
    w = null
    expect(revokeSpy).toHaveBeenCalledTimes(1)
  })

  it('发送失败 / 取消时接回输入区，URL 保持可用直到卸载', async () => {
    chatStub().send.mockResolvedValue(false)
    panel()
    await attachImage()

    await w!.find('.send-btn').trigger('click')
    await flushPromises()
    expect(w!.find('.image-item').exists()).toBe(true)
    expect(w!.get('.image-item img').attributes('src')).toBe('blob:panel-1')
    expect(revokeSpy).not.toHaveBeenCalled()

    w!.unmount()
    w = null
    expect(revokeSpy).toHaveBeenCalledTimes(1)
  })

  it('发送 Promise 拒绝也按失败恢复，不丢附件', async () => {
    chatStub().send.mockRejectedValue(new Error('cancelled'))
    panel()
    await attachImage()

    await w!.find('.send-btn').trigger('click')
    await flushPromises()
    expect(w!.find('.image-item').exists()).toBe(true)
    expect(revokeSpy).not.toHaveBeenCalled()
  })

  it('发送在途时切换 / 卸载，面板回收已接手 URL 且回执后不重复释放', async () => {
    let finish!: (ok: boolean) => void
    chatStub().send.mockReturnValue(new Promise<boolean>((resolve) => { finish = resolve }))
    panel()
    await attachImage()

    await w!.find('.send-btn').trigger('click')
    await flushPromises()
    expect(w!.find('.image-item').exists()).toBe(false)
    expect(revokeSpy).not.toHaveBeenCalled()

    w!.unmount()
    w = null
    expect(revokeSpy).toHaveBeenCalledTimes(1)
    finish(true)
    await flushPromises()
    expect(revokeSpy).toHaveBeenCalledTimes(1)
  })

  it('切换会话丢弃未发送图片并释放 URL，新会话不显示旧预览', async () => {
    panel()
    await attachImage()

    w!.unmount()
    w = null
    expect(revokeSpy).toHaveBeenCalledTimes(1)

    panel('s2')
    await flushPromises()
    expect(w!.find('.image-item').exists()).toBe(false)
    expect(revokeSpy).toHaveBeenCalledTimes(1)
  })

  it('失败后重试成功，附件只释放一次且不留在输入区', async () => {
    chatStub().send.mockResolvedValueOnce(false).mockResolvedValueOnce(true)
    panel()
    await attachImage()

    await w!.find('.send-btn').trigger('click')
    await flushPromises()
    const retryCall = hoisted.showToast.mock.calls.find((call) => call[3]?.label === '重试')
    expect(retryCall).toBeDefined()
    retryCall![3].run()
    await flushPromises()

    expect(chatStub().send).toHaveBeenCalledTimes(2)
    expect(w!.find('.image-item').exists()).toBe(false)
    expect(revokeSpy).toHaveBeenCalledTimes(1)
  })

  it('懒创建邮箱传入的 URL 发送成功后由面板释放，输入区不认领', async () => {
    const store = useSessionsStore()
    const pending: ImageAttachment = {
      base64: 'PENDING',
      mimeType: 'image/png',
      thumbnailUrl: 'blob:pending',
    }
    vi.spyOn(store, 'consumePendingFirstMessage').mockReturnValue({
      id: 's1',
      text: '',
      images: [pending],
    })
    chatStub().send.mockResolvedValue(true)

    panel()
    await flushPromises()
    expect(revokeSpy).toHaveBeenCalledTimes(1)
    expect(revokeSpy).toHaveBeenCalledWith('blob:pending')

    w!.unmount()
    w = null
    expect(revokeSpy).toHaveBeenCalledTimes(1)
  })
})

describe('Session 懒创建附件所有权', () => {
  it('创建成功即转移给邮箱，草稿卸载不得 revoke', async () => {
    const store = useSessionsStore()
    const create = vi.spyOn(store, 'createSessionWithFirstMessage').mockResolvedValue('s9')
    draft()
    await attachImage('first.png')

    await w!.find('.send-btn').trigger('click')
    await flushPromises()
    expect(create).toHaveBeenCalledWith({}, expect.objectContaining({
      text: '',
      images: [expect.objectContaining({ thumbnailUrl: 'blob:panel-1' })],
    }))
    expect(w!.emitted('created')).toEqual([['s9']])
    expect(revokeSpy).not.toHaveBeenCalled()

    w!.unmount()
    w = null
    expect(revokeSpy).not.toHaveBeenCalled()
  })

  it('创建失败接回草稿，所有权恢复且预览可用', async () => {
    const store = useSessionsStore()
    vi.spyOn(store, 'createSessionWithFirstMessage').mockRejectedValue(new Error('create failed'))
    draft()
    await attachImage()

    await w!.find('.send-btn').trigger('click')
    await flushPromises()
    expect(w!.find('.image-item').exists()).toBe(true)
    expect(w!.get('.image-item img').attributes('src')).toBe('blob:panel-1')
    expect(revokeSpy).not.toHaveBeenCalled()

    w!.unmount()
    w = null
    expect(revokeSpy).toHaveBeenCalledTimes(1)
  })

  it('创建在途时卸载，失败回执由草稿发送方回收 URL', async () => {
    const store = useSessionsStore()
    let fail!: (error: Error) => void
    vi.spyOn(store, 'createSessionWithFirstMessage').mockReturnValue(
      new Promise<string>((_resolve, reject) => { fail = reject }),
    )
    draft()
    await attachImage()

    await w!.find('.send-btn').trigger('click')
    await flushPromises()
    w!.unmount()
    w = null
    expect(revokeSpy).not.toHaveBeenCalled()

    fail(new Error('cancelled'))
    await flushPromises()
    expect(revokeSpy).toHaveBeenCalledTimes(1)
  })

  it('创建在途时卸载，成功回执仍把 URL 转交邮箱而不提前释放', async () => {
    const store = useSessionsStore()
    let finish!: (id: string) => void
    const create = vi.spyOn(store, 'createSessionWithFirstMessage').mockReturnValue(
      new Promise<string>((resolve) => { finish = resolve }),
    )
    draft()
    await attachImage()

    await w!.find('.send-btn').trigger('click')
    await flushPromises()
    w!.unmount()
    w = null
    finish('s9')
    await flushPromises()

    expect(create).toHaveBeenCalledTimes(1)
    expect(revokeSpy).not.toHaveBeenCalled()
  })

  it('未发送草稿直接卸载，由输入区释放 URL', async () => {
    draft()
    await attachImage()

    w!.unmount()
    w = null
    expect(revokeSpy).toHaveBeenCalledTimes(1)
    expect(revokeSpy).toHaveBeenCalledWith('blob:panel-1')
  })
})
