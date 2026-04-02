/**
 * 工作区存储
 *
 * 通过后端 doc 插件管理文档
 */

import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { callPlugin } from '@/services/plugin'

export interface Document {
  id: string
  title: string
  content: string
  parentId: string | null
  children?: string[]
}

export const useWorkspaceStore = defineStore('workspace', () => {
  // 文档树
  const documents = ref<Map<string, Document>>(new Map())

  // 当前活动文档 ID
  const activeDocumentId = ref<string | null>(null)

  // 是否已初始化
  const initialized = ref(false)

  // 计算属性：当前文档
  const activeDocument = computed(() => {
    if (!activeDocumentId.value) return null
    return documents.value.get(activeDocumentId.value) || null
  })

  // 计算属性：根文档列表
  const rootDocuments = computed(() => {
    const roots: Document[] = []
    documents.value.forEach((doc) => {
      if (doc.parentId === null) {
        roots.push(doc)
      }
    })
    return roots
  })

  // 获取子文档列表
  function getChildren(parentId: string | null): Document[] {
    const children: Document[] = []
    documents.value.forEach((doc) => {
      if (doc.parentId === parentId) {
        children.push(doc)
      }
    })
    return children
  }

  // 初始化 - 调用后端初始化并加载数据
  async function init() {
    if (initialized.value) return
    
    try {
      // 先调用后端初始化
      await callPlugin('doc', { action: 'init' })
      
      // 然后加载文档列表
      await loadDocuments()
      
      initialized.value = true
    } catch (e) {
      console.error('Failed to initialize workspace:', e)
    }
  }

  // 加载文档列表
  async function loadDocuments() {
    try {
      const result = await callPlugin<{ documents: Document[] }>('doc', {
        action: 'list'
      })
      documents.value.clear()
      result.documents.forEach((doc) => {
        documents.value.set(doc.id, doc)
      })
    } catch (e) {
      console.error('Failed to load documents:', e)
    }
  }

  // 创建文档
  async function createDocument(title: string, parentId: string | null = null): Promise<Document | null> {
    try {
      const result = await callPlugin<Document>('doc', {
        action: 'create',
        title,
        parentId
      })
      documents.value.set(result.id, result)

      // 更新父文档的 children
      if (parentId) {
        const parent = documents.value.get(parentId)
        if (parent) {
          parent.children = parent.children || []
          if (!parent.children.includes(result.id)) {
            parent.children.push(result.id)
          }
        }
      }

      return result
    } catch (e) {
      console.error('Failed to create document:', e)
      return null
    }
  }

  // 获取文档详情
  async function getDocument(id: string): Promise<Document | null> {
    try {
      const result = await callPlugin<Document>('doc', {
        action: 'get',
        id
      })
      documents.value.set(result.id, result)
      return result
    } catch (e) {
      console.error('Failed to get document:', e)
      return null
    }
  }

  // 更新文档
  async function updateDocument(id: string, updates: Partial<Document>): Promise<boolean> {
    try {
      await callPlugin('doc', {
        action: 'update',
        id,
        ...updates
      })
      const doc = documents.value.get(id)
      if (doc) {
        Object.assign(doc, updates)
      }
      return true
    } catch (e) {
      console.error('Failed to update document:', e)
      return false
    }
  }

  // 删除文档
  async function deleteDocument(id: string): Promise<boolean> {
    try {
      await callPlugin('doc', {
        action: 'delete',
        id
      })

      const doc = documents.value.get(id)
      if (doc?.parentId) {
        const parent = documents.value.get(doc.parentId)
        if (parent?.children) {
          parent.children = parent.children.filter((cid) => cid !== id)
        }
      }

      // 递归删除子文档
      function removeChildren(docId: string) {
        const d = documents.value.get(docId)
        if (d?.children) {
          d.children.forEach(removeChildren)
        }
        documents.value.delete(docId)
      }
      removeChildren(id)

      if (activeDocumentId.value === id) {
        activeDocumentId.value = null
      }

      return true
    } catch (e) {
      console.error('Failed to delete document:', e)
      return false
    }
  }

  // 移动文档（暂简单实现）
  async function moveDocument(id: string, newParentId: string | null, _newOrder: number): Promise<boolean> {
    try {
      const doc = documents.value.get(id)
      if (!doc) return false

      const oldParentId = doc.parentId

      // 更新父文档的 children
      if (oldParentId) {
        const oldParent = documents.value.get(oldParentId)
        if (oldParent?.children) {
          oldParent.children = oldParent.children.filter((cid) => cid !== id)
        }
      }

      if (newParentId) {
        const newParent = documents.value.get(newParentId)
        if (newParent) {
          newParent.children = newParent.children || []
          if (!newParent.children.includes(id)) {
            newParent.children.push(id)
          }
        }
      }

      // 更新文档
      await callPlugin('doc', {
        action: 'update',
        id,
        parentId: newParentId
      })
      
      doc.parentId = newParentId
      
      return true
    } catch (e) {
      console.error('Failed to move document:', e)
      return false
    }
  }

  // 设置活动文档
  function setActiveDocument(id: string | null) {
    activeDocumentId.value = id
    // 加载文档详情
    if (id) {
      getDocument(id)
    }
  }

  // 导出为 JSON
  function exportToJSON(): string {
    const data = {
      version: '1.0',
      exportedAt: new Date().toISOString(),
      documents: Array.from(documents.value.values()),
    }
    return JSON.stringify(data, null, 2)
  }

  // 清空存储
  async function clearStorage() {
    try {
      // 删除所有根文档
      const rootIds = rootDocuments.value.map(d => d.id)
      for (const id of rootIds) {
        await deleteDocument(id)
      }
      documents.value.clear()
      activeDocumentId.value = null
    } catch (e) {
      console.error('Failed to clear storage:', e)
    }
  }

  return {
    // State
    documents,
    activeDocumentId,
    initialized,

    // Computed
    activeDocument,
    rootDocuments,

    // Actions
    init,
    loadDocuments,
    createDocument,
    getDocument,
    updateDocument,
    deleteDocument,
    moveDocument,
    setActiveDocument,
    getChildren,

    // Import/Export
    exportToJSON,
    clearStorage,
  }
})
