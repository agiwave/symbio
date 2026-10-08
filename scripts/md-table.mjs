/**
 * md-table.mjs — Markdown 管道表的**最小**解析件。
 *
 * 为什么单独成库：本仓有两处要从 `.md` 的表里取数据（`core-naming-audit` 读 README §1.2
 * 的域前缀对照表、`gen-verify-facts` 读 01 §8 / 路线图阶梯总表 / CURRENT §2）。
 * 第二份解析器一旦写出来，「表读歪了」就会出现两种表现方式——修一处漏一处。
 *
 * 只做**形状**：分行、切单元格、认分隔行、去粗与反引号。列的语义、表头必须是什么、
 * 读不出来该不该抛错，全部留给调用方——那些是每个表自己的判据，不该被库里猜掉。
 */

/** `| a | b |` → `['a','b']`；不是表行返回 null。 */
export function cellsOf(line) {
  const t = line.trim()
  if (!t.startsWith('|')) return null
  const cells = t.split('|').slice(1, -1).map((x) => x.trim())
  return cells.length ? cells : null
}

/** 表头与数据行之间的 `|---|:-:|` 分隔行 */
export function isSeparator(cells) {
  return cells.length > 0 && cells.every((c) => /^:?-{1,}:?$/.test(c))
}

/** 去掉 `**` 与反引号，取单元格的**纯文本**（`**`store`**` → `store`） */
export function plain(cell) {
  return cell.replace(/\*\*/g, '').replace(/`/g, '').trim()
}

/** 单元格里第一个反引号片段（`投影 = \`readyset\`` → `readyset`）；没有则 null */
export function backticked(cell) {
  const m = cell.match(/`([^`]+)`/)
  return m ? m[1] : null
}

/**
 * 取 `text` 中**第一张**表头首列为 `headerFirstCell` 的表。
 *
 * 返回 `{ header, rows }`（rows 已去分隔行、去空首列行）。找不到返回 null——
 * **调用方必须把 null 变成红**：读不到规则源却报「没发现违规」的绿灯是假的。
 * `fromLine` 用来把搜索限定在某节之后（同名表头在长文档里可能重复出现）。
 */
export function tableWithHeader(text, headerFirstCell, { fromLine = 0 } = {}) {
  const lines = text.split(/\r?\n/)
  const header = []
  const rows = []
  let found = false
  for (let i = fromLine; i < lines.length; i++) {
    const cells = cellsOf(lines[i])
    if (!cells) {
      if (found) break
      continue
    }
    if (!found) {
      if (plain(cells[0]) !== headerFirstCell) continue
      header.push(...cells)
      found = true
      continue
    }
    if (isSeparator(cells)) continue
    rows.push(cells)
  }
  return found ? { header, rows } : null
}

/**
 * 取围栏代码块的内容。
 *
 * `info` 是围栏的 info string（```capability-assign → 'capability-assign'）。
 * 找不到返回 null——同样由调用方决定是抛错还是忽略（不同表的要求不同）。
 * 围栏是文档里**唯一适合放机器可读数据**的位置：它在渲染后仍是等宽块，
 * 不会被 Markdown 的行内格式改写，也不会与正文措辞互相污染。
 */
export function fencedBlock(text, info) {
  const lines = text.split(/\r?\n/)
  const open = new RegExp(`^\`{3,}\\s*${info}\\s*$`)
  const close = /^\`{3,}\s*$/
  const out = []
  let inBlock = false
  for (const l of lines) {
    if (!inBlock) {
      if (open.test(l.trim())) inBlock = true
      continue
    }
    if (close.test(l.trim())) break
    out.push(l)
  }
  return inBlock && out.length ? out : null
}
