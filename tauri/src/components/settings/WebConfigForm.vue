<!--
  WebConfigForm — 网络工具设置 editor（setting:web）

  数据通道：web/config get/set（editor 自持保存，不走统一资源 upload）。
-->
<template>
  <SettingsFormShell title="网络工具设置" description="控制 Web 工具的启用、超时与搜索服务凭据">
    <div class="setting-item">
      <div class="setting-info">
        <label>启用 Web 工具</label>
        <p class="setting-desc">允许网络请求</p>
      </div>
      <label class="toggle">
        <input type="checkbox" v-model="config.web_enabled" />
        <span class="toggle-slider"></span>
      </label>
    </div>

    <div class="setting-item">
      <div class="setting-info">
        <label>Web 超时（秒）</label>
        <p class="setting-desc">Web 请求超时时间</p>
      </div>
      <input v-model.number="config.web_timeout" type="number" min="1" max="300" />
    </div>

    <div class="setting-item">
      <div class="setting-info">
        <label>Tavily API Key</label>
        <p class="setting-desc">用于高级网页搜索（优先）</p>
      </div>
      <input v-model="config.tavily_api_key" type="password" placeholder="输入 Tavily API Key" />
    </div>

    <div class="setting-item">
      <div class="setting-info">
        <label>Serper API Key</label>
        <p class="setting-desc">用于 Google 网页搜索（备用）</p>
      </div>
      <input v-model="config.serper_api_key" type="password" placeholder="输入 Serper API Key" />
    </div>

    <template #footer>
      <button class="action-btn" :disabled="saving" @click="save">
        {{ saving ? '保存中…' : '保存配置' }}
      </button>
    </template>
  </SettingsFormShell>
</template>

<script setup lang="ts">
import { onMounted, reactive, ref } from 'vue'
import SettingsFormShell from './SettingsFormShell.vue'
import { getWebConfig, setWebConfig, type WebConfig } from '@/services/config'
import { useToast } from '@/composables/useToast'
import { logger } from '@/utils/logger'

defineProps<{
  /** 当前设置分区资源项（统一资源协议注入） */
  item?: { id: string; name?: string } | null
}>()

const toast = useToast()
const saving = ref(false)

const config = reactive<WebConfig>({
  web_enabled: true,
  web_timeout: 300,
  tavily_api_key: '',
  serper_api_key: '',
})

onMounted(async () => {
  try {
    const cfg = await getWebConfig()
    if (cfg) Object.assign(config, cfg)
  } catch (err) {
    logger.error('WebConfigForm', '加载网络工具配置失败', err)
    toast.showToast('error', '加载网络工具配置失败')
  }
})

async function save() {
  saving.value = true
  try {
    await setWebConfig(config)
    toast.showToast('success', '网络工具配置已保存')
  } catch (err) {
    toast.showToast('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}
</script>
