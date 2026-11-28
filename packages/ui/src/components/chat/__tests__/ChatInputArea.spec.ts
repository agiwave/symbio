// @vitest-environment happy-dom
/**
 * ChatInputArea — 输入区的两格反馈
 *
 * 钉住两条「界面必须说出来」的事实：
 * 1. 有内容时报字数（长文作者要看得见规模；此前完全没有反馈）；
 * 2. 运行中回车是**换行**不是发送——提示必须写明，否则用户以为发送失灵。
 *    这条说明住在输入框 placeholder 里（用户按键的地方），不再额外占一行；
 *    空输入时下方不该留下任何常驻提示行。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { ref } from 'vue'
import type { ImageAttachment } from '@/types'

const hoisted = vi.hoisted(() => ({ showToast: vi.fn() }))
vi.mock('@/composables/useToast', () => ({ useToast: () => ({ showToast: hoisted.showToast }) }))
vi.mock('@/utils/logger', () => ({ logger: { error: vi.fn() } }))

import ChatInputArea from '../ChatInputArea.vue'

describe('ChatInputArea：提示行', () => {
  it('有内容时显示字数（按用户可见字符计，emoji 不算两个字）', () => {
    const w = mount(ChatInputArea, { props: { isLoading: false, modelValue: '你好🙂' } })
    expect(w.find('.input-count').text()).toContain('3 字')
  })

  it('空输入不占提示行，回车说明在 placeholder 里', () => {
    const w = mount(ChatInputArea, { props: { isLoading: false, modelValue: '' } })
    expect(w.find('.input-meta').exists()).toBe(false)
    expect(w.find('textarea').attributes('placeholder')).toContain('Enter 发送')
  })

  it('运行中 placeholder 提示回车是换行（不发送）', () => {
    const w = mount(ChatInputArea, { props: { isLoading: true, modelValue: '' } })
    const placeholder = w.find('textarea').attributes('placeholder') ?? ''
    expect(placeholder).toContain('生成中')
    expect(placeholder).toContain('Enter 换行')
  })

  it('超长输入给出「消息较长」提示（不是硬上限）', () => {
    const w = mount(ChatInputArea, {
      props: { isLoading: false, modelValue: '甲'.repeat(2000) },
    })
    expect(w.find('.input-count').text()).toContain('消息较长')
    // 只是提示：发送键仍按内容可点
    expect(w.find('button.send-btn').attributes('disabled')).toBeUndefined()
  })
})

/** 可控 FileReader：批次的开始、成功、失败与取消都由用例按顺序触发。 */
class ControlledFileReader {
  static instances: ControlledFileReader[] = []
  result: string | null = null
  error: DOMException | null = null
  onload: (() => void) | null = null
  onerror: (() => void) | null = null
  onabort: (() => void) | null = null
  file: File | null = null
  aborted = false

  constructor() {
    ControlledFileReader.instances.push(this)
  }

  readAsDataURL(file: File): void {
    this.file = file
  }

  succeed(base64: string): void {
    this.result = `data:${this.file?.type ?? 'image/png'};base64,${base64}`
    this.onload?.()
  }

  fail(): void {
    this.error = new DOMException('broken', 'NotReadableError')
    this.onerror?.()
  }

  abort(): void {
    this.aborted = true
    this.onabort?.()
  }
}

interface InputHandle {
  transferImages: (images: ImageAttachment[]) => void
  restoreImages: (images: ImageAttachment[]) => boolean
}

const wrappers = new Set<VueWrapper>()
let blobSeq = 0
let createObjectUrl: ReturnType<typeof vi.spyOn>
let revokeObjectUrl: ReturnType<typeof vi.spyOn>

function track(w: VueWrapper): VueWrapper {
  wrappers.add(w)
  return w
}

