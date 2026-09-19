/**
 * VDFS 地址换算 —— 「浏览器地址 ↔ 数据地址」的**唯一来源**
 *
 * 数据地址（`.vdfs/<rel>`）与浏览器地址（`/vdfs/<rel>`）是**两个概念**：
 * 前者是 VDFS 的寻址空间（控件只认它），后者只是路由的承载形式。
 * 换算规则原先内联在 `views/VdfsView.vue` 里——那里是路由宿主，装配多、依赖多
 * （vue-router / store / 子组件），换算规则因此**测不到**。搬到这里之后规则与
 * 实现同处，回归由 `schemas/__tests__/vdfsAddress.spec.ts` 钉住。
 *
 * ## 三条规则
 *
 * 1. **相对路径 → 数据地址**：空 → `.vdfs`（根）；否则 `.vdfs/<rel>`。
 *    vue-router 的 `:dir(.*)*` 参数在**多级**时给数组、单级时给字符串、缺省时给
 *    空串，这里统一收口，调用方不必分辨。
 * 2. **数据地址 → 浏览器地址**：根 → `/vdfs`；`.vdfs/` 之下 → `/vdfs/<rel>`。
 *    ⚠️ 非 VDFS 地址（不在 `.vdfs` 之下）**一律兜底 `/vdfs`**，绝不把外来地址
 *    拼进 URL——拼错的地址会静默打开一个空页面。
 * 3. **深页面判定**：`/vdfs/<something>` 才是 push 出来的地址页（首页显示 logo，
 *    深页面在左上角显示返回键）。`/vdfs` 本身不是。
 *
 * 规范：docs/design/vdfs-frontend.md（路由宿主只做承载）
 */

import { VDFS_ROOT } from './vdfs'

/** 首页的浏览器地址 */
export const VDFS_HOME_PATH = '/vdfs'

/**
 * 路由参数（相对路径）→ 数据地址。
 *
 * `:dir(.*)*` 的取值有三种形态：多级 = `string[]`、单级 = `string`、
 * 缺省 = `''`。这里统一成「相对路径字符串」再拼。
 */
export function vdfsAddrOf(rel: string | string[] | undefined | null): string {
  if (Array.isArray(rel)) {
    const joined = rel.filter(Boolean).join('/')
    return joined ? `${VDFS_ROOT}/${joined}` : VDFS_ROOT
  }
  const s = (rel ?? '').trim()
  return s ? `${VDFS_ROOT}/${s}` : VDFS_ROOT
}

/**
 * 数据地址 → 浏览器地址。
 *
 * ⚠️ 非 VDFS 地址兜底 `/vdfs`：宁可回首页，也不把外来地址拼进 URL。
 */
export function vdfsBrowserPathOf(addr: string): string {
  const a = (addr ?? '').trim()
  if (a === VDFS_ROOT) return VDFS_HOME_PATH
  if (a.startsWith(`${VDFS_ROOT}/`)) return `${VDFS_HOME_PATH}/${a.slice(VDFS_ROOT.length + 1)}`
  return VDFS_HOME_PATH
}

/**
 * 是否**深地址页**（push 出来的页面，而非首页）。
 *
 * 首页（`/vdfs`）显示主 logo，深页面在左上角显示返回键——两者互斥，
 * 判据只有这一处。
 */
export function isVdfsDeepPage(path: string): boolean {
  return (path ?? '').startsWith(`${VDFS_HOME_PATH}/`)
}
