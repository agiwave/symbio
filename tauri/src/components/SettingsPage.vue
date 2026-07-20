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
          @click="(activeSection = section.id)"
        >
          <span class="nav-icon">{{ section.icon }}</span>
          <span class="nav-label">{{ section.label }}{{'' }}</span>
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
            <div class="setting-info">
              <label>上下文消息数量</label>
              <p class="setting-desc">Model \u5bf9\u8bdd时包含的上下文消息数量（0 表示不限制，6 表示 3 轮对话）</p>
            </div>
            <input v-model.number="sessionConfig.context_messages" type="number" min="0" max="200" />
          </div>

          <div class="setting-divider"></div>

          <div class="setting-item">
            <div class="setting-info"></div>
            <button class="action-btn" @click="saveSessionConfig" :disabled="saving">
              {{ saving ? '保存中...' : '保存配置' }}
            </button>
          </div>
        </div>
      </section>

      <!-- 本地工具设置 -->
      <section v-show="activeSection === 'local'" class="content-section">
        <h2>本地工具设置</h2>
        <div class="setting-group">
          <div class="setting-item">
            <div class="setting-info">
              <label>启用 Shell 工具</label>
              <p class="setting-desc">允许执行 Shell 命令</p>
            </div>
            <label class="toggle">
              <input type="checkbox" v-model="localConfig.shell_enabled" />
              <span class="toggle-slider"></span>
            </label>
          </div>

          <div class="setting-item">
            <div class="setting-info">
              <label>启用文件工具</label>
              <p class="setting-desc">允许文件读写操作</p>
            </div>
            <label class="toggle">
              <input type="checkbox" v-model="localConfig.file_enabled" />
              <span class="toggle-slider"></span>
            </label>
          </div>

          <div class="setting-item">
            <div class="setting-info">
              <label>Shell 超时（秒）</label>
              <p class="setting-desc">Shell 命令执行超时时间</p>
            </div>
            <input v-model.number="localConfig.shell_timeout" type="number" min="1" max="3600" />
          </div>

          <div class="setting-item">
            <div class="setting-info"></div>
            <button class="action-btn" @click="saveLocalConfig" :disabled="saving">
              {{ saving ? '保存中...' : '保存配置' }}
            </button>
          </div>
        </div>
      </section>

      <!-- 网络工具设置 -->
      <section v-show="activeSection === 'web'" class="content-section">
        <h2>网络工具设置</h2>
        <div class="setting-group">
          <div class="setting-item">
            <div class="setting-info">
              <label>启用 Web 工具</label>
              <p class="setting-desc">允许网络请求</p>
            </div>
            <label class="toggle">
              <input type="checkbox" v-model="webConfig.web_enabled" />
              <span class="toggle-slider"></span>
            </label>
          </div>

          <div class="setting-item">
            <div class="setting-info">
              <label>Web 超时（秒）</label>
              <p class="setting-desc">Web 请求超时时间</p>
            </div>
            <input v-model.number="webConfig.web_timeout" type="number" min="1" max="300" />
          </div>

          <div class="setting-item">
            <div class="setting-info">
              <label>Tavily API Key</label>
              <p class="setting-desc">用于高级网页搜索（优先）</p>
            </div>
            <input v-model="webConfig.tavily_api_key" type="password" placeholder="输入 Tavily API Key" />
          </div>

          <div class="setting-item">
            <div class="setting-info">
              <label>Serper API Key</label>
              <p class="setting-desc">用于 Google 网页搜索（备用）</p>
            </div>
            <input v-model="webConfig.serper_api_key" type="password" placeholder="输入 Serper API Key" />
          </div>

          <div class="setting-item">
            <div class="setting-info"></div>
            <button class="action-btn" @click="saveWebConfig" :disabled="saving">
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
import { ref, reactive, onMounted } from 'vue'
import { logger } from '@/utils/logger'
import logoUrl from '../assets/logo.svg'
import {
  getSessionConfig,
  setSessionConfig,
  getLocalConfig,
  setLocalConfig,
  getWebConfig,
  setWebConfig,
  type SessionConfig,
  type LocalConfig,
  type WebConfig,
} from '../services/config'

const activeSection = ref('appearance')
const saving = ref(false)
const message = ref<{ type: string; text: string } | null>(null)

const sections = [
  { id: 'appearance', icon: '🎨', label: '外观' },
  { id: 'session', icon: '💬', label: '会话设置' },
  { id: 'local', icon: '🔧', label: '本地工具' },
  { id: 'web', icon: '🌐', label: '网络工具' },
  { id: 'about', icon: 'ℹ️', label: '关于' },
]
// 外观设置
const settings = reactive({
  theme: 'light',
  fontSize: 'medium',
})

