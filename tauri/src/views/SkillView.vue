<!--
  SkillView — 技能浏览页（只读）

  设计原则：
  - Skill 是**文件系统级**概念：用户在 skill_dirs 中放置 SKILL.md 文件即可
  - 因此 SkillView 不提供"新建/编辑/删除"按钮（这些操作由文件系统完成）
  - 仅提供"列出 / 查看"功能
  - 按 source（来源）分组展示：工作区 / 系统 / 第三方

  BUG-FR8：检测并提示同名 skill（不同目录出现同名时标记）
  BUG-FR9：详情面板展示 SKILL.md body 预览（折叠/展开）
-->
<template>
  <ResourceShell
    title="Skill"
    :loading="loading"
    :has-list-content="skills.length > 0"
    :hide-default-new="true"
  >
    <template #header-actions>
      <!-- 只读视图：仅"刷新"按钮 -->
      <button
        class="icon-btn"
        title="刷新"
        :disabled="loading"
        @click="loadAll"
      >
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
          <polyline points="21 3 21 8 16 8" />
        </svg>
      </button>
    </template>

    <template #meta v-if="skills.length > 0">
      <span class="running-pulse" />
      共 {{ skills.length }} 个技能
      <span v-if="duplicateNames.length > 0" class="dup-warning" :title="dupTooltip">
        ⚠ {{ duplicateNames.length }} 个重名
      </span>
    </template>

    <template #list>
      <div class="skill-list">
        <ResourceCard
          v-for="skill in skills"
          :key="skill.file_path"
          :title="skill.name"
          :subtitle="skill.description"
          :status="skill.argument_hint ? 'warning' : 'active'"
          :status-title="skill.argument_hint ? '需要参数' : '可用'"
          :badge="sourceLabel(skill.source)"
          :badge-kind="sourceBadgeKind(skill.source)"
          :is-active="selectedName === skill.name"
          @click="selectedName = skill.name"
        >
          <template #meta>
            <span v-if="skill.argument_hint" class="tag tag-warn">需参数</span>
            <!-- BUG-FR8：重复名称提示 -->
            <span v-if="isDuplicate(skill.name)" class="tag tag-error" :title="dupNameTooltip(skill.name)">
              ⚠ 重名
            </span>
            <span class="tag tag-muted" :title="skill.file_path">{{ shortPath(skill.file_path) }}</span>
          </template>
        </ResourceCard>
      </div>
    </template>

    <template #empty>
      <p>暂无 Skill</p>
      <p class="hint">在 <code>~/.symbio/plugins/skills/</code> 或 <code>.symbio/skills/</code> 中放置 SKILL.md</p>
    </template>

    <template #detail>
      <div v-if="selectedSkill" class="skill-detail">
        <header class="detail-header">
          <h2 class="detail-title">{{ selectedSkill.name }}</h2>
          <span class="detail-badge" :class="`kind-${sourceBadgeKind(selectedSkill.source)}`">
            {{ sourceLabel(selectedSkill.source) }}
          </span>
        </header>
        <p class="detail-description">{{ selectedSkill.description }}</p>
        <div class="detail-section">
          <label>文件路径</label>
          <code class="detail-code">{{ selectedSkill.file_path }}</code>
        </div>
        <div v-if="selectedSkill.when_to_use" class="detail-section">
          <label>使用场景</label>
          <p class="detail-text">{{ selectedSkill.when_to_use }}</p>
        </div>
        <div v-if="selectedSkill.argument_hint" class="detail-section">
          <label>参数提示</label>
          <p class="detail-text">{{ selectedSkill.argument_hint }}</p>
        </div>

        <!-- BUG-FR9：SKILL.md body 预览 -->
        <div class="detail-section">
          <div class="body-section-header">
            <label>SKILL.md 预览</label>
            <div class="body-actions">
              <span v-if="bodyLoading" class="body-loading">加载中…</span>
              <span v-else-if="bodyChars > 0" class="body-meta">
                {{ bodyChars }} 字符
                <span v-if="bodyTruncated" class="body-truncated-tag">已截断</span>
              </span>
              <button
                v-if="bodyText && bodyText.length > BODY_PREVIEW_LIMIT"
                class="body-toggle"
                @click="bodyExpanded = !bodyExpanded"
              >
                {{ bodyExpanded ? '收起' : `展开 (${bodyText.length} 字符)` }}
              </button>
            </div>
          </div>
          <pre v-if="bodyText" class="body-preview" :class="{ collapsed: !bodyExpanded }">{{ displayedBody }}</pre>
          <p v-else class="body-empty">{{ bodyError || '（无 body 内容）' }}</p>
        </div>

        <div class="detail-section">
          <label class="muted">提示：Skill 是只读资源。如需新增/修改，请在文件系统中编辑 SKILL.md 文件后点击"刷新"。</label>
        </div>
      </div>
      <div v-else class="no-selection">
        <p>← 选择一个 Skill 查看详情</p>
      </div>
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
import { computed, onMounted, ref, watch } from 'vue'
import { listSkills, getSkill, sourceLabel } from '@/services/skills'
import { useResourceManager } from '@/composables/useResourceManager'
import type { SkillInfo } from '@/schemas/skill_list'
import type { SkillGet } from '@/schemas/skill_get'
import ResourceShell from '../components/common/ResourceShell.vue'
import ResourceCard from '../components/common/ResourceCard.vue'

