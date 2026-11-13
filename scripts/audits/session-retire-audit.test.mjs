/**
 * session-retire-audit 回归测试
 *
 * 判据面（SRA-001/002/003，见 D-451）：
 *  - SRA-001：豁免账里每条符号都必须在 `symbio-plugin-session/src` 里**真被用到**；
 *  - SRA-002：符号条数 ≤ 基线（只许降）；
 *  - SRA-003：`expires` 是**将来的**合法日期、`retire` 写明删除动作、`knife` 形如登记号。
 *
 * 这把尺最危险的失效是**读数失真**：
 *   ① 余量留在账上却没人报（E-009 只判「用了的必须在账上」——正是这个单向判据让
 *      15 条零命中条目静默积累）；
 *   ② 注释里提到某符号就被当成「在用」（那会让已退役的资格永久留在账上）；
 *   ③ 账本涨了不报（尺就失去意义）；
 *   ④ 豁免没有到期日 / `retire` 不写删除动作 ⇒ 变成永久豁免；
 *   ⑤ 输入读不出（JSON 缺失、源码目录被搬走）时静默通过。
 *
 * 夹具树用临时目录 + 脚本的 `--root=<dir>`（与 `port-type-coupling-audit.test.mjs` 同约定），
 * 不碰真实 crates。
 *
 * 跑法：node --test scripts/audits/session-retire-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./session-retire-audit.mjs', import.meta.url))

const EXEMPTION_REL = 'scripts/exemptions/plugin-session-sdk.json'
const SESSION_SRC_REL = 'crates/symbio-plugin-session/src'

/** 干净夹具：两条符号都真被用、到期日在将来、`retire` 写明删除、`knife` 是登记号。 */
function cleanFiles() {
  return {
    [EXEMPTION_REL]: JSON.stringify(
      {
        rule: 'E-009',
        crate: 'symbio-plugin-session',
        dependency: 'symbio-sdk',
        expires: '2099-12-31',
        knife: 'D-460',
        retire: '随身份 `session` 保留（D-460）；条数只许降；2027-06-30 是复核日',
        reason: '夹具',
        symbols: ['ClockNow', 'ReportDraft'],
      },
      null,
      2,
    ),
    [`${SESSION_SRC_REL}/lib.rs`]: [
      'use symbio_sdk::ClockNow;',
      'use symbio_sdk::ReportDraft;',
      '',
      'pub fn tick(d: ReportDraft) -> u64 { ClockNow::now() }',
      '',
    ].join('\n'),
    // 事实源开面：SRA-004 要求**恰好一家**（D-458）。夹具给一家，与真实仓同形。
    [`${SESSION_SRC_REL}/store/fact_source.rs`]: [
      'static SESSION_FACT_OPENER: SessionFactOpener = SessionFactOpener;',
      'symbio_sdk::submit_fact_source_opener!(SESSION_FACT_OPENER);',
      '',
    ].join('\n'),
    // 地址根：SRA-005 要求**至多一家**（D-459）。夹具给一家（vdfs 插件那一处），与真实仓同形。
    'crates/symbio-plugin-vdfs/src/plugin.rs': [
      '// 根名的静态声明：字面量只在本插件内。',
      'symbio_plugin_api::inventory::submit! {',
      '    symbio_plugin_api::AddrRootDecl(VDFS_ADDR_ROOT)',
      '}',
      '',
    ].join('\n'),
  }
}

function sandbox(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'sr-audit-'))
  for (const [rel, content] of Object.entries(files)) {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, content)
  }
  return { root, cleanup: () => fs.rmSync(root, { recursive: true, force: true }) }
}

function run(root, extra = []) {
  const r = spawnSync(process.execPath, [script, `--root=${root}`, ...extra], {
    encoding: 'utf8',
    env: { ...process.env, NO_COLOR: '1' },
    timeout: 30_000,
  })
  assert.ifError(r.error)
  return { status: r.status, out: `${r.stdout}${r.stderr}` }
}

/** 只改豁免 JSON，源码夹具不动。 */
function withExemption(files, patch) {
  const d = JSON.parse(files[EXEMPTION_REL])
  patch(d)
  return { ...files, [EXEMPTION_REL]: JSON.stringify(d, null, 2) }
}

