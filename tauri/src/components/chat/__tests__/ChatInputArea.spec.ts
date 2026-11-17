// @vitest-environment happy-dom
/**
 * ChatInputArea — 输入区的两格反馈
 *
 * 钉住两条「界面必须说出来」的事实：
 * 1. 有内容时报字数（长文作者要看得见规模；此前完全没有反馈）；
 * 2. 运行中回车是**换行**不是发送——提示行必须写明，否则用户以为发送失灵。
 */
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import ChatInputArea from '../ChatInputArea.vue'

describe('ChatInputArea：提示行', () => {
  it('有内容时显示字数（按用户可见字符计，emoji 不算两个字）', () => {
    const w = mount(ChatInputArea, { props: { isLoading: false, modelValue: '你好🙂' } })
    expect(w.find('.input-count').text()).toContain('3 字')
  })

  it('空输入不显示计数，只显示键盘说明', () => {
    const w = mount(ChatInputArea, { props: { isLoading: false, modelValue: '' } })
    expect(w.find('.input-count').exists()).toBe(false)
    expect(w.find('.input-note').text()).toContain('Enter 发送')
  })

  it('运行中提示回车是换行（不发送）', () => {
    const w = mount(ChatInputArea, { props: { isLoading: true, modelValue: '' } })
    expect(w.find('.input-note').text()).toContain('生成中')
    expect(w.find('.input-note').text()).toContain('Enter 换行')
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