const {
  loading,
  selectedId: selectedName,
  toast,
  showToast,
  select,
} = useResourceManager({ logTag: 'SkillView' })

// === 状态 ===
const skills = ref<SkillInfo[]>([])
// BUG-FR9：当前选中 skill 的 body 详情
const bodyLoading = ref(false)
const bodyText = ref<string | null>(null)
const bodyChars = ref(0)
const bodyTruncated = ref(false)
const bodyError = ref<string | null>(null)
const bodyExpanded = ref(false)
// 默认折叠时只显示前 N 字符，避免超长 body 占满详情面板
const BODY_PREVIEW_LIMIT = 800

// === 计算属性 ===
const selectedSkill = computed(() => {
  if (!selectedName.value) return null
  return skills.value.find((s) => s.name === selectedName.value) ?? null
})

// BUG-FR8：检测重复 name
const duplicateNames = computed<string[]>(() => {
  const counts = new Map<string, number>()
  for (const s of skills.value) {
    counts.set(s.name, (counts.get(s.name) ?? 0) + 1)
  }
  return [...counts.entries()].filter(([, n]) => n > 1).map(([name]) => name)
})

const dupTooltip = computed(() =>
  duplicateNames.value.length === 0
    ? ''
    : `重名 skill: ${duplicateNames.value.join(', ')}（后加载的覆盖之前的同名 skill）`
)

const displayedBody = computed(() => {
  if (!bodyText.value) return ''
  if (bodyExpanded.value) return bodyText.value
  if (bodyText.value.length <= BODY_PREVIEW_LIMIT) return bodyText.value
  return bodyText.value.slice(0, BODY_PREVIEW_LIMIT) + '\n\n…（已折叠）'
})

// === 方法 ===
function sourceBadgeKind(source: string): 'default' | 'primary' | 'info' | undefined {
  switch (source) {
    case 'workspace': return 'primary'
    case 'system': return 'info'
    case 'external': return 'default'
    default: return undefined
  }
}

function shortPath(p: string): string {
  // 取最后两级目录
  const normalized = p.replace(/\\/g, '/')
  const parts = normalized.split('/').filter(Boolean)
  return parts.slice(-2).join('/')
}

function isDuplicate(name: string): boolean {
  return duplicateNames.value.includes(name)
}

function dupNameTooltip(name: string): string {
  const files = skills.value
    .filter((s) => s.name === name)
    .map((s) => s.file_path)
  return `同名 skill 出现在多个目录：\n${files.join('\n')}`
}

async function loadAll() {
  loading.value = true
  try {
    // SkillView 是全局管理视图，不传 workdir
    // 系统级 skill（~/.symbio/plugins/skills）会通过 ~ 展开加载，与会话目录无关
    skills.value = await listSkills()
    if (!selectedName.value && skills.value.length > 0) {
      select(skills.value[0].name)
    }
  } catch (err) {
    showToast('error', `加载失败: ${err}`)
  } finally {
    loading.value = false
  }
}

// BUG-FR9：加载选中 skill 的 body
async function loadBody(name: string) {
  bodyLoading.value = true
  bodyError.value = null
  bodyText.value = null
  bodyChars.value = 0
  bodyTruncated.value = false
  bodyExpanded.value = false
  try {
    // 同 loadAll，不传 workdir，由后端按系统目录加载
    const detail: SkillGet.Response | null = await getSkill(name)
    if (!detail) {
      bodyError.value = '加载失败'
      return
    }
    bodyText.value = detail.body
    bodyChars.value = detail.body_chars
    bodyTruncated.value = detail.body_truncated
  } catch (err) {
    bodyError.value = `加载失败: ${err}`
  } finally {
    bodyLoading.value = false
  }
}

// 选中变化时重新拉取 body
watch(selectedName, (newName) => {
  if (newName) {
    loadBody(newName)
  } else {
    bodyText.value = null
    bodyChars.value = 0
    bodyTruncated.value = false
  }
})

