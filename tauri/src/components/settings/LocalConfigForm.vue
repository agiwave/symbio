<!--
  LocalConfigForm — 本地工具设置 editor（setting:local）

  数据通道：local/config get/set（editor 自持保存，不走统一资源 upload）。
-->
<template>
  <SettingsFormShell title="本地工具设置" description="控制本地 Shell / 文件工具的启用与超时">
    <div class="setting-item">
      <div class="setting-info">
        <label>启用 Shell 工具</label>
        <p class="setting-desc">允许执行 Shell 命令</p>
      </div>
      <label class="toggle">
        <input type="checkbox" v-model="config.shell_enabled" />
        <span class="toggle-slider"></span>
      </label>
    </div>

    <div class="setting-item">
      <div class="setting-info">
        <label>启用文件工具</label>
        <p class="setting-desc">允许文件读写操作</p>
      </div>
      <label class="toggle">
        <input type="checkbox" v-model="config.file_enabled" />
        <span class="toggle-slider"></span>
      </label>
    </div>

    <div class="setting-item">
      <div class="setting-info">
        <label>Shell 超时（秒）</label>
        <p class="setting-desc">Shell 命令执行超时时间</p>
      </div>
      <input v-model.number="config.shell_timeout" type="number" min="1" max="3600" />
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
import { getLocalConfig, setLocalConfig, type LocalConfig } from '@/services/config'
import { useToast } from '@/composables/useToast'
import { logger } from '@/utils/logger'

defineProps<{
  /** 当前设置分区资源项（统一资源协议注入） */
  item?: { id: string; name?: string } | null
}>()

const toast = useToast()
const saving = ref(false)

const config = reactive<LocalConfig>({
  shell_enabled: true,
  file_enabled: true,
  shell_timeout: 60,
})

onMounted(async () => {
  try {
    const cfg = await getLocalConfig()
    if (cfg) Object.assign(config, cfg)
  } catch (err) {
    logger.error('LocalConfigForm', '加载本地工具配置失败', err)
    toast.showToast('error', '加载本地工具配置失败')
  }
})

async function save() {
  saving.value = true
  try {
    await setLocalConfig(config)
    toast.showToast('success', '本地工具配置已保存')
  } catch (err) {
    toast.showToast('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}
</script>
