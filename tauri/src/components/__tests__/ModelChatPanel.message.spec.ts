// @vitest-environment happy-dom
/** 面板停止反馈，以及多模态消息编辑 / 删除恢复的组件级回归。 */
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { defineComponent, h, nextTick, type PropType } from 'vue'
import type { ChatNode, ContentPart } from '@/schemas/chat_node'
import { PRINCIPAL_KIND_PERSON, PRINCIPAL_USER } from '@/schemas/chat_node'
import type { ImageAttachment } from '@/types'

interface ChatStub {
  isLoading: { value: boolean }
  isStopping: { value: boolean }
  stopNotice: { value: { tone: 'info' | 'success' | 'error'; text: string } | null }
  messageTree: { value: ChatNode[] }
  send: Mock
  abort: Mock
  removeMessage: Mock
  resume: Mock
  expandSkeleton: Mock
  switchBranch: Mock
  skeletonLazy: { onCollapse: Mock; isReleased: Mock; summaryOf: Mock }
}

const hoisted = vi.hoisted(() => ({ chat: null as ChatStub | null, showToast: vi.fn() }))

vi.mock('@/composables/useChatConnection', async () => {
  const { ref, shallowRef } = await import('vue')
  hoisted.chat = {
    isLoading: ref(false),
    isStopping: ref(false),
    stopNotice: ref(null),
    messageTree: shallowRef([]),
    send: vi.fn(),
    abort: vi.fn(),
    removeMessage: vi.fn(),
    resume: vi.fn(),
    expandSkeleton: vi.fn(),
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

const FakeMessageNode = defineComponent({
  name: 'MessageNodeStub',
  props: { node: { type: Object, required: true } },
  emits: ['edit', 'delete'],
  setup: (props, { emit }) => () => h('div', { class: 'fake-message' }, [
    h('button', { class: 'edit-message', onClick: () => emit('edit', props.node.id) }, '编辑'),
    h('button', { class: 'delete-message', onClick: () => emit('delete', props.node.id) }, '删除'),
  ]),
})

const ComposerStub = defineComponent({
  name: 'ChatComposerStub',
  props: {
    modelValue: { type: String, default: '' },
    attachedImages: { type: Array as PropType<ImageAttachment[]>, default: () => [] },
    isLoading: Boolean,
  },
  emits: ['update:modelValue', 'update:attachedImages', 'submit'],
  setup: (props, { emit, expose }) => {
    expose({
      resetHeight: vi.fn(),
      transferImages: vi.fn(),
      restoreImages: (images: ImageAttachment[]) => {
        emit('update:attachedImages', [...props.attachedImages, ...images])
        return true
      },
    })
    return () => h('div')
  },
})

import ModelChatPanel from '../ModelChatPanel.vue'
import { useSessionsStore } from '@/stores/sessions'

const SID = 's1'
const IMAGE_A: ContentPart = {
  type: 'image_url',
  image_url: { url: 'data:image/png;base64,AAAA', detail: 'high' },
}
const IMAGE_B: ContentPart = {
  type: 'image_url',
  image_url: { url: 'data:image/jpeg;base64,BBBB' },
}

let w: VueWrapper | null = null

function chatStub(): ChatStub {
  if (!hoisted.chat) throw new Error('useChatConnection 桩未装配')
  return hoisted.chat
}

function userMessage(content: ChatNode['content']): ChatNode {
  return {
    id: 'u1',
    seq: 1,
    type: 'body',
    principal: { kind: PRINCIPAL_KIND_PERSON, id: PRINCIPAL_USER },
    content,
    timestamp: 1,
  }
}

function panel(message?: ChatNode): VueWrapper {
  if (message) {
    useSessionsStore().applyTranscriptMessage(SID, message)
    chatStub().messageTree.value = [message]
  }
  w = mount(ModelChatPanel, {
    props: { sessionId: SID, sessionAddr: `@vfs/session/${SID}` },
    attachTo: document.body,
    global: { stubs: { MessageNode: FakeMessageNode, ChatComposer: ComposerStub } },
  })
  return w
}

async function openEditor(): Promise<void> {
  await w!.get('.edit-message').trigger('click')
  await nextTick()
}

async function confirmDelete(): Promise<void> {
  await w!.get('.delete-message').trigger('click')
  await nextTick()
  await w!.get('.confirm-btn.primary').trigger('click')
  await flushPromises()
}

beforeEach(() => {
  setActivePinia(createPinia())
  hoisted.showToast.mockClear()
  const chat = chatStub()
  chat.isLoading.value = false
  chat.isStopping.value = false
  chat.stopNotice.value = null
  chat.messageTree.value = []
})

afterEach(() => {
  w?.unmount()
  w = null
  document.body.replaceChildren()
  vi.restoreAllMocks()
})

describe('ModelChatPanel 停止反馈', () => {
  it('polite 状态区始终挂载，停止过程与成功都有可见且不重复播报的反馈', async () => {
    panel()
    const live = w!.get('.stop-live')
    expect(live.attributes()).toMatchObject({
      role: 'status',
      'aria-live': 'polite',
      'aria-atomic': 'true',
    })
    expect(live.text()).toBe('')

    chatStub().isStopping.value = true
    chatStub().stopNotice.value = { tone: 'info', text: '正在停止当前请求…' }
    await nextTick()
    expect(w!.get('.stop-notice').text()).toBe('正在停止当前请求…')
    expect(w!.get('.stop-notice').attributes('aria-hidden')).toBe('true')
    expect(w!.get('.stop-live').text()).toBe('正在停止当前请求…')
    expect(w!.findComponent(ComposerStub).props('isLoading')).toBe(true)

    chatStub().isStopping.value = false
    chatStub().stopNotice.value = { tone: 'success', text: '已停止当前请求' }
    await nextTick()
    expect(w!.get('.stop-notice').text()).toBe('已停止当前请求')
    expect(w!.get('.stop-live').text()).toBe('已停止当前请求')
  })
})

describe('ModelChatPanel 多模态消息编辑', () => {
  it('展示已有附件，保存时保留原 image_url 分片（含 detail）', async () => {
    const message = userMessage([{ type: 'text', text: '原正文' }, IMAGE_A, IMAGE_B])
    const update = vi.spyOn(useSessionsStore(), 'updateMessage').mockResolvedValue(undefined)
    panel(message)
    await openEditor()

    expect(w!.findAll('.edit-attachment img').map((img) => img.attributes('src'))).toEqual([
      IMAGE_A.image_url.url,
      IMAGE_B.image_url.url,
    ])
    await w!.get('.edit-area').setValue('改后正文')
    await w!.get('.edit-save').trigger('click')
    await flushPromises()

    expect(update).toHaveBeenCalledWith(SID, expect.objectContaining({
      content: [{ type: 'text', text: '改后正文' }, IMAGE_A, IMAGE_B],
    }))
  })

  it('允许逐张移除附件，保存后只保留选中的图片', async () => {
    const message = userMessage([{ type: 'text', text: '原正文' }, IMAGE_A, IMAGE_B])
    const update = vi.spyOn(useSessionsStore(), 'updateMessage').mockResolvedValue(undefined)
    panel(message)
    await openEditor()

    await w!.get('.edit-attachment-remove').trigger('click')
    expect(w!.findAll('.edit-attachment img')).toHaveLength(1)
    await w!.get('.edit-save').trigger('click')
    await flushPromises()

    expect(update).toHaveBeenCalledWith(SID, expect.objectContaining({
      content: [{ type: 'text', text: '原正文' }, IMAGE_B],
    }))
  })

  it('全部移除后回落纯文本，不留下空图片数组', async () => {
    const message = userMessage([{ type: 'text', text: '原正文' }, IMAGE_A])
    const update = vi.spyOn(useSessionsStore(), 'updateMessage').mockResolvedValue(undefined)
    panel(message)
    await openEditor()

    await w!.get('.edit-attachment-remove').trigger('click')
    await w!.get('.edit-save').trigger('click')
    await flushPromises()

    expect(update).toHaveBeenCalledWith(SID, expect.objectContaining({ content: '原正文' }))
  })
})

describe('ModelChatPanel 删除用户消息', () => {
  it('正文与 data URL 附件一并恢复到输入区', async () => {
    const message = userMessage([{ type: 'text', text: '请继续' }, IMAGE_A, IMAGE_B])
    const remove = vi.spyOn(useSessionsStore(), 'deleteMessage').mockResolvedValue(undefined)
    panel(message)
    await confirmDelete()

    expect(remove).toHaveBeenCalledWith(SID, 'u1')
    const composer = w!.findComponent(ComposerStub)
    expect(composer.props('modelValue')).toBe('请继续')
    expect(composer.props('attachedImages')).toEqual([
      { base64: 'AAAA', mimeType: 'image/png', fileName: '图片 1', thumbnailUrl: IMAGE_A.image_url.url },
      { base64: 'BBBB', mimeType: 'image/jpeg', fileName: '图片 2', thumbnailUrl: IMAGE_B.image_url.url },
    ])
  })

  it('非 data URL 附件不伪造恢复，并明确提示不可恢复', async () => {
    const remote: ContentPart = { type: 'image_url', image_url: { url: 'https://example.test/a.png' } }
    vi.spyOn(useSessionsStore(), 'deleteMessage').mockResolvedValue(undefined)
    panel(userMessage([{ type: 'text', text: '请继续' }, remote]))
    await confirmDelete()

    expect(w!.findComponent(ComposerStub).props('modelValue')).toBe('请继续')
    expect(w!.findComponent(ComposerStub).props('attachedImages')).toEqual([])
    expect(hoisted.showToast).toHaveBeenCalledWith(
      'error',
      '删除完成，但 1 个附件无法恢复：仅支持内嵌 base64 图片',
    )
  })
})
