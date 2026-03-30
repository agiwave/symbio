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
        <button 
          class="nav-btn" 
          :class="{ active: aiSidebarVisible }"
          title="AI 助手" 
          @click="toggleAISidebar"
        >
          💬
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
        <div class="panel-actions">
          <button class="icon-btn" @click="createNewDoc" title="新建文档">+</button>
          <button class="icon-btn secondary" @click="exportWorkspace" title="导出">↓</button>
        </div>
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
          :documents="documents"
          @select="selectDocument"
          @create-child="createChildDoc"
          @delete="deleteDoc"
          @move="moveDoc"
        />
      </div>
      <!-- 底部操作栏 -->
      <div class="panel-footer">
        <button class="footer-btn" @click="clearAll" title="清空所有">
          🗑️ 清空
        </button>
      </div>
    </aside>

    <!-- 主编辑区 -->
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

    <!-- AI 独立栏 (全局，不受文档切换影响) -->
    <AISidebar
      :visible="aiSidebarVisible"
      ref="aiSidebarRef"
      @close="aiSidebarVisible = false"
      @send="handleAIMessage"
    />

    <!-- 悬浮输入框 -->
    <FloatingInput
      :visible="floatingInputVisible"
      :position="floatingInputPosition"
      :context="selectedText"
      placeholder="向 AI 提问..."
      @close="floatingInputVisible = false"
      @submit="handleFloatingSubmit"
    />

    <!-- AI 提示气泡 -->
    <AIBubble
      :visible="bubbleVisible"
      :type="bubbleType"
      :title="bubbleTitle"
      :message="bubbleMessage"
      :actions="bubbleActions"
      @close="bubbleVisible = false"
      @action="handleBubbleAction"
    />
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useRouter } from 'vue-router'
import { useWorkspaceStore } from '../stores/workspace'
import TreeNode from '../components/TreeNode.vue'
import MarkdownEditor from '../components/MarkdownEditor.vue'
import AISidebar from '../components/AISidebar.vue'
import FloatingInput from '../components/FloatingInput.vue'
import AIBubble from '../components/AIBubble.vue'

const router = useRouter()
const store = useWorkspaceStore()

// AI 组件状态 (全局，不随文档切换重置)
const aiSidebarVisible = ref(false)
const aiSidebarRef = ref<InstanceType<typeof AISidebar> | null>(null)
const floatingInputVisible = ref(false)
const floatingInputPosition = ref<{ x: number; y: number } | undefined>()
const selectedText = ref('')

// AI 提示气泡状态
const bubbleVisible = ref(false)
const bubbleType = ref<'info' | 'warning' | 'success' | 'error'>('info')
const bubbleTitle = ref('')
const bubbleMessage = ref('')
const bubbleActions = ref<{ id: string; label: string }[]>([])

// 文档状态
const documents = computed(() => store.documents)
const rootDocuments = computed(() => store.rootDocuments)
const activeDocument = computed(() => store.activeDocument)
const activeDocumentId = computed(() => store.activeDocumentId)

onMounted(() => {
  store.init()
  // 全局快捷键
  document.addEventListener('keydown', handleGlobalKeydown)
})

onUnmounted(() => {
  document.removeEventListener('keydown', handleGlobalKeydown)
})

function handleGlobalKeydown(e: KeyboardEvent) {
  // Ctrl+K / Cmd+K 打开悬浮输入框
  if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
    e.preventDefault()
    openFloatingInput()
  }
}

function goHome() {
  router.push('/')
}

function goSettings() {
  router.push('/settings')
}

function toggleAISidebar() {
  aiSidebarVisible.value = !aiSidebarVisible.value
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

// AI 相关方法
function openFloatingInput() {
  // 在光标位置或屏幕中央显示
  const selection = window.getSelection()
  if (selection && selection.rangeCount > 0) {
    const range = selection.getRangeAt(0)
    const rect = range.getBoundingClientRect()
    floatingInputPosition.value = {
      x: rect.left,
      y: rect.bottom + 10,
    }
    selectedText.value = selection.toString()
  } else {
    floatingInputPosition.value = undefined
    selectedText.value = ''
  }
  floatingInputVisible.value = true
}

function handleSelectionChange(text: string) {
  selectedText.value = text
}

async function handleAIMessage(message: string) {
  // TODO: 调用实际的 AI API
  console.log('AI message:', message)
  
  // 模拟 AI 响应
  aiSidebarRef.value?.setLoading(true)
  setTimeout(() => {
    aiSidebarRef.value?.addResponse('这是一个模拟的 AI 响应。实际使用时需要接入 AI API。')
  }, 1000)
}

function handleFloatingSubmit(text: string, context?: string) {
  // 显示 AI 侧边栏并发送消息
  aiSidebarVisible.value = true
  setTimeout(() => {
    handleAIMessage(context ? `上下文: ${context}\n\n问题: ${text}` : text)
  }, 100)
}

function handleBubbleAction(actionId: string) {
  console.log('Bubble action:', actionId)
  bubbleVisible.value = false
}

// 显示 AI 提示气泡的方法 (可被外部调用)
function showBubble(
  type: 'info' | 'warning' | 'success' | 'error',
  title: string,
  message?: string,
  actions?: { id: string; label: string }[]
) {
  bubbleType.value = type
  bubbleTitle.value = title
  bubbleMessage.value = message || ''
  bubbleActions.value = actions || []
  bubbleVisible.value = true
}

// 暴露方法给外部使用
defineExpose({
  showBubble,
  toggleAISidebar,
})
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
  flex-shrink: 0;
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
  flex-shrink: 0;
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

.panel-actions {
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

.panel-content {
  flex: 1;
  overflow-y: auto;
  padding: 0.5rem;
}

.panel-footer {
  padding: 0.5rem 1rem;
  border-top: 1px solid var(--color-border);
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
}

.editor-container {
  display: flex;
  flex-direction: column;
  height: 100%;
}

.editor-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--color-border);
  background: var(--color-surface);
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
}

.empty-editor {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-text-muted);
}
</style>