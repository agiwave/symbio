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
      <!-- 消息提示 -->
      <div v-if="message" :class="['message', message.type]">
        {{ message.text }}
      </div>

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
            <select v-model="llmProvider" @change="onProviderChange">
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
            <input v-model="aiConfig.api_base" type="text" placeholder="https://api.openai.com/v1" />
          </div>
          
          <div class="setting-item">
            <div class="setting-info">
              <label>API Key</label>
              <p class="setting-desc">输入您的 API 密钥</p>
            </div>
            <input v-model="aiConfig.api_key" type="password" placeholder="输入 API Key" />
          </div>
          
          <div class="setting-item">
            <div class="setting-info">
              <label>模型</label>
              <p class="setting-desc">选择使用的模型版本</p>
            </div>
            <select v-model="aiConfig.model">
              <option v-for="m in availableModels" :key="m" :value="m">{{ m }}</option>
            </select>
          </div>
          
          <div class="setting-item">
            <div class="setting-info">
              <label>Temperature</label>
              <p class="setting-desc">控制输出随机性 (0-2)</p>
            </div>
            <input v-model.number="aiConfig.temperature" type="number" min="0" max="2" step="0.1" />
          </div>
          
          <div class="setting-item">
            <div class="setting-info">
              <label>Max Tokens</label>
              <p class="setting-desc">最大输出长度</p>
            </div>
            <input v-model.number="aiConfig.max_tokens" type="number" min="100" max="128000" />
          </div>
          
          <div class="setting-item">
            <div class="setting-info"></div>
            <button class="action-btn" @click="saveAiConfig" :disabled="saving">
              {{ saving ? '保存中...' : '保存配置' }}
            </button>
          </div>
        </div>
      </section>

      <!-- 会话设置 -->
      <section v-show="activeSection === 'session'" class="content-section">
        <h2>会话设置</h2>
        <div class="setting-group">
          <div class="setting-item">
            <div class="setting-info">
              <label>最大消息数</label>
              <p class="setting-desc">每个会话保存的最大消息数量</p>
            </div>
            <input v-model.number="sessionConfig.max_messages" type="number" min="10" max="1000" />
          </div>
          
          <div class="setting-item">
            <div class="setting-info">
              <label>自动压缩</label>
              <p class="setting-desc">当消息数超过阈值时自动压缩历史</p>
            </div>
            <label class="toggle">
              <input type="checkbox" v-model="sessionConfig.auto_compress" />
              <span class="toggle-slider"></span>
            </label>
          </div>
          
          <div class="setting-item">
            <div class="setting-info">
              <label>压缩阈值</label>
              <p class="setting-desc">触发自动压缩的消息数量</p>
            </div>
            <input v-model.number="sessionConfig.compress_threshold" type="number" min="10" max="500" />
          </div>
          
          <div class="setting-item">
            <div class="setting-info"></div>
            <button class="action-btn" @click="saveSessionConfig" :disabled="saving">
              {{ saving ? '保存中...' : '保存配置' }}
            </button>
          </div>
        </div>
      </section>

      <!-- 工具设置 -->
      <section v-show="activeSection === 'tools'" class="content-section">
        <h2>工具设置</h2>
        <div class="setting-group">
          <div class="setting-item">
            <div class="setting-info">
              <label>启用 Shell 工具</label>
              <p class="setting-desc">允许执行 Shell 命令</p>
            </div>
            <label class="toggle">
              <input type="checkbox" v-model="toolsConfig.shell_enabled" />
              <span class="toggle-slider"></span>
            </label>
          </div>
          
          <div class="setting-item">
            <div class="setting-info">
              <label>启用文件工具</label>
              <p class="setting-desc">允许文件读写操作</p>
            </div>
            <label class="toggle">
              <input type="checkbox" v-model="toolsConfig.file_enabled" />
              <span class="toggle-slider"></span>
            </label>
          </div>
          
          <div class="setting-item">
            <div class="setting-info">
              <label>启用 Web 工具</label>
              <p class="setting-desc">允许网络请求</p>
            </div>
            <label class="toggle">
              <input type="checkbox" v-model="toolsConfig.web_enabled" />
              <span class="toggle-slider"></span>
            </label>
          </div>
          
          <div class="setting-item">
            <div class="setting-info">
              <label>Shell 超时（秒）</label>
              <p class="setting-desc">Shell 命令执行超时时间</p>
            </div>
            <input v-model.number="toolsConfig.shell_timeout" type="number" min="1" max="3600" />
          </div>
          
          <div class="setting-item">
            <div class="setting-info">
              <label>Web 超时（秒）</label>
              <p class="setting-desc">Web 请求超时时间</p>
            </div>
            <input v-model.number="toolsConfig.web_timeout" type="number" min="1" max="300" />
          </div>
          
          <div class="setting-item">
            <div class="setting-info"></div>
            <button class="action-btn" @click="saveToolsConfig" :disabled="saving">
              {{ saving ? '保存中...' : '保存配置' }}
            </button>
          </div>
        </div>
      </section>

      <!-- 工作区设置 -->
      <section v-show="activeSection === 'workspace'" class="content-section">
        <h2>工作区设置</h2>
        <div class="setting-group">
          <div class="setting-item">
            <div class="setting-info">
              <label>工作区路径</label>
              <p class="setting-desc">当前工作区目录</p>
            </div>
            <input v-model="workConfig.workspace_path" type="text" placeholder="~/projects" />
          </div>
          
          <div class="setting-item" v-if="workConfig.recent_workspaces?.length">
            <div class="setting-info">
              <label>最近工作区</label>
              <p class="setting-desc">最近打开的工作区</p>
            </div>
            <div class="recent-list">
              <span v-for="path in workConfig.recent_workspaces" :key="path" class="recent-item">
                {{ path }}
              </span>
            </div>
          </div>
          
          <div class="setting-item">
            <div class="setting-info"></div>
            <button class="action-btn" @click="saveWorkConfig" :disabled="saving">
              {{ saving ? '保存中...' : '保存配置' }}
            </button>
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
import {
  getOpenAiConfig,
  setOpenAiConfig,
  getSessionConfig,
  setSessionConfig,
  getToolsConfig,
  setToolsConfig,
  getWorkConfig,
  setWorkConfig,
  type OpenAiConfig,
  type SessionConfig,
  type ToolsConfig,
  type WorkConfig,
} from '../services/config'

