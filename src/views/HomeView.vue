<template>
  <div class="home-view">
    <!-- 引导页：未选择工作区时显示 -->
    <div v-if="!workspaceReady" class="welcome-screen">
      <div class="welcome-content">
        <img :src="logoUrl" alt="Symbio" class="welcome-logo" />
        <h1 class="welcome-title">Symbio</h1>
        <p class="welcome-subtitle">在做中学：生信分析的互动学习平台</p>
        
        <div class="workspace-section">
          <h2>选择工作区</h2>
          <p class="workspace-hint">工作区是您的项目目录，所有文件操作都在工作区内进行</p>
          
          <div class="workspace-input-group">
            <input 
              v-model="workspaceInput" 
              type="text" 
              placeholder="输入工作区路径，如 ~/projects"
              @keyup.enter="openWorkspace"
            />
            <button class="browse-btn" @click="browseWorkspace" title="浏览">📂</button>
          </div>
          
          <div class="recent-workspaces" v-if="recentWorkspaces.length > 0">
            <h3>最近使用</h3>
            <div 
              v-for="ws in recentWorkspaces" 
              :key="ws" 
              class="recent-item"
              @click="selectWorkspace(ws)"
            >
              <span class="recent-icon">📁</span>
              <span class="recent-path">{{ ws }}</span>
            </div>
          </div>
          
          <button class="open-btn" @click="openWorkspace" :disabled="!workspaceInput.trim()">
            打开工作区
          </button>
          
          <p v-if="error" class="error-msg">{{ error }}</p>
        </div>
      </div>
    </div>
    
    <!-- 主界面：工作区就绪后显示 -->
    <template v-else>
      <!-- 导航条 -->
      <nav class="nav-bar">
        <div class="nav-logo" @click="currentPage = 'workspace'">
          <img :src="logoUrl" alt="Symbio" class="logo-img" />
        </div>
        
        <!-- 工作区指示器 -->
        <div class="workspace-indicator" @click="showWorkspaceSwitcher = true" :title="workspacePath">
          <span class="workspace-icon">📂</span>
        </div>
        
        <div class="nav-items">
          <button 
            class="nav-btn" 
            :class="{ active: currentPage === 'workspace' }"
            title="工作区" 
            @click="currentPage = 'workspace'"
          >
            📄
          </button>
          <button 
            class="nav-btn" 
            :class="{ active: currentPage === 'agent' }"
            title="AI 对话" 
            @click="currentPage = 'agent'"
          >
            💬
          </button>
          <button 
            class="nav-btn" 
            :class="{ active: currentPage === 'settings' }"
            title="设置" 
            @click="currentPage = 'settings'"
          >
            ⚙️
          </button>
        </div>
      </nav>

      <!-- 主内容区 -->
      <div class="content-area">
        <WorkspacePage v-if="currentPage === 'workspace'" />
        <AgentPage v-else-if="currentPage === 'agent'" />
        <SettingsPage v-else-if="currentPage === 'settings'" />
      </div>
      
      <!-- 工作区切换弹窗 -->
      <div v-if="showWorkspaceSwitcher" class="modal-overlay" @click.self="showWorkspaceSwitcher = false">
        <div class="modal-content workspace-switcher">
          <h2>切换工作区</h2>
          <div class="current-workspace">
            <span class="label">当前工作区：</span>
            <span class="path">{{ workspacePath }}</span>
          </div>
          <div class="workspace-input-group">
            <input 
              v-model="newWorkspacePath" 
              type="text" 
              placeholder="输入新的工作区路径"
            />
            <button class="browse-btn" @click="browseWorkspace" title="浏览">📂</button>
          </div>
          <div class="modal-actions">
            <button class="cancel-btn" @click="showWorkspaceSwitcher = false">取消</button>
            <button class="switch-btn" @click="switchWorkspace" :disabled="!newWorkspacePath.trim()">
              切换
            </button>
          </div>
          <div class="recent-workspaces" v-if="recentWorkspaces.length > 0">
            <h3>最近使用</h3>
            <div 
              v-for="ws in recentWorkspaces" 
              :key="ws" 
              class="recent-item"
              @click="newWorkspacePath = ws"
            >
              <span class="recent-icon">📁</span>
              <span class="recent-path">{{ ws }}</span>
            </div>
          </div>
        </div>
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import WorkspacePage from '../components/WorkspacePage.vue'
import AgentPage from '../components/AgentPage.vue'
import SettingsPage from '../components/SettingsPage.vue'
import logoUrl from '../assets/logo.svg'
import { getWorkspacePath, setWorkspacePath, getWorkConfig } from '../services/config'

