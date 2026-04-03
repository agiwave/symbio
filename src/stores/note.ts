/**
 * 笔记存储
 *
 * 通过后端 note 插件管理笔记
 */

import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { callPlugin } from '@/services/plugin'

export interface Note {
  id: string
  title: string
  content: string
  parentId: string | null
  children?: string[]
}

export const useNoteStore = defineStore('note', () => {
  // 笔记树
  const notes = ref<Map<string, Note>>(new Map())

  // 当前活动笔记 ID
  const activeNoteId = ref<string | null>(null)

  // 是否已初始化
  const initialized = ref(false)

  // 计算属性：当前笔记
  const activeNote = computed(() => {
    if (!activeNoteId.value) return null
    return notes.value.get(activeNoteId.value) || null
  })

  // 计算属性：根笔记列表
  const rootNotes = computed(() => {
    const roots: Note[] = []
    notes.value.forEach((note) => {
      if (note.parentId === null) {
        roots.push(note)
      }
    })
    return roots
  })

  // 获取子笔记列表
  function getChildren(parentId: string | null): Note[] {
    const children: Note[] = []
    notes.value.forEach((note) => {
      if (note.parentId === parentId) {
        children.push(note)
      }
    })
    return children
  }

  // 初始化 - 调用后端初始化并加载数据
  async function init() {
    if (initialized.value) return
    
    try {
      // 先调用后端初始化
      await callPlugin('note', { action: 'init' })
      
      // 然后加载笔记列表
      await loadNotes()
      
      initialized.value = true
    } catch (e) {
      console.error('Failed to initialize notes:', e)
    }
  }

  // 加载笔记列表
  async function loadNotes() {
    try {
      const result = await callPlugin<{ documents: Note[] }>('note', {
        action: 'list'
      })
      notes.value.clear()
      result.documents.forEach((note) => {
        notes.value.set(note.id, note)
      })
    } catch (e) {
      console.error('Failed to load notes:', e)
    }
  }

  // 创建笔记
  async function createNote(title: string, parentId: string | null = null): Promise<Note | null> {
    try {
      const result = await callPlugin<Note>('note', {
        action: 'create',
        title,
        parentId
      })
      notes.value.set(result.id, result)

      // 更新父笔记的 children
      if (parentId) {
        const parent = notes.value.get(parentId)
        if (parent) {
          parent.children = parent.children || []
          if (!parent.children.includes(result.id)) {
            parent.children.push(result.id)
          }
        }
      }

      return result
    } catch (e) {
      console.error('Failed to create note:', e)
      return null
    }
  }

  // 获取笔记详情
  async function getNote(id: string): Promise<Note | null> {
    try {
      const result = await callPlugin<Note>('note', {
        action: 'get',
        id
      })
      notes.value.set(result.id, result)
      return result
    } catch (e) {
      console.error('Failed to get note:', e)
      return null
    }
  }

  // 更新笔记
  async function updateNote(id: string, updates: Partial<Note>): Promise<boolean> {
    try {
      await callPlugin('note', {
        action: 'update',
        id,
        ...updates
      })
      const note = notes.value.get(id)
      if (note) {
        Object.assign(note, updates)
      }
      return true
    } catch (e) {
      console.error('Failed to update note:', e)
      return false
    }
  }

  // 删除笔记
  async function deleteNote(id: string): Promise<boolean> {
    try {
      await callPlugin('note', {
        action: 'delete',
        id
      })

      const note = notes.value.get(id)
      if (note?.parentId) {
        const parent = notes.value.get(note.parentId)
        if (parent?.children) {
          parent.children = parent.children.filter((cid) => cid !== id)
        }
      }

      // 递归删除子笔记
      function removeChildren(noteId: string) {
        const n = notes.value.get(noteId)
        if (n?.children) {
          n.children.forEach(removeChildren)
        }
        notes.value.delete(noteId)
      }
      removeChildren(id)

      if (activeNoteId.value === id) {
        activeNoteId.value = null
      }

      return true
    } catch (e) {
      console.error('Failed to delete note:', e)
      return false
    }
  }

  // 移动笔记
  async function moveNote(id: string, newParentId: string | null, _newOrder: number): Promise<boolean> {
    try {
      const note = notes.value.get(id)
      if (!note) return false

      const oldParentId = note.parentId

      // 更新父笔记的 children
      if (oldParentId) {
        const oldParent = notes.value.get(oldParentId)
        if (oldParent?.children) {
          oldParent.children = oldParent.children.filter((cid) => cid !== id)
        }
      }

      if (newParentId) {
        const newParent = notes.value.get(newParentId)
        if (newParent) {
          newParent.children = newParent.children || []
          if (!newParent.children.includes(id)) {
            newParent.children.push(id)
          }
        }
      }

      // 更新笔记
      await callPlugin('note', {
        action: 'update',
        id,
        parentId: newParentId
      })
      
      note.parentId = newParentId
      
      return true
    } catch (e) {
      console.error('Failed to move note:', e)
      return false
    }
  }

  // 设置活动笔记
  function setActiveNote(id: string | null) {
    activeNoteId.value = id
    // 加载笔记详情
    if (id) {
      getNote(id)
    }
  }

  // 导出为 JSON
  function exportToJSON(): string {
    const data = {
      version: '1.0',
      exportedAt: new Date().toISOString(),
      notes: Array.from(notes.value.values()),
    }
    return JSON.stringify(data, null, 2)
  }

  // 清空存储
  async function clearStorage() {
    try {
      // 删除所有根笔记
      const rootIds = rootNotes.value.map(n => n.id)
      for (const id of rootIds) {
        await deleteNote(id)
      }
      notes.value.clear()
      activeNoteId.value = null
    } catch (e) {
      console.error('Failed to clear storage:', e)
    }
  }

  return {
    // State
    notes,
    activeNoteId,
    initialized,

    // Computed
    activeNote,
    rootNotes,

    // Actions
    init,
    loadNotes,
    createNote,
    getNote,
    updateNote,
    deleteNote,
    moveNote,
    setActiveNote,
    getChildren,

    // Import/Export
    exportToJSON,
    clearStorage,
  }
})