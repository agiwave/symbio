#!/usr/bin/env node
/**
 * facade-loc-audit — `plugin-api` 自有实现的行数棘轮（FA-001…004）
 *
 * ## 它治什么
 *
 * `plugin-api` 是扩展层的**契约面**：插件作者读它、实现它，宿主与机制层在它后面。
 * 一旦某段实现被「顺手」写进这里，扩展层的读者就多了一份与契约无关的机制代码，
 * 而这件事此前只有散文——「`plugin-api` 自有实现只许降」写在文档里，没有任何判据。
 * 行数会自己长回来，只有数字拦得住。
 *
 * 基线 `scripts/baselines/plugin-api-loc.json`：自有实现文件逐项行数（含退役格子）+ 全
 * crate 生产行数合计。**超一行即红**；行数下降也必须同批下调基线，否则「删 N 行再加 N 行」
 * 能把棘轮抹平（与 `SR-002` 同一条理由）。
 *
 * | 码 | 判什么 | 处理 |
 * |---|---|---|
 * | `FA-001` | 某文件或合计的行数**超过**基线 | 红；实现只许往 `symbio-sdk` / `symbio-host` 搬，不许往回长 |
 * | `FA-002` | 行数**低于**基线却没同批下调基线 | 红；否则基线留出余量，「删了再加」就不触发 FA-001 |
 * | `FA-003` | 基线条目形状不对（缺 `path` / `lines` / `adr`、`adr` 打不开、重名、登记的文件不在生产里、退役条目却还指着活文件） | 红；每个数字都必须带能打开的决策号 |
 * | `FA-004` | 范围读不出（`crates/symbio-plugin-api/src` 缺失、没有生产 `.rs`、基线缺失或 JSON 坏） | 退 2；读不出不许当「0 行 ⇒ 通过」 |
 *
 * ## 退役条目：文件整份删除时，格子留 0，不留空
 *
 * 门面里的纯转发文件（整份 `pub use …::*`）搬到 sdk 后**不该继续存在**，而删文件必须
 * 同批删基线条目（否则「删了再加」能抹平棘轮）。删格本身会被 `35-baseline` 的棘轮元守卫
 * 记成「放松」（它看不见文件已经不在），所以退役的形态不是删格，而是**把这一格钉到 0**
 * 并写明它搬去了哪（`retired`）：0 是没有余量的上限，文件若再出现，FA-003 直接红。
 *
 * | 字段 | 要求 |
 * |---|---|
 * | `lines` | 必须是 **0**（退役 = 这一层不再承载任何实现行） |
 * | `retired` | 非空字符串，写新家（`crates/symbio-sdk/src/plugin/<mod>.rs`） |
 * | 文件 | `CRATE_DIR` 下**必须不存在**（存在 ⇒ 退役没做完或又长回来了） |
 * | `adr` | 照旧：能打开的 `### D-NNN` |
 *
 * ## 统计口径（这条比脚本本身重要）
 *
 * 数的是**生产文件的行数**：`*.test.rs` / `tests.rs` 及 `#[cfg(test)] mod x;` 声明的
 * 测试模块整体不计（`isTestFile` 是唯一实现），文件内的空行与注释照算——
 * 契约面的文档注释也是读者要读的那部分。
 *
 * **清单镜像不计行数**（`D-470`）：`events.rs` / `routes.rs` 是各插件 `PLUGIN.yml` 清单的
 * 投影，由 `scripts/facts/` 的描述符重新生成——它们不是「手写进契约面的实现」。把它们
 * 计进合计，会让「声明一条跨插件路由」直接顶破基线，而 `FA-001` 没有豁免通道：合法的
 * 协议扩展被一条不针对它的棘轮挡住，可它一行实现都没写。
 *
 * 判定只有一个注册处（`generatedPaths`：描述符的 `OUT_REL`，与
 * `capability-migration-audit` / `extension-locality-audit` 共用），**不按文件名猜**——
 * `routes.rs` 若不再由描述符产出，它就是手写文件，照旧计件。生成物仍留在扫描结果里：
 * 基线的存在性登记（`FA-003`）照旧判，只是行数不参与逐项与合计的比较。
 *
 * ## 用法
 *
 *   node scripts/facade-loc-audit.mjs                  # 对真仓判定
 *   node scripts/facade-loc-audit.mjs --strict         # 门禁统一参数（本脚本无 WARNING 级）
 *   node scripts/facade-loc-audit.mjs --root=<dir>     # 回归测试夹具
 *
 * 退出码：0 = 通过；1 = 有违规；2 = 范围读不出。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns FA 门面（plugin-api）自有实现行数棘轮
// @codes FA-001 FA-002 FA-003 FA-004

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, dim } from './color.mjs'
import { walk, isTestFile } from './fs-walk.mjs'
import { hasDecisionAnchor } from './decision-anchors.mjs'
import { generatedPaths } from './capability-migration-audit.mjs'

const argv = process.argv.slice(2)
const rootArg = argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const BASELINE_REL = 'scripts/baselines/plugin-api-loc.json'
const CRATE_DIR = 'crates/symbio-plugin-api/src'

/** 物理行数：`\n` 分隔，末尾换行不算多一行。 */
export function physicalLines(text) {
  const norm = text.replace(/\r\n/g, '\n')
  return norm.split('\n').length - (norm.endsWith('\n') ? 1 : 0)
}