// 状态
const workspaceReady = ref(false)
const workspacePath = ref('')
const workspaceInput = ref('')
const newWorkspacePath = ref('')
const currentPage = ref<'workspace' | 'agent' | 'settings'>('workspace')
const showWorkspaceSwitcher = ref(false)
const recentWorkspaces = ref<string[]>([])
const error = ref('')

// 加载工作区状态
async function loadWorkspaceState() {
  try {
    // 获取当前工作区路径
    const result = await getWorkspacePath()
    workspacePath.value = result.workspace_path
    
    // 如果有工作区，直接进入
    if (workspacePath.value) {
      workspaceInput.value = workspacePath.value
      workspaceReady.value = true
    }
    
    // 获取最近工作区列表
    const config = await getWorkConfig()
    if (config.recent_files && config.recent_files.length > 0) {
      recentWorkspaces.value = config.recent_files.slice(0, 5)
    }
  } catch (err) {
    console.error('加载工作区状态失败:', err)
    // 使用默认路径
    workspaceInput.value = '~/projects'
  }
}

// 打开工作区
async function openWorkspace() {
  const path = workspaceInput.value.trim()
  if (!path) return
  
  error.value = ''
  
  try {
    const result = await setWorkspacePath(path)
    workspacePath.value = result.workspace_path
    workspaceReady.value = true
    
    // 添加到最近列表
    if (!recentWorkspaces.value.includes(path)) {
      recentWorkspaces.value.unshift(path)
      recentWorkspaces.value = recentWorkspaces.value.slice(0, 5)
    }
  } catch (err) {
    error.value = err instanceof Error ? err.message : '打开工作区失败'
  }
}

// 选择工作区（从最近列表）
function selectWorkspace(path: string) {
  workspaceInput.value = path
  openWorkspace()
}

// 切换工作区
async function switchWorkspace() {
  const path = newWorkspacePath.value.trim()
  if (!path) return
  
  try {
    const result = await setWorkspacePath(path)
    workspacePath.value = result.workspace_path
    showWorkspaceSwitcher.value = false
    
    // 添加到最近列表
    if (!recentWorkspaces.value.includes(path)) {
      recentWorkspaces.value.unshift(path)
      recentWorkspaces.value = recentWorkspaces.value.slice(0, 5)
    }
  } catch (err) {
    alert('切换工作区失败: ' + (err instanceof Error ? err.message : err))
  }
}

// 浏览工作区（TODO: 调用系统文件选择器）
function browseWorkspace() {
  // 目前使用简单输入，后续可集成 Tauri 文件对话框
  alert('文件浏览器功能待实现，请直接输入路径')
}

onMounted(() => {
  loadWorkspaceState()
})

// 调试
watch(currentPage, (newVal, oldVal) => {
  console.log(`[HomeView] 页面切换: ${oldVal} -> ${newVal}`)
})
</script>

<style scoped>
.home-view {
  display: flex;
  height: 100%;
  width: 100%;
  background: var(--color-bg);
}

/* 欢迎页 */
.welcome-screen {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
  background: linear-gradient(135deg, #1a1a2e 0%, #16213e 100%);
}

.welcome-content {
  text-align: center;
  max-width: 500px;
  padding: 2rem;
}

.welcome-logo {
  width: 80px;
  height: 80px;
  border-radius: 16px;
  margin-bottom: 1.5rem;
}

.welcome-title {
  color: #fff;
  font-size: 2.5rem;
  margin-bottom: 0.5rem;
}

.welcome-subtitle {
  color: rgba(255, 255, 255, 0.7);
  font-size: 1rem;
  margin-bottom: 3rem;
}

.workspace-section {
  background: rgba(255, 255, 255, 0.1);
  border-radius: 16px;
  padding: 2rem;
}

.workspace-section h2 {
  color: #fff;
  font-size: 1.25rem;
  margin-bottom: 0.5rem;
}

.workspace-hint {
  color: rgba(255, 255, 255, 0.6);
  font-size: 0.875rem;
  margin-bottom: 1.5rem;
}

.workspace-input-group {
  display: flex;
  gap: 0.5rem;
  margin-bottom: 1.5rem;
}

.workspace-input-group input {
  flex: 1;
  padding: 0.75rem 1rem;
  border: 1px solid rgba(255, 255, 255, 0.2);
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.1);
  color: #fff;
  font-size: 1rem;
}