test('干净夹具树 ⇒ 通过，且三条判据都读到输入', () => {
  const box = sandbox(cleanFiles())
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
    assert.match(r.out, /SRA-001/)
    assert.match(r.out, /SRA-002/)
    assert.match(r.out, /SRA-003/)
  } finally {
    box.cleanup()
  }
})

test('SRA-001：账上一条符号零命中 ⇒ 红并点名（余量冒充进度）', () => {
  const box = sandbox(
    withExemption(cleanFiles(), (d) => {
      d.symbols = ['ClockNow', 'ReportDraft', 'NeverUsed']
    }),
  )
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SRA-001.*`NeverUsed`/s)
    assert.match(r.out, /不经 .*symbio_sdk::.* 路径命中/)
  } finally {
    box.cleanup()
  }
})

test('SRA-001：名字在源码里出现、但只走 kernel 路径 ⇒ 红（按路径判，不按名字）', () => {
  const files = cleanFiles()
  files[`${SESSION_SRC_REL}/lib.rs`] = [
    'use symbio_sdk::ClockNow;',
    'use symbio_kernel::chat::chat_node::ChatNode;',
    'pub fn tick(n: ChatNode) -> u64 { ClockNow::now() }',
    '',
  ].join('\n')
  const box = sandbox(
    withExemption(files, (d) => {
      d.symbols = ['ClockNow', 'ChatNode']
    }),
  )
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SRA-001.*`ChatNode`/s)
    assert.doesNotMatch(r.out, /`ClockNow`/, '真用的那条不该被点名')
  } finally {
    box.cleanup()
  }
})

test('SRA-001：符号只出现在注释里 ⇒ 仍红（注释不是用法）', () => {
  const files = cleanFiles()
  files[`${SESSION_SRC_REL}/lib.rs`] = [
    'use symbio_sdk::ClockNow;',
    '// 这里曾经用 ReportDraft 做汇报（已迁走）',
    '/// ReportDraft 的旧读口已退役',
    'pub fn tick() -> u64 { ClockNow::now() }',
    '',
  ].join('\n')
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /`ReportDraft`/)
  } finally {
    box.cleanup()
  }
})

test('SRA-002：符号条数超过基线 ⇒ 红（这条边只许收窄）', () => {
  // 基线是 50：造 52 条**都真被用**的符号，把 SRA-001 摘干净，只看 SRA-002。
  const symbols = Array.from({ length: 52 }, (_, i) => `Sym${i}`)
  const files = withExemption(cleanFiles(), (d) => {
    d.symbols = symbols
  })
  files[`${SESSION_SRC_REL}/lib.rs`] = symbols.map((s) => `use symbio_sdk::${s};`).join('\n')
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SRA-002.*52 条 > 基线 50/s)
    assert.doesNotMatch(r.out, /SRA-001/)
  } finally {
    box.cleanup()
  }
})

test('SRA-003：到期日已过 ⇒ 红（限期豁免过期即红）', () => {
  const box = sandbox(
    withExemption(cleanFiles(), (d) => {
      d.expires = '2000-01-01'
    }),
  )
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SRA-003.*已过期/s)
  } finally {
    box.cleanup()
  }
})

test('SRA-003：到期日不可解析 ⇒ 红（没有到期日就是永久豁免）', () => {
  const box = sandbox(
    withExemption(cleanFiles(), (d) => {
      d.expires = '待定'
    }),
  )
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SRA-003.*不是可解析的日期/s)
  } finally {
    box.cleanup()
  }
})

test('SRA-003：retire 空 / 不含删除动作 ⇒ 红', () => {
  for (const retire of ['', '以后再看', '   ']) {
    const box = sandbox(
      withExemption(cleanFiles(), (d) => {
        d.retire = retire
      }),
    )
    try {
      const r = run(box.root)
      assert.equal(r.status, 1, r.out)
      assert.match(r.out, /SRA-003/)
    } finally {
      box.cleanup()
    }
  }
})

test('SRA-003：knife 不是登记号 ⇒ 红（这条账要指得出主人）', () => {
  for (const knife of ['', 'T034', 'T-34', 'knife-1']) {
    const box = sandbox(
      withExemption(cleanFiles(), (d) => {
        d.knife = knife
      }),
    )
    try {
      const r = run(box.root)
      assert.equal(r.status, 1, r.out)
      assert.match(r.out, /SRA-003.*knife/s)
    } finally {
      box.cleanup()
    }
  }
})

