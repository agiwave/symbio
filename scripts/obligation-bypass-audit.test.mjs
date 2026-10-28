/**
 * obligation-bypass-audit 回归测试
 *
 * 这条守卫判四件事：两类计数各自 ≤ 基线（OB-001 调度旁路 / OB-004 幂等点问）、低于
 * 基线要提示下调（OB-002）、每处点名的**行必须在 `ROWS` 里**（OB-003）、承认标记本身
 * 站得住（OB-004 的另一半：行得真的接了执行体、理由得有内容、标记不许悬空）。它最危险
 * 的失效形态全是**静默变绿**——数字算错、算宽、算不到，报出来都是「通过」，所以逐条钉：
 *
 *   ① **算得对**：调用点计数、多行调用抓得到行名、两格基线下方都要给提示；
 *   ② **不算多**：`fn owes(` 定义行、`*.test.rs`、`drive_steps` 函数体、引擎的现读
 *      点 `row_owed`、注释里的 `owes(` 都不得进计数。**这几类每一条都会把基线喂松**，
 *      而松了的棘轮等于没有；
 *   ③ **行名只在 `ROWS` 里找**：同一文件的 `ACTORS` / 端口声明也写 `name: "…"`，
 *      整文件抓会让一个**不是行**的名字满足 OB-003（用例用执行体名 `courier` 钉它）；
 *      旁路直调与 `Dispatch { row: "…" }` 的字面点名都要核——后者是引擎现读的输入，
 *      点错一行等于那一格永远派不出去；
 *   ④ **承认不是洗白**（`D-158` 之后这条判据最容易死在这儿）：把「调度旁路」标成
 *      「护栏」就能让归零数当场达标，所以标记的效力来自**行本身**——`by: Some` 的行
 *      才有「动手前复核」这回事；`by = None` 的行贴了标记判红，哪怕基线已经放到 0。
 *      同理，一句没有内容的理由、一个近旁没有直调点的标记，都不许当承认用；
 *   ⑤ **读不出即失败**：没有 `.rs`、没有 `ROWS` 都必须 exit 2，不退化成
 *      「没什么可判 ⇒ 通过」。
 *
 * 跑法：node --test scripts/obligation-bypass-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./obligation-bypass-audit.mjs', import.meta.url))

/**
 * 行表：一行**没接执行体**（`alpha`，调度旁路的形状）、一行接了（`beta`，幂等点问的
 * 形状）+ 一个**不在行表里**的执行体名（OB-003 的作用域用例要用）。
 *
 * `by:` 的写法不是装饰：OB-004 的「洗白」那半边就靠它分辨复核与第二套调度。
 */
const ROWS_RS = `pub(crate) static ROWS: &[Decl] = &[
    Decl { name: "alpha", by: None },
    Decl { name: "beta", by: Some(BETA) },
];

const ACTOR_COURIER: ActorDecl = ActorDecl { name: "courier", writes: &[] };

pub(crate) fn owes(name: &str, events: &[Event], turn: u64, now: i64) -> bool {
    name.is_empty() && !events.is_empty() && turn == 0 && now == 0
}
`

const ROWS_PATH = 'crates/symbio-plugin-loop/src/loop_rows.rs'

