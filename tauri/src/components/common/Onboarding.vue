<!--
  Onboarding — 首启引导（三步，可跳过、可从「关于」重开）

  ## 它只说三件事
  1. **系统目录**就位（本机 homedir / 远端网关）——状态只读展示，切换入口在左下角；
  2. **配一个模型**——这是唯一的阻塞点：模型不是「设置」里的一项，而是
     `<根>/model` 下的一个资源条目（要填 API Key）。没有它，输入框里的话发不出去；
  3. **发第一条消息**——会话就在冷启动落点，点「新建」即可。

  ## 为什么它不含任何类型知识
  三步的**就绪状态**全部来自数据（`stores/onboarding`），地址也全部是**认出来的**
  （`findMountDirByNewTypeExt`，而不是写死 `/vdfs/model`）：段名由后端 provider 决定，
  写死会在段名变更时静默跳到一个空目录。文案里的「模型」是**产品词**，不是地址。

  系统目录一栏刻意**不给切换按钮**：切换器是左下角那个常驻入口（`HomedirEntry`），
  此处再放一个等于同一件事两条路——引导应当把用户送到**目的地**，而不是复制导航。
-->
<template>
  <BaseModal
    :visible="store.open"
    panel-class="onboarding"
    labelledby="onboarding-title"
    @close="store.close()"
  >
    <h2 id="onboarding-title" class="ob-title">三步开始</h2>
    <p class="ob-sub">
      别处找不到「设置」：这个应用里的一切都是资源树上的一个节点。把下面三件做完，就能开始对话。
    </p>

    <ol class="ob-steps">
      <li class="ob-step" :class="{ 'is-done': store.systemReady, 'is-error': systemError }">
        <span class="ob-mark" aria-hidden="true">{{ store.systemReady ? '✓' : '!' }}</span>
        <div class="ob-body">
          <p class="ob-step-title">系统目录</p>
          <p class="ob-step-desc">{{ systemText }}</p>
        </div>
      </li>

      <li class="ob-step" :class="{ 'is-done': store.modelReady }">
        <span class="ob-mark" aria-hidden="true">{{ store.modelReady ? '✓' : '2' }}</span>
        <div class="ob-body">
          <p class="ob-step-title">配一个模型</p>
          <p class="ob-step-desc">
            {{
              store.modelReady
                ? '已经有可用的模型了。'
                : '模型是对话的引擎：在模型目录下新建一个条目，填入服务商与 API Key。'
            }}
          </p>
        </div>
        <button v-if="!store.modelReady" class="ob-btn" type="button" @click="goModel">
          去配置模型
        </button>
      </li>

      <li class="ob-step" :class="{ 'is-done': store.sessionReady }">
        <span class="ob-mark" aria-hidden="true">{{ store.sessionReady ? '✓' : '3' }}</span>
        <div class="ob-body">
          <p class="ob-step-title">发第一条消息</p>
          <p class="ob-step-desc">在会话目录里点右上角「新建」，写下第一句话。发送后可以随时中止。</p>
        </div>
        <button class="ob-btn" type="button" @click="goSession">开始第一段对话</button>
      </li>
    </ol>

    <div class="ob-actions">
      <button class="ob-btn ghost" type="button" @click="store.close()">稍后再说</button>
      <button class="ob-btn primary" type="button" @click="store.dismiss()">
        {{ store.hasPending ? '跳过引导' : '开始使用' }}
      </button>
    </div>
  </BaseModal>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useRouter } from 'vue-router'
import BaseModal from './BaseModal.vue'
import { useOnboardingStore } from '@/stores/onboarding'
import { currentLocation, formatLocation, locationError } from '@/services/systemLocation'
import { VDFS_HOME_PATH, vdfsBrowserPathOf } from '@/schemas/vdfsAddress'

const store = useOnboardingStore()
const router = useRouter()

const systemError = computed(() => Boolean(locationError.value))

/** 当前连接目标（本机路径 / 远端地址）；连接异常时把原因说出来 */
const systemText = computed(() =>
  systemError.value
    ? locationError.value || '连接异常'
    : `已连接：${formatLocation(currentLocation.value)}（可在左下角切换）`,
)

/**
 * 去某个类别目录：地址由 store 认出来（`null` = 该类别没挂上 ⇒ 退回资源根，
 * 用户在那里能看到全部类别，而不是被引导丢到一个不存在的地址上）。
 */
function openDir(dir: string | null) {
  void router.push(dir ? vdfsBrowserPathOf(dir) : VDFS_HOME_PATH)
}

/** 去配模型：关掉面板但**不记为已读**——用户还没走到第 3 步 */
function goModel() {
  openDir(store.modelDir)
  store.close()
}

/** 去发第一条消息：这一步之后引导就算读完了 */
function goSession() {
  openDir(store.sessionDir)
  store.dismiss()
}
</script>

<style scoped>
.onboarding {
  width: min(34rem, 94vw);
  padding: var(--space-5);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.ob-title {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
}

.ob-sub {
  margin: 0;
  font-size: var(--font-size-sm);
  line-height: var(--line-height-normal);
  color: var(--text-muted);
}

.ob-steps {
  list-style: none;
  margin: var(--space-2) 0 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

/* 每一步是一行：标记 + 说明 +（可选）动作。动作靠右，不与说明抢视线 */
.ob-step {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--surface-sunken);
}

.ob-step.is-done {
  border-color: var(--border-default);
  opacity: 0.72;
}

.ob-step.is-error {
  border-color: var(--danger-border);
}

.ob-mark {
  flex-shrink: 0;
  width: 1.5rem;
  height: 1.5rem;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius-full);
  background: var(--surface-panel);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  font-weight: var(--font-weight-semibold);
}

.ob-step.is-done .ob-mark {
  color: var(--accent);
}

.ob-step.is-error .ob-mark {
  color: var(--danger-fg);
}

.ob-body {
  flex: 1 1 auto;
  min-width: 0;
}

.ob-step-title {
  margin: 0;
  font-size: var(--font-size-sm);
  font-weight: var(--font-weight-medium);
  color: var(--text-primary);
}

.ob-step-desc {
  margin: 0.15rem 0 0;
  font-size: var(--font-size-xs);
  line-height: var(--line-height-normal);
  color: var(--text-muted);
}

.ob-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
  margin-top: var(--space-2);
}

.ob-btn {
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  cursor: pointer;
  white-space: nowrap;
  transition: background var(--motion-fast) var(--motion-ease);
}

.ob-btn:hover {
  background: var(--surface-hover);
  color: var(--text-primary);
}

.ob-btn.primary {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--text-on-accent);
}

.ob-btn.primary:hover {
  background: var(--accent-hover);
  border-color: var(--accent-hover);
  color: var(--text-on-accent);
}

.ob-btn.ghost {
  border-color: transparent;
}
</style>
