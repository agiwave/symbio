#!/usr/bin/env node
/**
 * dynamic-port-registration-audit — 动态注册四类反例的静态文本判据（D-445 / D-447 ③）
 *
 * ## 它判什么
 *
 * 逐行扫 `crates/` 下的生产 `.rs`（排除 `.test.rs`），剥掉注释后匹配四类**反例形状**；
 * 命中即逐条点名 `文件:行号`。退出码：0 = 四类形状都不存在；1 = 有命中，或判据没有输入。
 *
 * | 码 | 反例形状 | 它要钉死的走样 | 承接体（应存在的守卫） |
 * |---|---|---|---|
 * | DPR-001 | `strip_prefix("<port:")` / `split_once(">:")` | `capability_port_check` 里靠 `<port:…>` 行名**反推**端口集，而不是按端口键判重 | 一次遍历按行上的 `PortPayload::port` 分组，同键两绑定当场落错误 |
 * | DPR-002 | `executors.insert(` | host 折执行体表用裸 `insert` **覆盖**同端口键的旧绑定 | 按端口键的冲突检查 + 拒绝重复（不是 insert 覆盖） |
 * | DPR-003 | `list(TOOL)` | 工具清单是 `list(TOOL)` 的**全量投影**，没有过滤有效绑定 | 清单 = 显式暴露 ∩ 当前作用域 ∩ 有效绑定 |
 * | DPR-004 | `p2.route(` | 工具派发「能力表未命中」就**回落宿主路由**兜底 | 未知 wire 名 / 未暴露名称一律失败事实，不回落 |
 *
 * ## 为什么是静态文本判据（不是靠注释声明）
 *
 * D-447 ③ 要求「每类反例都有可跑的判据，不靠注释声明」。本脚本本身就是那道**可执行
 * 守卫**：逐行读真实代码、逐条正则匹配，而不是在某处写一句「已删除」。注释一律剥掉——
 * doc comment 里写「`strip_prefix("<port:")` 那层反推已删除」是**叙述**，不该把已经
 * 完成的解耦判成红；反过来，在真实代码行上面补一句「已删除」也**骗不过**本判据，
 * 因为剩下的代码还在。回归测试里两条用例分别钉住这两个方向。
 *
 * 判据钉的是**反例形状不存在**；正面守卫由删掉这些形状后的代码本身承担——想让反例
 * 回来，就得先拆掉守卫，静态文本看得见。
 *
 * ## 已知边界（写在绿灯里，别把它当证明）
 *
 * - **纯文本、不做 Rust 类型/控制流解析**：`use ... as X` 起别名、`macro_rules!` 拼出的
 *   调用、字符串字面量里的同名文本不在语义内（别名会漏、字符串会误报；本仓当前 0 处）。
 *   字符串里的 `//` / C 风格块注释起点会被当注释处理，同样按边界看待。
 * - **DPR-002 命中 `executors.insert(` 即红**，即便同一文件已有 `owners.insert(` 冲突检查：
 *   D-445 ③ 的原话是「不 `insert` 覆盖」——`insert` 这个原语本身仍能静默覆盖（同 owner
 *   同键二次绑定），冲突检查是**承接体**，不是保留裸 insert 的许可。终态要换成拒绝重复的
 *   受检插入。夹具里「只有注释声明有冲突检查、没有真实守卫」也必须红。
 * - **DPR-003 认的是 `list(TOOL)` 这一规范形态**（D-447 ③ 点名的裸全量投影写法）。带族
 *   限定并在 `exposed_tools(... &bound_ports)` 里过滤有效绑定的承接体、`declared_table_of`
 *   与名字解析里的 `list(family::TOOL)` 合法读表都不在扫描面内——按字面量收窄是为避免
 *   把合法读表误判成清单装配，代价是换一种拼法的全量投影要靠别的判据（arch-shape）兜。
 * - `.test.rs` 排除：夹具里的反例形状不是生产路径。
 * - **读不出 ⇒ 红**：`crates/` 下 0 个生产 `.rs` 时不亮绿灯（「没什么可判 ⇒ 通过」是这族
 *   守卫最危险的假绿）。
 * - **当下可能红**：这些 Rust 文件正在并行解耦，脚本写好后未必立刻绿。按 D-447
 *   「不为消红放宽判据」，红就如实报，不改 crates、不改判据。
 *
 * 用法：node scripts/audits/dynamic-port-registration-audit.mjs [--root=<仓库根>]
 * 退出码：0 = 通过；1 = 有反例形状或判据没有输入。
 */
