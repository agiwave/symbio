import type { Directive } from 'vue'

let nextScrollbarId = 0
const cleanups = new WeakMap<HTMLElement, () => void>()
const MIN_THUMB_HEIGHT = 24
const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [contenteditable="true"], [tabindex]:not([tabindex="-1"])'

function hasKeyboardTarget(el: HTMLElement): boolean {
  return el.tabIndex >= 0 || Array.from(el.querySelectorAll<HTMLElement>(FOCUSABLE))
    .some((target) => !target.closest('.overlay-scrollbar-rail'))
}

function detach(el: HTMLElement): void {
  cleanups.get(el)?.()
  cleanups.delete(el)
}

function attach(el: HTMLElement): void {
  detach(el)
  el.classList.add('overlay-scrollbar-host')
  if (!el.id) el.id = `overlay-scroll-${++nextScrollbarId}`

  const rail = document.createElement('div')
  rail.className = 'overlay-scrollbar-rail'
  rail.hidden = true
  const thumb = document.createElement('div')
  thumb.className = 'overlay-scrollbar-thumb'
  thumb.setAttribute('role', 'scrollbar')
  thumb.setAttribute('aria-orientation', 'vertical')
  thumb.setAttribute('aria-controls', el.id)
  thumb.setAttribute('aria-valuemin', '0')
  thumb.setAttribute('aria-valuemax', '100')
  thumb.tabIndex = -1
  rail.appendChild(thumb)
  el.appendChild(rail)

  const metrics = () => {
    const viewport = el.clientHeight
    const scrollMax = Math.max(0, el.scrollHeight - viewport)
    const thumbHeight = Math.min(
      viewport,
      Math.max(MIN_THUMB_HEIGHT, viewport * (viewport / Math.max(1, el.scrollHeight))),
    )
    return {
      viewport,
      scrollMax,
      thumbHeight,
      travel: Math.max(0, viewport - thumbHeight),
    }
  }

  const update = () => {
    const { viewport, scrollMax, thumbHeight, travel } = metrics()
    const overflowing = scrollMax > 1
    rail.hidden = !overflowing
    const tabIndex = overflowing && !hasKeyboardTarget(el) ? 0 : -1
    if (thumb.tabIndex !== tabIndex) thumb.tabIndex = tabIndex
    if (!overflowing) return

    rail.style.top = `${el.scrollTop}px`
    rail.style.height = `${viewport}px`
    thumb.style.height = `${thumbHeight}px`
    const offset = travel > 0 ? (el.scrollTop / scrollMax) * travel : 0
    thumb.style.transform = `translateY(${offset}px)`
    thumb.setAttribute('aria-valuenow', String(Math.round((el.scrollTop / scrollMax) * 100)))
  }

  let frame: number | null = null
  const scheduleUpdate = () => {
    if (frame !== null) return
    frame = requestAnimationFrame(() => {
      frame = null
      update()
    })
  }

  let dragging = false
  let dragStartY = 0
  let dragStartTop = 0

  const onPointerMove = (event: PointerEvent) => {
    if (!dragging) return
    event.preventDefault()
    const { scrollMax, travel } = metrics()
    if (travel <= 0) return
    const next = dragStartTop + ((event.clientY - dragStartY) / travel) * scrollMax
    el.scrollTop = Math.min(scrollMax, Math.max(0, next))
    update()
  }

  const onPointerUp = () => {
    if (!dragging) return
    dragging = false
    rail.classList.remove('is-dragging')
    window.removeEventListener('pointermove', onPointerMove)
    window.removeEventListener('pointerup', onPointerUp)
    window.removeEventListener('pointercancel', onPointerUp)
  }

  const onPointerDown = (event: PointerEvent) => {
    if (event.pointerType === 'mouse' && event.button !== 0) return
    event.preventDefault()
    const { scrollMax, thumbHeight, travel } = metrics()
    if (scrollMax <= 0) return

    if (event.target === rail && travel > 0) {
      const top = rail.getBoundingClientRect().top
      const offset = Math.min(travel, Math.max(0, event.clientY - top - thumbHeight / 2))
      el.scrollTop = (offset / travel) * scrollMax
      update()
    }

    dragging = true
    dragStartY = event.clientY
    dragStartTop = el.scrollTop
    rail.classList.add('is-dragging')
    thumb.focus({ preventScroll: true })
    window.addEventListener('pointermove', onPointerMove)
    window.addEventListener('pointerup', onPointerUp)
    window.addEventListener('pointercancel', onPointerUp)
  }

  const onThumbKeydown = (event: KeyboardEvent) => {
    const { scrollMax, viewport } = metrics()
    let next = el.scrollTop
    if (event.key === 'ArrowDown') next += 40
    else if (event.key === 'ArrowUp') next -= 40
    else if (event.key === 'PageDown') next += viewport
    else if (event.key === 'PageUp') next -= viewport
    else if (event.key === 'Home') next = 0
    else if (event.key === 'End') next = scrollMax
    else return
    event.preventDefault()
    el.scrollTop = Math.min(scrollMax, Math.max(0, next))
    update()
  }

  rail.addEventListener('pointerdown', onPointerDown)
  thumb.addEventListener('keydown', onThumbKeydown)
  el.addEventListener('scroll', update, { passive: true })

  const resizeObserver =
    typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(scheduleUpdate)
  resizeObserver?.observe(el)
  const mutationObserver = new MutationObserver(scheduleUpdate)
  mutationObserver.observe(el, {
    childList: true,
    subtree: true,
    characterData: true,
    attributes: true,
    attributeFilter: ['tabindex', 'disabled', 'contenteditable'],
  })

  cleanups.set(el, () => {
    onPointerUp()
    if (frame !== null) cancelAnimationFrame(frame)
    resizeObserver?.disconnect()
    mutationObserver.disconnect()
    rail.removeEventListener('pointerdown', onPointerDown)
    thumb.removeEventListener('keydown', onThumbKeydown)
    el.removeEventListener('scroll', update)
    rail.remove()
    el.classList.remove('overlay-scrollbar-host')
  })

  update()
}

export const vOverlayScrollbar: Directive<HTMLElement, boolean | undefined> = {
  mounted(el, binding) {
    if (binding.value !== false) attach(el)
  },
  updated(el, binding) {
    if (binding.value === false) detach(el)
    else if (!cleanups.has(el)) attach(el)
  },
  unmounted(el) {
    detach(el)
  },
}