/**
 * 扫描 crate 的生产文件。
 *
 * @returns {{ files: {path: string, lines: number}[], total: number, error?: string }}
 */
export function scanCrate(root = ROOT) {
  const dir = path.join(root, CRATE_DIR)
  if (!fs.existsSync(dir)) return { files: [], total: 0, error: `${CRATE_DIR} 不存在` }
  // 清单镜像只登记存在性（FA-003），行数不参与判定（D-470）
  const generated = generatedPaths(root)
  const files = []
  for (const abs of walk(dir)) {
    if (!abs.endsWith('.rs') || isTestFile(abs)) continue
    const rel = path.relative(dir, abs).split(path.sep).join('/')
    files.push({
      path: rel,
      lines: physicalLines(fs.readFileSync(abs, 'utf8')),
      generated: generated.has(`${CRATE_DIR}/${rel}`),
    })
  }
  files.sort((a, b) => a.path.localeCompare(b.path))
  if (files.length === 0) return { files, total: 0, error: `${CRATE_DIR} 下没有生产 .rs` }
  const counted = files.filter((f) => !f.generated)
  return { files, total: counted.reduce((n, f) => n + f.lines, 0) }
}

/** 读基线；缺失或不是合法 JSON 都返回 `error`（调用方退 2）。 */
export function readBaseline(root = ROOT) {
  const file = path.join(root, BASELINE_REL)
  if (!fs.existsSync(file)) return { error: `基线缺失：${BASELINE_REL}` }
  let obj
  try {
    obj = JSON.parse(fs.readFileSync(file, 'utf8'))
  } catch (e) {
    return { error: `基线不是合法 JSON：${e.message}` }
  }
  if (!Array.isArray(obj?.files)) return { error: `基线缺少 files 数组：${BASELINE_REL}` }
  if (!obj.total || typeof obj.total !== 'object') {
    return { error: `基线缺少 total 对象：${BASELINE_REL}` }
  }
  return { note: obj.note ?? '', files: obj.files, total: obj.total }
}

/** 基数判据：正整数。 */
const isCount = (n) => Number.isInteger(n) && n > 0
const isAdr = (s) => typeof s === 'string' && /^D-\d{3}$/.test(s.trim())

/**
 * 基线条目的形状与署名（FA-003）。决策号存在性走 `decision-anchors`（唯一扫描实现）。
 *
 * @param {{files: object[], total: object}} baseline
 * @param {{path: string}[]} scanned  当前生产文件，用来判「登记的文件还在不在」
 */
