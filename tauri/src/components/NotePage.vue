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
      <div class="chat-content">
        <AIChatPanel
          :session-id="NOTE_SESSION_ID"
          :messages="chatMessages"
          :on-update-messages="updateChatMessages"
          :show-context="true"
        />
      </div>
    </aside>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useNoteStore } from '../stores/note'
import TreeNode from './TreeNode.vue'
import MarkdownEditor from './MarkdownEditor.vue'
import AIChatPanel from './AIChatPanel.vue'
import { type SessionMessage } from '../services/session'

const store = useNoteStore()

// UI 状态
const chatVisible = ref(false)

// AI 对话状态 - 使用固定的 session_id
const NOTE_SESSION_ID = 'note-ai-session'
const chatMessages = ref<SessionMessage[]>([])

function updateChatMessages(messages: SessionMessage[]) {
  chatMessages.value = messages
}

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
  min-height: 0;
  overflow: hidden;
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

.chat-content {
  flex: 1;
  overflow: hidden;
}
</style>