/** 造一棵最小仓库：`files` 是 相对路径 → 内容（`ROWS_PATH` 默认给一份两行表） */
function audit(files, { baseline = '2', precheck = '9' } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'obligation-bypass-'))
  try {
    const all = { [ROWS_PATH]: ROWS_RS, ...files }
    for (const [rel, content] of Object.entries(all)) {
      if (content === null) continue
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const r = spawnSync(process.execPath, [script, `--root=${root}`], {
      env: {
        ...process.env,
        NO_COLOR: '1',
        OBLIGATION_BYPASS_BASELINE: baseline,
        OBLIGATION_PRECHECK_BASELINE: precheck,
      },
      encoding: 'utf8',
      timeout: 30000,
    })
    assert.ifError(r.error)
    return r
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

const out = (r) => `${r.stdout}${r.stderr}`

const caller = (rows) =>
  `use super::loop_rows::owes;\n\nfn step() {\n${rows.map((r) => `    let _ = owes("${r}", &[], 0, 0);\n`).join('')}}\n`

/** 带承认标记的直调点（`gap` = 标记与调用之间隔几行，0 是紧邻） */
const marked = (row, { gap = 0, reason = '执行体动手前的复核：重放第二次进入同一动作' } = {}) =>
  `fn step() {\n    // obligation-precheck OB-004: ${reason}\n${'    let _ = 0;\n'.repeat(gap)}    let _ = owes("${row}", &[], 0, 0);\n}\n`

test('干净仓：2 处调度旁路 / 基线 2、无承认 ⇒ 通过', () => {
  const r = audit({ 'crates/symbio-plugin-loop/src/a.rs': caller(['alpha', 'beta']) })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /通过/)
  assert.match(out(r), /调度旁路 2\/2/)
  assert.match(out(r), /幂等点问 0\/9/)
})

test('OB-001 调度旁路超出基线 ⇒ 读数（已退场，不判红），并列出每一处', () => {
  const r = audit({ 'crates/symbio-plugin-loop/src/a.rs': caller(['alpha', 'beta', 'alpha']) })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /\[READ\]/)
  assert.match(out(r), /OB-001 派发面变宽：调度旁路.*点数 3 > 基线 2/)
  assert.match(out(r), /a\.rs:\d+ {2}alpha/)
})

test('OB-002 调度旁路低于基线 ⇒ 通过但提示下调（余量不许白留）', () => {
  const r = audit({ 'crates/symbio-plugin-loop/src/a.rs': caller(['alpha']) }, { baseline: '5' })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /提示.*调度旁路.*1<5.*BYPASS_BASELINE/)
})

test('OB-002 幂等点问低于基线也要提示（两格都不许白留余量）', () => {
  const r = audit({ 'crates/symbio-plugin-loop/src/a.rs': marked('beta') }, { baseline: '0', precheck: '5' })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /提示.*幂等点问.*1<5.*PRECHECK_BASELINE/)
})

test('承认过的点**不进**调度旁路那格：旁路基线 0 也能通过（数的是两个数）', () => {
  // 一处未承认（`alpha`）+ 一处已承认（`beta`）：`baseline=1` 过、`baseline=0` 必红，
  // 才算证明标记真的改变了归类，而不是只改了打印文字。
  const both = `fn step() {\n    // obligation-precheck OB-004: 执行体动手前的复核\n    let _ = owes("beta", &[], 0, 0);\n    let _ = owes("alpha", &[], 0, 0);\n}\n`
  const ok = audit({ 'crates/symbio-plugin-loop/src/a.rs': both }, { baseline: '1' })
  assert.equal(ok.status, 0, out(ok))
  assert.match(out(ok), /调度旁路 1\/1（alpha×1）· 幂等点问 1\/9（beta×1）/)
  // 退场后不再判红，但读数仍要照旧算出来（`[READ]` 是唯一证据）
  const red = audit({ 'crates/symbio-plugin-loop/src/a.rs': both }, { baseline: '0' })
  assert.equal(red.status, 0, out(red))
  assert.match(out(red), /\[READ\].*点数 1 > 基线 0/)
  assert.match(out(red), /调度旁路.*点数 1 > 基线 0/)
})

test('`fn owes(` 定义行不算调用点（13 与 12 的那个 off-by-one）', () => {
  // 行表文件里那份定义若被算进来，计数就天生多一格 ⇒ 棘轮永远松着
  const r = audit({ 'crates/symbio-plugin-loop/src/a.rs': caller(['alpha', 'beta']) })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /调度旁路 2\/2/)
})