test('输入读不出：豁免 JSON 缺失 ⇒ 红（宁红不静默）', () => {
  const files = cleanFiles()
  delete files[EXEMPTION_REL]
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SRA-003.*不存在/s)
  } finally {
    box.cleanup()
  }
})

test('输入读不出：session 源码目录被搬走 ⇒ 红（没有源码可量）', () => {
  const files = cleanFiles()
  // 夹具里每个 `src/` 下的文件都要删掉——留一个文件，「目录不存在」就没成立，
  // 判据会退化成「符号零命中」（那也是红，但红的理由不是这一条）。
  for (const rel of Object.keys(files)) {
    if (rel.startsWith(`${SESSION_SRC_REL}/`)) delete files[rel]
  }
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SRA-001.*没有源码可量/s)
  } finally {
    box.cleanup()
  }
})

// ================= --breakdown：账本按「住处」分解（读数，不是判据） =================
//
// 分解口径会随目录结构漂，所以这里钉的是**算术**：一条符号只落一个桶、四个桶的和等于
// 账本条数。具体数字对不对由人看报告，等式由用例守。

/** 取「标签 … N」那一行的 N（标签里有空格，故从尾部取）。 */
function bucketCount(out, label) {
  const line = out.split(String.fromCharCode(10)).find((l) => l.includes(label))
  assert.ok(line, `报告里没有「${label}」这一行：${out}`)
  const n = Number(line.trim().split(' ').pop())
  assert.ok(Number.isFinite(n), `「${label}」的条数读不出：${line}`)
  return n
}

test('--breakdown：每块代码一条符号 ⇒ 分桶互斥且合计等于条数', () => {
  const files = cleanFiles()
  files[`${SESSION_SRC_REL}/store/wal.rs`] = 'use symbio_sdk::ClockNow;'
  files[`${SESSION_SRC_REL}/plugin.rs`] = 'use symbio_sdk::ReportDraft;'
  delete files[`${SESSION_SRC_REL}/lib.rs`]
  const box = sandbox(files)
  try {
    const r = run(box.root, ['--breakdown'])
    assert.equal(r.status, 0, r.out)
    assert.ok(r.out.includes('分解（2 条）'), r.out)
    assert.equal(bucketCount(r.out, '只出现在 装配面'), 1, r.out)
    assert.equal(bucketCount(r.out, '只出现在 存储'), 1, r.out)
    assert.equal(bucketCount(r.out, '两处都有'), 0, r.out)
    assert.equal(bucketCount(r.out, '两处都没有'), 0, r.out)
  } finally {
    box.cleanup()
  }
})

test('--breakdown：同一条符号两处都出现 ⇒ 计入「两处都有」，不重复计入两侧', () => {
  const files = cleanFiles()
  files[`${SESSION_SRC_REL}/store/wal.rs`] = 'use symbio_sdk::ClockNow; use symbio_sdk::ReportDraft;'
  files[`${SESSION_SRC_REL}/plugin.rs`] = 'use symbio_sdk::ReportDraft;'
  delete files[`${SESSION_SRC_REL}/lib.rs`]
  const box = sandbox(files)
  try {
    const r = run(box.root, ['--breakdown'])
    assert.equal(r.status, 0, r.out)
    assert.equal(bucketCount(r.out, '只出现在 存储'), 1, r.out)
    assert.equal(bucketCount(r.out, '两处都有'), 1, r.out)
    assert.equal(bucketCount(r.out, '只出现在 装配面'), 0, r.out)
  } finally {
    box.cleanup()
  }
})

// ================= --coverage：代码按「谁的活」分解（读数，不是判据） =================
//
// 与 `--breakdown` 同一条纪律：分桶口径会随目录结构漂，所以钉的是**算术**——每个文件只落
// 一个桶、五个桶的和等于表头总数、测试文件不进账。具体数字由人看报告（D-457 就是用这份
// 读数发现「三条边只点到全 crate 的三成」）。

