/**
 * 工作区存储
 *
 * 通过后端 work 插件管理文档
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

  // 加载文档列表
  async function loadDocuments() {
    try {
      const result = await callPlugin<{ documents: Document[] }>('work', {
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
      const result = await callPlugin<Document>('work', {
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
          parent.children.push(result.id)
        }
      }

      return result
    } catch (e) {
      console.error('Failed to create document:', e)
      return null
    }
  }

  // 获取文档
  async function getDocument(id: string): Promise<Document | null> {
    try {
      const result = await callPlugin<Document>('work', {
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
      await callPlugin('work', {
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
      await callPlugin('work', {
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

      documents.value.delete(id)

      if (activeDocumentId.value === id) {
        activeDocumentId.value = null
      }

      return true
    } catch (e) {
      console.error('Failed to delete document:', e)
      return false
    }
  }

  // 设置活动文档
  function setActiveDocument(id: string | null) {
    activeDocumentId.value = id
  }

  // 初始化
  async function init() {
    await loadDocuments()
  }

  return {
    // State
    documents,
    activeDocumentId,

    // Computed
    activeDocument,
    rootDocuments,

    // Actions
    loadDocuments,
    createDocument,
    getDocument,
    updateDocument,
    deleteDocument,
    setActiveDocument,
    getChildren,

    // Init
    init,
  }
})
