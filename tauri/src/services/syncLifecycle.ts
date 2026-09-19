/**
 * 全局唯一订阅的生命周期 —— 「一个进程只订一条」的唯一实现
 *
 * ## 它收掉的是什么
 *
 * 两个 VDFS 同步器（`stores/sessionNodeSync` 清单收敛、`services/vdfsTranscriptSync`
 * 转写收敛）都是**全局唯一**的订阅：模块级持有一个取消句柄，幂等启动、可显式停止。
 * 这段接线原先各写一遍，于是同一个陷阱在两边**命运不同**——一处加了 HMR 守卫，
 * 另一处漏了。收成一处之后，漏掉这件事不会再发生第二次。
 *
 * ## 模块级订阅的固有陷阱：HMR 会重置模块级状态
 *
 * `let unsub` 在**模块热更新时会被重置为 `null`**（模块整体重新执行），而挂在
 * `globalThis` 上的标记不会。没有守卫时，HMR 之后再次 `start` 就会**订第二条**：
 * 同一条变更被两个 handler 各处理一次——转写表现为流式文本叠字，清单表现为重复重拉。
 *
 * 这是模块级订阅**固有**的坑，与业务无关，因此由本模块统一承担。
 *
 * ## 为什么标记在 `attach` 时置位，而不是 `begin`
 *
 * 启动前常有一步**可能失败的解析**（会话挂载目录 / 地址方案）。解析失败时调用方
 * 会放弃订阅——那一刻标记必须是**未置位**的，否则一次失败会把同步器永久锁死
 * （之后再也起不来）。因此「申请启动权」与「真的订上了」必须是两步。
 *
 * 代价（**有意接受**）：`begin` 不是"占位"，`begin` 与 `attach` 之间是异步窗口，
 * 若两次 `start` 在窗口内并发进入，两边都会拿到 `true`。生产侧不存在这种调用
 * （外壳 `await` 一次；`startTranscriptSync` 同步订完），而另一侧的收益更值：
 * 启动失败**不需要**调用方记得善后——忘记善后是静默故障（永久锁死），
 * 并发启动则是可观察的重复订阅。
 *
 * ## 本模块**不管**什么
 *
 * 订阅原语（作用域过滤 / 按地址分派）、变更合并策略（防抖重拉 / 逐路径串行链）、
 * 落地目标（sink）都留在各同步器自己手里——它们按业务不同而**有意不同**，
 * 统一它们会得到一个比两份实现更难读的配置对象。
 */

import { logger } from '@/utils/logger'

export interface SyncLifecycle {
  /**
   * 申请启动权。返回 `false` = 已在运行（已留痕），调用方直接 `return`。
   *
   * 返回 `true` **不等于**已经订上——订阅建立之后还要调 `attach`。
   */
  begin(): boolean
  /** 登记取消句柄（`begin` 返回 true、且订阅已建立之后调用一次） */
  attach(unsub: () => void): void
  /** 释放订阅并复位标记（HMR / 测试用） */
  end(): void
}

/**
 * @param tag     日志标签，如 `'[vdfs-transcript]'`
 * @param hmrFlag `globalThis` 上的标记名，全局唯一（如 `'__symTranscriptSyncStarted'`）
 */
export function createSyncLifecycle(tag: string, hmrFlag: string): SyncLifecycle {
  // 标记挂 globalThis：模块热更新重置的是模块级变量，不会重置它
  const flags = globalThis as unknown as Record<string, unknown>
  let unsub: (() => void) | null = null

  return {
    begin(): boolean {
      if (unsub || flags[hmrFlag]) {
        logger.warn(tag, 'already started')
        return false
      }
      return true
    },

    attach(fn: () => void): void {
      unsub = fn
      flags[hmrFlag] = true
    },

    end(): void {
      flags[hmrFlag] = false
      if (!unsub) return
      unsub()
      unsub = null
      logger.info(tag, 'stopped')
    },
  }
}
