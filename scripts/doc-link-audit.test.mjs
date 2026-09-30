/**
 * doc-link-audit 回归测试 —— 证明它**会红**
 *
 * 补它的两个理由：① 它是 `gate.mjs` 声称"每个判定型守卫都先跑回归测试"里**缺的那个**
 * （2026-09-20 复核发现 8 个里缺 3 个）；② 它原先只有 `--strict` 才失败，而门禁从不带
 * 该参数 ⇒ **它从未真的红过**。后者已一并修掉（失效链接是精确判定，没有"疑似"中间态）。
 *
 * 用法：node --test scripts/doc-link-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./doc-link-audit.mjs', import.meta.url))

/** 脚本要求 5 个扫描根都存在（缺了会以「找不到扫描根」退出 1），故夹具先铺空目录 */
const SCAN_ROOTS = ['docs', 'symbio/src', 'tauri', 'cli', 'examples']

function audit(files, argv = []) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-link-audit-'))
  try {
    for (const d of SCAN_ROOTS) fs.mkdirSync(path.join(root, d), { recursive: true })
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const result = spawnSync(process.execPath, [script, `--root=${root}`, ...argv], {
      cwd: root,
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 20_000,
    })
    assert.ifError(result.error)
    return result
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

// ── 命中：指向不存在的文件 ──────────────────────────────────────────────
test('失效链接 → 失败（默认即失败，不再需要 --strict）', () => {
  const r = audit({ 'docs/a.md': '[去这儿](./nope.md)\n' })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /nope\.md/)
})

test('失效链接：带锚点也要判（先把 # 后段剥掉再查文件）', () => {
  const r = audit({ 'docs/a.md': '[去这儿](./nope.md#section)\n' })
  assert.equal(r.status, 1)
})

test('失效链接：上级目录的相对路径同样判', () => {
  const r = audit({ 'docs/sub/a.md': '[上级](../../nope.md)\n' })
  assert.equal(r.status, 1)
})

// ── 不误报 ──────────────────────────────────────────────────────────────
test('有效链接 → 通过', () => {
  const r = audit({
    'docs/a.md': '[去这儿](./b.md)\n',
    'docs/b.md': '# B\n',
  })
  assert.equal(r.status, 0)
})

test('外链 / 空目标一律跳过（不是失效链接）；纯锚点归 D-007 判标题存在性', () => {
  const r = audit({
    'docs/a.md':
      '# section\n\n[外链](https://example.com/x)\n[锚点](#section)\n[邮件](mailto:a@b.c)\n[空]()\n',
  })
  assert.equal(r.status, 0, r.stdout)
  // 外链 / 邮件 / 空目标在计数**之前**就跳过 ⇒ 不进 D-001 的「相对链接」数；
  // 纯锚点没有文件目标，也不进该数，但 D-007 会查它的标题
  assert.match(r.stdout, /扫描相对链接 0 条/)
  assert.match(r.stdout, /D-007 站内锚点：判定 1 条，失效 0 条/)
})

test('docs/archive/ 整体豁免 → 其中的失效链接不判（改写归档等于篡改历史）', () => {
  const r = audit({ 'docs/archive/old.md': '[当年的兄弟文档](./gone.md)\n' })
  assert.equal(r.status, 0)
  assert.match(r.stdout, /豁免/)
})

// ── D-002：过程文档必须归档 ────────────────────────────────────────────
// 归档是动作，能保持住的才是机制。这组用例钉住「它真的会红」——包括**豁免必须有理由**，
// 否则加一行注释就能把规则绕成橡皮图章。
const REVIEW_DOC = '# 某系统评审\n\n> **文档类型：评审（一次性结论，不是规范）**\n\n正文。\n'
const IMPLEMENTED_DOC = '# 某改动实施方案\n\n状态：**已实施**（S1–S6 全部落地）\n\n正文。\n'

test('D-002：评审类文档留在 docs/design/ → 失败', () => {
  const r = audit({ 'docs/design/review.md': REVIEW_DOC })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /docs\/design\/review\.md/)
})

test('D-002：过程文档留在**模块目录**同样失败（只扫 docs/ 会漏掉这一整类）', () => {
  const r = audit({ 'symbio/src/plugins/foo/docs/migration.md': REVIEW_DOC })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /symbio\/src\/plugins\/foo\/docs\/migration\.md/)
})

test('D-002：examples/ 不扫（是示例包内容，不是项目文档）', () => {
  const r = audit({ 'examples/pkg/docs/review.md': REVIEW_DOC })
  assert.equal(r.status, 0)
})

