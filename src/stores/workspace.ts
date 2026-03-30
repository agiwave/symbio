import { defineStore } from 'pinia'
import { ref, computed } from 'vue'

export interface Document {
  id: string
  title: string
  content: string
  parentId: string | null
  children: string[]
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

  // 计算属性：根文档列表
  const rootDocuments = computed(() => {
    const roots: Document[] = []
    documents.value.forEach((doc) => {
      if (doc.parentId === null) {
        roots.push(doc)
      }
    })
    return roots.sort((a, b) => a.createdAt - b.createdAt)
  })

  // 创建文档
  function createDocument(title: string, parentId: string | null = null): Document {
    const id = `doc-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`
    const now = Date.now()
    const doc: Document = {
      id,
      title,
      content: '',
      parentId,
      children: [],
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
    
    return doc
  }

  // 更新文档
  function updateDocument(id: string, updates: Partial<Document>) {
    const doc = documents.value.get(id)
    if (doc) {
      Object.assign(doc, updates, { updatedAt: Date.now() })
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

  // 初始化演示数据
  function initDemo() {
    if (documents.value.size > 0) return
    
    const root = createDocument('RNA-seq 差异表达分析', null)
    const design = createDocument('实验设计', root.id)
    const qc = createDocument('数据预处理', root.id)
    const analysis = createDocument('差异分析', root.id)
    
    root.content = `# RNA-seq 差异表达分析

这是一个完整的 RNA-seq 分析流程示例。

## 分析步骤

1. 实验设计
2. 数据预处理
3. 差异分析
`
    
    qc.content = `## FastQC 质控

\`\`\`bash run
fastqc *.fastq.gz -o qc_results
\`\`\`
`
    
    setActiveDocument(root.id)
  }

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
    setActiveDocument,
    addExecution,
    initDemo,
  }
})