test('`*.test.rs` 里的断言不算调用点', () => {
  const r = audit({
    'crates/symbio-plugin-loop/src/a.rs': caller(['alpha', 'beta']),
    'crates/symbio-plugin-loop/src/a.test.rs': caller(['alpha', 'alpha', 'alpha', 'beta']),
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /调度旁路 2\/2/)
})

test('`drive_steps` 函数体里的问债不算旁路（它就是派发器本体）', () => {
  const r = audit({
    'crates/symbio-plugin-loop/src/a.rs': caller(['alpha', 'beta']),
    'crates/symbio-plugin-loop/src/turn_exec.rs':
      'fn drive_steps(rows: &[Decl]) {\n    let _ = owes("alpha", &[], 0, 0);\n    let _ = owes("beta", &[], 0, 0);\n}\n',
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /调度旁路 2\/2/)
})

test('注释里的 `owes(` 不算调用点', () => {
  const r = audit({
    'crates/symbio-plugin-loop/src/a.rs':
      '// 这里曾经直调 owes("alpha")，收编后改走 drive\n/* owes("beta") */\n' + caller(['alpha', 'beta']),
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /调度旁路 2\/2/)
})

test('引擎的现读点 `row_owed` 不算旁路（它是派发本体的一半，行名另有核对）', () => {
  // D-136：派发前问表只剩这一处，它问的行名来自调用点声明的 `Dispatch { row }`。
  // 把它算进旁路会让基线虚高一格，而那一格正是「谁都能再加一处」的余量。
  const r = audit({
    'crates/symbio-plugin-loop/src/a.rs': caller(['alpha', 'beta']),
    'crates/symbio-plugin-loop/src/port.rs':
      'impl Dispatch<\'_> {\n    fn row_owed<S>(&self, store: &S) -> bool {\n        let _ = super::loop_rows::owes(self.row, &[], 0, 0);\n    }\n}\n',
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /调度旁路 2\/2/)
})

test('OB-003 也核 `Dispatch { row: "…" }` 的字面点名（点错 ⇒ 那一格永不派发）', () => {
  const r = audit({
    'crates/symbio-plugin-loop/src/a.rs':
      caller(['alpha', 'beta']) +
      'fn round() {\n    let _ = Dispatch {\n        row: "gamma",\n    };\n}\n',
  })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /OB-003 .*派发点名 `row: "gamma"`/)
  // 点名合法时不报，也不进旁路计数
  const ok = audit({
    'crates/symbio-plugin-loop/src/a.rs': caller(['alpha', 'beta']) + 'fn round() {\n    let _ = Dispatch { row: "beta" };\n}\n',
  })
  assert.equal(ok.status, 0, out(ok))
  assert.match(out(ok), /调度旁路 2\/2/)
})

test('OB-003 点名不存在的行 ⇒ 红（`owes` 恒 false ⇒ 这段旁路永不执行）', () => {
  const r = audit({ 'crates/symbio-plugin-loop/src/a.rs': caller(['alpha', 'beta', 'gamma']) }, { baseline: '9' })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /OB-003 .*owes\("gamma"\).*ROWS.*没有这一行/)
})

test('OB-003 只认 `ROWS` 块：执行体名 / 端口名不是行名', () => {
  // `courier` 在同一文件的 `ActorDecl { name: "courier" }` 里写着，但它不是一行
  const r = audit({ 'crates/symbio-plugin-loop/src/a.rs': caller(['alpha', 'courier']) }, { baseline: '9' })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /OB-003 .*owes\("courier"\)/)
})

test('OB-004 洗白：标记贴在 `by = None` 的行上 ⇒ 红（哪怕旁路基线已经放到 0）', () => {
  // 这是拆成两格之后**唯一的新失效面**：给策略行的直问补一句「这是护栏」，
  // `BYPASS_BASELINE` 当场归零而代码一行没动。牙在这一条上。
  const r = audit({ 'crates/symbio-plugin-loop/src/a.rs': marked('alpha') }, { baseline: '0' })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /OB-004 .*标记贴在了未接执行体的行 `"alpha"`/)
  assert.match(out(r), /唯一的派发点/)
})

