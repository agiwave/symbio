<template>
  <div class="settings-view">
    <header class="settings-header">
      <button class="back-btn" @click="goBack">← 返回</button>
      <h1>设置</h1>
    </header>
    
    <main class="settings-content">
      <section class="settings-section">
        <h2>外观</h2>
        <div class="setting-item">
          <label>主题</label>
          <select v-model="settings.theme">
            <option value="light">浅色</option>
            <option value="dark">深色</option>
            <option value="auto">跟随系统</option>
          </select>
        </div>
      </section>
      
      <section class="settings-section">
        <h2>AI 设置</h2>
        <div class="setting-item">
          <label>LLM 提供商</label>
          <select v-model="settings.llmProvider">
            <option value="openai">OpenAI</option>
            <option value="claude">Claude</option>
            <option value="local">本地模型</option>
          </select>
        </div>
        <div class="setting-item">
          <label>API Key</label>
          <input v-model="settings.apiKey" type="password" placeholder="输入 API Key" />
        </div>
      </section>
      
      <section class="settings-section">
        <h2>执行环境</h2>
        <div class="setting-item">
          <label>Docker 镜像</label>
          <input v-model="settings.dockerImage" placeholder="symbio/bioinfo:latest" />
        </div>
        <div class="setting-item">
          <label>资源限制</label>
          <div class="resource-limits">
            <input v-model.number="settings.cpuLimit" type="number" min="1" max="16" />
            <span>CPU 核心</span>
            <input v-model.number="settings.memoryLimit" type="number" min="1" max="64" />
            <span>GB 内存</span>
          </div>
        </div>
      </section>
    </main>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive } from 'vue'
import { useRouter } from 'vue-router'

const router = useRouter()

const settings = reactive({
  theme: 'light',
  llmProvider: 'openai',
  apiKey: '',
  dockerImage: 'symbio/bioinfo:latest',
  cpuLimit: 4,
  memoryLimit: 8,
})

function goBack() {
  router.back()
}
</script>

<style scoped>
.settings-view {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: var(--color-bg);
}

.settings-header {
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 1rem 2rem;
  background: var(--color-surface);
  border-bottom: 1px solid var(--color-border);
}

.back-btn {
  padding: 0.5rem 1rem;
  background: transparent;
  border: 1px solid var(--color-border);
  border-radius: 6px;
  cursor: pointer;
}

.settings-header h1 {
  font-size: 1.25rem;
}

.settings-content {
  flex: 1;
  padding: 2rem;
  max-width: 600px;
  overflow-y: auto;
}

.settings-section {
  background: var(--color-surface);
  border-radius: 8px;
  padding: 1.5rem;
  margin-bottom: 1.5rem;
}

.settings-section h2 {
  font-size: 1rem;
  margin-bottom: 1rem;
  padding-bottom: 0.5rem;
  border-bottom: 1px solid var(--color-border);
}

.setting-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 1rem;
}

.setting-item:last-child {
  margin-bottom: 0;
}

.setting-item label {
  color: var(--color-text-secondary);
}

.setting-item select,
.setting-item input[type="text"],
.setting-item input[type="password"] {
  padding: 0.5rem 0.75rem;
  border: 1px solid var(--color-border);
  border-radius: 6px;
  min-width: 200px;
}

.resource-limits {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.resource-limits input {
  width: 60px;
  padding: 0.5rem;
  border: 1px solid var(--color-border);
  border-radius: 6px;
  text-align: center;
}

.resource-limits span {
  color: var(--color-text-muted);
  font-size: 0.875rem;
}
</style>