export function checkBaseline(baseline, scanned, root = ROOT) {
  const errors = []
  const seen = new Set()
  const live = new Set(scanned.map((f) => f.path))
  const cache = new Map()
  const exists = (adr) => {
    if (!cache.has(adr)) cache.set(adr, hasDecisionAnchor(adr.replace(/^D-/, ''), root))
    return cache.get(adr)
  }
  for (const [i, f] of baseline.files.entries()) {
    const where = `${BASELINE_REL}#files[${i}]`
    if (!f || typeof f !== 'object' || Array.isArray(f)) {
      errors.push({ code: 'FA-003', message: `${where} 不是对象` })
      continue
    }
    if (typeof f.path !== 'string' || f.path.trim() === '') {
      errors.push({ code: 'FA-003', message: `${where} 缺少 path` })
      continue
    }
    if (seen.has(f.path)) errors.push({ code: 'FA-003', message: `${where} 重复登记 ${f.path}` })
    seen.add(f.path)
    const retired = f.retired !== undefined
    if (retired && (typeof f.retired !== 'string' || f.retired.trim() === '')) {
      errors.push({ code: 'FA-003', message: `${where}（${f.path}）retired 必须是非空字符串（写新家路径）` })
    }
    if (retired) {
      if (f.lines !== 0) {
        errors.push({
          code: 'FA-003',
          message: `${where}（${f.path}）退役条目的 lines 必须是 0（实为 ${f.lines}）—— 退役不留下限余量`,
        })
      }
      if (live.has(f.path)) {
        errors.push({
          code: 'FA-003',
          message: `${where} 登记了退役的 ${f.path}（→ ${f.retired}），但它仍在 ${CRATE_DIR} 下 —— 要么没删干净，要么又长回来了`,
        })
      }
    } else if (!isCount(f.lines)) {
      errors.push({ code: 'FA-003', message: `${where}（${f.path}）缺少正整数 lines` })
    }
    if (!isAdr(f.adr)) {
      errors.push({ code: 'FA-003', message: `${where}（${f.path}）缺少 adr（D-NNN）` })
    } else if (!exists(f.adr.trim())) {
      errors.push({
        code: 'FA-003',
        message: `${where}（${f.path}）挂的决策 ${f.adr} 不存在 ⇒ 每个数字都要带能打开的 D 号`,
      })
    }
    if (!retired && !live.has(f.path)) {
      errors.push({
        code: 'FA-003',
        message: `${where} 登记了 ${f.path}，但 ${CRATE_DIR} 下没有这个生产文件 —— 删文件必须同批删基线条目（防「删了再加」抹平）`,
      })
    }
  }
  const t = baseline.total
  if (!isCount(t.lines)) {
    errors.push({ code: 'FA-003', message: `${BASELINE_REL}#total 缺少正整数 lines` })
  }
  if (!isAdr(t.adr)) {
    errors.push({ code: 'FA-003', message: `${BASELINE_REL}#total 缺少 adr（D-NNN）` })
  } else if (!exists(t.adr.trim())) {
    errors.push({
      code: 'FA-003',
      message: `${BASELINE_REL}#total 挂的决策 ${t.adr} 不存在`,
    })
  }
  return errors
}

/** 行数对账：增多判 FA-001，减少未同批下调判 FA-002。 */
export function judge(scanned, baseline) {
  const errors = []
  const nowByPath = new Map(
    scanned.files.filter((f) => !f.generated).map((f) => [f.path, f.lines]),
  )
  const compare = (label, now, was) => {
    if (!isCount(was)) return
    if (now > was) {
      errors.push({
        code: 'FA-001',
        message: `${label} 实测 ${now} 行 / 基线 ${was} 行（超 ${now - was} 行）—— 门面自有实现只许降：实现往 symbio-sdk / symbio-host 搬，不许写回 plugin-api`,
      })
    } else if (now < was) {
      errors.push({
        code: 'FA-002',
        message: `${label} 实测 ${now} 行 / 基线 ${was} 行（少 ${was - now} 行）—— 行数下降必须同批下调 ${BASELINE_REL}，否则基线留出的余量会让「删 N 行再加 N 行」逃过 FA-001`,
      })
    }
  }
  for (const f of baseline.files) {
    if (typeof f?.path !== 'string' || f.path.trim() === '') continue
    const now = nowByPath.get(f.path)
    if (now === undefined) continue // 缺文件由 FA-003 报，别在这里重复一条
    compare(f.path, now, f.lines)
  }
  compare('合计', scanned.total, baseline.total?.lines)
  return errors
}

const baseline = readBaseline(ROOT)
const scanned = scanCrate(ROOT)

if (scanned.error) {
  console.error(red(`范围读不出：${scanned.error} —— 棘轮审计不许在看不见生产文件时通过`))
  process.exit(2)
}
if (baseline.error) {
  console.error(red(`范围读不出：${baseline.error} —— 棘轮审计不许在看不见基线时通过`))
  process.exit(2)
}

const errors = [...checkBaseline(baseline, scanned.files, ROOT), ...judge(scanned, baseline)]

if (errors.length) {
  for (const e of errors) console.log(red(`[ERROR] ${e.code} ${e.message}`))
  console.log(
    `facade-loc-audit 未通过：${errors.length} 处违规（生产合计 ${scanned.total} 行 / 基线 ${baseline.total.lines} 行）`,
  )
  process.exit(1)
}

console.log(
  green(
    `facade-loc-audit 通过：门面自有实现 ${baseline.files.length} 项 / 生产合计 ${scanned.total} 行，` +
      `基线合计 ${baseline.total.lines} 行（只许降）`,
  ),
)
for (const f of baseline.files) {
  console.log(dim(`  · ${f.path}  ${f.lines} 行`))
}
