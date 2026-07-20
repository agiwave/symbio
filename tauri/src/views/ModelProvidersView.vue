<!--
  ModelProvidersView — 基于 ResourceShell 的两栏管理页
-->
<template>
  <ResourceShell
    title="Model Provider"
    :loading="loading"
    :has-list-content="providers.length > 0"
    @new="enterCreateMode"
  >
    <template #meta v-if="defaultProvider">
      <span class="running-pulse" />
      默认：{{ defaultProvider.name || defaultProvider.id }}
    </template>

    <template #list>
      <div class="provider-list">
        <ModelProviderCard
          v-for="p in providers"
          :key="p.id"
          :provider="p"
          :is-active="selectedId === p.id"
          :is-default="defaultProviderId === p.id"
          @click="select(p.id)"
        />
      </div>
    </template>

    <template #empty>
      <p>暂无 Provider</p>
      <p class="hint">点击右上角 + 创建新 Provider</p>
    </template>

    <template #detail>
      <ModelProvidersSettings
        :provider="selectedProvider"
        :is-default="selectedProvider ? defaultProviderId === selectedProvider.id : false"
        :saving="saving"
        :testing="testing"
        :deleting="deletingId === selectedId"
        @save="handleSave"
        @test="handleTest"
        @delete="handleDelete"
        @set-default="handleSetDefault"
      />
    </template>

    <template #toast>
      <Transition name="toast">
        <div v-if="toast" :class="['toast', toast.type]" @click="toast = null">
          {{ toast.text }}
        </div>
      </Transition>
    </template>
  </ResourceShell>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { callPlugin } from '@/services/plugin'
import {
  listModelProviders,
  setModelProvider,
  deleteModelProvider,
  setDefaultModelProvider
} from '@/services/modelProviders'
import { providerPresets } from '@/constants/modelProviders'
import type { ModelProviderConfig, ModelProvidersConfig } from '@/schemas/model_providers'
import { useResourceManager } from '@/composables/useResourceManager'
import ModelProvidersSettings from '../components/settings/ModelProvidersSettings.vue'
import ModelProviderCard from '../components/settings/ModelProviderCard.vue'
import ResourceShell from '../components/common/ResourceShell.vue'

const {
  loading,
  saving,
  testing,
  creating,
  selectedId,
  deletingId,
  toast,
  showToast,
  enterCreateMode,
  select,
  markDeleting,
} = useResourceManager({ logTag: 'ModelProvidersView' })

// === 状态 ===
const providers = ref<ModelProviderConfig[]>([])
const defaultProviderId = ref<string | null>(null)

// === 计算属性 ===
const defaultProvider = computed(() =>
  providers.value.find((p) => p.id === defaultProviderId.value) ?? null
)

const selectedProvider = computed<ModelProviderConfig | null>(() => {
  if (creating.value) {
    return {
      id: '',
      name: '',
      provider: 'openai',
      api_base: providerPresets['openai']?.apiBase || '',
      api_key: '',
      model: '',
      temperature: 0.7,
      max_tokens: 4096,
      api_protocol: 'openai_responses',
      rate_limit_ms: 0,
      enabled: true
    }
  }
  if (!selectedId.value) return null
  return providers.value.find((p) => p.id === selectedId.value) ?? null
})

// === 操作 ===
async function loadAll() {
  loading.value = true
  try {
    const cfg: ModelProvidersConfig = await listModelProviders()
    providers.value = Object.values(cfg.providers ?? {})
    defaultProviderId.value = cfg.default_provider_id ?? null

    if (!selectedId.value && !creating.value) {
      selectedId.value = providers.value[0]?.id ?? null
    }
  } catch (err) {
    showToast('error', `加载失败: ${err}`)
  } finally {
    loading.value = false
  }
}

async function handleSave(payload: { provider: ModelProviderConfig; skipValidation: boolean }) {
  const p = payload.provider
  if (!p.id) {
    showToast('error', '请填写 Provider ID')
    return
  }
  saving.value = true
  try {
    if (creating.value) {
      if (providers.value.some((x) => x.id === p.id)) {
        showToast('error', `ID "${p.id}" 已存在，请换一个`)
        return
      }
    }
    const saved = await setModelProvider(p, { skipValidation: payload.skipValidation })
    showToast('success', `Provider ${saved.id} 已保存`)
    creating.value = false
    selectedId.value = saved.id
    await loadAll()
  } catch (err) {
    showToast('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}

async function handleTest(provider: ModelProviderConfig) {
  testing.value = true
  try {
    await callPlugin('model_providers/test', { provider, skip_validation: false })
    showToast('success', '连接校验通过')
  } catch (err) {
    showToast('error', `校验失败: ${err}`)
  } finally {
    testing.value = false
  }
}

async function handleDelete(p: ModelProviderConfig) {
  if (!confirm(`确认删除 Provider 「${p.name || p.id}」？`)) return
  markDeleting(p.id)
  try {
    await deleteModelProvider(p.id)
    showToast('success', `Provider ${p.id} 已删除`)
    if (selectedId.value === p.id) {
      selectedId.value = providers.value.find((x) => x.id !== p.id)?.id ?? null
    }
    await loadAll()
  } catch (err) {
    showToast('error', `删除失败: ${err}`)
  } finally {
    markDeleting(null)
  }
}

async function handleSetDefault(providerId: string) {
  try {
    await setDefaultModelProvider(providerId)
    showToast('success', `已设置默认 Provider: ${providerId}`)
    await loadAll()
  } catch (err) {
    showToast('error', `设置失败: ${err}`)
  }
}

onMounted(() => loadAll())
</script>

<style scoped>
.provider-list {
  flex: 1;
  overflow-y: auto;
  padding: 0.25rem 0;
}

/* Toast */
.toast {
  position: absolute;
  bottom: 1.5rem;
  left: 50%;
  transform: translateX(-50%);
  padding: 0.55rem 1rem;
  border-radius: 6px;
  font-size: 0.85rem;
  cursor: pointer;
  z-index: 100;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
  color: #fff;
  background: #22c55e;
}
.toast.error { background: #ef4444; }
.toast.info { background: #4f46e5; }

.toast-enter-active,
.toast-leave-active {
  transition: all 0.2s ease;
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateX(-50%) translateY(10px);
}
</style>
