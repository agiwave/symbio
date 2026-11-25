/**
 * 首启引导的状态与**数据判据**（唯一实现）
 *
 * ## 它解决什么问题
 *
 * 「发不出消息」是这款应用唯一的流失漏斗：模型不是「设置」里的一个选项，而是
 * `<根>/model` 下的一个资源条目（要填 API Key）。README 写了这件事，界面零提示——
 * 新用户第一分钟必然卡在一句发不出去的话上。
 *
 * ## 三步各自的**就绪判据由数据给**，不是一张静态清单
 *
 * | 步 | 判据 |
 * |---|---|
 * | 系统目录 | `services/systemLocation` 的 `locationError` 为空 |
 * | 配一个模型 | `<根>/model` 里有条目（`vdfs/list` 非空） |
 * | 发第一条消息 | 会话挂载点认得出（`ensureSessionMountDir` 不抛） |
 *
 * 因此它**不会**给已经配好的用户弹一张「请去配置」的空话卡片：`maybeOpen()` 只在
 * 第②步未就绪时自动打开。判据全来自后端数据，新增一类资源不需要改这里。
 *
 * ## 「已读过」只记一条布尔
 *
 * 与外观设置同源（localStorage），因为它是**用户选择**而不是领域数据。
 * 刻意不记「走到第几步」：那会变成第四个状态需要维护，而调用方可以随时
 * 把引导重新打开（`openManually`）。
 */

import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { listVdfs } from '@/services/vdfs'
import { subscribeVdfsChanged } from '@/services/eventBus'
import { READBACK_REASON } from '@/services/readback'
import { findMountDirByNewTypeExt } from '@/services/vdfsScheme'
import { locationError } from '@/services/systemLocation'
import { VDFS_EXT_MODEL, VDFS_EXT_SESSION } from '@/schemas/vdfs'
import { logger } from '@/utils/logger'

const STORAGE_KEY = 'symbio.onboarding'
const MODULE_TAG = 'onboarding'

/** 「已读过」是否落盘（localStorage 不可用时按未读过处理，只是会再弹一次） */
function loadDismissed(): boolean {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    return raw ? Boolean((JSON.parse(raw) as { dismissed?: unknown }).dismissed) : false
  } catch {
    return false
  }
}

function persistDismissed(value: boolean): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ dismissed: value }))
  } catch {
    /* 隐私模式等 storage 不可用：非致命，下次启动会再弹一次 */
  }
}

export const useOnboardingStore = defineStore('onboarding', () => {
  /** 引导面板是否可见 */
  const open = ref(false)
  /** 用户是否已经读过（关掉过一次就不再自动弹） */
  const dismissed = ref(loadDismissed())

  // ── 三步的就绪状态（由数据推导，见 `refresh`）──
  const systemReady = ref(!locationError.value)
  const modelReady = ref(false)
  const sessionReady = ref(false)
  const checking = ref(false)
  /**
   * 用户点了「去配置模型」⇒ 引导关掉但没读完。此时监听资源变更：一旦模型目录里
   * 出现了条目（用户配好了），自动把引导带回来继续第③步——否则第②步是死胡同，
   * 用户配完模型后再也看不到「发第一条消息」的引导。
   */
  const awaitingModel = ref(false)
  let stopWatching: (() => void) | null = null

  /** 模型类别的挂载目录（认不出 = 该类别没挂上，引导据此降级到资源根） */
  const modelDir = ref<string | null>(null)
  /** 会话类别的挂载目录（认不出 = 会话插件的装配有问题） */
  const sessionDir = ref<string | null>(null)

  /** 是否还有没做的事（面板据此决定要不要把某一步标成「待完成」） */
  const hasPending = computed(() => !modelReady.value)

  /**
   * 重新判定三步状态。
   *
   * 每一小步都**独立吞掉异常**：引导是「锦上添花」的面板，任何一个后端读取失败
   * 都不该让它整个炸掉（那样用户既看不到引导，也看不到错误原因）。
   */
  async function refresh(): Promise<void> {
    checking.value = true
    systemReady.value = !locationError.value
    try {
      const dir = await findMountDirByNewTypeExt(VDFS_EXT_MODEL)
      modelDir.value = dir
      if (dir) {
        const resp = await listVdfs(READBACK_REASON.GUIDE, dir)
        modelReady.value = (resp.items ?? []).length > 0
      } else {
        modelReady.value = false
      }
    } catch (e) {
      logger.warn(MODULE_TAG, '判定「模型是否已就绪」失败:', e)
      modelReady.value = false
    }
    try {
      sessionDir.value = await findMountDirByNewTypeExt(VDFS_EXT_SESSION)
      sessionReady.value = Boolean(sessionDir.value)
    } catch (e) {
      logger.warn(MODULE_TAG, '判定「会话是否就绪」失败:', e)
      sessionReady.value = false
    }
    checking.value = false
  }

  /**
   * 首启动的自动弹出：**只在该做的事还没做时才弹**。
   *
   * 判据是「有没有配过模型」而不是「有没有读过引导」——已配好的老用户不该被
   * 一张「请去配置模型」的空话卡片拦在门口。
   */
  async function maybeOpen(): Promise<void> {
    if (dismissed.value) return
    await refresh()
    if (dismissed.value) return
    if (!modelReady.value) open.value = true
  }

  /**
   * 用户点「去配置模型」：标记在等模型，挂一次资源变更监听（幂等）。
   * 模型一旦配好 ⇒ 刷新判据并把引导带回第③步。这样第②步不再是死胡同。
   */
  function beginModelSetup(): void {
    if (dismissed.value) return
    awaitingModel.value = true
    if (stopWatching) return
    // 监听整棵资源树（模型目录的挂载点由数据认出，这里不硬编码），任何变更都
    // 重新判一次；只在「等待中 + 模型就绪 + 未读」时才重开面板。
    stopWatching = subscribeVdfsChanged({ prefix: '' }, () => {
      // 面板正开着就不用重开；已读 / 不在等待中也不打扰
      if (!awaitingModel.value || dismissed.value || open.value) return
      void refresh().then(() => {
        if (awaitingModel.value && modelReady.value && !dismissed.value) {
          awaitingModel.value = false
          stopWatching?.()
          stopWatching = null
          open.value = true
        }
      })
    })
  }

  /** 由调用方手动打开（不看 `dismissed`） */
  function openManually(): void {
    open.value = true
    void refresh()
  }

  /** 关闭面板但不记为已读（调用方随时可以再打开） */
  function close(): void {
    open.value = false
  }

  /** 用户读完 / 跳过 ⇒ 记一笔，之后不再自动弹 */
  function dismiss(): void {
    open.value = false
    dismissed.value = true
    awaitingModel.value = false
    stopWatching?.()
    stopWatching = null
    persistDismissed(true)
  }

  return {
    open,
    dismissed,
    systemReady,
    modelReady,
    sessionReady,
    checking,
    hasPending,
    modelDir,
    sessionDir,
    refresh,
    maybeOpen,
    beginModelSetup,
    openManually,
    close,
    dismiss,
  }
})
