<template>
  <div class="settings-page">
    <!-- 左侧导航 -->
    <aside class="settings-nav">
      <div class="nav-header">
        <h3>设置</h3>
      </div>
      <div class="nav-items">
        <button 
          v-for="section in sections"
          :key="section.id"
          class="nav-item"
          :class="{ active: activeSection === section.id }"
          @click="activeSection = section.id"
        >
          <span class="nav-icon">{{ section.icon }}</span>
          <span class="nav-label">{{ section.label }}</span>
        </button>
      </div>
    </aside>

    <!-- 右侧设置内容 -->
    <main class="settings-content">
      <!-- 外观设置 -->
      <section v-show="activeSection === 'appearance'" class="content-section">
        <h2>外观</h2>
        <div class="setting-group">
          <div class="setting-item">
            <div class="setting-info">
              <label>主题</label>
              <p class="setting-desc">选择应用的主题风格</p>
            </div>
            <select v-model="settings.theme">
              <option value="light">浅色</option>
              <option value="dark">深色</option>
              <option value="auto">跟随系统</option>
            </select>
          </div>
          <div class="setting-item">
            <div class="setting-info">
              <label>字体大小</label>
              <p class="setting-desc">调整界面字体大小</p>
            </div>
            <select v-model="settings.fontSize">
              <option value="small">小</option>
              <option value="medium">中</option>
              <option value="large">大</option>
            </select>
          </div>
        </div>
      </section>

      <!-- AI 设置 -->
      <section v-show="activeSection === 'ai'" class="content-section">
        <h2>AI 设置</h2>
        <div class="setting-group">
          <div class="setting-item">
            <div class="setting-info">
              <label>LLM 提供商</label>
              <p class="setting-desc">选择 AI 模型提供商</p>
            </div>
            <select v-model="settings.llmProvider">
              <option value="openai">OpenAI</option>
              <option value="claude">Claude</option>
              <option value="local">本地模型</option>
            </select>
          </div>
          <div class="setting-item">
            <div class="setting-info">
              <label>API Key</label>
              <p class="setting-desc">输入您的 API 密钥</p>
            </div>
            <input v-model="settings.apiKey" type="password" placeholder="输入 API Key" />
          </div>
          <div class="setting-item">
            <div class="setting-info">
              <label>模型</label>
              <p class="setting-desc">选择使用的模型版本</p>
            </div>
            <select v-model="settings.model">
              <option value="gpt-4">GPT-4</option>
              <option value="gpt-4-turbo">GPT-4 Turbo</option>
              <option value="gpt-3.5-turbo">GPT-3.5 Turbo</option>
              <option value="claude-3-opus">Claude 3 Opus</option>
              <option value="claude-3-sonnet">Claude 3 Sonnet</option>
            </select>
          </div>
        </div>
      </section>

      <!-- 执行环境设置 -->
      <section v-show="activeSection === 'execution'" class="content-section">
        <h2>执行环境</h2>
        <div class="setting-group">
          <div class="setting-item">
            <div class="setting-info">
              <label>Docker 镜像</label>
              <p class="setting-desc">代码执行使用的 Docker 镜像</p>
            </div>
            <input v-model="settings.dockerImage" placeholder="symbio/bioinfo:latest" />
          </div>
          <div class="setting-item">
            <div class="setting-info">
              <label>CPU 限制</label>
              <p class="setting-desc">容器可使用的最大 CPU 核心数</p>
            </div>
            <input v-model.number="settings.cpuLimit" type="number" min="1" max="16" />
            <span class="unit">核心</span>
          </div>
          <div class="setting-item">
            <div class="setting-info">
              <label>内存限制</label>
              <p class="setting-desc">容器可使用的最大内存</p>
            </div>
            <input v-model.number="settings.memoryLimit" type="number" min="1" max="64" />
            <span class="unit">GB</span>
          </div>
          <div class="setting-item">
            <div class="setting-info">
              <label>执行超时</label>
              <p class="setting-desc">代码执行的最大时间</p>
            </div>
            <input v-model.number="settings.timeout" type="number" min="10" max="600" />
            <span class="unit">秒</span>
          </div>
        </div>
      </section>

      <!-- 数据管理 -->
      <section v-show="activeSection === 'data'" class="content-section">
        <h2>数据管理</h2>
        <div class="setting-group">
          <div class="setting-item">
            <div class="setting-info">
              <label>自动保存</label>
              <p class="setting-desc">自动保存编辑内容</p>
            </div>
            <label class="toggle">
              <input type="checkbox" v-model="settings.autoSave" />
              <span class="toggle-slider"></span>
            </label>
          </div>
          <div class="setting-item">
            <div class="setting-info">
              <label>导出数据</label>
              <p class="setting-desc">导出所有工作区数据</p>
            </div>
            <button class="action-btn" @click="exportData">导出</button>
          </div>
          <div class="setting-item danger">
            <div class="setting-info">
              <label>清除数据</label>
              <p class="setting-desc">清除所有本地存储的数据</p>
            </div>
            <button class="action-btn danger" @click="clearData">清除</button>
          </div>
        </div>
      </section>

      <!-- 关于 -->
      <section v-show="activeSection === 'about'" class="content-section">
        <h2>关于</h2>
        <div class="about-info">
          <div class="app-logo">🌊</div>
          <h1>Symbio</h1>
          <p class="version">版本 0.1.0</p>
          <p class="description">在做中学：生信分析的互动学习平台</p>
          <div class="links">
            <a href="#" class="link-btn">文档</a>
            <a href="#" class="link-btn">GitHub</a>
            <a href="#" class="link-btn">反馈</a>
          </div>
        </div>
      </section>
    </main>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive } from 'vue'