const activeSection = ref('ai')
const saving = ref(false)
const message = ref<{ type: string; text: string } | null>(null)

const sections = [
  { id: 'appearance', icon: '🎨', label: '外观' },
  { id: 'ai', icon: '🤖', label: 'AI 设置' },
  { id: 'session', icon: '💬', label: '会话设置' },
  { id: 'tools', icon: '🔧', label: '工具设置' },
  { id: 'workspace', icon: '📁', label: '工作区' },
  { id: 'about', icon: 'ℹ️', label: '关于' },
]

// 外观设置（本地存储）
const settings = reactive({
  theme: 'light',
  fontSize: 'medium',
})

// AI 设置
const llmProvider = ref('openai')
const aiConfig = reactive<Partial<OpenAiConfig> & { api_key?: string }>({
  api_base: 'https://api.openai.com/v1',
  api_key: '',
  model: 'gpt-4o-mini',
  temperature: 0.7,
  max_tokens: 4096,
})

// 会话设置
const sessionConfig = reactive<SessionConfig>({
  storage_dir: '',
  max_messages: 100,
  auto_compress: true,
  compress_threshold: 50,
})

// 工具设置
const toolsConfig = reactive<ToolsConfig>({
  shell_enabled: true,
  file_enabled: true,
  web_enabled: true,
  allowed_paths: ['~'],
  blocked_commands: ['rm -rf', 'sudo', 'chmod 777'],
  shell_timeout: 60,
  web_timeout: 30,
})

// 工作区设置
const workConfig = reactive<WorkConfig>({
  workspace_path: '~/projects',
  recent_workspaces: [],
})

// 提供商预设
const providerPresets: Record<string, { apiBase: string; models: string[] }> = {
  openai: {
    apiBase: 'https://api.openai.com/v1',
    models: ['gpt-4o-mini', 'gpt-4o', 'gpt-4-turbo', 'gpt-3.5-turbo', 'o1', 'o1-mini', 'o3-mini']
  },
  deepseek: {
    apiBase: 'https://api.deepseek.com/v1',
    models: ['deepseek-chat', 'deepseek-coder', 'deepseek-reasoner']
  },
  moonshot: {
    apiBase: 'https://api.moonshot.cn/v1',
    models: ['moonshot-v1-8k', 'moonshot-v1-32k', 'moonshot-v1-128k']
  },
  zhipu: {
    apiBase: 'https://open.bigmodel.cn/api/paas/v4',
    models: ['glm-4', 'glm-4-flash', 'glm-4-plus', 'glm-3-turbo']
  },
  aiyuanjing: {
    apiBase: 'https://maas-api.ai-yuanjing.com/openapi/compatible-mode/v1',
    models: ['glm-5', 'glm-4-plus', 'glm-4']
  },
  local: {
    apiBase: 'http://localhost:11434/v1',
    models: ['llama3', 'qwen2', 'mistral', 'deepseek-coder-v2']
  },
  custom: {
    apiBase: '',
    models: []
  }
}

