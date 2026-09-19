/**
 * 时间格式化 —— **前端唯一的实现处**
 *
 * 此前有三份：本文件一份（`formatTime`，零引用但被测试养着）、
 * `VdfsWorkbench` 一份（列表项的相对时间）、`VdfsReadonlyDetail` 一份
 * （详情页的绝对时间）。三份各写各的，且都没有处理「时间戳是秒还是毫秒」。
 *
 * 这里只保留两个**对外语义明确**的函数：相对时间（列表项）与绝对时间
 * （详情字段）。口径不一致的格式化不要往这里加第三个——先想清楚它是哪一种。
 */

/**
 * 时间戳 → 毫秒。
 *
 * 后端的时间戳两种口径都有（秒 / 毫秒），按**量级**判定：毫秒级的当下是
 * 1e12 量级，而秒级的当下只有 1e9 —— 分界取 1e12 时，秒级要走到公元 33658 年
 * 才会被误判，因此这条判据是安全的。
 */
function toMillis(ts: number): number {
  return ts < 1e12 ? ts * 1000 : ts
}

/**
 * 相对时间（列表项用）：刚刚 / N 分钟前 / N 小时前 / N 天前。
 *
 * 未来时间戳或无效值返回**空串**——宁可不显示，也不要显示「-3 分钟前」。
 */
export function relativeTime(ts?: number | null): string {
  if (!ts) return ''
  const diff = Date.now() - toMillis(ts)
  if (diff < 0) return ''
  if (diff < 60_000) return '刚刚'
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`
  return `${Math.floor(diff / 86_400_000)} 天前`
}

/**
 * 绝对时间（详情字段用）：本地化的完整日期时间。
 *
 * 无效值返回空串（调用方通常另有 `v-if`，这里是第二道保险）。
 */
export function formatDateTime(ts?: number | null): string {
  if (!ts) return ''
  return new Date(toMillis(ts)).toLocaleString()
}