test('D-002：已落地的实施方案留在活跃目录 → 失败', () => {
  const r = audit({ 'docs/design/plan.md': IMPLEMENTED_DOC })
  assert.equal(r.status, 1)
})

test('D-002：同样的内容放进 docs/archive/ → 通过', () => {
  const r = audit({
    'docs/archive/review.md': REVIEW_DOC,
    'docs/archive/plan.md': IMPLEMENTED_DOC,
  })
  assert.equal(r.status, 0)
  assert.match(r.stdout, /应归档 0 篇/)
})

test('D-002：现行规范不误报（「状态：现行规范」不是过程文档）', () => {
  const r = audit({
    'docs/design/vdfs.md': '# VDFS 规范\n\n状态：现行规范（纯接口 + 统一文件系统 + 容器拓扑）\n',
  })
  assert.equal(r.status, 0)
})

test('D-002：豁免必须带理由，空理由视为未豁免', () => {
  const withReason = `<!-- doc-link-allow D-002: 本文是现行规范，「评审」指代码评审流程 -->\n${REVIEW_DOC}`
  assert.equal(audit({ 'docs/design/kept.md': withReason }).status, 0)

  const emptyReason = `<!-- doc-link-allow D-002:    -->\n${REVIEW_DOC}`
  assert.equal(audit({ 'docs/design/kept.md': emptyReason }).status, 1, '空理由不算豁免')
})

test('D-002：豁免写在第 15 行之后无效（只看头部自述）', () => {
  const late = `${REVIEW_DOC}\n${'填充行\n'.repeat(20)}<!-- doc-link-allow D-002: 理由 -->\n`
  assert.equal(audit({ 'docs/design/late.md': late }).status, 1)
})

// ── D-003：行数预算（活跃文档 ≤ 800 行）────────────────────────────────
// 钉住三件事：**会红**、**边界准确**（800 通过 / 801 失败）、**豁免只有 archive**。
const longDoc = (lines) => `# 长文档\n${'正文\n'.repeat(lines - 1)}`

test('D-003：801 行 → 失败（并报出超限行数）', () => {
  const r = audit({ 'docs/big.md': longDoc(801) })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /D-003/)
  assert.match(r.stdout, /docs\/big\.md/)
})

test('D-003：恰好 800 行 → 通过（上限是「不超过」）', () => {
  assert.equal(audit({ 'docs/exact.md': longDoc(800) }).status, 0)
})

test('D-003：模块文档同样判（不只扫 docs/）', () => {
  const r = audit({ 'symbio/src/plugins/foo/docs/big.md': longDoc(900) })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /symbio\/src\/plugins\/foo\/docs\/big\.md/)
})

test('D-003：根目录散落 md 同样判（README / CONTRIBUTING 也会臃肿）', () => {
  const r = audit({ 'CONTRIBUTING.md': longDoc(900) })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /CONTRIBUTING\.md/)
})

test('D-003：examples/ 不判（是示例包内容，不是项目文档）', () => {
  assert.equal(audit({ 'examples/pkg/docs/big.md': longDoc(900) }).status, 0)
})

test('D-003：docs/archive/ 不判（归档记录当时形态，改写等于篡改历史）', () => {
  assert.equal(audit({ 'docs/archive/big.md': longDoc(2000) }).status, 0)
})

// ── D-004：变更史不得混入活跃文档 ───────────────────────────────────────
test('D-004：正文出现变更史叙述 → 失败（报出文件:行号）', () => {
  const r = audit({ 'docs/a.md': '# A\n\n## 决策\n\n这条路径曾经走的是另一条路。\n' })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /D-004/)
  assert.match(r.stdout, /docs\/a\.md:5/)
})

test('D-004：结构式残留（后记 / 修订小节 / 测试基线 A → B）都判', () => {
  for (const body of ['## 后记（2026-01-01）\n', '### 修订\n', 'rustTests 930 → 926\n']) {
    const r = audit({ 'docs/a.md': `# A\n\n${body}` })
    assert.equal(r.status, 1, `未判出：${body.trim()}`)
  }
})

test('D-004：「不再 / 以前」这类现行语义常用词不误报', () => {
  const r = audit({ 'docs/a.md': '# A\n\n资源不再走两套抽象；以前那种按事件类型分派的写法已无。\n' })
  assert.equal(r.status, 0, r.stdout)
})

test('D-004：头部豁免带理由 → 通过（定义该规矩的文档要能引用反例措辞）', () => {
  const allowed =
    '<!-- doc-link-allow D-004: 本文定义该规矩，需引用反例措辞 -->\n# A\n\n不写「曾经是什么」。\n'
  assert.equal(audit({ 'docs/a.md': allowed }).status, 0)
})

