<template>
  <div class="workspace-panel">
    <!-- 文档树 -->
    <aside class="doc-tree">
      <div class="doc-tree-header">
        <h3>文档</h3>
        <div class="doc-tree-actions">
          <button class="icon-btn" @click="createNewDoc" title="新建文档">+</button>
          <button class="icon-btn secondary" @click="exportWorkspace" title="导出">↓</button>
        </div>
      </div>
      <div class="doc-tree-content">
        <div v-if="rootDocuments.length === 0" class="empty-state">
          <p>暂无文档</p>
          <button @click="createNewDoc">创建第一个文档</button>
        </div>
        <TreeNode
          v-for="doc in rootDocuments"
          :key="doc.id"
          :document="doc"
          :level="0"
          :active-id="activeDocumentId"
          :documents="documents"
          @select="selectDocument"
          @create-child="createChildDoc"
          @delete="deleteDoc"
          @move="moveDoc"
        />
      </div>
      <div class="doc-tree-footer">
        <button class="footer-btn" @click="clearAll" title="清空所有">
          🗑️ 清空
        </button>
      </div>
    </aside>

    <!-- 编辑区 -->
    <main class="editor-area">
      <div v-if="activeDocument" class="editor-container">
        <header class="editor-header">
          <input
            v-model="activeDocument.title"
            class="title-input"
            placeholder="无标题"
            @blur="saveDocument"
          />
          <div class="editor-actions">
            <button class="action-btn" @click="openFloatingInput" title="AI 助手 (Ctrl+K)">
              🤖
            </button>
          </div>
        </header>
        <div class="editor-content">
          <MarkdownEditor
            v-model="activeDocument.content"
            @selection-change="handleSelectionChange"
          />
        </div>
      </div>
      <div v-else class="empty-editor">
        <p>选择或创建一个文档开始</p>
      </div>
    </main>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted } from 'vue'
import { useWorkspaceStore } from '../stores/workspace'
import TreeNode from './TreeNode.vue'
import MarkdownEditor from './MarkdownEditor.vue'

const store = useWorkspaceStore()
const emit = defineEmits<{
  'selection-change': [text: string]
  'open-floating-input': []
}>()

// 文档状态
const documents = computed(() => store.documents)
const rootDocuments = computed(() => store.rootDocuments)
const activeDocument = computed(() => store.activeDocument)
const activeDocumentId = computed(() => store.activeDocumentId)

onMounted(() => {
  store.init()
})

function createNewDoc() {
  const doc = store.createDocument('新文档')
  store.setActiveDocument(doc.id)
}

function createChildDoc(parentId: string) {
  const doc = store.createDocument('新子文档', parentId)
  store.setActiveDocument(doc.id)
}

function selectDocument(id: string) {
  store.setActiveDocument(id)
}

function deleteDoc(id: string) {
  if (confirm('确定要删除此文档及其所有子文档吗？')) {
    store.deleteDocument(id)
  }
}

function moveDoc(payload: { id: string; targetParentId: string | null }) {
  store.moveDocument(payload.id, payload.targetParentId, 0)
}

function saveDocument() {
  if (activeDocument.value) {
    store.updateDocument(activeDocument.value.id, {
      title: activeDocument.value.title,
      content: activeDocument.value.content,
    })
  }
}

function exportWorkspace() {
  const json = store.exportToJSON()
  const blob = new Blob([json], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = 'symbio-workspace-' + Date.now() + '.json'
  a.click()
  URL.revokeObjectURL(url)
}

function clearAll() {
  if (confirm('确定要清空所有文档吗？此操作不可撤销。')) {
    store.clearStorage()
  }
}

function handleSelectionChange(text: string) {
  emit('selection-change', text)
}

function openFloatingInput() {
  emit('open-floating-input')
}
</script>

<style scoped>
.workspace-panel {
  display: flex;
  height: 100%;
  min-width: 0;
  flex: 1;
}

/* 文档树 */
.doc-tree {
  width: var(--panel-width, 240px);
  background: var(--color-surface);
  border-right: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  height: 100%;
  flex-shrink: 0;
  z-index: 10;
}

.doc-tree-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}

.doc-tree-header h3 {
  font-size: 0.875rem;
  font-weight: 600;
  color: var(--color-text-secondary);
}

.doc-tree-actions {
  display: flex;
  gap: 0.5rem;
}

.icon-btn {
  width: 24px;
  height: 24px;
  border: none;
  background: var(--color-primary);
  color: white;
  border-radius: 4px;
  cursor: pointer;
  font-size: 1rem;
  line-height: 1;
}

.icon-btn.secondary {
  background: #6c757d;
}

.doc-tree-content {
  flex: 1;
  overflow-y: auto;
  padding: 0.5rem;
}

.doc-tree-footer {
  padding: 0.5rem 1rem;
  border-top: 1px solid var(--color-border);
  flex-shrink: 0;
}

.footer-btn {
  width: 100%;
  padding: 0.5rem;
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  cursor: pointer;
  font-size: 12px;
  border-radius: 4px;
}

.footer-btn:hover {
  background: #f0f0f0;
}

.empty-state {
  text-align: center;
  padding: 2rem 1rem;
  color: var(--color-text-muted);
}

.empty-state button {
  margin-top: 1rem;
  padding: 0.5rem 1rem;
  background: var(--color-primary);
  color: white;
  border: none;
  border-radius: 6px;
  cursor: pointer;
}

/* 编辑区 */
.editor-area {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  height: 100%;
}

.editor-container {
  display: flex;
  flex-direction: column;
  height: 100%;
  width: 100%;
}

.editor-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--color-border);
  background: var(--color-surface);
  flex-shrink: 0;
}

.title-input {
  flex: 1;
  border: none;
  font-size: 1.25rem;
  font-weight: 600;
  background: transparent;
  outline: none;
}

.editor-actions {
  display: flex;
  gap: 0.5rem;
}

.editor-actions .action-btn {
  width: 32px;
  height: 32px;
  border: none;
  background: transparent;
  border-radius: 6px;
  cursor: pointer;
  font-size: 1rem;
  transition: background 0.2s;
}

.editor-actions .action-btn:hover {
  background: #f0f0f0;
}

.editor-content {
  flex: 1;
  overflow-y: auto;
  width: 100%;
  min-height: 0;
}

.empty-editor {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-text-muted);
}
</style>
