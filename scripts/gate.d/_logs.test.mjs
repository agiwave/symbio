// `gate.d/_logs.mjs` 的回归测试。
//
// ## 为什么必须反向验证
//
// 这两个机制的失效形态**全是静默的**，且方向一致——「看起来有日志、其实读不到」：
//
//   - **每轮一个目录**：写错一级（`logRoot/<name>.log` 而不是 `logRoot/<runId>/…`）
//     或者 `latest-run.txt` 指针没写，门禁照样全绿，只是下一轮的读者拿不到数据
//     （e2e 并发度那条判据会从「每次跑完印一行」退化成「永远不印」）。
//   - **失败详情**：`failureDetail` 少读一段（比如忘了 `note`、或者只认单个
//     `logFile` 而不认数组），失败处就少一块信息——而这种缺失在「没有失败的
//     运行」里永远看不见。
//   - **清理旧轮次**：删错东西（删了文件 / 删了不像 runId 的目录）不会报错，
//     只会让某一天日志少一批。
//
// 所以这里每条都注入坏形态，断言它**必须**表现出预期的红/空/不删。
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { fileURLToPath } from 'node:url'

import {
  makeRunId,
  pruneOldRuns,
  writeRunPointer,
  latestRunDir,
  tailOf,
  logFileName,
  failureDetail,
  latestE2ELogs,
  FAIL_TAIL_LINES,
  MAX_DETAILED_LOGS,
} from './_logs.mjs'

const tmp = () => fs.mkdtempSync(path.join(os.tmpdir(), 'gate-logs-'))

test('logFileName：两个不同的中文标签不许得到同一个文件名（否则互相覆盖日志）', () => {
  const a = logFileName('演示任务：必然失败')
  const b = logFileName('演示任务：另一种失败')
  assert.notEqual(a, b, '中文标签 slug 后都是 step-，必须靠哈希分开')
  assert.equal(logFileName('演示任务：必然失败'), a, '同一标签必须稳定（否则上一轮的日志对不上）')
  assert.match(a, /^step-[0-9a-f]{6}$/, `中文标签的文件名应带短哈希：${a}`)
  // 英文标签保持可读，不加哈希
  assert.equal(logFileName('docs: doc-link-audit'), 'docs-doc-link-audit')
  assert.equal(logFileName('e2e: t59 llm tool surface'), 'e2e-t59-llm-tool-surface')
})

test('makeRunId：本地时间 YYYYMMDD-HHMMSS，同秒再来一轮加后缀', () => {
  const id = makeRunId(new Date(2026, 9, 15, 9, 5, 3))
  assert.equal(id, '20261015-090503')
  // 同名已占用 ⇒ 加 -2；-2 也占用 ⇒ -3（不能无声覆盖已经存在的那一轮）
  assert.equal(makeRunId(new Date(2026, 9, 15, 9, 5, 3), (x) => x === '20261015-090503'), '20261015-090503-2')
  assert.equal(
    makeRunId(new Date(2026, 9, 15, 9, 5, 3), (x) => x === '20261015-090503' || x === '20261015-090503-2'),
    '20261015-090503-3',
  )
})

test('pruneOldRuns：只删最老的 runId 目录，留够 keep 个', () => {
  const root = tmp()
  for (const n of ['20260101-000001', '20260102-000001', '20260103-000001']) {
    fs.mkdirSync(path.join(root, n))
  }
  // 不像 runId 的目录、以及任何文件：两者都不该被动（目录里可能有人放别的东西）
  fs.mkdirSync(path.join(root, 'not-a-run'))
  fs.writeFileSync(path.join(root, 'latest-run.txt'), '20260103-000001\n')
  const removed = pruneOldRuns(root, 2)
  assert.deepEqual(removed, ['20260101-000001'])
  assert.equal(fs.existsSync(path.join(root, '20260103-000001')), true)
  assert.equal(fs.existsSync(path.join(root, 'not-a-run')), true, '不像 runId 的目录不许删')
  assert.equal(fs.existsSync(path.join(root, 'latest-run.txt')), true, '指针文件不许删')
})

test('指针：写了才读得到；指针悬空（目录被删）返回 null 而不是平铺目录', () => {
  const root = tmp()
  assert.equal(latestRunDir(root), null, '没有指针 ⇒ null（不许回退到平铺目录——那会让「没有最近一轮」看不见）')
  writeRunPointer(root, '20260103-000001')
  assert.equal(latestRunDir(root), null, '指针指向不存在的目录 ⇒ null')
  fs.mkdirSync(path.join(root, '20260103-000001'))
  assert.equal(latestRunDir(root), path.join(root, '20260103-000001'))
})

test('tailOf：末 N 行、去 \r、去尾部空行、超长截断', () => {
  assert.equal(tailOf('a\nb\nc\n', { lines: 2 }), 'b\nc')
  assert.equal(tailOf('a\r\nb\r\nc\r\n', { lines: 2 }), 'b\nc', 'CRLF 不该在行尾留下 \\r')
  assert.equal(tailOf('a\nb\n\n\n', { lines: 10 }), 'a\nb', '尾部空行没有信息量')
  const long = tailOf('x'.repeat(500) + '\n' + 'y'.repeat(500), { lines: 40, maxChars: 100 })
  // 截断说明那一行本身要占一点长度，所以上界是 maxChars + 说明行的长度；
  // 断言「说明行 + 内容 ≤ maxChars + 说明行」即可——真正的判据是内容被截到 100。
  const content = long.split('\n').slice(1).join('\n')
  assert.ok(content.length <= 100, `内容仍过长：${content.length}`)
  assert.ok(long.includes('只取末 100 字符'), '截断要说明自己截了')
})

