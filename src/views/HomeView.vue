<template>
  <div class="home-view">
    <!-- 引导页：未选择工作区时显示 -->
    <div v-if="!workspaceReady" class="welcome-screen">
      <div class="welcome-content">
        <h1 class="welcome-title">选择工作区</h1>
        <p class="welcome-subtitle">工作区是您的项目目录，所有文件操作都在工作区内进行</p>
        
        <div class="workspace-section">
          <button class="browse-btn-large" @click="browseWorkspace">
            <span class="btn-icon">📁</span>
            <span class="btn-text">浏览目录...</span>
          </button>
          
          <div class="divider">
            <span>或选择最近使用</span>
          </div>
          
          <div class="recent-workspaces" v-if="recentWorkspaces.length > 0">
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
          
          <div class="recent-workspaces empty-hint" v-else>
            <p>暂无最近使用的工作区</p>
          </div>
          
          <p v-if="error" class="error-msg">{{ error }}</p>
        </div>
      </div>
    </div>
    
    <!-- 主界面：工作区就绪后显示 -->
    <div v-else class="main-container">
      <!-- 导航条 -->
      <nav class="nav-bar">
        <div class="nav-logo" @click="currentPage = 'workspace'">
          <svg class="logo-img" viewBox="0 0 512 512" xmlns="http://www.w3.org/2000/svg">
            <defs>
              <linearGradient id="dnaGrad1" x1="0%" y1="0%" x2="0%" y2="100%">
                <stop offset="0%" style="stop-color:#22d3ee"/>
                <stop offset="100%" style="stop-color:#06b6d4"/>
              </linearGradient>
              <linearGradient id="dnaGrad2" x1="0%" y1="0%" x2="0%" y2="100%">
                <stop offset="0%" style="stop-color:#a78bfa"/>
                <stop offset="100%" style="stop-color:#8b5cf6"/>
              </linearGradient>
              <filter id="glow">
                <feGaussianBlur stdDeviation="4" result="coloredBlur"/>
                <feMerge>
                  <feMergeNode in="coloredBlur"/>
                  <feMergeNode in="SourceGraphic"/>
                </feMerge>
              </filter>
            </defs>
            <g filter="url(#glow)">
              <path d="M140 60 Q190 130 140 200 Q90 270 140 340 Q190 410 140 480" fill="none" stroke="url(#dnaGrad1)" stroke-width="24" stroke-linecap="round"/>
              <path d="M240 60 Q190 130 240 200 Q290 270 240 340 Q190 410 240 480" fill="none" stroke="url(#dnaGrad2)" stroke-width="24" stroke-linecap="round"/>
              <ellipse cx="190" cy="110" rx="35" ry="10" fill="#e0e7ff" opacity="0.9"/>
              <ellipse cx="190" cy="200" rx="35" ry="10" fill="#c7d2fe" opacity="0.9"/>
              <ellipse cx="190" cy="290" rx="35" ry="10" fill="#e0e7ff" opacity="0.9"/>
              <ellipse cx="190" cy="380" rx="35" ry="10" fill="#c7d2fe" opacity="0.9"/>
            </g>
            <g transform="translate(280, 130)">
              <circle cx="90" cy="100" r="28" fill="#fbbf24" filter="url(#glow)"/>
              <line x1="90" y1="100" x2="20" y2="50" stroke="#fcd34d" stroke-width="4" opacity="0.8"/>
              <line x1="90" y1="100" x2="160" y2="50" stroke="#fcd34d" stroke-width="4" opacity="0.8"/>
              <line x1="90" y1="100" x2="20" y2="150" stroke="#fcd34d" stroke-width="4" opacity="0.8"/>
              <line x1="90" y1="100" x2="160" y2="150" stroke="#fcd34d" stroke-width="4" opacity="0.8"/>
              <line x1="90" y1="100" x2="90" y2="170" stroke="#fcd34d" stroke-width="4" opacity="0.8"/>
              <line x1="90" y1="100" x2="90" y2="30" stroke="#fcd34d" stroke-width="4" opacity="0.8"/>
              <circle cx="20" cy="50" r="14" fill="#fcd34d"/>
              <circle cx="160" cy="50" r="14" fill="#fcd34d"/>
              <circle cx="20" cy="150" r="14" fill="#fcd34d"/>
              <circle cx="160" cy="150" r="14" fill="#fcd34d"/>
              <circle cx="90" cy="30" r="14" fill="#fcd34d"/>
              <circle cx="90" cy="170" r="14" fill="#fcd34d"/>
            </g>
          </svg>
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
            title="切换工作区" 
            @click="showWorkspaceSwitcher = true"
          >
            📂
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
          
          <button class="browse-btn-modal" @click="browseWorkspaceInModal">
            <span>📁</span> 浏览目录...
          </button>
          
          <div class="recent-section" v-if="recentWorkspaces.length > 0">
            <h3>最近使用</h3>
            <div 
              v-for="ws in recentWorkspaces" 
              :key="ws" 
              class="recent-item"
              @click="switchToWorkspace(ws)"
            >
              <span class="recent-icon">📁</span>
              <span class="recent-path">{{ ws }}</span>
            </div>
          </div>
          
          <div class="modal-actions">
            <button class="cancel-btn" @click="showWorkspaceSwitcher = false">取消</button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, watch } from 'vue'