// 会话设置
const sessionConfig = reactive<SessionConfig>({
  storage_dir: '',
  max_messages: 100,
  auto_compress: true,
  compress_threshold: 50,
  context_messages: 6,
  default_agent_id: 'default_assistant',
})

// 本地工具设置
const localConfig = reactive<LocalConfig>({
  shell_enabled: true,
  file_enabled: true,
  shell_timeout: 60,
})

// 网络工具设置
const webConfig = reactive<WebConfig>({
  web_enabled: true,
  web_timeout: 300,
  tavily_api_key: '',
  serper_api_key: '',
})

function showMessage(type: string, text: string) {
  message.value = { type, text }
  setTimeout(() => { message.value = null }, 3000)
}

async function loadConfigs() {
  try {
    const sessCfg = await getSessionConfig()
    if (sessCfg) Object.assign(sessionConfig, sessCfg)

    const localCfg = await getLocalConfig()
    if (localCfg) Object.assign(localConfig, localCfg)

    const webCfg = await getWebConfig()
    if (webCfg) Object.assign(webConfig, webCfg)
  } catch (err) {
    logger.error('SettingsPage', '加载配置失败', err)
  }
}

async function saveSessionConfig() {
  saving.value = true
  try {
    await setSessionConfig(sessionConfig)
    showMessage('success', '会话配置已保存')
  } catch (err) {
    showMessage('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}

async function saveLocalConfig() {
  saving.value = true
  try {
    await setLocalConfig(localConfig)
    showMessage('success', '本地工具配置已保存')
  } catch (err) {
    showMessage('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}

async function saveWebConfig() {
  saving.value = true
  try {
    await setWebConfig(webConfig)
    showMessage('success', '网络工具配置已保存')
  } catch (err) {
    showMessage('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}

onMounted(() => loadConfigs())
</script>

<style scoped>
/* 保持原有样式不变 */
.settings-page { display: flex; height: 100%; width: 100%; }
.settings-nav { width: 200px; background: var(--color-surface); border-right: 1px solid var(--color-border); display: flex; flex-direction: column; flex-shrink: 0; }
.nav-header { padding: 1rem; border-bottom: 1px solid var(--color-border); }
.nav-header h3 { font-size: 0.875rem; font-weight: 600; color: var(--color-text-secondary); }
.nav-items { padding: 0.5rem; }
.nav-item { width: 100%; display: flex; align-items: center; gap: 0.75rem; padding: 0.75rem 1rem; border: none; background: transparent; border-radius: 8px; cursor: pointer; text-align: left; transition: background 0.2s; }
.nav-item:hover { background: #f0f0f0; }
.nav-item.active { background: #e8e8f0; }
.nav-icon { font-size: 1rem; }
.nav-label { font-size: 0.875rem; color: var(--color-text); }
.settings-content { flex: 1; padding: 2rem 3rem; overflow-y: auto; }
.content-section h2 { font-size: 1.25rem; margin-bottom: 1.5rem; }
.setting-group { background: var(--color-surface); border-radius: 12px; padding: 0.5rem; }
.message { padding: 0.75rem 1rem; margin-bottom: 1rem; border-radius: 8px; font-size: 0.875rem; }
.message.success { background: #d4edda; color: #155724; }
.message.error { background: #f8d7da; color: #721c24; }
.setting-item { display: flex; align-items: center; padding: 1rem; border-radius: 8px; }
.setting-item:hover { background: #fafafa; }
.setting-info { flex: 1; }
.setting-info label { display: block; font-weight: 500; margin-bottom: 0.25rem; }
.setting-desc { font-size: 0.75rem; color: var(--color-text-muted); margin: 0; }
.active-provider-summary {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  color: var(--color-text);
}
.provider-pill {
  background: #eef2ff;
  color: #4338ca;
  padding: 0.25rem 0.6rem;
  border-radius: 6px;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 0.8rem;
}
.provider-divider { color: var(--color-text-muted); }
.provider-model { font-size: 0.85rem; }
.setting-item select, .setting-item input[type="text"], .setting-item input[type="password"], .setting-item input[type="number"] { padding: 0.5rem 0.75rem; border: 1px solid var(--color-border); border-radius: 6px; min-width: 200px; font-size: 0.875rem; }
.setting-item input[type="number"] { width: 100px; min-width: auto; }
.toggle { position: relative; display: inline-block; width: 48px; height: 24px; }
.toggle input { opacity: 0; width: 0; height: 0; }
.toggle-slider { position: absolute; cursor: pointer; top: 0; left: 0; right: 0; bottom: 0; background-color: #ccc; transition: 0.3s; border-radius: 24px; }
.toggle-slider::before { position: absolute; content: ""; height: 18px; width: 18px; left: 3px; bottom: 3px; background-color: white; transition: 0.3s; border-radius: 50%; }
.toggle input:checked + .toggle-slider { background-color: var(--color-primary); }
.toggle input:checked + .toggle-slider::before { transform: translateX(24px); }
.action-btn { padding: 0.5rem 1rem; background: var(--color-primary); color: white; border: none; border-radius: 6px; cursor: pointer; font-size: 0.875rem; }
.action-btn:disabled { opacity: 0.6; cursor: not-allowed; }
.setting-divider { height: 1px; background: var(--color-border); margin: 1rem 0; }
.about-info { text-align: center; padding: 3rem; }
.app-logo { width: 80px; height: 80px; border-radius: 16px; margin-bottom: 1rem; }
.about-info h1 { font-size: 2rem; margin-bottom: 0.5rem; }
.version { color: var(--color-text-muted); margin-bottom: 1rem; }
.description { color: var(--color-text-secondary); margin-bottom: 2rem; }
.links { display: flex; gap: 1rem; justify-content: center; }
.link-btn { padding: 0.5rem 1.5rem; background: var(--color-surface); border: 1px solid var(--color-border); border-radius: 6px; color: var(--color-text); text-decoration: none; transition: background 0.2s; }
.link-btn:hover { background: #f0f0f0; }
.recent-list { display: flex; flex-direction: column; gap: 0.25rem; max-width: 300px; }
.recent-item { font-size: 0.75rem; color: var(--color-text-muted); padding: 0.25rem 0.5rem; background: #f5f5f5; border-radius: 4px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.section-desc { color: var(--color-text-muted); font-size: 0.875rem; margin-bottom: 1rem; }
.mcp-servers { display: flex; flex-direction: column; gap: 0.75rem; }
.mcp-server-card { background: var(--color-surface); border: 1px solid var(--color-border); border-radius: 8px; padding: 1rem; }
.server-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.5rem; }
.server-name { font-weight: 600; font-size: 0.95rem; }
.server-actions { display: flex; align-items: center; gap: 0.5rem; }
.server-info { display: flex; flex-direction: column; gap: 0.25rem; }
.server-info code { background: #f0f0f0; padding: 0.25rem 0.5rem; border-radius: 4px; font-size: 0.8rem; }
.server-info .args { color: var(--color-text-muted); font-size: 0.75rem; }
.icon-btn { background: transparent; border: none; cursor: pointer; font-size: 0.875rem; padding: 0.25rem; border-radius: 4px; }
.icon-btn:hover { background: #f0f0f0; }
.icon-btn.danger:hover { background: #fee; }
.add-server-btn { padding: 0.75rem; border: 2px dashed var(--color-border); border-radius: 8px; background: transparent; color: var(--color-text-muted); cursor: pointer; font-size: 0.875rem; transition: all 0.2s; }
.add-server-btn:hover { border-color: var(--color-primary); color: var(--color-primary); }
.toggle.small { width: 36px; height: 18px; }
.toggle.small .toggle-slider::before { height: 12px; width: 12px; }
.toggle.small input:checked + .toggle-slider::before { transform: translateX(18px); }
.dialog-overlay { position: fixed; top: 0; left: 0; right: 0; bottom: 0; background: rgba(0, 0, 0, 0.5); display: flex; align-items: center; justify-content: center; z-index: 1000; }
.dialog { background: white; border-radius: 12px; padding: 1.5rem; width: 100%; max-width: 480px; max-height: 90vh; overflow-y: auto; }
.dialog h3 { margin-bottom: 1rem; }
.form-group { margin-bottom: 1rem; }
.form-group label { display: block; font-size: 0.875rem; font-weight: 500; margin-bottom: 0.25rem; }
.form-group input, .form-group textarea { width: 100%; padding: 0.5rem 0.75rem; border: 1px solid var(--color-border); border-radius: 6px; font-size: 0.875rem; box-sizing: border-box; }
.form-group textarea { font-family: monospace; }
.checkbox-label { display: flex !important; align-items: center; gap: 0.5rem; cursor: pointer; }
.checkbox-label input { width: auto; }
.dialog-actions { display: flex; justify-content: flex-end; gap: 0.75rem; margin-top: 1.5rem; }
.action-btn.secondary { background: #f0f0f0; color: var(--color-text); }
</style>
