// @vitest-environment happy-dom
/**
 * ChatComposer —— **装配与透传**的单测
 *
 * 本组件存在的理由是：输入区（`ChatInputArea + ChatOptionBar`）此前在
 * `Session.vue`（草稿态）与 `ModelChatPanel.vue`（会话态）各装配一遍，
 * 「输入区由什么组成」有两个答案。收成一处后，这里钉住三件事：
 *
 * 1. **两件都在** —— 装配不会悄悄少一件（选项行是「输入区」的一部分，不是可选配件）；
 * 2. **`sessionId` 原样透传，且草稿态必须是 `undefined`** —— 选项行按「有没有
 *    sessionId」决定「选择写后端」还是「缓冲待创建」。透传成**空串**会让它
 *    误以为有会话（空串是合法 prop 值），从而把草稿选择丢掉；
 * 3. **暴露面只经一个 ref** —— `resetHeight` / `getDraftMetadata` 分别委派给
 *    两件子件，父级不必各持一个 ref。
 *
 * 环境说明：只桩掉选项行（它经 `useSessionOptions` 出站到后端）。输入框用真件
 * ——`resetHeight` 委派与 `autofocus` 都是它自己的行为，桩掉就测了个空。
 */
import { describe, it, expect, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { nextTick } from 'vue'

/**
 * 选项行桩：记录收到的 `sessionId`，并暴露 `getDraftMetadata`（真件同样暴露）。
 * `sessionId` 为 `undefined` 时不落 DOM 属性，以便用属性存在与否区分「没传」与
 * 「传了空串」。
 */
vi.mock('../ChatOptionBar.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return {
    default: defineComponent({
      name: 'ChatOptionBarStub',
      props: { sessionId: { type: String, default: undefined } },
      setup(props, { expose }) {
        expose({
          getDraftMetadata: () => ({ probe: props.sessionId ?? 'draft' }),
        })
        return () =>
          h('div', {
            class: 'option-bar-stub',
            ...(props.sessionId !== undefined ? { 'data-session': props.sessionId } : {}),
          })
      },
    }),
  }
})

import ChatComposer from '../ChatComposer.vue'

function optionBar(w: ReturnType<typeof mount>) {
  return w.findComponent({ name: 'ChatOptionBarStub' })
}

describe('ChatComposer：装配', () => {
  it('输入框与选项行一起装配（选项行不是可选配件）', () => {
    const w = mount(ChatComposer)
    expect(w.find('textarea').exists()).toBe(true)
    expect(optionBar(w).exists()).toBe(true)
  })
})

describe('ChatComposer：sessionId 两种模式', () => {
  it('草稿态（不传 sessionId）⇒ 选项行拿到 undefined，而不是空串', () => {
    const w = mount(ChatComposer)
    expect(optionBar(w).props('sessionId')).toBeUndefined()
    // 空串是合法 prop 值 ⇒ 选项行会当成「某个会话」⇒ 草稿选择不再缓冲。挡住它。
    expect(optionBar(w).attributes('data-session')).toBeUndefined()
  })

  it('会话态 ⇒ sessionId 原样透传给选项行', () => {
    const w = mount(ChatComposer, { props: { sessionId: 's9' } })
    expect(optionBar(w).props('sessionId')).toBe('s9')
    expect(optionBar(w).attributes('data-session')).toBe('s9')
  })
})

describe('ChatComposer：暴露面（父级只认一个 ref）', () => {
  it('text 走 v-model 双向：父级给初值，输入回流父级', async () => {
    let text = '初始'
    const w = mount(ChatComposer, {
      props: { modelValue: text, 'onUpdate:modelValue': (v: string) => (text = v) },
    })
    expect((w.find('textarea').element as HTMLTextAreaElement).value).toBe('初始')

    await w.find('textarea').setValue('改过了')
    expect(text).toBe('改过了')
  })

  it('resetHeight() 委派给输入框（复位自动高度）', () => {
    const w = mount(ChatComposer)
    const ta = w.find('textarea').element as HTMLTextAreaElement
    ta.style.height = '120px'

    ;(w.vm as unknown as { resetHeight: () => void }).resetHeight()

    expect(ta.style.height).toBe('auto')
  })

  it('getDraftMetadata() 委派给选项行（草稿选择缓冲）', () => {
    const w = mount(ChatComposer)
    expect((w.vm as unknown as { getDraftMetadata: () => unknown }).getDraftMetadata()).toEqual({
      probe: 'draft',
    })
  })

  it('autofocus ⇒ 挂载后焦点落在输入框（聚焦由输入框自己管）', async () => {
    // 必须真的挂到 document 上：focus() 对游离节点无效（activeElement 不动）
    const w = mount(ChatComposer, { props: { autofocus: true }, attachTo: document.body })
    await flushPromises()
    await nextTick()
    expect(document.activeElement).toBe(w.find('textarea').element)
    w.unmount()
  })

  it('未开 autofocus ⇒ 不抢焦点', async () => {
    const w = mount(ChatComposer, { attachTo: document.body })
    await flushPromises()
    await nextTick()
    expect(document.activeElement).not.toBe(w.find('textarea').element)
    w.unmount()
  })
})
