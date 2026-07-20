import { ref } from 'vue'

export function useResizable(initialWidth: number, options: { minWidth?: number; maxWidth?: number; direction?: 'ltr' | 'rtl' } = {}) {
  const width = ref(initialWidth)
  const isResizing = ref(false)
  const startWidth = ref(0)
  const startX = ref(0)

  const { minWidth = 100, maxWidth = 1000, direction = 'rtl' } = options

  function startResize(e: MouseEvent) {
    e.preventDefault()
    isResizing.value = true
    startWidth.value = width.value
    startX.value = e.clientX

    document.addEventListener('mousemove', doResize)
    document.addEventListener('mouseup', stopResize)
  }

  function doResize(e: MouseEvent) {
    if (!isResizing.value) return
    
    const delta = direction === 'rtl' ? startX.value - e.clientX : e.clientX - startX.value
    const newWidth = startWidth.value + delta
    
    width.value = Math.max(minWidth, Math.min(maxWidth, newWidth))
  }

  function stopResize() {
    isResizing.value = false
    document.removeEventListener('mousemove', doResize)
    document.removeEventListener('mouseup', stopResize)
  }

  return {
    width,
    isResizing,
    startResize
  }
}