test('一句承认只顶一个点：同一标记下的第二处直问仍是调度旁路', () => {
  // 按「窗口内所有点都算被承认」实现，一处护栏会顺手洗白其下三行里的每一处直问，
  // 而那多半是另一件事（这里两处点的行都接了执行体，红只能来自计数）。
  const r = audit(
    {
      'crates/symbio-plugin-loop/src/a.rs':
        'fn step() {\n    // obligation-precheck OB-004: 复核第一处\n    let _ = owes("beta", &[], 0, 0);\n    let _ = owes("beta", &[], 1, 0);\n}\n',
    },
    { baseline: '0' },
  )
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /\[READ\].*点数 1 > 基线 0/)
  assert.match(out(r), /幂等点问 1\/9/)
})

test('OB-004 理由没有内容 ⇒ 红（承认账本不许写成装饰）', () => {
  // 判空口径同 R-002：至少要一个字母 / 数字 / 汉字，一个句号不算理由
  const r = audit({ 'crates/symbio-plugin-loop/src/a.rs': marked('beta', { reason: '。' }) }, { baseline: '0' })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /OB-004 .*承认理由为空/)
})

test('OB-004 标记悬空 ⇒ 红（近旁没有直调点的承认不算承认）', () => {
  // 第 4 行才隔开：回看窗口是「同行 + 其上 3 行」，越窗的标记既没贴上任何点，
  // 也不许悄悄留在仓里充当承认——它下面那段旁路删了，承认必须跟着删。
  const dangling = audit(
    {
      'crates/symbio-plugin-loop/src/a.rs': caller(['alpha', 'beta']),
      'crates/symbio-plugin-loop/src/b.rs':
        'fn old() {\n    // obligation-precheck OB-004: 这段旁路已经收编进 drive\n    let _ = 1;\n}\n',
    },
    { baseline: '2' },
  )
  assert.equal(dangling.status, 0, out(dangling))
  assert.match(out(dangling), /OB-004 .*b\.rs:\d+ 承认标记悬空/)

  // 窗口内（其上第 3 行）仍然算贴上 ⇒ 钉住「悬空」判的是距离，不是标记本身
  const inWindow = audit({ 'crates/symbio-plugin-loop/src/a.rs': marked('beta', { gap: 2 }) }, { baseline: '0' })
  assert.equal(inWindow.status, 0, out(inWindow))
  assert.match(out(inWindow), /幂等点问 1\/9/)
})

test('OB-004 幂等点问超出自己的基线 ⇒ 红（护栏不许随手加）', () => {
  const r = audit(
    {
      'crates/symbio-plugin-loop/src/a.rs':
        'fn step() {\n    // obligation-precheck OB-004: 复核一\n    let _ = owes("beta", &[], 0, 0);\n    // obligation-precheck OB-004: 复核二\n    let _ = owes("beta", &[], 1, 0);\n}\n',
    },
    { baseline: '0', precheck: '1' },
  )
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /OB-004 幂等点问变宽：承认在册的直问 2 > 基线 1/)
})

test('多行调用也能抓到行名', () => {
  const r = audit({
    'crates/symbio-plugin-loop/src/a.rs':
      'fn step() {\n    let _ = super::super::loop_rows::owes(\n        "alpha",\n        &[],\n        0,\n        0,\n    );\n}\n',
  }, { baseline: '1' })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /alpha×1/)
})

test('范围读不出（一个 `.rs` 都没有）⇒ exit 2，不退化成通过', () => {
  const r = audit({ [ROWS_PATH]: null, 'README.md': '# 空仓\n' })
  assert.equal(r.status, 2, out(r))
  assert.match(out(r), /审计范围读不出/)
})

test('行表读不出（`ROWS` 不在）⇒ exit 2，宁红不静默', () => {
  const r = audit({
    [ROWS_PATH]: 'pub(crate) static OTHER: &[Decl] = &[];\n',
    'crates/symbio-plugin-loop/src/a.rs': caller(['alpha', 'beta']),
  })
  assert.equal(r.status, 2, out(r))
  assert.match(out(r), /读不出 `ROWS` 行表/)
})
