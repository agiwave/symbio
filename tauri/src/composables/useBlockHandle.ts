import { reactive, ref, type Ref, type ShallowRef } from 'vue'
import { Editor, editorViewCtx } from '@milkdown/kit/core'

export function useBlockHandle(
  editor: ShallowRef<Editor | null>,
  editorRef: Ref<HTMLElement | null>,
  isDragging: Ref<boolean>
) {
  const blockHandle = reactive({
    visible: false,
    top: 0,
    left: 0,
    activeNode: null as any,
    activePos: 0,
  })

  const showToolbar = ref(false)
  let expandTimer: ReturnType<typeof setTimeout> | null = null
  let collapseTimer: ReturnType<typeof setTimeout> | null = null

  function updateBlockHandle() {
    if (isDragging.value || !editor.value || !editorRef.value) return
    
    try {
      const view = editor.value.ctx.get(editorViewCtx)
      const { selection, doc } = view.state
      const { $from } = selection
      
      let depth = $from.depth
      let node = $from.node(depth)
      
      while (depth > 0 && node.isText) {
        depth--
        node = $from.node(depth)
      }
      
      if (doc.content.size <= 2) {
        blockHandle.visible = false
        showToolbar.value = false
        return
      }
      
      const pos = $from.before(depth)
      const dom = view.nodeDOM(pos)
      
      if (dom && dom instanceof HTMLElement) {
        const rect = dom.getBoundingClientRect()
        const editorRect = editorRef.value.getBoundingClientRect()
        blockHandle.visible = true
        blockHandle.top = rect.top
        blockHandle.left = editorRect.left + 8
        blockHandle.activeNode = { node, pos, el: dom }
        blockHandle.activePos = pos
      } else {
        blockHandle.visible = false
        showToolbar.value = false
      }
    } catch (e) {
      // ignore
    }
  }

  function handleMouseEnter() {
    if (collapseTimer) {
      clearTimeout(collapseTimer)
      collapseTimer = null
    }
    if (!showToolbar.value && !isDragging.value) {
      if (expandTimer) clearTimeout(expandTimer)
      expandTimer = setTimeout(() => { showToolbar.value = true }, 100)
    }
  }

  function handleMouseLeave() {
    if (expandTimer) {
      clearTimeout(expandTimer)
      expandTimer = null
    }
    if (isDragging.value) return
    if (showToolbar.value) {
      collapseTimer = setTimeout(() => {
        if (!isDragging.value) showToolbar.value = false
      }, 300)
    }
  }

  return {
    blockHandle,
    showToolbar,
    updateBlockHandle,
    handleMouseEnter,
    handleMouseLeave,
    clearTimers: () => {
      if (expandTimer) clearTimeout(expandTimer)
      if (collapseTimer) clearTimeout(collapseTimer)
    }
  }
}