const availableModels = ref<string[]>(providerPresets.openai.models)

// 切换提供商时更新配置
function onProviderChange() {
  const preset = providerPresets[llmProvider.value]
  if (preset) {
    aiConfig.api_base = preset.apiBase
    availableModels.value = preset.models
    if (preset.models.length > 0) {
      aiConfig.model = preset.models[0]
    }
  }
}

// 显示消息
function showMessage(type: string, text: string) {
  message.value = { type, text }
  setTimeout(() => { message.value = null }, 3000)
}

// 加载所有配置
async function loadConfigs() {
  try {
    // 加载 AI 配置
    const aiCfg = await getOpenAiConfig()
    if (aiCfg) {
      aiConfig.api_base = aiCfg.api_base || 'https://api.openai.com/v1'
      aiConfig.model = aiCfg.model || 'gpt-4o-mini'
      aiConfig.temperature = aiCfg.temperature ?? 0.7
      aiConfig.max_tokens = aiCfg.max_tokens ?? 4096
      
      // 根据 api_base 推断提供商
      for (const [name, preset] of Object.entries(providerPresets)) {
        if (preset.apiBase === aiCfg.api_base) {
          llmProvider.value = name
          availableModels.value = preset.models
          break
        }
      }
    }

    // 加载会话配置
    const sessCfg = await getSessionConfig()
    if (sessCfg) {
      Object.assign(sessionConfig, sessCfg)
    }

    // 加载工具配置
    const toolsCfg = await getToolsConfig()
    if (toolsCfg) {
      Object.assign(toolsConfig, toolsCfg)
    }

    // 加载工作区配置
    const workCfg = await getWorkConfig()
    if (workCfg) {
      Object.assign(workConfig, workCfg)
    }
  } catch (err) {
    console.error('加载配置失败:', err)
  }
}

// 保存 AI 配置
async function saveAiConfig() {
  saving.value = true
  try {
    await setOpenAiConfig({
      api_base: aiConfig.api_base,
      api_key: aiConfig.api_key,
      model: aiConfig.model,
      temperature: aiConfig.temperature,
      max_tokens: aiConfig.max_tokens,
    })
    showMessage('success', 'AI 配置已保存')
  } catch (err) {
    showMessage('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}

// 保存会话配置
async function saveSessionConfig() {
  saving.value = true
  try {
    await setSessionConfig({
      max_messages: sessionConfig.max_messages,
      auto_compress: sessionConfig.auto_compress,
      compress_threshold: sessionConfig.compress_threshold,
    })
    showMessage('success', '会话配置已保存')
  } catch (err) {
    showMessage('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}

// 保存工具配置
async function saveToolsConfig() {
  saving.value = true
  try {
    await setToolsConfig({
      shell_enabled: toolsConfig.shell_enabled,
      file_enabled: toolsConfig.file_enabled,
      web_enabled: toolsConfig.web_enabled,
      shell_timeout: toolsConfig.shell_timeout,
      web_timeout: toolsConfig.web_timeout,
    })
    showMessage('success', '工具配置已保存')
  } catch (err) {
    showMessage('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}

// 保存工作区配置
async function saveWorkConfig() {
  saving.value = true
  try {
    await setWorkConfig({
      workspace_path: workConfig.workspace_path,
    })
    showMessage('success', '工作区配置已保存')
  } catch (err) {
    showMessage('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}

onMounted(() => {
  loadConfigs()
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

.message {
  padding: 0.75rem 1rem;
  margin-bottom: 1rem;
  border-radius: 8px;
  font-size: 0.875rem;
}

.message.success {
  background: #d4edda;
  color: #155724;
}

.message.error {
  background: #f8d7da;
  color: #721c24;
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
  width: 100px;
  min-width: auto;
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

.action-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
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

/* 最近工作区列表 */
.recent-list {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  max-width: 300px;
}

.recent-item {
  font-size: 0.75rem;
  color: var(--color-text-muted);
  padding: 0.25rem 0.5rem;
  background: #f5f5f5;
  border-radius: 4px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>