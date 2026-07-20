import { shallowRef, type Ref, type ShallowRef } from 'vue'
import { Editor, editorViewCtx } from '@milkdown/kit/core'
import { setModelContext } from '@/composables/useModelContext'

export interface SelectionInfo {
  text: string
  rect: DOMRect
  startLine?: number
  endLine?: number
}

export function useEditorSelection(
  editor: ShallowRef<Editor | null>,
  filePath: Ref<string | undefined>,
  modelValue: Ref<string>
) {
  const savedSelection = shallowRef<SelectionInfo | null>(null)

  function updateModelContextFromSelection(ctx: any) {
    if (!editor.value || !filePath.value) return

    try {
      const view = ctx.get(editorViewCtx)
      const { state } = view
      const { from, to } = state.selection

      if (from !== to) {
        const selectedTextFromDoc = state.doc.textBetween(from, to, '\n')
        const textBefore = state.doc.textBetween(0, from, '\n')
        const linesBefore = textBefore.split('\n').length
        const selectedLines = selectedTextFromDoc.split('\n').length

        setModelContext({
          filePath: filePath.value,
          fileContent: modelValue.value,
          selectedText: selectedTextFromDoc.trim() || undefined,
          startLine: linesBefore,
          endLine: linesBefore + selectedLines - 1,
        })

        const range = document.getSelection()?.getRangeAt(0)
        const rect = range?.getBoundingClientRect() || { left: 0, top: 0, width: 0, height: 0 } as DOMRect
        savedSelection.value = {
          text: selectedTextFromDoc.trim(),
          rect,
          startLine: linesBefore,
          endLine: linesBefore + selectedLines - 1,
        }
      } else {
        const textBefore = state.doc.textBetween(0, from, '\n')
        const currentLine = textBefore.split('\n').length

        setModelContext({
          filePath: filePath.value,
          fileContent: modelValue.value,
          selectedText: undefined,
          startLine: currentLine,
          endLine: currentLine,
        })
        savedSelection.value = null
      }
    } catch (e) {
      // ignore
    }
  }

  function handleEditorMouseUp() {
    setTimeout(() => {
      try {
        const selection = window.getSelection()
        if (selection && !selection.isCollapsed) {
          const text = selection.toString().trim()
          if (text.length > 0) {
            const range = selection.getRangeAt(0)
            const rect = range.getBoundingClientRect()
            let startLine: number | undefined
            let endLine: number | undefined
            let selectedContent = text

            if (editor.value && editor.value.ctx) {
              const view = editor.value.ctx.get(editorViewCtx)
              const { state } = view
              const { from, to } = state.selection
              if (from !== to) {
                const textBefore = state.doc.textBetween(0, from, '\n')
                const selectedTextFromDoc = state.doc.textBetween(from, to, '\n')
                if (selectedTextFromDoc.trim().length > 0) selectedContent = selectedTextFromDoc.trim()
                const linesBefore = textBefore.split('\n').length
                startLine = linesBefore
                const selectedLines = selectedContent.split('\n').length
                endLine = linesBefore + selectedLines - 1
              }
            }

            if (startLine === undefined && modelValue.value) {
              const cleanSelected = selectedContent.replace(/\s+/g, ' ').trim()
              const cleanMarkdown = modelValue.value.replace(/\s+/g, ' ')
              const startIndex = cleanMarkdown.indexOf(cleanSelected)
              if (startIndex !== -1) {
                const beforeStart = modelValue.value.substring(0, startIndex)
                startLine = (beforeStart.match(/\n/g) || []).length + 1
                const endIndex = startIndex + selectedContent.length
                const beforeEnd = modelValue.value.substring(0, endIndex)
                endLine = (beforeEnd.match(/\n/g) || []).length + 1
              }
            }

            savedSelection.value = { text: selectedContent, rect, startLine, endLine }
            setModelContext({
              filePath: filePath.value,
              fileContent: modelValue.value,
              selectedText: selectedContent,
              startLine,
              endLine,
            })
          }
        } else {
          savedSelection.value = null
          setModelContext({
            filePath: filePath.value,
            fileContent: modelValue.value,
            selectedText: undefined,
          })
        }
      } catch (e) {
        // ignore
      }
    }, 10)
  }

  return {
    savedSelection,
    updateModelContextFromSelection,
    handleEditorMouseUp
  }
}
