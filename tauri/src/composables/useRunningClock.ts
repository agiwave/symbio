/**
 * useRunningClock — **全应用共享的秒级时钟**
 *
 * ## 它解决什么
 *
 * 长任务的节点（工具执行、子智能体、语义索引）需要显示"已经跑了多久"。
 * 「运行中」是一个**断言**，「已运行 47s」才是**判据**——用户靠后者区分
 * "还在跑"与"卡住了"，这正是静态标签给不出的信息。
 *
 * ## 为什么必须共享，而不是每个组件一个 `setInterval`
 *
 * `MessageNode` 是**递归组件**：一条长会话会实例化出成百个节点。每节点自带定时器
 * 会让定时器数量随会话长度线性增长，且它们在同一毫秒集体触发（重渲染尖峰）。
 * 这里全局只有一个定时器，且**只在有消费者时走时**——没有运行中的节点就停表，
 * 空闲会话零开销。
 *
 * 停表而不是继续空转：定时器虽小，但"永远在跑的后台定时器"会让
 * `getSessionStaleReason` 一类的看门狗逻辑难以推理（分不清是任务在跑还是时钟在跑）。
 *
 * ## 代价
 *
 * `nowMs` 只递增一个 ref。Vue 的依赖收集是精确的：**没有**读取它的组件不会因此
 * 重渲染，所以共享一个 ref 并不会让整棵会话树每秒刷新一次。
 *
 * ## 用法（`acquire` / `release` 必须成对）
 *
 * ```ts
 * const { nowMs, acquire, release } = useRunningClock()
 * watch(isRunning, (v) => (v ? acquire() : release()), { immediate: true })
 * onScopeDispose(() => { if (isRunning.value) release() })
 * ```
 *
 * 组件卸载时忘记 `release` 的后果是"时钟永不停止"——功能不会错，只是白耗电，
 * 因此每个调用点都必须写 `onScopeDispose` 兜底。
 */

import { ref, type Ref } from 'vue'

/** 当前时刻（毫秒），每秒递增一次 */
const nowMs = ref(Date.now())

/** 消费者计数：归零即停表 */
let consumers = 0
let timer: ReturnType<typeof setInterval> | null = null

function start(): void {
  if (timer === null) {
    timer = setInterval(() => {
      nowMs.value = Date.now()
    }, 1000)
  }
}

function stop(): void {
  if (timer !== null) {
    clearInterval(timer)
    timer = null
  }
}

export function useRunningClock(): {
  nowMs: Ref<number>
  acquire: () => void
  release: () => void
} {
  return {
    nowMs,
    acquire: () => {
      consumers += 1
      start()
    },
    release: () => {
      consumers = Math.max(0, consumers - 1)
      if (consumers === 0) stop()
    },
  }
}
