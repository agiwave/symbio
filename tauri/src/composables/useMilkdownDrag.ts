import { ref, type Ref, type ShallowRef } from 'vue'
import { editorViewCtx } from '@milkdown/kit/core'
import { Editor } from '@milkdown/kit/core'
import { logger } from '@/utils/logger'

export function useEditorDrag(
  editor: ShallowRef<Editor | null>,
  editorRef: Ref<HTMLElement | null>,
  activePos: Ref<number>,
  onDragStateChange: (isDragging: boolean) => void
) {
  const isDragging = ref(false)
  const dragSourcePos = ref<number | null>(null)
  const dragTargetPos = ref<number | null>(null)
  const dragInsertBefore = ref(true)

  const dropIndicator = ref({
    visible: false,
    top: 0,
    left: 0,
    width: 0,
  })

  function handleDragMouseDown(e: MouseEvent) {
    e.preventDefault()
    e.stopPropagation()
    
    const sourcePos = activePos.value
    if (sourcePos === undefined || sourcePos === null) return
    
    isDragging.value = true
    onDragStateChange(true)
    dragSourcePos.value = sourcePos
    dragTargetPos.value = null
    
    document.addEventListener('mousemove', handleDragMouseMove)
    document.addEventListener('mouseup', handleDragMouseUp)
  }

  function handleDragMouseMove(e: MouseEvent) {
    if (!isDragging.value || !editor.value || !editorRef.value) return
    
    try {
      const view = editor.value.ctx.get(editorViewCtx)
      const editorRect = editorRef.value.getBoundingClientRect()
      
      if (e.clientX < editorRect.left || e.clientX > editorRect.right ||
          e.clientY < editorRect.top || e.clientY > editorRect.bottom) {
        dropIndicator.value.visible = false
        dragTargetPos.value = null
        return
      }
      
      const posAtCoords = view.posAtCoords({ left: e.clientX, top: e.clientY })
      
      if (posAtCoords) {
        const $pos = view.state.doc.resolve(posAtCoords.pos)
        let depth = $pos.depth
        let node = $pos.node(depth)
        
        while (depth > 0) {
          node = $pos.node(depth)
          if (node.isBlock) break
          depth--
        }
        
        const blockPos = $pos.before(depth)
        const dom = view.nodeDOM(blockPos)
        
        if (dom && dom instanceof HTMLElement) {
          const domRect = dom.getBoundingClientRect()
          const midY = domRect.top + domRect.height / 2
          const insertBefore = e.clientY < midY
          
          dragTargetPos.value = blockPos
          dragInsertBefore.value = insertBefore
          
          dropIndicator.value.visible = true
          dropIndicator.value.left = domRect.left
          dropIndicator.value.width = domRect.width
          dropIndicator.value.top = insertBefore ? domRect.top - 1 : domRect.bottom - 1
        }
      } else {
        dropIndicator.value.visible = false
        dragTargetPos.value = null
      }
    } catch (err) {
      logger.error('Drag', 'Move error', err)
    }
  }

  function handleDragMouseUp(_e: MouseEvent) {
    document.removeEventListener('mousemove', handleDragMouseMove)
    document.removeEventListener('mouseup', handleDragMouseUp)
    
    dropIndicator.value.visible = false
    
    if (!isDragging.value || dragSourcePos.value === null || !editor.value) {
      isDragging.value = false
      onDragStateChange(false)
      dragSourcePos.value = null
      dragTargetPos.value = null
      return
    }
    
    const sourcePos = dragSourcePos.value
    const targetPos = dragTargetPos.value
    
    if (targetPos !== null && targetPos !== sourcePos) {
      try {
        const view = editor.value.ctx.get(editorViewCtx)
        const state = view.state
        const sourceNode = state.doc.nodeAt(sourcePos)
        
        if (sourceNode) {
          let tr = state.tr
          const nodeSize = sourceNode.nodeSize
          let insertPos = targetPos
          
          if (!dragInsertBefore.value) {
            const targetNode = state.doc.nodeAt(targetPos)
            if (targetNode) insertPos = targetPos + targetNode.nodeSize
          }
          
          if (sourcePos < insertPos) {
            tr = tr.delete(sourcePos, sourcePos + nodeSize)
            insertPos -= nodeSize
            tr = tr.insert(insertPos, sourceNode)
          } else {
            tr = tr.insert(insertPos, sourceNode)
            tr = tr.delete(sourcePos + nodeSize, sourcePos + nodeSize + nodeSize)
          }
          view.dispatch(tr)
        }
      } catch (err) {
        logger.error('Drag', 'Drop error', err)
      }
    }
    
    isDragging.value = false
    onDragStateChange(false)
    dragSourcePos.value = null
    dragTargetPos.value = null
  }

  return {
    isDragging,
    dropIndicator,
    handleDragMouseDown
  }
}