.workspace-input-group input::placeholder {
  color: rgba(255, 255, 255, 0.4);
}

.browse-btn {
  width: 44px;
  height: 44px;
  border: 1px solid rgba(255, 255, 255, 0.2);
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.1);
  color: #fff;
  font-size: 1.25rem;
  cursor: pointer;
}

.browse-btn:hover {
  background: rgba(255, 255, 255, 0.2);
}

.open-btn {
  width: 100%;
  padding: 0.875rem;
  background: var(--color-primary);
  color: #fff;
  border: none;
  border-radius: 8px;
  font-size: 1rem;
  font-weight: 500;
  cursor: pointer;
}

.open-btn:hover:not(:disabled) {
  opacity: 0.9;
}

.open-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.error-msg {
  color: #f87171;
  font-size: 0.875rem;
  margin-top: 1rem;
}

.recent-workspaces {
  margin-top: 1.5rem;
  text-align: left;
}

.recent-workspaces h3 {
  color: rgba(255, 255, 255, 0.7);
  font-size: 0.75rem;
  font-weight: 500;
  text-transform: uppercase;
  margin-bottom: 0.5rem;
}

.recent-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 0.75rem;
  border-radius: 6px;
  cursor: pointer;
  margin-bottom: 0.25rem;
}

.recent-item:hover {
  background: rgba(255, 255, 255, 0.1);
}

.recent-icon {
  font-size: 1rem;
}

.recent-path {
  color: rgba(255, 255, 255, 0.8);
  font-size: 0.875rem;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 导航条 */
.nav-bar {
  width: var(--sidebar-width, 56px);
  background: #1a1a2e;
  display: flex;
  flex-direction: column;
  align-items: center;
  flex-shrink: 0;
  z-index: 10;
  position: relative;
  padding-top: 0.5rem;
}

.nav-logo {
  cursor: pointer;
  padding: 0.5rem;
}

.logo-img {
  width: 36px;
  height: 36px;
  display: block;
}

.workspace-indicator {
  width: 40px;
  height: 40px;
  display: flex;
  align-items: center;
  justify-content: center;
  margin-top: 0.5rem;
  border-radius: 8px;
  cursor: pointer;
  background: rgba(255, 255, 255, 0.05);
}

.workspace-indicator:hover {
  background: rgba(255, 255, 255, 0.1);
}

.workspace-icon {
  font-size: 1.25rem;
}

.nav-items {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  margin-top: 1rem;
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
  height: 100%;
  min-width: 0;
  overflow: hidden;
}

.content-area :deep(> *) {
  height: 100%;
  width: 100%;
}

/* 模态框 */
.modal-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}

.modal-content {
  background: var(--color-surface);
  border-radius: 12px;
  padding: 1.5rem;
  max-width: 500px;
  width: 90%;
  box-shadow: 0 4px 20px rgba(0, 0, 0, 0.15);
}

.workspace-switcher h2 {
  font-size: 1.25rem;
  margin-bottom: 1rem;
}

.current-workspace {
  background: #f5f5f5;
  border-radius: 8px;
  padding: 0.75rem 1rem;
  margin-bottom: 1rem;
}

.current-workspace .label {
  color: var(--color-text-muted);
  font-size: 0.75rem;
}

.current-workspace .path {
  display: block;
  color: var(--color-text);
  font-size: 0.875rem;
  margin-top: 0.25rem;
  word-break: break-all;
}

.modal-content .workspace-input-group input {
  background: #fff;
  border-color: var(--color-border);
  color: var(--color-text);
}

.modal-content .workspace-input-group input::placeholder {
  color: var(--color-text-muted);
}

.modal-actions {
  display: flex;
  gap: 0.75rem;
  margin-top: 1rem;
}

.cancel-btn {
  flex: 1;
  padding: 0.75rem;
  border: 1px solid var(--color-border);
  background: transparent;
  border-radius: 8px;
  cursor: pointer;
}

.switch-btn {
  flex: 1;
  padding: 0.75rem;
  background: var(--color-primary);
  color: #fff;
  border: none;
  border-radius: 8px;
  cursor: pointer;
}

.switch-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.modal-content .recent-workspaces {
  margin-top: 1rem;
}

.modal-content .recent-workspaces h3 {
  color: var(--color-text-muted);
  font-size: 0.75rem;
}

.modal-content .recent-item {
  background: #f5f5f5;
  margin-bottom: 0.5rem;
}

.modal-content .recent-path {
  color: var(--color-text);
}
</style>