function mountImages(initial: ImageAttachment[] = []) {
  const images = ref(initial)
  let w: VueWrapper
  w = track(mount(ChatInputArea, {
    props: {
      isLoading: false,
      attachedImages: images.value,
      'onUpdate:attachedImages': (value: ImageAttachment[]) => {
        images.value = value
        void w.setProps({ attachedImages: value })
      },
    },
  }))
  return { w, images }
}

function imageFile(name: string): File {
  return new File(['image'], name, { type: 'image/png' })
}

async function selectFiles(w: VueWrapper, files: File[]): Promise<void> {
  const input = w.get('input[type="file"]').element as HTMLInputElement
  Object.defineProperty(input, 'files', { configurable: true, value: files })
  input.dispatchEvent(new Event('change', { bubbles: true }))
  await flushPromises()
}

async function pasteFiles(w: VueWrapper, files: File[]): Promise<void> {
  const event = new Event('paste', { bubbles: false, cancelable: true })
  Object.defineProperty(event, 'clipboardData', { value: { files } })
  w.get('textarea').element.dispatchEvent(event)
  await flushPromises()
}

async function completeReaders(start: number, end: number): Promise<void> {
  for (let i = start; i < end; i++) {
    ControlledFileReader.instances[i].succeed(`data-${i}`)
    await flushPromises()
  }
}

beforeEach(() => {
  ControlledFileReader.instances = []
  blobSeq = 0
  hoisted.showToast.mockClear()
  vi.stubGlobal('FileReader', ControlledFileReader)
  vi.spyOn(URL, 'createObjectURL').mockImplementation(() => `blob:chat-${++blobSeq}`)
  revokeObjectUrl = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
  createObjectUrl = vi.mocked(URL.createObjectURL)
})