/** 取「标签 … N 行 …%」那一行的 N。 */
function coverageRow(out, label) {
  const line = out.split(String.fromCharCode(10)).find((l) => l.includes(label))
  assert.ok(line, `报告里没有「${label}」这一行：${out}`)
  const m = /(\d+)\s+行\s+[\d.]+%/.exec(line)
  assert.ok(m, `「${label}」的行数读不出：${line}`)
  return Number(m[1])
}

function coverageTotal(out) {
  const m = /非测试源码 (\d+) 行/.exec(out)
  assert.ok(m, `报告里没有表头总数：${out}`)
  return Number(m[1])
}

test('--coverage：五个桶互斥且合计等于表头总数', () => {
  const files = cleanFiles()
  // 每块一地一个文件，另加两个测试文件（必须被排除）。
  files[`${SESSION_SRC_REL}/store/wal.rs`] = 'a\nb\n'
  files[`${SESSION_SRC_REL}/store/wal.test.rs`] = 'x\n'.repeat(80)
  files[`${SESSION_SRC_REL}/store/tests.rs`] = 'y\n'.repeat(80)
  files[`${SESSION_SRC_REL}/plugin/vdfs_catalog.rs`] = 'a\nb\nc\n'
  files[`${SESSION_SRC_REL}/capabilities.rs`] = 'a\n'
  const box = sandbox(files)
  try {
    const r = run(box.root, ['--coverage'])
    assert.equal(r.status, 0, r.out)
    const labels = [
      '边二 装配',
      '边三 存储·转写',
      '地址面（VDFS）',
      '开轮序列',
      '没有归属的其余代码',
    ]
    const sum = labels.reduce((n, l) => n + coverageRow(r.out, l), 0)
    assert.equal(sum, coverageTotal(r.out), r.out)
    // 测试文件不进账：80 行的两个测试文件若被计入，总数会翻好几倍。
    assert.ok(coverageTotal(r.out) < 80, `测试源码不该计入：${r.out}`)
  } finally {
    box.cleanup()
  }
})

test('--coverage：判不出的文件落「没有归属」并被逐条列出，不静默并进别的桶', () => {
  const files = cleanFiles()
  files[`${SESSION_SRC_REL}/stats_reading.rs`] = 'a\nb\n'
  const box = sandbox(files)
  try {
    const r = run(box.root, ['--coverage'])
    assert.equal(r.status, 0, r.out)
    assert.equal(coverageRow(r.out, '边二 装配'), 0, r.out)
    assert.match(r.out, /stats_reading\.rs/s, '判不出的文件要列出来')
  } finally {
    box.cleanup()
  }
})

// ================= SRA-004：事实源开面是单槽（迁移的第一步必须响） =================
//
// `declared_fact_source_opener()` 取 `inventory::iter().next()`，没有去重、没有按域分派。
// 存储迁移的第一步必然是「注册第二个开面」，而这一步在单槽下是**静的**：谁生效由链接
// 顺序决定。两条用例分别钉住「多一家即红」与「零家也红」——后者是迁移走完却没换新家
// 的形态，读面会硬错。

test('SRA-004：两处登记开面 ⇒ 红，并点名两处（单槽下第二个是静默死码）', () => {
  const files = cleanFiles()
  files[['crates', 'symbio-plugin-session', 'src', 'store', 'second_opener.rs'].join('/')] =
    'static OTHER: OtherOpener = OtherOpener;' + String.fromCharCode(10) +
    'symbio_sdk::submit_fact_source_opener!(OTHER);' + String.fromCharCode(10)
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SRA-004.*单槽/s)
    assert.match(r.out, /fact_source\.rs:2/s, '第一个登记点要点名')
    assert.match(r.out, /second_opener\.rs:2/s, '第二个登记点也要点名')
  } finally {
    box.cleanup()
  }
})

test('SRA-004：零处登记 ⇒ 红（不是「边三收窄完成」，是没有人开面了）', () => {
  const files = cleanFiles()
  delete files[[SESSION_SRC_REL, 'store', 'fact_source.rs'].join('/')]
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SRA-004.*零登记/s)
  } finally {
    box.cleanup()
  }
})

