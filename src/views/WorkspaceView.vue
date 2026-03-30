<template>
  <div class="workspace-view">
    <!-- 左侧导航条 -->
    <nav class="nav-sidebar">
      <div class="nav-logo" @click="goHome">🌊</div>
      <div class="nav-items">
        <button class="nav-btn" :class="{ active: true }" title="工作区">
          📁
        </button>
        <button class="nav-btn" title="搜索">
          🔍
        </button>
        <button class="nav-btn" title="设置" @click="goSettings">
          ⚙️
        </button>
      </div>
    </nav>

    <!-- 目录区 -->
    <aside class="panel-sidebar">
      <div class="panel-header">
        <h3>文档</h3>
        <button class="icon-btn" @click="createNewDoc" title="新建文档">+</button>
      </div>
      <div class="panel-content">
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
          @select="selectDocument"
          @create-child="createChildDoc"
        />
      </div>
    </aside>

    <!-- 主编辑区 -->
    <main class="editor-area">
      <div v-if="activeDocument" class="editor-container">
        <header class="editor-header">
          <input
            v-model="activeDocument.title"
            class="title-input"
            @blur="saveDocument"
          />
        </header>
        <div class="editor-content">
          <textarea
            v-model="activeDocument.content"
            class="markdown-editor"
            placeholder="开始编写..."
            @blur="saveDocument"
          />
        </div>
        <!-- AI 互动区 -->
        <footer class="ai-interaction">
          <input
            v-model="aiInput"
            class="ai-input"
            placeholder="向 AI 提问..."
            @keyup.enter="sendToAI"
          />
          <button class="send-btn" @click="sendToAI">发送</button>
        </footer>
      </div>
      <div v-else class="empty-editor">
        <p>选择或创建一个文档开始</p>
      </div>
    </main>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { useWorkspaceStore, type Document } from '../stores/workspace'
import TreeNode from '../components/TreeNode.vue'

const router = useRouter()
const store = useWorkspaceStore()

const aiInput = ref('')

const rootDocuments = computed(() => store.rootDocuments)
const activeDocument = computed(() => store.activeDocument)
const activeDocumentId = computed(() => store.activeDocumentId)

onMounted(() => {
  store.initDemo()
})

function goHome() {
  router.push('/')
}

function goSettings() {
  router.push('/settings')
}

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

function saveDocument() {
  if (activeDocument.value) {
    store.updateDocument(activeDocument.value.id, {
      title: activeDocument.value.title,
      content: activeDocument.value.content,
    })
  }
}

function sendToAI() {
  if (!aiInput.value.trim()) return
  // TODO: 实现 AI 对话
  console.log('AI input:', aiInput.value)
  aiInput.value = ''
}
</script>

<style scoped>
.workspace-view {
  display: flex;
  height: 100vh;
  background: var(--color-bg);
}

/* 导航条 */
.nav-sidebar {
  width: var(--sidebar-width);
  background: #1a1a2e;
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 1rem 0;
}

.nav-logo {
  font-size: 1.5rem;
  cursor: pointer;
  margin-bottom: 2rem;
}

.nav-items {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.nav-btn {
  width: 36px;
  height: 36px;
  border: none;
  background: transparent;
  border-radius: 8px;
  cursor: pointer;
  font-size: 1.25rem;
  opacity: 0.6;
  transition: all 0.2s;
}

.nav-btn:hover,
.nav-btn.active {
  background: rgba(255, 255, 255, 0.1);
  opacity: 1;
}

/* 目录区 */
.panel-sidebar {
  width: var(--panel-width);
  background: var(--color-surface);
  border-right: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
}

.panel-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--color-border);
}

.panel-header h3 {
  font-size: 0.875rem;
  font-weight: 600;
  color: var(--color-text-secondary);
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

.panel-content {
  flex: 1;
  overflow-y: auto;
  padding: 0.5rem;
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
}

.editor-container {
  display: flex;
  flex-direction: column;
  height: 100%;
}

.editor-header {
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--color-border);
  background: var(--color-surface);
}

.title-input {
  width: 100%;
  border: none;
  font-size: 1.25rem;
  font-weight: 600;
  background: transparent;
  outline: none;
}

.editor-content {
  flex: 1;
  padding: 1rem;
  overflow-y: auto;
}

.markdown-editor {
  width: 100%;
  height: 100%;
  border: none;
  background: transparent;
  resize: none;
  font-family: 'Monaco', 'Menlo', monospace;
  font-size: 14px;
  line-height: 1.6;
  outline: none;
}

.empty-editor {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-text-muted);
}

/* AI 互动区 */
.ai-interaction {
  display: flex;
  gap: 0.5rem;
  padding: 1rem;
  border-top: 1px solid var(--color-border);
  background: var(--color-surface);
}

.ai-input {
  flex: 1;
  padding: 0.75rem 1rem;
  border: 1px solid var(--color-border);
  border-radius: 8px;
  outline: none;
  font-size: 14px;
}

.ai-input:focus {
  border-color: var(--color-primary);
}

.send-btn {
  padding: 0.75rem 1.5rem;
  background: var(--color-primary);
  color: white;
  border: none;
  border-radius: 8px;
  cursor: pointer;
  font-weight: 500;
}

.send-btn:hover {
  background: var(--color-primary-dark);
}
</style>
