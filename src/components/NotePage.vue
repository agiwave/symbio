<template>
  <div class="note-page" :class="{ 'chat-visible': chatVisible }">
    <!-- 笔记树 -->
    <aside class="note-tree">
      <div class="note-tree-header">
        <h3>笔记</h3>
        <div class="note-tree-actions">
          <button class="icon-btn" @click="createNewNote" title="新建笔记">+</button>
          <button class="icon-btn secondary" @click="exportNotes" title="导出">↓</button>
        </div>
      </div>
      <div class="note-tree-content">
        <div v-if="rootNotes.length === 0" class="empty-state">
          <p>暂无笔记</p>
          <button @click="createNewNote">创建第一个笔记</button>
        </div>
        <TreeNode
          v-for="note in rootNotes"
          :key="note.id"
          :document="note"
          :level="0"
          :active-id="activeNoteId"
          :documents="notes"
          @select="selectNote"
          @create-child="createChildNote"
          @delete="deleteNote"
          @move="moveNoteHandler"
        />
      </div>
      <div class="note-tree-footer">
        <button class="footer-btn" @click="clearAll" title="清空所有">
          🗑️ 清空
        </button>
      </div>
    </aside>

    <!-- 编辑区 -->
    <main class="editor-area">
      <div v-if="activeNote" class="editor-container">
        <header class="editor-header">
          <input
            v-model="activeNote.title"
            class="title-input"
            placeholder="无标题"
            @blur="saveNote"
          />
          <div class="editor-actions">
            <button 
              class="action-btn" 
              :class="{ active: chatVisible }"
              @click="chatVisible = !chatVisible" 
              title="AI 对话"
            >
              💬
            </button>
          </div>
        </header>
        <div class="editor-content">
          <MarkdownEditor
            v-model="activeNote.content"
            @selection-change="handleSelectionChange"
          />
        </div>
      </div>
      <div v-else class="empty-editor">
        <p>选择或创建一个笔记开始</p>
      </div>
    </main>

    <!-- AI 对话侧边栏 (可拉出/隐藏) -->
    <aside class="chat-sidebar" v-show="chatVisible">
      <div class="chat-header">
        <h3>AI 对话</h3>
        <button class="close-btn" @click="chatVisible = false" title="隐藏">×</button>
      </div>
      <div class="chat-history" ref="historyRef">
        <div v-if="messages.length === 0" class="empty-chat">
          <p>开始与 AI 对话</p>
          <p class="hint">输入问题或粘贴代码进行分析</p>
        </div>
        <div 
          v-for="(msg, index) in messages" 
          :key="index" 
          class="message"
          :class="msg.role"
        >
          <div class="message-avatar">
            {{ msg.role === 'user' ? '👤' : '🤖' }}
          </div>
          <div class="message-content">
            <div class="message-text">{{ msg.content }}</div>
          </div>
        </div>
        <div v-if="isLoading" class="message assistant loading">
          <div class="message-avatar">🤖</div>
          <div class="message-content">
            <div class="typing-indicator">
              <span></span><span></span><span></span>
            </div>
          </div>
        </div>
      </div>
      <div class="chat-input">
        <textarea
          v-model="inputText"
          placeholder="输入消息... (Enter 发送)"
          @keydown.enter.exact.prevent="sendMessage"
          :disabled="isLoading"
        ></textarea>
        <button @click="sendMessage" :disabled="!inputText.trim() || isLoading">
          发送
        </button>
      </div>
    </aside>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, nextTick, watch } from 'vue'
import { useNoteStore, type Note } from '../stores/note'
import TreeNode from './TreeNode.vue'
import MarkdownEditor from './MarkdownEditor.vue'

interface Message {
  role: 'user' | 'assistant'
  content: string
}

const store = useNoteStore()

// UI 状态
const chatVisible = ref(false)
const messages = ref<Message[]>([])
const inputText = ref('')
const isLoading = ref(false)
const historyRef = ref<HTMLElement | null>(null)

// 笔记状态
const notes = computed(() => store.notes)
const rootNotes = computed(() => store.rootNotes)
const activeNote = computed(() => store.activeNote)
const activeNoteId = computed(() => store.activeNoteId)

onMounted(() => {
  store.init()
})

// 笔记操作
function createNewNote() {
  store.createNote('新笔记').then((note) => {
    if (note) store.setActiveNote(note.id)
  })
}

function createChildNote(parentId: string) {
  store.createNote('新子笔记', parentId).then((note) => {
    if (note) store.setActiveNote(note.id)
  })
}

function selectNote(id: string) {
  store.setActiveNote(id)
}

function deleteNote(id: string) {
  if (confirm('确定要删除此笔记及其所有子笔记吗？')) {
    store.deleteNote(id)
  }
}

function moveNoteHandler(payload: { id: string; targetParentId: string | null }) {
  store.moveNote(payload.id, payload.targetParentId, 0)
}

