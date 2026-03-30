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
        
        <div v-if="saveStatus === 'success'" class="save-success">
          配置已保存
        </div>
        
        <div class="setting-group">
          <div class="setting-item">
            <div class="setting-info">
              <label>LLM 提供商</label>
              <p class="setting-desc">选择 AI 模型提供商</p>
            </div>
            <select v-model="settings.llmProvider">
              <option value="openai">OpenAI</option>
              <option value="deepseek">DeepSeek</option>
              <option value="moonshot">Moonshot (月之暗面)</option>
              <option value="zhipu">智谱 GLM</option>
              <option value="aiyuanjing">AI 远景</option>
              <option value="local">本地模型 (Ollama)</option>
              <option value="custom">自定义</option>
            </select>
          </div>
          
          <div class="setting-item">
            <div class="setting-info">
              <label>API Base URL</label>
              <p class="setting-desc">API 服务地址</p>
            </div>
            <input v-model="settings.apiBase" type="text" placeholder="https://api.openai.com/v1" />
          </div>
          
          <div class="setting-item">
            <div class="setting-info">
              <label>API Key</label>
              <p class="setting-desc">输入您的 API 密钥</p>
            </div>
            <input v-model="settings.apiKey" type="password" placeholder="输入 API Key" />
          </div>
          
          <div class="setting-item" v-if="availableModels.length > 0">
            <div class="setting-info">
              <label>模型</label>
              <p class="setting-desc">选择使用的模型版本</p>
            </div>
            <select v-model="settings.model">
              <option v-for="m in availableModels" :key="m" :value="m">{{ m }}</option>
            </select>
          </div>
          
          <div class="setting-item">
            <div class="setting-info">
              <label>自定义模型</label>
              <p class="setting-desc">输入自定义模型名称（可选）</p>
            </div>
            <input v-model="settings.customModel" type="text" placeholder="留空则使用上方选择的模型" />
          </div>
          
          <div class="setting-item">
            <div class="setting-info">
              <label>Temperature</label>
              <p class="setting-desc">控制输出随机性 (0-2)</p>
            </div>
            <input v-model.number="settings.temperature" type="number" min="0" max="2" step="0.1" />
          </div>
          
          <div class="setting-item">
            <div class="setting-info">
              <label>Max Tokens</label>
              <p class="setting-desc">最大输出长度</p>
            </div>
            <input v-model.number="settings.maxTokens" type="number" min="100" max="32000" />
          </div>
          
          <div class="setting-item">
            <div class="setting-info"></div>
            <button class="action-btn" @click="saveAiConfig">保存配置</button>
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
          <img :src="logoUrl" alt="Symbio" class="app-logo" />
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
import { ref, reactive, onMounted, watch } from 'vue'
import logoUrl from '../assets/logo.svg'
import { configureProvider, getProviderConfig } from '../services/ai'

const activeSection = ref('appearance')
const saveStatus = ref<string | null>(null)

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
  apiBase: 'https://api.openai.com/v1',
  apiKey: '',
  model: 'gpt-4o-mini',
  customModel: '',
  temperature: 0.7,
  maxTokens: 4096,
  dockerImage: 'symbio/bioinfo:latest',
  cpuLimit: 4,
  memoryLimit: 8,
  timeout: 60,
  autoSave: true,
})

// 提供商预设
const providerPresets: Record<string, { apiBase: string; models: string[] }> = {
  openai: {
    apiBase: 'https://api.openai.com/v1',
    models: ['gpt-4o-mini', 'gpt-4o', 'gpt-4-turbo', 'gpt-3.5-turbo']
  },
  deepseek: {
    apiBase: 'https://api.deepseek.com/v1',
    models: ['deepseek-chat', 'deepseek-coder']
  },
  moonshot: {
    apiBase: 'https://api.moonshot.cn/v1',
    models: ['moonshot-v1-8k', 'moonshot-v1-32k', 'moonshot-v1-128k']
  },
  zhipu: {
    apiBase: 'https://open.bigmodel.cn/api/paas/v4',
    models: ['glm-4', 'glm-4-flash', 'glm-3-turbo']
  },
  aiyuanjing: {
    apiBase: 'https://maas-api.ai-yuanjing.com/openapi/compatible-mode/v1',
    models: ['glm-5', 'glm-4-plus', 'glm-4']
  },
  local: {
    apiBase: 'http://localhost:11434/v1',
    models: ['llama3', 'qwen2', 'mistral']
  },
  custom: {
    apiBase: '',
    models: []
  }
}

const availableModels = ref<string[]>(providerPresets.openai.models)

// 切换提供商时更新 API Base 和模型列表
watch(() => settings.llmProvider, (provider) => {
  const preset = providerPresets[provider]
  if (preset) {
    settings.apiBase = preset.apiBase
    availableModels.value = preset.models
    if (preset.models.length > 0) {
      settings.model = preset.models[0]
    }
  }
})

// 加载保存的配置
async function loadConfig() {
  try {
    const config = await getProviderConfig()
    settings.llmProvider = config.name || 'openai'
    settings.apiBase = config.api_base
    settings.model = config.model
    settings.temperature = config.temperature ?? 0.7
    settings.maxTokens = config.max_tokens ?? 4096
    
    // 更新模型列表
    const preset = providerPresets[settings.llmProvider]
    if (preset) {
      availableModels.value = preset.models
    }
  } catch (err) {
    console.error('加载配置失败:', err)
  }
}

// 保存 AI 配置
async function saveAiConfig() {
  saveStatus.value = null
  
  try {
    const result = await configureProvider({
      name: settings.llmProvider,
      api_base: settings.apiBase,
      api_key: settings.apiKey,
      model: settings.customModel || settings.model,
      temperature: settings.temperature,
      max_tokens: settings.maxTokens
    })
    
    if (result.message) {
      saveStatus.value = 'success'
      setTimeout(() => { saveStatus.value = null }, 2000)
    }
  } catch (err) {
    saveStatus.value = 'error'
    console.error('保存配置失败:', err)
  }
}

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

onMounted(() => {
  loadConfig()
})
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

.save-success {
  padding: 0.75rem 1rem;
  margin-bottom: 1rem;
  background: #d4edda;
  color: #155724;
  border-radius: 8px;
  font-size: 0.875rem;
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
  width: 80px;
  height: 80px;
  border-radius: 16px;
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