test('D-004：豁免理由为空 → 仍失败（同 D-002 的口径）', () => {
  const empty = '<!-- doc-link-allow D-004:   -->\n# A\n\n这条曾经存在过。\n'
  assert.equal(audit({ 'docs/a.md': empty }).status, 1)
})

// ── D-006：反引号里的文件路径（D-001 的盲区补丁）─────────────────────────
// D-001 只认 `[文字](目标)`，而本仓正文**更常**用行内反引号指路。实测
// `tauri/docs/FRONTEND.md` 首段引用的两个文件都不存在，D-001 却报「失效 0 条」
// 很久了——**守卫报 0 不等于没有坏链，只等于它看不见**。这组用例钉住它会红。
test('D-006：反引号指向不存在的路径 → 失败（D-001 看不见这类）', () => {
  const r = audit({ 'docs/a.md': '# A\n\n详见 `docs/nope.md`。\n' })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /D-006/)
  assert.match(r.stdout, /docs\/nope\.md/)
})

test('D-006：双根解析——相对当前文件命中即通过', () => {
  assert.equal(
    audit({ 'docs/a.md': '# A\n\n见 `./b.md`。\n', 'docs/b.md': '# B\n' }).status,
    0,
    '相对当前文件应命中'
  )
})

test('D-006：双根解析——相对仓库根命中即通过（模块 README 引用系统文档的写法）', () => {
  assert.equal(
    audit({
      'symbio/src/plugins/foo/README.md': '# Foo\n\n见 `docs/DECISIONS.md`。\n',
      'docs/DECISIONS.md': '# 决策\n',
    }).status,
    0,
    '相对仓库根应命中'
  )
})

test('D-006：不可判定的写法一律跳过（无斜杠 / 通配 / 省略号 / 绝对路径）', () => {
  const r = audit({
    'docs/a.md': [
      '# A',
      '',
      '泛指 `README.md`；通配 `plugins/*/README.md`；',
      '省略 `a/b/...md`；绝对 `/etc/x.md`；纯锚 · 无路径 `foo.md`。',
      '',
    ].join('\n'),
  })
  assert.equal(r.status, 0, r.stdout)
})

test('D-006：缩写引用 → 失败（启发式判不准，故必须带豁免出口）', () => {
  const r = audit({ 'docs/a.md': '# A\n\n见 `session/docs/core-loop.md`。\n' })
  assert.equal(r.status, 1)
})

