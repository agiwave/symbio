<!--
  SessionConfigForm — 会话设置 editor（setting:session）

  数据通道：session/config get/set（editor 自持保存，不走统一资源 upload）。
-->
<template>
  <SettingsFormShell title="会话设置" description="控制会话存储与上下文行为">
    <div class="setting-item">
      <div class="setting-info">
        <label>最大消息数</label>
        <p class="setting-desc">每个会话保存的最大消息数量</p>
      </div>
      <input v-model.number="config.max_messages" type="number" min="10" max="1000" />
    </div>

    <div class="setting-item">
      <div class="setting-info">
        <label>自动压缩</label>
        <p class="setting-desc">当消息数超过阈值时自动压缩历史</p>
      </div>
      <label class="toggle">
        <input type="checkbox" v-model="config.auto_compress" />
        <span class="toggle-slider"></span>
      </label>
    </div>

    <div class="setting-item">
      <div class="setting-info">
        <label>压缩阈值</label>
        <p class="setting-desc">触发自动压缩的消息数量</p>
      </div>
      <input v-model.number="config.compress_threshold" type="number" min="10" max="500" />
    </div>

    <div class="setting-item">
      <div class="setting-info">
        <label>上下文消息数量</label>
        <p class="setting-desc">Model 对话时包含的上下文消息数量（0 表示不限制，6 表示 3 轮对话）</p>
      </div>
      <input v-model.number="config.context_messages" type="number" min="0" max="200" />
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
import { getSessionConfig, setSessionConfig, type SessionConfig } from '@/services/config'
import { useToast } from '@/composables/useToast'
import { logger } from '@/utils/logger'

defineProps<{
  /** 当前设置分区资源项（统一资源协议注入） */
  item?: { id: string; name?: string } | null
}>()

const toast = useToast()
const saving = ref(false)

const config = reactive<SessionConfig>({
  storage_dir: '',
  max_messages: 100,
  auto_compress: true,
  compress_threshold: 50,
  context_messages: 6,
})

onMounted(async () => {
  try {
    const cfg = await getSessionConfig()
    if (cfg) Object.assign(config, cfg)
  } catch (err) {
    logger.error('SessionConfigForm', '加载会话配置失败', err)
    toast.showToast('error', '加载会话配置失败')
  }
})

async function save() {
  saving.value = true
  try {
    await setSessionConfig(config)
    toast.showToast('success', '会话配置已保存')
  } catch (err) {
    toast.showToast('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}
</script>
