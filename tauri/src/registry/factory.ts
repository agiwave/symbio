/**
 * 渲染器注册表工厂 —— 「标识 → 组件（+ 兜底）」这条机制的**唯一实现**
 *
 * VDFS 详情域与消息域需要的是同一样东西：一张私有表 + 几个动作
 * （登记 / 取用 / 带兜底解析）。这条机制此前被发明了两次
 * （`vdfsTypes.ts` 一份、`messageTypes.ts` 一份），两份唯一的差别只是标识的
 * 联合类型不同——「同一回事写两遍」的典型形态，且「未登记时谁来接管」这条
 * 约定要各自维护一次。
 *
 * ## 为什么是工厂，不是一张共享的表
 *
 * 两个域的标识空间**必须互不污染**：`text` 在 VDFS 域是文本编辑器、在消息域是
 * 折叠内容节点，同名的两个组件若共用一张表就会互相覆盖。故每次
 * `createRendererRegistry()` 都开**独立**的一张表——机制共用，状态不共用。
 *
 * ## 兜底键由创建者显式声明
 *
 * 兜底不是「最后登记的那个」，而是创建时就命名好的键（两域都叫 `fallback`）。
 * 于是「未登记标识 → 谁接管」是一条可读的约定，而不是登记顺序的副作用。
 * 连兜底键自身都没登记时 `resolve` 返回 `undefined`，由调用方决定怎么兜
 * （正常构建下不会发生）。
 *
 * ## 三个动作都不依赖 `this`
 *
 * 实现一律闭包捕获那张表（而非 `this.components`），因此三个方法可以安全地
 * 解构出去单独导出（`export const registerX = registry.register`）——
 * 这正是两个域想要的：对外只暴露域内命名的函数，工厂本身不进入调用方的视野。
 */

import type { Component } from 'vue'

/** 「标识 → 组件」表 + 带兜底的解析 */
export interface RendererRegistry<TKey extends string> {
  /** 登记；同键后登记者胜（装配点只在启动时各调一次，不会真的覆盖） */
  register(key: TKey, component: Component): void
  /** 取用；未登记返回 `undefined`（调用方自行决定是否兜底） */
  get(key: TKey): Component | undefined
  /** 带兜底取用；未登记 → `fallbackKey` 对应的组件（它也没登记 → `undefined`） */
  resolve(key: TKey): Component | undefined
}

/**
 * 开一张独立的渲染器注册表。
 *
 * @param fallbackKey 兜底标识——`resolve` 在未登记时回落到它
 */
export function createRendererRegistry<TKey extends string>(
  fallbackKey: TKey
): RendererRegistry<TKey> {
  const components: Partial<Record<TKey, Component>> = {}
  return {
    register(key, component) {
      components[key] = component
    },
    get(key) {
      return components[key]
    },
    resolve(key) {
      return components[key] ?? components[fallbackKey]
    },
  }
}