const activeSection = ref('appearance')

const sections = [
  { id: 'appearance', icon: '🎨', label: '外观' },
  { id: 'ai', icon: '🤖', label: 'AI 设置' },
  { id: 'execution', icon: '⚡', label: '执行环境' },
  { id: 'data', icon: '📦', label: '数据管理' },
  { id: 'about', icon: 'ℹ️', label: '关于' },
]

const settings = reactive({
  theme: 'light',
  fontSize: 'medium',
  llmProvider: 'openai',
  apiKey: '',
  model: 'gpt-4',
  dockerImage: 'symbio/bioinfo:latest',
  cpuLimit: 4,
  memoryLimit: 8,
  timeout: 60,
  autoSave: true,
})

function exportData() {
  console.log('Export data...')
  alert('数据导出功能待实现')
}

function clearData() {
  if (confirm('确定要清除所有数据吗？此操作不可撤销。')) {
    localStorage.clear()
    location.reload()
  }
}
</script>

<style scoped>
.settings-page {
  display: flex;
  height: 100%;
  width: 100%;
}

/* 左侧导航 */
.settings-nav {
  width: 200px;
  background: var(--color-surface);
  border-right: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}

.nav-header {
  padding: 1rem;
  border-bottom: 1px solid var(--color-border);
}

.nav-header h3 {
  font-size: 0.875rem;
  font-weight: 600;
  color: var(--color-text-secondary);
}

.nav-items {
  padding: 0.5rem;
}

.nav-item {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.75rem 1rem;
  border: none;
  background: transparent;
  border-radius: 8px;
  cursor: pointer;
  text-align: left;
  transition: background 0.2s;
}

.nav-item:hover {
  background: #f0f0f0;
}

.nav-item.active {
  background: #e8e8f0;
}

.nav-icon {
  font-size: 1rem;
}

.nav-label {
  font-size: 0.875rem;
  color: var(--color-text);
}

/* 右侧设置内容 */
.settings-content {
  flex: 1;
  padding: 2rem 3rem;
  overflow-y: auto;
}

.content-section h2 {
  font-size: 1.25rem;
  margin-bottom: 1.5rem;
}

.setting-group {
  background: var(--color-surface);
  border-radius: 12px;
  padding: 0.5rem;
}

.setting-item {
  display: flex;
  align-items: center;
  padding: 1rem;
  border-radius: 8px;
}

.setting-item:hover {
  background: #fafafa;
}

.setting-item.danger {
  border-top: 1px solid var(--color-border);
  margin-top: 0.5rem;
}

.setting-info {
  flex: 1;
}

.setting-info label {
  display: block;
  font-weight: 500;
  margin-bottom: 0.25rem;
}

.setting-desc {
  font-size: 0.75rem;
  color: var(--color-text-muted);
  margin: 0;
}

.setting-item select,
.setting-item input[type="text"],
.setting-item input[type="password"],
.setting-item input[type="number"] {
  padding: 0.5rem 0.75rem;
  border: 1px solid var(--color-border);
  border-radius: 6px;
  min-width: 200px;
  font-size: 0.875rem;
}

.setting-item input[type="number"] {
  width: 80px;
  min-width: auto;
}

.unit {
  margin-left: 0.5rem;
  color: var(--color-text-muted);
  font-size: 0.875rem;
}

/* Toggle 开关 */
.toggle {
  position: relative;
  display: inline-block;
  width: 48px;
  height: 24px;
}

.toggle input {
  opacity: 0;
  width: 0;
  height: 0;
}

.toggle-slider {
  position: absolute;
  cursor: pointer;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background-color: #ccc;
  transition: 0.3s;
  border-radius: 24px;
}

.toggle-slider::before {
  position: absolute;
  content: "";
  height: 18px;
  width: 18px;
  left: 3px;
  bottom: 3px;
  background-color: white;
  transition: 0.3s;
  border-radius: 50%;
}

.toggle input:checked + .toggle-slider {
  background-color: var(--color-primary);
}

.toggle input:checked + .toggle-slider::before {
  transform: translateX(24px);
}

.action-btn {
  padding: 0.5rem 1rem;
  background: var(--color-primary);
  color: white;
  border: none;
  border-radius: 6px;
  cursor: pointer;
  font-size: 0.875rem;
}

.action-btn.danger {
  background: #dc3545;
}

.action-btn.danger:hover {
  background: #c82333;
}

/* 关于页面 */
.about-info {
  text-align: center;
  padding: 3rem;
}

.app-logo {
  font-size: 4rem;
  margin-bottom: 1rem;
}

.about-info h1 {
  font-size: 2rem;
  margin-bottom: 0.5rem;
}

.version {
  color: var(--color-text-muted);
  margin-bottom: 1rem;
}

.description {
  color: var(--color-text-secondary);
  margin-bottom: 2rem;
}

.links {
  display: flex;
  gap: 1rem;
  justify-content: center;
}

.link-btn {
  padding: 0.5rem 1.5rem;
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: 6px;
  color: var(--color-text);
  text-decoration: none;
  transition: background 0.2s;
}

.link-btn:hover {
  background: #f0f0f0;
}
</style>