function saveNote() {
  if (activeNote.value) {
    store.updateNote(activeNote.value.id, {
      title: activeNote.value.title,
      content: activeNote.value.content,
    })
  }
}

function exportNotes() {
  const json = store.exportToJSON()
  const blob = new Blob([json], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = 'symbio-notes-' + Date.now() + '.json'
  a.click()
  URL.revokeObjectURL(url)
}

function clearAll() {
  if (confirm('确定要清空所有笔记吗？此操作不可撤销。')) {
    store.clearStorage()
  }
}

function handleSelectionChange(text: string) {
  console.log('Selection:', text)
}

// AI 对话
async function sendMessage() {
  const text = inputText.value.trim()
  if (!text || isLoading.value) return

  messages.value.push({ role: 'user', content: text })
  inputText.value = ''
  isLoading.value = true

  await nextTick()
  scrollToBottom()

  // 模拟 AI 响应
  setTimeout(() => {
    messages.value.push({
      role: 'assistant',
      content: '这是一个模拟的 AI 响应。实际使用时需要接入 AI API。'
    })
    isLoading.value = false
    scrollToBottom()
  }, 1000)
}

function scrollToBottom() {
  if (historyRef.value) {
    historyRef.value.scrollTop = historyRef.value.scrollHeight
  }
}

watch(chatVisible, (visible) => {
  if (visible) {
    nextTick(scrollToBottom)
  }
})
</script>

<style scoped>
.note-page {
  display: flex;
  height: 100%;
  width: 100%;
}

/* 笔记树 */
.note-tree {
  width: var(--panel-width, 240px);
  background: var(--color-surface);
  border-right: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}

.note-tree-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}

.note-tree-header h3 {
  font-size: 0.875rem;
  font-weight: 600;
  color: var(--color-text-secondary);
}

.note-tree-actions {
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

.note-tree-content {
  flex: 1;
  overflow-y: auto;
  padding: 0.5rem;
}

.note-tree-footer {
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

.action-btn {
  width: 32px;
  height: 32px;
  border: none;
  background: transparent;
  border-radius: 6px;
  cursor: pointer;
  font-size: 1rem;
  transition: background 0.2s;
}

.action-btn:hover,
.action-btn.active {
  background: #f0f0f0;
}

.editor-content {
  flex: 1;
  overflow-y: auto;
  min-height: 0;
}

.empty-editor {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-text-muted);
}

/* AI 对话侧边栏 */
.chat-sidebar {
  width: 320px;
  background: var(--color-surface);
  border-left: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}

.chat-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}

.chat-header h3 {
  font-size: 0.875rem;
  font-weight: 600;
}

.close-btn {
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: 1.25rem;
  color: var(--color-text-muted);
  border-radius: 4px;
}

.close-btn:hover {
  background: #f0f0f0;
}

.chat-history {
  flex: 1;
  overflow-y: auto;
  padding: 1rem;
}

.empty-chat {
  text-align: center;
  color: var(--color-text-muted);
  padding: 2rem;
}

.empty-chat .hint {
  font-size: 0.75rem;
  opacity: 0.7;
  margin-top: 0.5rem;
}

.message {
  display: flex;
  gap: 0.75rem;
  margin-bottom: 1rem;
}

.message.user {
  flex-direction: row-reverse;
}

.message-avatar {
  font-size: 1rem;
  flex-shrink: 0;
}

.message-content {
  max-width: 80%;
}

.message-text {
  padding: 0.5rem 0.75rem;
  border-radius: 12px;
  font-size: 0.875rem;
  line-height: 1.5;
  word-break: break-word;
}

.message.user .message-text {
  background: var(--color-primary);
  color: white;
}

.message.assistant .message-text {
  background: #f0f0f0;
  color: var(--color-text);
}

.typing-indicator {
  display: flex;
  gap: 4px;
  padding: 0.5rem;
}

.typing-indicator span {
  width: 6px;
  height: 6px;
  background: var(--color-text-muted);
  border-radius: 50%;
  animation: bounce 1.4s infinite ease-in-out both;
}

.typing-indicator span:nth-child(1) { animation-delay: -0.32s; }
.typing-indicator span:nth-child(2) { animation-delay: -0.16s; }

@keyframes bounce {
  0%, 80%, 100% { transform: scale(0); }
  40% { transform: scale(1); }
}

.chat-input {
  padding: 0.75rem;
  border-top: 1px solid var(--color-border);
}

.chat-input textarea {
  width: 100%;
  min-height: 60px;
  max-height: 120px;
  padding: 0.5rem;
  border: 1px solid var(--color-border);
  border-radius: 8px;
  resize: none;
  font-size: 0.875rem;
  outline: none;
}

.chat-input textarea:focus {
  border-color: var(--color-primary);
}

.chat-input button {
  width: 100%;
  margin-top: 0.5rem;
  padding: 0.5rem;
  background: var(--color-primary);
  color: white;
  border: none;
  border-radius: 6px;
  cursor: pointer;
}

.chat-input button:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>