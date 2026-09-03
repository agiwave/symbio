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
      <div class="provider-list" role="listbox" aria-label="Model Provider 列表">
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
  </ResourceShell>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import {
  listModelProviders,
  setModelProvider,
  deleteModelProvider,
  setDefaultModelProvider,
  testModelProvider,
  generateUniqueProviderId
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
  const p = { ...payload.provider }
  // 新建时自动生成唯一 ID（对用户不可见），无须手动填写
  if (creating.value) {
    p.id = generateUniqueProviderId(
      p.name || p.model || p.provider,
      providers.value.map((x) => x.id)
    )
  }
  if (!p.id) {
    showToast('error', '无法生成 Provider ID')
    return
  }
  saving.value = true
  try {
    const saved = await setModelProvider(p, { skipValidation: payload.skipValidation })
    showToast('success', `Provider「${saved.name || saved.id}」已保存`)
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
    await testModelProvider(provider)
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

</style>