afterEach(() => {
  for (const w of wrappers) w.unmount()
  wrappers.clear()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('ChatInputArea：附件批次', () => {
  it('连续两批按提交顺序串行完成，后批不覆盖先批', async () => {
    const { w, images } = mountImages()
    await selectFiles(w, [imageFile('first.png')])
    const firstReader = ControlledFileReader.instances[0]

    await pasteFiles(w, [imageFile('second.png')])
    // 第二批在队列里等待，不能与第一批并发写各自的旧快照。
    expect(ControlledFileReader.instances).toHaveLength(1)

    firstReader.succeed('FIRST')
    await flushPromises()
    expect(images.value.map((image) => image.fileName)).toEqual(['first.png'])
    expect(ControlledFileReader.instances).toHaveLength(2)

    ControlledFileReader.instances[1].succeed('SECOND')
    await flushPromises()
    expect(images.value.map((image) => image.fileName)).toEqual(['first.png', 'second.png'])
    expect(images.value.map((image) => image.base64)).toEqual(['FIRST', 'SECOND'])
  })

  it('超过十张时只保留最新十张，并释放被挤掉的 object URL', async () => {
    const { w, images } = mountImages()
    const files = Array.from({ length: 13 }, (_, i) => imageFile(`${i}.png`))
    await selectFiles(w, files.slice(0, 8))
    await completeReaders(0, 8)
    await selectFiles(w, files.slice(8))
    await completeReaders(8, 13)

    expect(images.value.map((image) => image.fileName)).toEqual(files.slice(3).map((file) => file.name))
    expect(createObjectUrl).toHaveBeenCalledTimes(13)
    expect(revokeObjectUrl.mock.calls.map((call: unknown[]) => String(call[0]))).toEqual(['blob:chat-1', 'blob:chat-2', 'blob:chat-3'])
    expect(hoisted.showToast).toHaveBeenCalledWith('error', expect.stringContaining('超出 10 张上限'))
  })

  it('父区直接回填超过十张时也统一截断到最新十张', async () => {
    const { w, images } = mountImages()
    const external = Array.from({ length: 11 }, (_, i) => ({
      base64: `${i}`,
      mimeType: 'image/png',
      fileName: `${i}.png`,
      thumbnailUrl: `blob:external-${i}`,
    }))
    await w.setProps({ attachedImages: external })
    await flushPromises()

    expect(images.value.map((image) => image.fileName)).toEqual(external.slice(1).map((image) => image.fileName))
    expect(revokeObjectUrl).not.toHaveBeenCalled()
  })

  it('单项读取失败给出反馈，同批成功项仍保留', async () => {
    const { w, images } = mountImages()
    await selectFiles(w, [imageFile('good-1.png'), imageFile('bad.png'), imageFile('good-2.png')])
    ControlledFileReader.instances[0].succeed('ONE')
    await flushPromises()
    ControlledFileReader.instances[1].fail()
    await flushPromises()
    ControlledFileReader.instances[2].succeed('TWO')
    await flushPromises()

    expect(images.value.map((image) => image.fileName)).toEqual(['good-1.png', 'good-2.png'])
    expect(hoisted.showToast).toHaveBeenCalledWith('error', expect.stringContaining('bad.png：读取失败'))
    expect(revokeObjectUrl).not.toHaveBeenCalled()
  })
})

describe('ChatInputArea：object URL 所有权', () => {
  it('卸载释放本区创建且未发送的 URL', async () => {
    const { w } = mountImages()
    await selectFiles(w, [imageFile('draft.png')])
    ControlledFileReader.instances[0].succeed('DRAFT')
    await flushPromises()

    w.unmount()
    wrappers.delete(w)
    expect(revokeObjectUrl).toHaveBeenCalledTimes(1)
    expect(revokeObjectUrl).toHaveBeenCalledWith('blob:chat-1')
  })

  it('卸载中止在途读取，不创建悬空 URL', async () => {
    const { w, images } = mountImages()
    await selectFiles(w, [imageFile('pending.png')])
    const reader = ControlledFileReader.instances[0]

    w.unmount()
    wrappers.delete(w)
    expect(reader.aborted).toBe(true)
    expect(createObjectUrl).not.toHaveBeenCalled()
    expect(images.value).toEqual([])
  })

  it('已转移 URL 由接收方释放，输入区卸载不重复 revoke', async () => {
    const { w, images } = mountImages()
    await selectFiles(w, [imageFile('sent.png')])
    ControlledFileReader.instances[0].succeed('SENT')
    await flushPromises()
    const image = images.value[0]

    ;(w.vm as unknown as InputHandle).transferImages([image])
    URL.revokeObjectURL(image.thumbnailUrl!)
    w.unmount()
    wrappers.delete(w)
    expect(revokeObjectUrl).toHaveBeenCalledTimes(1)
  })

  it('发送失败接回所有权后，卸载仍会释放', async () => {
    const { w, images } = mountImages()
    await selectFiles(w, [imageFile('retry.png')])
    ControlledFileReader.instances[0].succeed('RETRY')
    await flushPromises()
    const image = images.value[0]
    const handle = w.vm as unknown as InputHandle

    handle.transferImages([image])
    expect(handle.restoreImages([image])).toBe(true)
    w.unmount()
    wrappers.delete(w)
    expect(revokeObjectUrl).toHaveBeenCalledTimes(1)
    expect(revokeObjectUrl).toHaveBeenCalledWith('blob:chat-1')
  })

  it('外部传入的 URL 不归输入区所有，移除时不得释放', async () => {
    const external: ImageAttachment = {
      base64: 'EXTERNAL',
      mimeType: 'image/png',
      thumbnailUrl: 'blob:external',
    }
    const { w } = mountImages([external])
    await w.get('.remove-image').trigger('click')
    expect(revokeObjectUrl).not.toHaveBeenCalled()
  })

  it('有未发送图片时明确说明切换会话会清除', async () => {
    const { w } = mountImages([{ base64: 'A', mimeType: 'image/png', thumbnailUrl: 'blob:a' }])
    expect(w.get('.attachment-note').text()).toContain('切换会话后会清除')
  })
})