onMounted(() => loadAll())
</script>

<style scoped>
.skill-list {
  flex: 1;
  overflow-y: auto;
  padding: 0.25rem 0;
}

.skill-detail {
  flex: 1;
  padding: 1.5rem 2rem;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.detail-header {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding-bottom: 0.5rem;
  border-bottom: 1px solid var(--color-border, #e5e7eb);
}

.detail-title {
  font-size: 1.25rem;
  font-weight: 600;
  margin: 0;
  color: var(--color-text, #1f2937);
}

.detail-badge {
  font-size: 0.7rem;
  padding: 0.2rem 0.5rem;
  border-radius: 4px;
  background: rgba(102, 126, 234, 0.1);
  color: var(--color-primary, #667eea);
}
.detail-badge.kind-info { background: rgba(59, 130, 246, 0.1); color: #3b82f6; }
.detail-badge.kind-muted { background: rgba(0, 0, 0, 0.04); color: #6b7280; }

.detail-description {
  font-size: 0.95rem;
  color: var(--color-text, #1f2937);
  margin: 0;
  line-height: 1.5;
}

.detail-section {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.detail-section label {
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--color-text-muted, #6b7280);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}
.detail-section label.muted {
  text-transform: none;
  letter-spacing: 0;
  font-weight: 400;
  font-style: italic;
  color: var(--color-text-muted, #6b7280);
}

.detail-code {
  display: block;
  padding: 0.5rem 0.75rem;
  background: rgba(0, 0, 0, 0.04);
  border-radius: 6px;
  font-size: 0.8rem;
  font-family: 'Menlo', 'Monaco', monospace;
  word-break: break-all;
  color: var(--color-text, #1f2937);
}

.detail-text {
  font-size: 0.85rem;
  color: var(--color-text, #1f2937);
  margin: 0;
  line-height: 1.5;
}

/* BUG-FR9：body 预览样式 */
.body-section-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
}

.body-actions {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.body-meta {
  font-size: 0.7rem;
  color: var(--color-text-muted, #6b7280);
}

.body-loading {
  font-size: 0.7rem;
  color: var(--color-text-muted, #6b7280);
  font-style: italic;
}

.body-truncated-tag {
  display: inline-block;
  margin-left: 0.4rem;
  padding: 0.1rem 0.4rem;
  background: rgba(245, 158, 11, 0.15);
  color: #b45309;
  border-radius: 3px;
  font-size: 0.65rem;
  font-weight: 600;
}

.body-toggle {
  padding: 0.2rem 0.6rem;
  font-size: 0.7rem;
  border: 1px solid var(--color-border, #e5e7eb);
  background: var(--color-bg, #fff);
  border-radius: 4px;
  cursor: pointer;
  color: var(--color-text, #1f2937);
}
.body-toggle:hover { background: rgba(0, 0, 0, 0.04); }

.body-preview {
  display: block;
  padding: 0.75rem 1rem;
  background: rgba(0, 0, 0, 0.04);
  border-radius: 6px;
  font-size: 0.8rem;
  font-family: 'Menlo', 'Monaco', monospace;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--color-text, #1f2937);
  max-height: 480px;
  overflow-y: auto;
  margin: 0;
  line-height: 1.5;
}
.body-preview.collapsed {
  max-height: 200px;
  position: relative;
  mask-image: linear-gradient(to bottom, black 70%, transparent 100%);
  -webkit-mask-image: linear-gradient(to bottom, black 70%, transparent 100%);
}

.body-empty {
  font-size: 0.85rem;
  color: var(--color-text-muted, #6b7280);
  margin: 0;
  font-style: italic;
}

/* BUG-FR8：重名提示 */
.dup-warning {
  margin-left: 0.5rem;
  padding: 0.1rem 0.5rem;
  background: rgba(239, 68, 68, 0.1);
  color: #b91c1c;
  border-radius: 4px;
  font-size: 0.7rem;
  font-weight: 600;
}

.tag.tag-error {
  background: rgba(239, 68, 68, 0.12);
  color: #b91c1c;
  font-weight: 600;
}

.no-selection {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-text-muted, #6b7280);
  font-size: 0.9rem;
}

/* Toast */
.toast {
  position: absolute;
  bottom: 1.5rem;
  left: 50%;
  transform: translateX(-50%);
  padding: 0.6rem 1rem;
  border-radius: 6px;
  font-size: 0.85rem;
  color: #fff;
  background: #333;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
  cursor: pointer;
  z-index: 100;
}

.toast.success { background: #22c55e; }
.toast.error { background: #ef4444; }
.toast.info { background: #3b82f6; }

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