test('SRA-004：注释里写宏名不算登记（去注释后判）', () => {
  const files = cleanFiles()
  files[[SESSION_SRC_REL, 'store', 'fact_source.rs'].join('/')] =
    '// 将来会有人写 submit_fact_source_opener!(X);' + String.fromCharCode(10)
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SRA-004.*零登记/s, '注释不是登记')
  } finally {
    box.cleanup()
  }
})

// ================= SRA-005：地址根也是单槽（0 家是合法降级） =================
//
// 与 SRA-004 同一个病（`inventory::iter().next()` 单槽），但**两个方向的判定不同**：
// 事实源开面 0 家会让读面硬错，所以 0 家红；地址根 0 家只是「构建里没有 vdfs 插件」的
// 既定降级（`declared_addr_root()` 返回 `None`），所以 0 家绿 + 提示。这三条用例钉的
// 就是这个差别——判据不许把合法的降级当成错误，也不许把静默的第二家当成正常。

test('SRA-005：两处声明地址根 ⇒ 红，并点名两处（单槽下第二个是静默死码）', () => {
  const files = cleanFiles()
  files[['crates', 'symbio-plugin-other', 'src', 'plugin.rs'].join('/')] =
    'symbio_plugin_api::inventory::submit! {' + String.fromCharCode(10) +
    '    symbio_plugin_api::AddrRootDecl("other-root")' + String.fromCharCode(10) +
    '}' + String.fromCharCode(10)
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SRA-005.*单槽/s)
    assert.match(r.out, /symbio-plugin-vdfs\/src\/plugin\.rs:3/s, '第一个声明点要点名')
    assert.match(r.out, /symbio-plugin-other\/src\/plugin\.rs:2/s, '第二个声明点也要点名')
  } finally {
    box.cleanup()
  }
})

test('SRA-005：零处声明 ⇒ 绿 + 提示（没有 vdfs 插件是合法降级，不是错误）', () => {
  const files = cleanFiles()
  delete files['crates/symbio-plugin-vdfs/src/plugin.rs']
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
    assert.match(r.out, /SRA-005[^\n]*零声明/s)
    assert.match(r.out, /合法降级/s)
  } finally {
    box.cleanup()
  }
})

test('SRA-005：注释里写声明不算声明（去注释后判）', () => {
  const files = cleanFiles()
  files['crates/symbio-plugin-vdfs/src/plugin.rs'] =
    '// 将来会有人写 symbio_plugin_api::AddrRootDecl(VDFS_ADDR_ROOT)' + String.fromCharCode(10) +
    '// 现在这一行只是注释' + String.fromCharCode(10)
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
    assert.match(r.out, /SRA-005[^\n]*零声明/s, '注释不是声明')
  } finally {
    box.cleanup()
  }
})

// ============ SRA-003 的「保留」分支（D-460：删除这个出口写不出来） ============
//
// 终态是「随身份留下、只许降」时，「将来删除」是不可兑现的承诺。判据允许写「保留」，
// 但要求同时写出**依据的决策号**——否则「保留」就是一句谁都不用负责的话，与本脚本
// 存在的理由正好相反（它就是为了消灭「没有出口的将来会删」）。

test('SRA-003：写「保留」但没给依据决策号 ⇒ 红（一句话没人负责）', () => {
  const files = cleanFiles()
  files[EXEMPTION_REL] = JSON.stringify(
    {
      rule: 'E-009',
      crate: 'symbio-plugin-session',
      dependency: 'symbio-sdk',
      expires: '2099-12-31',
      knife: 'D-460',
      retire: '随身份保留，以后再说',
      reason: '夹具',
      symbols: ['ClockNow', 'ReportDraft'],
    },
    null,
    2,
  )
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SRA-003.*可兑现的处置/s)
  } finally {
    box.cleanup()
  }
})

test('SRA-003：写「保留 + 依据决策号」⇒ 绿（这是本账今天的真实形态）', () => {
  const files = cleanFiles()
  files[EXEMPTION_REL] = JSON.stringify(
    {
      rule: 'E-009',
      crate: 'symbio-plugin-session',
      dependency: 'symbio-sdk',
      expires: '2099-12-31',
      knife: 'D-460',
      retire: '随身份 `session` 保留（D-460）；条数只许降',
      reason: '夹具',
      symbols: ['ClockNow', 'ReportDraft'],
    },
    null,
    2,
  )
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
  } finally {
    box.cleanup()
  }
})