test('D-006：行内豁免（写在被豁免行的前一行）→ 通过，且不影响其它行', () => {
  const r = audit({
    'docs/a.md': [
      '# A',
      '',
      '<!-- doc-link-allow D-006: 此处引用示意性路径 -->',
      '示意：`docs/not-real.md`',
      '但下面这条是真断链：`docs/really-nope.md`',
      '',
    ].join('\n'),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /really-nope\.md/)
  assert.doesNotMatch(r.stdout, /not-real\.md/, '被豁免的行不应出现在报告里')
})

test('D-006：全文豁免（头部注释）→ 整篇通过', () => {
  const r = audit({
    'docs/a.md': '<!-- doc-link-allow D-006: 本文举例说明路径写法，路径均非入链 -->\n# A\n\n`docs/x.md`\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('D-006：豁免理由为空 → 仍失败（同 D-002 / D-004 的口径）', () => {
  const r = audit({ 'docs/a.md': '# A\n\n<!-- doc-link-allow D-006:   -->\n见 `docs/nope.md`。\n' })
  assert.equal(r.status, 1)
})

test('D-006：docs/archive/ 整体豁免（归档记录当时形态，改写等于篡改历史）', () => {
  assert.equal(audit({ 'docs/archive/old.md': '# 旧\n\n见 `docs/gone.md`。\n' }).status, 0)
})

// ── D-007：站内锚点 ─────────────────────────────────────────────────────
test('D-007：跨册锚点指向不存在的标题 → 失败（D-001 会剥掉 # 后段，看不见这类）', () => {
  const r = audit({
    'docs/decisions/core.md': '# 平台基座\n\n## ADR-001: 分形插件架构\n',
    'docs/README.md': '[001](./decisions/core.md#adr-999-不存在的那条)\n',
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-007/)
  assert.match(r.stdout, /adr-999-不存在的那条/)
})

test('D-007：锚点指向真实标题 → 通过', () => {
  const r = audit({
    'docs/decisions/core.md': '# 平台基座\n\n## ADR-001: 分形插件架构\n',
    'docs/README.md': '[001](./decisions/core.md#adr-001-分形插件架构)\n',
  })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /D-007 站内锚点：判定 1 条，失效 0 条/)
})

test('D-007：slug 算法钉住真实形状——加粗 / 反引号 / 全角括号 / —— / + 全被丢，空格逐个转 -', () => {
  // 标题取自 `docs/decisions/core.md` ADR-020 的真实形状：`EventSink`（出）后**无空格**，
  // 而 `+ ` 那个空格才产生连字符 ⇒ `eventsink出-abortsignal入`。
  const heading =
    '## ADR-020: 执行期与传输层**分离**——`EventSink`（出）+ `AbortSignal`（入）取代 `PluginChannel` 的双职责'
  const r = audit({
    'docs/decisions/core.md': `# 平台基座\n\n${heading}\n`,
    'docs/README.md':
      '[020](./decisions/core.md#adr-020-执行期与传输层分离eventsink出-abortsignal入取代-pluginchannel-的双职责)\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('D-007：` 是 ` 两侧的空格各产生一个连字符（漏了就红——这正是真仓库 3 处失效的形态）', () => {
  // 标题取自 ADR-025：`顺序是**节点属性**；`delta` 是 `updated` 的**传输形态**`
  const heading = '## ADR-025: 顺序是**节点属性**；`delta` 是 `updated` 的**传输形态**'
  const r = audit({
    'docs/decisions/session.md': `# 会话与执行\n\n${heading}\n`,
    'docs/good.md': '[025-good](./decisions/session.md#adr-025-顺序是节点属性delta-是-updated-的传输形态)\n',
    'docs/README.md': '[025-bad](./decisions/session.md#adr-025-顺序是节点属性delta-是updated的传输形态)\n',
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /README\.md.*#adr-025-顺序是节点属性delta-是updated的传输形态/s)
  assert.doesNotMatch(r.stdout, /good\.md/)
})

test('D-007：纯锚点 `#x` 查本文件的标题（这类链接 D-001 与 D-006 都看不见）', () => {
  const bad = audit({
    'docs/README.md': '[§0.1](#02-不存在的小节)\n\n## 01. 存在的小节\n',
  })
  assert.equal(bad.status, 1, bad.stdout)
  assert.match(bad.stdout, /#02-不存在的小节/)

  const good = audit({
    'docs/README.md': '[§0.1](#01-存在的小节)\n\n## 01. 存在的小节\n',
  })
  assert.equal(good.status, 0, good.stdout)
})

test('D-007：重复标题接受 GitHub 的 `-1` 后缀变体（但不超过重复数）', () => {
  const r = audit({
    'docs/a.md': [
      '[第一条](#x-标题)',
      '[第二条](#x-标题-1)',
      '[第三条](#x-标题-2)',
      '',
      '## X: 标题',
      '## X: 标题',
    ].join('\n'),
  })
  // 2 个同名标题 ⇒ 合法后缀只到 `-1`；`-2` 失败，前两条不算失效
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /#x-标题-2/)
  assert.doesNotMatch(r.stdout, /#x-标题-1 /)
})

test('D-007：非 .md 目标的 fragment 不判（`x.rs#L10` 的 L10 不是标题 slug）', () => {
  const r = audit({
    'symbio/src/lib.rs': 'pub const X: u8 = 1;\n',
    'docs/README.md': '[源码](../symbio/src/lib.rs#L1)\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('D-007：目标文件不存在时只报 D-001，锚点不重复报', () => {
  const r = audit({ 'docs/README.md': '[去这儿](./nope.md#section)\n' })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /nope\.md/)
  assert.match(r.stdout, /D-007 站内锚点：判定 0 条，失效 0 条/)
})

test('D-007：docs/archive/ 作为**源文件**整体豁免；作为**目标**仍受查', () => {
  const r = audit({
    'docs/archive/old.md': '[坏锚](#不存在)\n\n## 真标题\n', // 源文件被豁免
    'docs/README.md': '[归档](./archive/old.md#真标题)\n', // 目标受查
  })
  assert.equal(r.status, 0, r.stdout)
})

// ── 空树 ────────────────────────────────────────────────────────────────
test('空树通过（守卫不是空转即红）', () => {
  assert.equal(audit({}).status, 0)
})

// ── 真实仓库当前状态 ────────────────────────────────────────────────────
test('真实仓库当前状态通过（防本守卫在真仓库上误报）', () => {
  const r = spawnSync(process.execPath, [script], {
    cwd: path.resolve(path.dirname(script), '..'),
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    timeout: 60_000,
  })
  assert.ifError(r.error)
  assert.equal(r.status, 0, r.stdout)
})