test('failureDetail：无子进程日志时如实说明，不假装有文件', () => {
  const { header, body } = failureDetail({ label: '某判定', pass: false, note: '自己的断言', logFile: null }, { repoRoot: tmp() })
  assert.match(header, /^✗ 某判定 — 自己的断言$/)
  assert.equal(body.length, 1)
  assert.match(body[0], /没有子进程日志/)
})

test('failureDetail：末 N 行与相对路径都在正文里', () => {
  const dir = tmp()
  const logFile = path.join(dir, '某任务.log')
  fs.writeFileSync(logFile, '第一行\n第二行\n最后的断言失败在这里\n', 'utf8')
  const { body } = failureDetail({ label: '某任务', pass: false, note: 'exit=1', logFile }, { repoRoot: dir, lines: 2 })
  assert.ok(body.some((l) => l.includes('某任务.log')), '路径必须在正文里')
  assert.ok(body.some((l) => l.includes('末 2 行')), '要说明截了多少')
  assert.ok(body.some((l) => l.includes('最后的断言失败在这里')), '末 N 行的内容要在')
  assert.ok(!body.some((l) => l.includes('第一行')), '末 N 行之外的内容不该出现')
})

test('failureDetail：聚合任务带回多个日志时只细印前 N 个，其余列路径', () => {
  const dir = tmp()
  const logs = Array.from({ length: MAX_DETAILED_LOGS + 2 }, (_, i) => {
    const f = path.join(dir, `e2e-t${i}.log`)
    fs.writeFileSync(f, `用例 ${i} 失败\n`, 'utf8')
    return f
  })
  const { body } = failureDetail({ label: 'e2e 用例集', pass: false, note: '失败: t0', logFile: logs }, { repoRoot: dir })
  const detailed = body.filter((l) => l.includes('── 末')).length
  assert.equal(detailed, MAX_DETAILED_LOGS, `只细印 ${MAX_DETAILED_LOGS} 个`)
  assert.ok(body.some((l) => l.includes(`其余 ${logs.length - MAX_DETAILED_LOGS} 个日志`)), '其余必须列出来，不许悄悄丢')
})

test('failureDetail：日志读不出（已被删）时如实说明，不许抛异常打断汇总', () => {
  const dir = tmp()
  const { body } = failureDetail(
    { label: '某任务', pass: false, logFile: path.join(dir, 'gone.log') },
    { repoRoot: dir },
  )
  assert.ok(body.some((l) => l.includes('读不出或为空')), '缺文件要说出来')
})

test('latestE2ELogs：跨轮次按用例名取最新的一份（同名不再按文件名的字典序猜）', () => {
  const root = tmp()
  const oldDir = path.join(root, '20260101-000001')
  const newDir = path.join(root, '20260102-000001')
  fs.mkdirSync(oldDir)
  fs.mkdirSync(newDir)
  fs.writeFileSync(path.join(oldDir, 'e2e-t1.log'), '✓ t1 (10000ms)\n')
  // 新一轮里 t1 变快了、t2 是新增的
  fs.writeFileSync(path.join(newDir, 'e2e-t1.log'), '✓ t1 (3000ms)\n')
  fs.writeFileSync(path.join(newDir, 'e2e-t2.log'), '✗ t2 (5000ms)\n')
  writeRunPointer(root, '20260102-000001')

  const byName = new Map(latestE2ELogs(root).map((c) => [c.name, c]))
  assert.equal(byName.get('t1').ms, 3000, '同名取新轮次的那份')
  assert.equal(byName.get('t2').ms, 5000)
  assert.equal(byName.get('t2').ok, false, '✗ 开头的用例是不ok')
  assert.equal(byName.get('t1').ok, true)
})

test('latestE2ELogs：runDir 优先于指针（门内那一轮读自己，不读上一轮）', () => {
  const root = tmp()
  const prev = path.join(root, '20260101-000001')
  fs.mkdirSync(prev)
  fs.writeFileSync(path.join(prev, 'e2e-t1.log'), '✓ t1 (9000ms)\n')
  writeRunPointer(root, '20260101-000001')
  const thisRun = path.join(root, '20260102-000001')
  fs.mkdirSync(thisRun)
  fs.writeFileSync(path.join(thisRun, 'e2e-t1.log'), '✓ t1 (1000ms)\n')

  const got = latestE2ELogs(root, { runDir: thisRun })
  assert.equal(got.find((c) => c.name === 't1').ms, 1000, '门内判据必须用本轮的数据')
})

test('latestE2ELogs：一个日志都没有时返回空数组（调用方据此跳过，不许抛）', () => {
  assert.deepEqual(latestE2ELogs(tmp()), [])
})

test('常量：默认尾行数与细印上限必须是正的、可被环境变量覆盖的默认值', () => {
  assert.ok(FAIL_TAIL_LINES >= 10, '末几行少于 10 行往往装不下失败块')
  assert.ok(MAX_DETAILED_LOGS >= 1)
})
