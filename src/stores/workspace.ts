import { defineStore } from 'pinia'
import { ref, computed, watch } from 'vue'

export interface Document {
  id: string
  title: string
  content: string
  parentId: string | null
  children: string[]
  order: number // 排序字段
  createdAt: number
  updatedAt: number
}

export interface Execution {
  id: string
  documentId: string
  code: string
  language: string
  status: 'running' | 'success' | 'failed'
  output?: string
  error?: string
  startedAt: number
  finishedAt?: number
}

const STORAGE_KEY = 'symbio-workspace'

export const useWorkspaceStore = defineStore('workspace', () => {
  // 当前工作区 ID
  const currentWorkspaceId = ref<string | null>(null)
  
  // 文档树
  const documents = ref<Map<string, Document>>(new Map())
  
  // 当前活动文档 ID
  const activeDocumentId = ref<string | null>(null)
  
  // 执行记录
  const executions = ref<Map<string, Execution[]>>(new Map())

  // 计算属性：当前文档
  const activeDocument = computed(() => {
    if (!activeDocumentId.value) return null
    return documents.value.get(activeDocumentId.value) || null
  })

  // 计算属性：根文档列表（按 order 排序）
  const rootDocuments = computed(() => {
    const roots: Document[] = []
    documents.value.forEach((doc) => {
      if (doc.parentId === null) {
        roots.push(doc)
      }
    })
    return roots.sort((a, b) => a.order - b.order)
  })

  // 获取子文档列表（按 order 排序）
  function getChildren(parentId: string | null): Document[] {
    const children: Document[] = []
    documents.value.forEach((doc) => {
      if (doc.parentId === parentId) {
        children.push(doc)
      }
    })
    return children.sort((a, b) => a.order - b.order)
  }

  // 创建文档
  function createDocument(title: string, parentId: string | null = null): Document {
    const id = `doc-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`
    const now = Date.now()
    
    // 计算排序值：同级文档的最大 order + 1
    const siblings = getChildren(parentId)
    const maxOrder = siblings.length > 0 ? Math.max(...siblings.map(d => d.order)) : 0
    
    const doc: Document = {
      id,
      title,
      content: '',
      parentId,
      children: [],
      order: maxOrder + 1,
      createdAt: now,
      updatedAt: now,
    }
    documents.value.set(id, doc)
    
    // 更新父文档的 children
    if (parentId) {
      const parent = documents.value.get(parentId)
      if (parent) {
        parent.children.push(id)
      }
    }
    
    saveToStorage()
    return doc
  }

  // 更新文档
  function updateDocument(id: string, updates: Partial<Document>) {
    const doc = documents.value.get(id)
    if (doc) {
      Object.assign(doc, updates, { updatedAt: Date.now() })
      saveToStorage()
    }
  }

  // 删除文档
  function deleteDocument(id: string) {
    const doc = documents.value.get(id)
    if (!doc) return
    
    // 递归删除子文档
    doc.children.forEach((childId) => deleteDocument(childId))
    
    // 从父文档的 children 中移除
    if (doc.parentId) {
      const parent = documents.value.get(doc.parentId)
      if (parent) {
        parent.children = parent.children.filter((cid) => cid !== id)
      }
    }
    
    documents.value.delete(id)
    
    // 如果删除的是当前活动文档，清除选择
    if (activeDocumentId.value === id) {
      activeDocumentId.value = null
    }
    
    saveToStorage()
  }

  // 移动文档（拖拽排序）
  function moveDocument(id: string, newParentId: string | null, newOrder: number) {
    const doc = documents.value.get(id)
    if (!doc) return
    
    const oldParentId = doc.parentId
    
    // 从旧父文档的 children 中移除
    if (oldParentId) {
      const oldParent = documents.value.get(oldParentId)
      if (oldParent) {
        oldParent.children = oldParent.children.filter((cid) => cid !== id)
      }
    }
    
    // 更新文档的父级和排序
    doc.parentId = newParentId
    doc.order = newOrder
    
    // 添加到新父文档的 children
    if (newParentId) {
      const newParent = documents.value.get(newParentId)
      if (newParent) {
        newParent.children.push(id)
      }
    }
    
    // 重新计算同级文档的排序
    reorderSiblings(newParentId)
    
    saveToStorage()
  }

  // 重新排序同级文档
  function reorderSiblings(parentId: string | null) {
    const siblings = getChildren(parentId)
    siblings.forEach((doc, index) => {
      doc.order = index + 1
    })
  }

  // 设置活动文档
  function setActiveDocument(id: string | null) {
    activeDocumentId.value = id
  }

  // 添加执行记录
  function addExecution(execution: Execution) {
    const docExecutions = executions.value.get(execution.documentId) || []
    docExecutions.push(execution)
    executions.value.set(execution.documentId, docExecutions)
  }

  // ========== 持久化 ==========

  // 保存到 localStorage
  function saveToStorage() {
    const data = {
      documents: Array.from(documents.value.entries()),
      activeDocumentId: activeDocumentId.value,
    }
    localStorage.setItem(STORAGE_KEY, JSON.stringify(data))
  }

  // 从 localStorage 加载
  function loadFromStorage() {
    try {
      const stored = localStorage.getItem(STORAGE_KEY)
      if (stored) {
        const data = JSON.parse(stored)
        documents.value = new Map(data.documents)
        if (data.activeDocumentId && documents.value.has(data.activeDocumentId)) {
          activeDocumentId.value = data.activeDocumentId
        }
        return true
      }
    } catch (e) {
      console.error('Failed to load workspace from storage:', e)
    }
    return false
  }

  // 清除存储
  function clearStorage() {
    localStorage.removeItem(STORAGE_KEY)
    documents.value.clear()
    activeDocumentId.value = null
    executions.value.clear()
  }

  // ========== 导入导出 ==========

  // 导出为 JSON
  function exportToJSON(): string {
    const data = {
      version: '1.0',
      exportedAt: Date.now(),
      documents: Array.from(documents.value.values()),
    }
    return JSON.stringify(data, null, 2)
  }

  // 从 JSON 导入
  function importFromJSON(json: string): boolean {
    try {
      const data = JSON.parse(json)
      if (!data.documents || !Array.isArray(data.documents)) {
        throw new Error('Invalid format')
      }
      
      // 清除现有数据
      documents.value.clear()
      
      // 导入文档
      data.documents.forEach((doc: Document) => {
        documents.value.set(doc.id, doc)
      })
      
      saveToStorage()
      return true
    } catch (e) {
      console.error('Failed to import:', e)
      return false
    }
  }

  // 导出单个文档为 Markdown
  function exportDocumentToMarkdown(id: string): string {
    const doc = documents.value.get(id)
    if (!doc) return ''
    
    let markdown = `# ${doc.title}\n\n`
    markdown += doc.content
    
    // 递归导出子文档
    const exportChildren = (parentId: string, level: number) => {
      const children = getChildren(parentId)
      children.forEach(child => {
        markdown += `\n\n${'#'.repeat(level + 1)} ${child.title}\n\n`
        markdown += child.content
        exportChildren(child.id, level + 1)
      })
    }
    
    exportChildren(id, 1)
    return markdown
  }

  // 初始化（从存储加载或创建演示数据）
  function init() {
    if (loadFromStorage()) {
      return // 已从存储加载
    }
    initDemo()
  }

  // 初始化演示数据
  function initDemo() {
    if (documents.value.size > 0) return
    
    const root = createDocument('RNA-seq 差异表达分析', null)
    const design = createDocument('实验设计', root.id)
    const qc = createDocument('数据预处理', root.id)
    const analysis = createDocument('差异分析', root.id)
    
    // 更新内容需要手动调用 updateDocument
    documents.value.get(root.id)!.content = `# RNA-seq 差异表达分析

这是一个完整的 RNA-seq 分析流程示例。

## 分析步骤

1. 实验设计
2. 数据预处理
3. 差异分析
`
    
    documents.value.get(qc.id)!.content = `## FastQC 质控

\`\`\`bash run
fastqc *.fastq.gz -o qc_results
\`\`\`
`
    
    setActiveDocument(root.id)
    saveToStorage()
  }

  // 监听变化自动保存
  watch([documents, activeDocumentId], () => {
    saveToStorage()
  }, { deep: true })

  return {
    // State
    currentWorkspaceId,
    documents,
    activeDocumentId,
    executions,
    
    // Computed
    activeDocument,
    rootDocuments,
    
    // Actions
    createDocument,
    updateDocument,
    deleteDocument,
    moveDocument,
    setActiveDocument,
    addExecution,
    getChildren,
    
    // Persistence
    saveToStorage,
    loadFromStorage,
    clearStorage,
    
    // Import/Export
    exportToJSON,
    importFromJSON,
    exportDocumentToMarkdown,
    
    // Init
    init,
    initDemo,
  }
})