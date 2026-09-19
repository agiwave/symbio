/**
 * useGenerationGuard —— 「代次守卫」机制（唯一实现）
 *
 * ## 它替掉的是什么
 *
 * 同一件事此前在三处各写了一遍：**取一个代次号 → 异步响应回来比对 → 过期就丢**。
 * 三处写法还各不相同（`useVdfs.detailToken` / `useVdfs.appendGen+appendGenPath` /
 * `ChatMainPanel.loadSequence`），于是同一类竞态（慢响应覆盖新状态）要修三遍，
 * 而且每新写一处就多一次重犯的机会——与「注册表机制被发明两次」是同一种病。
 *
 * ## 一个原语，两种形态
 *
 * - **单槽**（键缺省）：一个全局单调代次。用于「同时只认一个在途请求，
 *   后来者作废先来者」——详情读取、会话切换。
 * - **键控**：每个键各自计数，**换键即视为 0**。用于「读取前的快照必须仍是
 *   当前的」——节点正文的追加代际（读取期间若有增量落地，响应就是旧快照）。
 *
 * 键控槽**只保留最近一个键**（不是一张无界表）：换键即丢弃上一个键的计数。
 * 这既是内存有界，也正合语义——只有「当前打开的那个节点」谈得上
 * 「已应用过几次追加」。
 *
 * ## 用法
 *
 * ```ts
 * const guard = useGenerationGuard()
 *
 * // 形态一：后来者作废先来者
 * const rev = guard.advance()          // 发起请求前取号
 * const resp = await load()
 * if (guard.revision() !== rev) return // 过期 → 静默丢弃（不报错）
 *
 * // 形态二：快照式（读取前记，回来比对）
 * const snap = guard.revision(path)
 * const resp = await read(path)
 * if (guard.revision(path) !== snap) return
 * ```
 *
 * ## 边界（刻意不做的事）
 *
 * - **不取消请求**：请求已经在飞，丢结果即可；代价是白跑一次读，
 *   换来的是「不必为取消引入 AbortController 这对机制」。
 * - **不承载忙态**：`loading` 的复位必须与代次判定**解耦**。若在 `finally` 里
 *   按一个会被中途自增的令牌复位，加载态就永远复位不了——所以调用方的
 *   `finally` 要用**自己取到的那个代次**比对，而不是重新 `revision()` 取。
 * - **不持有响应式状态**：守卫只有两个数字，没有 ref。它不参与渲染，
 *   因此也不是 `computed`/`watch` 的依赖——这正是它该做的（派生是纯函数）。
 */

/**
 * 代次守卫实例。按**关注点**各起一个：不同关注点的代次互不作废。
 * （`useVdfs` 里详情读取与追加代际是两个实例；会话切换是第三个。）
 */
export interface GenerationGuard {
  /**
   * 取当前代次（只读）。
   *
   * - 单槽：当前全局代次；
   * - 键控：该键的计数；**键与当前键不同 ⇒ 0**（换过键，旧计数不再有意义）。
   */
  revision(key?: string): number
  /**
   * 前进一代并返回新代次。
   *
   * 调用即**作废此前所有在途响应**——故它在两处使用：发起异步读取前
   * （取号），以及状态被外部改动时（使旧响应失效，如清空选中）。
   */
  advance(key?: string): number
}

export function useGenerationGuard(): GenerationGuard {
  /** 当前键（`null` = 还没用过；单槽用空串，与键控共用一条路径） */
  let key: string | null = null
  let rev = 0

  /** 键归一：缺省即为单槽（空串），于是单槽与键控不必分两套状态 */
  const norm = (k?: string) => k ?? ''

  return {
    revision(k) {
      return key === norm(k) ? rev : 0
    },
    advance(k) {
      const kk = norm(k)
      if (key !== kk) {
        key = kk
        rev = 0
      }
      return ++rev
    },
  }
}