// @gate guard

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, dim } from '../color.mjs'
import { blankComments as blankSourceComments } from '../rust-scan.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : path.resolve(scriptDir, '..', '..')

/** 四类反例：`forbidden` 命中一行即一条违规。正则不带 `g`（`RegExp.test` 无状态）。 */
const CRITERIA = [
  {
    code: 'DPR-001',
    label: '端口判重按端口键',
    forbidden: /strip_prefix\("<port:"|split_once\(">:"\)/,
    why: 'capability_port_check 里不得再有 `<port:…>` 行名反推（D-445 ③）',
  },
  {
    code: 'DPR-002',
    label: 'host 折执行体表不裸覆盖',
    forbidden: /executors\.insert\(/,
    why: '不得有裸 `executors.insert(` 覆盖；同键两绑定要当场指名冲突（D-445 ③）',
  },
  {
    code: 'DPR-003',
    label: '工具清单装配过滤有效绑定',
    forbidden: /\blist\s*\(\s*TOOL\s*\)/,
    why: '不得是裸 `list(TOOL)` 全量投影；清单只认显式暴露 ∩ 有效绑定（D-445 ①）',
  },
  {
    code: 'DPR-004',
    label: '工具派发无宿主路由回落',
    forbidden: /\bp2\.route\(/,
    why: '不得有「未命中回落宿主路由」的 `p2.route(` 兜底（D-445 ②）',
  },
]

/** 注释剥离：唯一真源 rust-scan.mjs（保留行结构，返回按行数组）。 */
const blankComments = (text) => blankSourceComments(text).join('\n')

/** 递归收 `crates/` 下的生产 `.rs`（排除 `.test.rs`），相对 ROOT 的正斜杠路径。 */
function walkProductionRs(absDir, out = []) {
  let entries
  try {
    entries = fs.readdirSync(absDir, { withFileTypes: true })
  } catch {
    return out
  }
  for (const e of entries) {
    if (e.name.startsWith('.')) continue
    const full = path.join(absDir, e.name)
    if (e.isDirectory()) {
      if (e.name === 'target' || e.name === 'node_modules') continue
      walkProductionRs(full, out)
    } else if (e.isFile() && e.name.endsWith('.rs') && !e.name.endsWith('.test.rs')) {
      out.push(path.relative(ROOT, full).split(path.sep).join('/'))
    }
  }
  return out
}

function main() {
  const files = walkProductionRs(path.join(ROOT, 'crates'))
  const violations = []

  for (const rel of files) {
    const text = fs.readFileSync(path.join(ROOT, rel), 'utf8').replace(/\r\n/g, '\n')
    const original = text.split('\n')
    const stripped = blankComments(text).split('\n')
    for (let i = 0; i < stripped.length; i++) {
      for (const c of CRITERIA) {
        if (c.forbidden.test(stripped[i])) {
          violations.push({
            code: c.code,
            rel,
            line: i + 1,
            text: (original[i] ?? stripped[i]).trim(),
            why: c.why,
          })
        }
      }
    }
  }

  // 读不出 ⇒ 红（不许「没什么可判 ⇒ 通过」）。
  if (files.length === 0) {
    violations.push({ code: 'DPR', rel: 'crates/', line: 0, text: '读不出：crates/ 下 0 个生产 .rs——判据没有输入', why: '' })
  }

  console.log('--- dynamic-port-registration-audit: 动态注册四类反例（DPR-001…004）---')
  if (violations.length > 0) {
    console.log(red(`  ✗ dynamic-port-registration-audit 红：${violations.length} 处反例形状 / 读不出`))
    for (const v of violations) {
      console.log(red(`    ${v.code} ${v.rel}:${v.line}  ${v.text}`))
      if (v.why) console.log(dim(`      ${v.why}`))
    }
    console.log(dim('    D-447 ③：每类反例都要有可跑判据，不靠注释声明；不为消红放宽判据。'))
    process.exit(1)
  }
  console.log(
    green(
      `  ✓ dynamic-port-registration-audit 通过：${files.length} 份生产 .rs 里 ` +
        '0 处 `strip_prefix` 反推 / 裸 `executors.insert` / 裸 `list(TOOL)` / `p2.route` 回落',
    ),
  )
  process.exit(0)
}

main()
