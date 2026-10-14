/**
 * facts/_lib — 事实生成器的**共享契约**与幂等落盘原语
 *
 * ## 这份文件是什么
 *
 * 一份「事实文件」= 由代码投影出来的一份文档（结构事实表 / 判据码登记表 /
 * 前端路由常量…）。它们此前是三份各自独立的脚本，各带一份「怎么写文件、怎么判
 * 漂移、怎么报溯源」的副本，门禁里也是三段手抄的调用。
 *
 * 本模块把那份副本收成一处：`scripts/facts/` 下每个 `.mjs` 导出一份**描述符**，
 * runner（`scripts/gen-facts.mjs`）遍历目录逐个调用。**新增一份事实的成本 =
 * 建一个文件**，不需要改门禁、不需要登记名单。
 *
 * ## 描述符契约（runner 逐条机检，违约即红）
 *
 * | 字段 | 含义 |
 * |---|---|
 * | `id` | 事实的稳定标识（文件去后缀），用于日志与去重 |
 * | `title` | 人读的一句话标题，进生成物横幅与门禁日志 |
 * | `out` | 产物路径，**仓根相对**、正斜杠分隔 |
 * | `stamp` | 是否带「生成时间 / 溯源」行（默认 true；确定性产物如前端常量带它会红） |
 * | `render()` | `(ctx) => string`，**纯函数**：同一份代码 ⇒ 同一份输出 |
 *
 * ## 为什么 `render` 必须是纯函数
 *
 * 门禁把生成当**确定性的机械工作**（`gate.d/_shared.mjs` 的 `autoWork`）：跑成功
 * 即通过，产物被改写不算失败，CI 侧则把「跑完仍有差异」判红。
 * 这条约定成立的前提就是 `gen(代码) = 产物` 是函数——一旦 render 里掺了时间、
 * 随机数或运行顺序，`autoWork` 就会因为「每次内容都不同」而永远报红，
 * 而红的原因不是漂移，是**生成器自己不确定**。溯源行是唯一的例外，已单列处理。
 */

/**
 * 溯源行的行首。
 *
 * 它表示的是「**内容最后一次真正变化**是在何时、基于哪个 commit」，
 * 比「最后一次运行时间」更有意义——但正因如此它**不能参与**「内容变没变」的判定，
 * 否则 `gen` 就不再是函数（每次运行都产出一份不同的文件，而 `autoWork`
 * 比对的是内容哈希 ⇒ 每次门禁都报「修复了 1 个文件」，CI 无法提交、只能判红，
 * 且红的原因是时间戳而不是漂移）。
 */
export const STAMP_PREFIX = '> 生成时间：'

/** 剔除溯源行（比对「内容有没有漂移」时用）。 */
export function stripStamp(text) {
  return text
    .split('\n')
    .filter((l) => !l.startsWith(STAMP_PREFIX))
    .join('\n')
}

/**
 * 描述符校验。返回违规数组（空 = 合法）。
 *
 * 宁可判红不判过：这是本仓反复买到教训的一侧（`registry-audit` 的 RG-010
 * 「范围读不出 ⇒ 退化成什么都没校验是最坏的失败」同款）。
 */
export function validateFact(mod, { id, outRoot }) {
  const err = []
  const where = `scripts/facts/${id}.mjs`
  if (!mod || typeof mod !== 'object') return [`${where}：没有 default 导出（事实描述符）`]
  const { id: fid, title, out, render } = mod
  if (typeof fid !== 'string' || !fid) err.push(`${where}：缺 \`id\``)
  else if (fid !== id) err.push(`${where}：\`id\` 写的是 \`${fid}\`，与文件名 \`${id}\` 不一致`)
  if (typeof title !== 'string' || !title) err.push(`${where}：缺 \`title\``)
  if (typeof out !== 'string' || !out) {
    err.push(`${where}：缺 \`out\`（产物路径，仓根相对）`)
  } else if (path_isAbsolute(out) || out.includes('\\') || out.split('/').includes('..')) {
    // 产物路径越界会把生成器变成一个任意写文件的口子，而门禁在 CI 里以仓库身份跑它。
    err.push(`${where}：\`out\` 必须是不含 \`..\` 的仓根相对正斜杠路径，读到 ${JSON.stringify(out)}`)
  } else if (outRoot && !out.startsWith(outRoot)) {
    err.push(`${where}：\`out\` 必须落在 \`${outRoot}\` 下，读到 ${JSON.stringify(out)}`)
  }
  if (typeof render !== 'function') err.push(`${where}：缺 \`render()\``)
  return err
}

function path_isAbsolute(p) {
  return p.startsWith('/') || /^[A-Za-z]:/.test(p)
}

/**
 * 幂等落盘：**内容（剔溯源行后）没变就整份沿用**。
 *
 * 「沿用已有文件」而不是「重新写一遍」是刻意的：同输入 ⇒ 同输出要**逐字节**成立，
 * 否则 autoWork 的内容哈希每次都变，门禁每次都报「自动修复了 1 个文件」。
 */
export function emit({ out, content, stamp = true }) {
  const existing = readOrNull(out)
  if (existing !== null && stripStamp(existing) === stripStamp(content)) {
    return { changed: false, lines: content.split('\n').length }
  }
  mkdirp(dirname(out))
  writeFileSync(out, content, 'utf8')
  return { changed: true, lines: content.split('\n').length }
}

/** `--check` 口径下的比对：返回 null = 一致，否则是给 人 看的一句话。 */
export function driftOf({ out, content }) {
  const existing = readOrNull(out)
  if (existing === null) return `${out} 不存在——先跑一次 \`node scripts/gen-facts.mjs\``
  if (stripStamp(existing) !== stripStamp(content)) return `${out} 与代码漂移——重跑 \`node scripts/gen-facts.mjs\``
  return null
}

// —— 薄薄一层 fs 包装：让本模块的 import 列表一眼可数 ——
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'

function readOrNull(p) {
  try {
    return existsSync(p) ? readFileSync(p, 'utf8') : null
  } catch {
    return null
  }
}

function mkdirp(d) {
  mkdirSync(d, { recursive: true })
}