import WorkspacePage from '../components/WorkspacePage.vue'
import AgentPage from '../components/AgentPage.vue'
import SettingsPage from '../components/SettingsPage.vue'
import { getWorkspacePath, setWorkspacePath, getWorkConfig } from '../services/config'
import { open } from '@tauri-apps/plugin-dialog'

// 状态
const workspaceReady = ref(false)
const workspacePath = ref('')
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
      workspaceReady.value = true
    }
    
    // 获取最近工作区列表
    const config = await getWorkConfig()
    if (config.recent_files && config.recent_files.length > 0) {
      recentWorkspaces.value = config.recent_files.slice(0, 5)
    }
  } catch (err) {
    console.error('加载工作区状态失败:', err)
  }
}

// 浏览工作区（引导页）
async function browseWorkspace() {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: '选择工作区目录'
    })
    
    if (selected) {
      await openWorkspaceWithPath(selected as string)
    }
  } catch (err) {
    error.value = err instanceof Error ? err.message : '选择目录失败'
  }
}

// 浏览工作区（切换弹窗）
async function browseWorkspaceInModal() {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: '选择工作区目录'
    })
    
    if (selected) {
      await switchToWorkspace(selected as string)
    }
  } catch (err) {
    alert('选择目录失败: ' + (err instanceof Error ? err.message : err))
  }
}

// 打开工作区
async function openWorkspaceWithPath(path: string) {
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
  openWorkspaceWithPath(path)
}

// 切换工作区
async function switchToWorkspace(path: string) {
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
  max-width: 450px;
  width: 90%;
}

.welcome-title {
  color: #fff;
  font-size: 1.75rem;
  margin-bottom: 0.75rem;
  font-weight: 500;
}

.welcome-subtitle {
  color: rgba(255, 255, 255, 0.6);
  font-size: 0.9rem;
  margin-bottom: 2rem;
}

.workspace-section {
  background: rgba(255, 255, 255, 0.08);
  border-radius: 12px;
  padding: 1.5rem;
}

.browse-btn-large {
  width: 100%;
  padding: 1rem 1.5rem;
  background: rgba(255, 255, 255, 0.1);
  border: 2px dashed rgba(255, 255, 255, 0.3);
  border-radius: 10px;
  color: #fff;
  font-size: 1rem;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.75rem;
  transition: all 0.2s;
}

.browse-btn-large:hover {
  background: rgba(255, 255, 255, 0.15);
  border-color: rgba(255, 255, 255, 0.5);
}

.btn-icon {
  font-size: 1.5rem;
}

.btn-text {
  font-weight: 500;
}

.divider {
  display: flex;
  align-items: center;
  margin: 1.25rem 0;
}

.divider::before,
.divider::after {
  content: '';
  flex: 1;
  height: 1px;
  background: rgba(255, 255, 255, 0.2);
}

.divider span {
  padding: 0 1rem;
  color: rgba(255, 255, 255, 0.4);
  font-size: 0.8rem;
}

.recent-workspaces {
  text-align: left;
}

.recent-workspaces.empty-hint {
  text-align: center;
}

.recent-workspaces.empty-hint p {
  color: rgba(255, 255, 255, 0.4);
  font-size: 0.85rem;
  margin: 0;
}

.recent-item {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.75rem 1rem;
  border-radius: 8px;
  cursor: pointer;
  margin-bottom: 0.5rem;
  background: rgba(255, 255, 255, 0.05);
  transition: background 0.2s;
}

.recent-item:hover {
  background: rgba(255, 255, 255, 0.12);
}

.recent-item:last-child {
  margin-bottom: 0;
}

.recent-icon {
  font-size: 1.1rem;
  opacity: 0.8;
}

.recent-path {
  color: rgba(255, 255, 255, 0.85);
  font-size: 0.9rem;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.error-msg {
  color: #f87171;
  font-size: 0.85rem;
  margin-top: 1rem;
  text-align: center;
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
}

.nav-logo {
  position: absolute;
  top: 0.5rem;
  left: 50%;
  transform: translateX(-50%);
  cursor: pointer;
  padding: 0.5rem;
  opacity: 0.6;
  z-index: 1;
}

.logo-img {
  width: 36px;
  height: 36px;
  display: block;
}

.nav-items {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  flex: 1;
  gap: 0.5rem;
  width: 100%;
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

/* 主容器 */
.main-container {
  display: flex;
  height: 100%;
  width: 100%;
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
  max-width: 450px;
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

.browse-btn-modal {
  width: 100%;
  padding: 0.75rem 1rem;
  background: var(--color-primary);
  color: #fff;
  border: none;
  border-radius: 8px;
  cursor: pointer;
  font-size: 0.9rem;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.5rem;
}

.browse-btn-modal:hover {
  opacity: 0.9;
}

.recent-section {
  margin-top: 1rem;
}

.recent-section h3 {
  font-size: 0.8rem;
  color: var(--color-text-muted);
  margin-bottom: 0.5rem;
}

.modal-content .recent-item {
  background: #f5f5f5;
  margin-bottom: 0.5rem;
}

.modal-actions {
  margin-top: 1rem;
  text-align: right;
}

.cancel-btn {
  padding: 0.5rem 1rem;
  background: transparent;
  border: 1px solid var(--color-border);
  border-radius: 6px;
  cursor: pointer;
}
</style>