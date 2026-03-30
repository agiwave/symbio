<template>
  <div class="home-view" :class="{
    'workspace-only': workspaceVisible && !agentVisible,
    'agent-only': !workspaceVisible && agentVisible,
    'both-visible': workspaceVisible && agentVisible
  }">
    <!-- 顶层导航条 (始终显示) -->
    <nav class="nav-bar">
      <div class="nav-logo" @click="goHome">🌊</div>
      <div class="nav-items">
        <button 
          class="nav-btn" 
          :class="{ active: workspaceVisible }"
          title="工作区" 
          @click="toggleWorkspace"
        >
          📁
        </button>
        <button 
          class="nav-btn" 
          :class="{ active: agentVisible }"
          title="Agent" 
          @click="toggleAgent"
        >
          💬
        </button>
        <button class="nav-btn" title="设置" @click="goSettings">
          ⚙️
        </button>
      </div>
    </nav>

    <!-- 主内容区 -->
    <div class="content-area">
      <!-- 欢迎页 (当工作区和Agent都隐藏时显示) -->
      <WelcomePanel 
        v-if="!workspaceVisible && !agentVisible"
        @start-workspace="showWorkspace"
      />

      <!-- 工作区面板 -->
      <WorkspacePanel 
        v-show="workspaceVisible"
        @selection-change="handleSelectionChange"
        @open-floating-input="openFloatingInput"
      />

      <!-- Agent 交互区 -->
      <AgentPanel
        :visible="agentVisible"
        :full-width="!workspaceVisible"
        ref="agentRef"
        @close="agentVisible = false"
        @send="handleAgentMessage"
      />
    </div>

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
import { ref, onMounted, onUnmounted } from 'vue'
import { useRouter } from 'vue-router'
import WelcomePanel from '../components/WelcomePanel.vue'
import WorkspacePanel from '../components/WorkspacePanel.vue'
import AgentPanel from '../components/AgentPanel.vue'
import FloatingInput from '../components/FloatingInput.vue'
import AIBubble from '../components/AIBubble.vue'

const router = useRouter()

// UI 状态
const agentVisible = ref(false)
const workspaceVisible = ref(false)  // 默认隐藏，显示欢迎页
const agentRef = ref<InstanceType<typeof AgentPanel> | null>(null)
const floatingInputVisible = ref(false)
const floatingInputPosition = ref<{ x: number; y: number } | undefined>()
const selectedText = ref('')

// AI 提示气泡状态
const bubbleVisible = ref(false)
const bubbleType = ref<'info' | 'warning' | 'success' | 'error'>('info')
const bubbleTitle = ref('')
const bubbleMessage = ref('')
const bubbleActions = ref<{ id: string; label: string }[]>([])

onMounted(() => {
  document.addEventListener('keydown', handleGlobalKeydown)
})

onUnmounted(() => {
  document.removeEventListener('keydown', handleGlobalKeydown)
})

function handleGlobalKeydown(e: KeyboardEvent) {
  if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
    e.preventDefault()
    openFloatingInput()
  }
}

function goHome() {
  workspaceVisible.value = false
  agentVisible.value = false
}

function goSettings() {
  router.push('/settings')
}

function toggleWorkspace() {
  workspaceVisible.value = !workspaceVisible.value
}

function toggleAgent() {
  agentVisible.value = !agentVisible.value
}

function showWorkspace() {
  workspaceVisible.value = true
}

function handleSelectionChange(text: string) {
  selectedText.value = text
}

// AI 相关方法
function openFloatingInput() {
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

async function handleAgentMessage(message: string) {
  console.log('Agent message:', message)
  agentRef.value?.setLoading(true)
  setTimeout(() => {
    agentRef.value?.addResponse('这是一个模拟的 AI 响应。实际使用时需要接入 AI API。')
  }, 1000)
}

function handleFloatingSubmit(text: string, context?: string) {
  agentVisible.value = true
  setTimeout(() => {
    handleAgentMessage(context ? `上下文: ${context}\n\n问题: ${text}` : text)
  }, 100)
}

function handleBubbleAction(actionId: string) {
  console.log('Bubble action:', actionId)
  bubbleVisible.value = false
}

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

defineExpose({
  showBubble,
  toggleAgent,
})
</script>

<style scoped>
.home-view {
  display: flex;
  height: 100%;
  width: 100%;
  background: var(--color-bg);
}

/* 导航条 (始终显示) */
.nav-bar {
  width: var(--sidebar-width, 56px);
  background: #1a1a2e;
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 1rem 0;
  flex-shrink: 0;
  z-index: 10;
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

/* 主内容区 */
.content-area {
  flex: 1;
  display: flex;
  height: 100%;
  min-width: 0;
  overflow: hidden;
}

/* 过渡动画 */
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.2s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>
