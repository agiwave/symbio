// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { defineComponent } from 'vue'
import CodeEditor from '../CodeEditor.vue'

const Host = defineComponent({
  components: { CodeEditor },
  template: `
    <div>
      <button id="before">前一个控件</button>
      <CodeEditor model-value="const value = 1" />
      <button id="after">后一个控件</button>
    </div>
  `,
})

let w: ReturnType<typeof mount>

beforeEach(() => {
  w = mount(Host, { attachTo: document.body })
})

afterEach(() => {
  w.unmount()
})

/**
 * happy-dom 不实现浏览器原生的 Tab 遍历；仅在事件未被取消后模拟该默认动作。
 * 旧编辑器会 preventDefault，因此焦点仍被困在 textarea 内。
 */
function tabOut(shiftKey = false) {
  const textarea = w.find('textarea').element as HTMLTextAreaElement
  const target = w.find(shiftKey ? '#before' : '#after').element as HTMLElement
  textarea.focus()
  const event = new KeyboardEvent('keydown', {
    key: 'Tab',
    shiftKey,
    bubbles: true,
    cancelable: true,
  })
  textarea.dispatchEvent(event)
  if (!event.defaultPrevented) target.focus()
  return { event, textarea, target }
}

describe('CodeEditor：键盘焦点', () => {
  it('Tab 不被拦截，焦点可移到后继控件且不插入空格', () => {
    const { event, textarea, target } = tabOut()

    expect(event.defaultPrevented).toBe(false)
    expect(document.activeElement).toBe(target)
    expect(textarea.value).toBe('const value = 1')
    expect(w.findComponent(CodeEditor).emitted('update:modelValue')).toBeUndefined()
  })

  it('Shift+Tab 不被拦截，焦点可移到前一控件', () => {
    const { event, target } = tabOut(true)

    expect(event.defaultPrevented).toBe(false)
    expect(document.activeElement).toBe(target)
  })

  it('保留 Ctrl/Cmd+S 保存快捷键', () => {
    const textarea = w.find('textarea').element as HTMLTextAreaElement
    textarea.focus()
    const event = new KeyboardEvent('keydown', {
      key: 's',
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    })
    textarea.dispatchEvent(event)

    expect(event.defaultPrevented).toBe(true)
    expect(w.findComponent(CodeEditor).emitted('request-save')).toHaveLength(1)
  })
})
