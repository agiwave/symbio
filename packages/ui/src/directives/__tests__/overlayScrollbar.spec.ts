// @vitest-environment happy-dom
import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import { defineComponent } from 'vue'
import { vOverlayScrollbar } from '../overlayScrollbar'

function mountScroller(scrollHeight = 1000, viewportHeight = 200) {
  const wrapper = mount(
    defineComponent({
      directives: { overlayScrollbar: vOverlayScrollbar },
      template: '<div id="scroller" v-overlay-scrollbar class="scroller">内容</div>',
    }),
  )
  const el = wrapper.element as HTMLElement
  Object.defineProperty(el, 'clientHeight', { configurable: true, value: viewportHeight })
  Object.defineProperty(el, 'scrollHeight', { configurable: true, value: scrollHeight })
  let scrollTop = 0
  Object.defineProperty(el, 'scrollTop', {
    configurable: true,
    get: () => scrollTop,
    set: (value: number) => {
      scrollTop = value
    },
  })
  el.dispatchEvent(new Event('scroll'))
  return { wrapper, el }
}

describe('v-overlay-scrollbar', () => {
  it('无溢出时隐藏滚动条，并移出键盘顺序', () => {
    const { el } = mountScroller(180, 200)
    const rail = el.querySelector<HTMLElement>('.overlay-scrollbar-rail')!
    const thumb = rail.querySelector<HTMLElement>('[role="scrollbar"]')!
    expect(rail.hidden).toBe(true)
    expect(thumb.tabIndex).toBe(-1)
  })

  it('有溢出时提供命名滚动条，且轨道固定在可视区顶部', () => {
    const { el } = mountScroller()
    el.scrollTop = 300
    el.dispatchEvent(new Event('scroll'))
    const rail = el.querySelector<HTMLElement>('.overlay-scrollbar-rail')!
    const thumb = rail.querySelector<HTMLElement>('[role="scrollbar"]')!
    expect(rail.hidden).toBe(false)
    expect(rail.style.top).toBe('300px')
    expect(rail.style.height).toBe('200px')
    expect(thumb.style.height).toBe('40px')
    expect(thumb.tabIndex).toBe(0)
    expect(thumb.getAttribute('aria-controls')).toBe(el.id)
    expect(thumb.getAttribute('aria-valuenow')).toBe('38')
  })

  it('宿主已有键盘目标时，滚动条不额外占用 Tab 顺序', () => {
    const { el } = mountScroller()
    el.appendChild(document.createElement('button'))
    el.dispatchEvent(new Event('scroll'))
    const thumb = el.querySelector<HTMLElement>('[role="scrollbar"]')!
    expect(thumb.tabIndex).toBe(-1)
  })

  it('支持方向键、PageUp/PageDown 与 Home/End', () => {
    const { el } = mountScroller()
    const thumb = el.querySelector<HTMLElement>('[role="scrollbar"]')!
    thumb.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }))
    expect(el.scrollTop).toBe(40)
    thumb.dispatchEvent(new KeyboardEvent('keydown', { key: 'PageDown', bubbles: true }))
    expect(el.scrollTop).toBe(240)
    thumb.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true }))
    expect(el.scrollTop).toBe(800)
    thumb.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }))
    expect(el.scrollTop).toBe(0)
  })

  it('点击轨道与拖动滑块都会更新宿主滚动位置', () => {
    const { el } = mountScroller()
    const rail = el.querySelector<HTMLElement>('.overlay-scrollbar-rail')!
    const thumb = rail.querySelector<HTMLElement>('[role="scrollbar"]')!
    rail.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, clientY: 120 }))
    expect(el.scrollTop).toBe(500)
    rail.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, clientY: 20 }))
    window.dispatchEvent(new PointerEvent('pointermove', { clientY: 60 }))
    expect(el.scrollTop).toBe(200)
    window.dispatchEvent(new PointerEvent('pointerup'))
    expect(rail.classList.contains('is-dragging')).toBe(false)
    expect(thumb.getAttribute('aria-valuenow')).toBe('25')
  })

  it('卸载后移除增强节点与宿主标记', () => {
    const { wrapper, el } = mountScroller()
    wrapper.unmount()
    expect(el.querySelector('.overlay-scrollbar-rail')).toBeNull()
    expect(el.classList.contains('overlay-scrollbar-host')).toBe(false)
  })
})
