// `gate.d/_shared.mjs` 的回归测试。
//
// 这里覆盖 `autoWork`（门禁的「自动执行的工作」原语）与**棘轮判据一族**
// （`ratchetVerdict` / `cargoTestRatchet` / `BASELINE` 只增）。前者值得有回归测试的理由，
// 与 `30-docs.mjs` 开头那句是同一个：**一个只会亮绿灯的机制等于没有机制**。
// `autoWork` 的失效方式恰恰是「看起来在修、其实没把修复带进提交」，而这一点
// 在正常流程里看不出来（本地跑门禁 → 文件确实被格式化了 → 一切正常），
// 只在「修复前就已脏/已暂存」时暴露。这个设计第一版就写反了方向，见下。
//
// 跑法：node --test scripts/gate.d/_shared.test.mjs

import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import {
  autoWork,
  cargoTestRatchet,
  BASELINE,
  BASELINE_GRACE,
  baselineErosion,
  baselineWaivers,
  parseBaselineCells,
  ratchetVerdict,
} from './_shared.mjs'

const repoRoot = path.resolve(import.meta.dirname, '../..')

// ================= 一次性临时仓库 =================
//
// 用真实 `git` 而不是打桩：`autoWork` 的全部行为都建立在
// `git status --porcelain -z` 与 `git add` 的真实语义上，打桩等于把被测对象换掉。
//
// 所有用例共用**一个**仓库、各自用**不同的文件**：`dirtyPaths` 是仓库级的，
// 但只要某用例的 `run` 只写自己的文件，别人的脏文件既不会被改写、也就不会被暂存。

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-autowork-'))

const git = (args) =>
  spawnSync('git', args, { cwd: root, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024 })

git(['-c', 'init.defaultBranch=main', 'init', '-q'])
git(['config', 'user.email', 'test@example.com'])
git(['config', 'user.name', 'test'])
git(['config', 'core.autocrlf', 'false'])
// 被测文件**先以「已格式化」的样子进初始提交**，这样它们都是**已跟踪**文件，
// 「已暂存 / 未暂存」才有明确含义。（若用新建文件，`git diff --cached` 会因为
// 「相对 HEAD 是新增」而恒为真，判据就废了。）
const SEED = {
  't1.rs': 'fn a() {}\n',
  't2.rs': 'fn b() {}\n',
  't4-untouched.rs': 'fn u() {}\n',
  't7.rs': 'fn g() {}\n',
  't8.rs': 'fn h() {}\n',
}
for (const [p, c] of Object.entries(SEED)) fs.writeFileSync(path.join(root, p), c)
git(['add', '-A'])
git(['commit', '-q', '-m', 'init'])

after(() => {
  // 尽力清理。沙箱的 safe-delete 垫片在「单回合删除数过多」时会拒删并抛——
  // 那是环境限制，不是测试失败；临时目录交给 OS 回收。
  try {
    fs.rmSync(root, { recursive: true, force: true, maxRetries: 3 })
  } catch {
    /* 环境限制，忽略 */
  }
})

// ================= 小工具 =================

const abs = (p) => path.join(root, p)
const write = (p, c) => fs.writeFileSync(abs(p), c)
const read = (p) => fs.readFileSync(abs(p), 'utf8')
const porcelain = (p) => (git(['status', '--porcelain', '--', p]).stdout || '').replace(/\r?\n$/, '')
/** 索引 vs HEAD：这次提交**会**带上这个文件的改动。 */
const isStaged = (p) => Boolean((git(['diff', '--cached', '--name-only', '--', p]).stdout || '').trim())
/** 工作区 vs 索引：还有**未暂存**的改动（即「脏」）。 */
const worktreeDiffers = (p) => Boolean((git(['diff', '--name-only', '--', p]).stdout || '').trim())
const stagedContent = (p) => git(['show', `:${p}`]).stdout

/** 模拟一个「确定性的机械工作」：把 writes 里的内容写进文件。 */
const worker = (writes) => async () => {
  for (const [p, c] of Object.entries(writes)) write(p, c)
  return { ok: true, code: 0, signal: null, output: '', timedOut: false }
}
const brokenWorker = () => async () => ({
  ok: false,
  code: 1,
  signal: null,
  output: 'boom',
  timedOut: false,
})

const work = (run, ci = false) =>
  autoWork({ repoRoot: root, ci, run }, { label: 'probe', cmd: 'noop', args: [], cwd: root })

// ================= 用例 =================

test('不改写任何文件 ⇒ 通过、不暂存、无 note', async () => {
  const f = 't1.rs'
  write(f, 'fn a(){}\n') // 脏、未暂存（正常流程里提交前的常态）
  assert.equal(worktreeDiffers(f), true, '前置：工作区有未暂存改动')
  assert.equal(isStaged(f), false, '前置：索引里没有它')

  const r = await work(worker({ [f]: 'fn a(){}\n' })) // 修复结果 == 现状

  assert.equal(r.ok, true)
  assert.equal(r.note, undefined, '没有差异就不该有 note')
  assert.equal(isStaged(f), false, '没有差异就不该动索引')
  assert.equal(worktreeDiffers(f), true, '未暂存的改动仍留在工作区，不该被吞掉')
})

test('修复前就已脏（未暂存）+ 被改写 ⇒ 当场暂存（第一版就是在这里写反了方向）', async () => {
  const f = 't2.rs'
  write(f, 'fn b(){ }\n') // 脏、未暂存
  assert.equal(worktreeDiffers(f), true)
  assert.equal(isStaged(f), false)

  // ⚠️ 修复结果必须**不同于 HEAD**（`'fn b() {}\n'`）。若相同，文件修完就变干净、
  // 不再出现在「修复后」那一侧，也就没有东西可暂存——那时断言失败是数据的问题，不是实现的问题。
  const FIXED = 'fn b() {\n}\n'
  const r = await work(worker({ [f]: FIXED }))

  assert.equal(r.ok, true)
  assert.match(r.note ?? '', /自动修复/)
  assert.equal(isStaged(f), true, '⚠️ 修复前已脏的文件也必须进索引——否则提交里留下的是未格式化那一版')
  assert.equal(stagedContent(f), FIXED)
  assert.equal(worktreeDiffers(f), false, '修复后索引与工作区应一致')
})

test('修复前干净 + 修复产生新文件 ⇒ 暂存', async () => {
  const f = 't3.rs'
  assert.equal(porcelain(f), '', '前置：不存在')

  const r = await work(worker({ [f]: 'fn c() {}\n' }))

  assert.equal(r.ok, true)
  assert.equal(isStaged(f), true)
  assert.equal(stagedContent(f), 'fn c() {}\n')
})

test('修复前就脏、但修复没碰它 ⇒ 不暂存（门禁不替人决定哪些改动进本次提交）', async () => {
  const untouched = 't4-untouched.rs'
  const touched = 't4-touched.rs'
  write(untouched, 'fn u(){ }\n') // 人为的、与格式化无关的未提交改动
  assert.equal(worktreeDiffers(untouched), true)
  assert.equal(isStaged(untouched), false)

  const r = await work(worker({ [touched]: 'fn t() {}\n' }))

  assert.equal(r.ok, true)
  assert.equal(isStaged(touched), true, '被修复改写的要暂存')
  assert.equal(isStaged(untouched), false, '没被碰的脏文件不该被顺手暂存')
  assert.equal(read(untouched), 'fn u(){ }\n', '也不该被改写')
})

test('幂等：第二次运行不再产生修复', async () => {
  const f = 't5.rs'
  write(f, 'fn e(){}\n')
  await work(worker({ [f]: 'fn e() {}\n' }))
  const r2 = await work(worker({ [f]: 'fn e() {}\n' }))
  assert.equal(r2.ok, true)
  assert.equal(r2.note, undefined)
})

test('命令本身报错 ⇒ 不通过，且带上退出码', async () => {
  const r = await work(brokenWorker())
  assert.equal(r.ok, false)
  assert.match(r.note ?? '', /exit=1/)
})

test('CI：有差异 ⇒ 报红、提示回本地、不动索引', async () => {
  const f = 't7.rs'
  write(f, 'fn g(){}\n')
  // 同 t2：修复结果要 ≠ HEAD，否则「跑完没有差异」，本用例就没在测 CI 分支。
  const r = await work(worker({ [f]: 'fn g() {\n}\n' }), true)

  assert.equal(r.ok, false, 'CI 无法提交，只能判红')
  assert.match(r.note ?? '', /本地/)
  assert.equal(isStaged(f), false, 'CI 不得动索引')
})

test('CI：无差异 ⇒ 通过（不误报）', async () => {
  const f = 't8.rs'
  write(f, 'fn h() {}\n')
  const r = await work(worker({ [f]: 'fn h() {}\n' }), true)
  assert.equal(r.ok, true)
})

// ⚠️ 这条守卫针对的失效模式很具体：`autoWork` 的「CI 判红」完全依赖 `--ci`，
// 而 facts 阶段在 ci.yml 里跑的是 `--only=docs,facts`——若那里漏了 `--ci`，
// 这一步就变成「自动修复 + 暂存」并**静默放过漂移**，即一个只亮绿灯的检查项。
// 参数漏写不会报错、不会变慢、本地也复现不出来，只能靠断言钉住。
test('ci.yml 里包含 facts 阶段的 gate 调用必须传 --ci', () => {
  const yml = fs.readFileSync(path.join(repoRoot, '.github/workflows/ci.yml'), 'utf8')
  const calls = yml.split('\n').filter((l) => l.includes('gate.mjs') && !l.trim().startsWith('#'))
  assert.ok(calls.length > 0, '没找到 gate.mjs 调用——守卫失效了')
  const factsCalls = calls.filter((l) => /only=[^\s]*facts/.test(l))
  assert.ok(factsCalls.length > 0, '没找到含 facts 阶段的 gate 调用——守卫失效了')
  for (const l of factsCalls) {
    assert.match(l, /--ci/, `facts 阶段必须传 --ci，否则漂移会静默通过：${l.trim()}`)
  }
})

// ⚠️ 下面两条钉的是**同一种失效模式：没有信号的漏跑**——漏跑时门禁与日志都一片绿，
// 事后只能靠人记住「今天本该跑的没跑」。
//
// 2026-10-07 实测：触发分支只有 `[ main, master ]`，而 `v2-plan` 领先 `main` 80 个
// 提交，`gh run list --branch v2-plan` **一条 run 都没有、也没有 PR**——1339 个 Rust
// 单测、42 个 e2e、14 个守卫的绿灯**只存在于开发者本机**。这与 ci.yml 文件头记的
// 「白名单漏掉 `*.css` ⇒ 四个 job 都不跑」是同一课的第二次：第一次漏的是文件类型，
// 这次漏的是分支。
//
// 为什么只能靠断言：分支名单写错不报错、不变慢、本地复现不出来（本地跑 gate 与
// CI 触发毫无关系）。且 `--ci` 只认这个命令行开关、**不读 `process.env.CI`**
// （gate.mjs `const CI = hasFlag('--ci')`），所以连「CI 环境里会自动生效」的想当然
// 都不成立。
test('ci.yml：v2-plan 必须在 push 与 pull_request 的触发分支里，且保留手动触发', () => {
  const yml = fs.readFileSync(path.join(repoRoot, '.github/workflows/ci.yml'), 'utf8')
  const branchLines = yml.split('\n').filter((l) => /^\s*branches:\s*\[/.test(l))
  assert.equal(
    branchLines.length,
    2,
    `期望 push / pull_request 各一个 branches 行，实得 ${branchLines.length}——守卫失效了`,
  )
  for (const l of branchLines) {
    assert.match(
      l,
      /v2-plan/,
      `触发分支里没有 v2-plan ⇒ 该分支的 push 永远不触发 CI，且**没有任何提示**：${l.trim()}`,
    )
  }
  // 手动触发是第三条路：分支不在名单里时，连补跑都做不到。
  assert.match(yml, /^ {2}workflow_dispatch:/m, '没有 workflow_dispatch ⇒ 分支名写错时连手动补跑都不可能')
})

// verify 阶段是 `docs/plan/verify/` 那 14 个「架构证据」的自动化 runner（C29 兑现）。
// 在此之前它们只能手敲 rustc 跑、结果抄进 OUTPUT.md 提交——**快照不是守卫**。
// 少了这个 job，证据是否仍然成立取决于有没有人记得去手跑一遍。
test('ci.yml：verify 阶段必须上 CI', () => {
  const yml = fs.readFileSync(path.join(repoRoot, '.github/workflows/ci.yml'), 'utf8')
  const calls = yml.split('\n').filter((l) => l.includes('gate.mjs') && !l.trim().startsWith('#'))
  const verifyCalls = calls.filter((l) => /only=[^\s]*verify/.test(l))
  assert.ok(
    verifyCalls.length > 0,
    '没有含 verify 阶段的 gate 调用 ⇒ 14 个验证程序退回「人手跑过一次」的状态',
  )
})

// ⚠️ 与上两条同族（**参数漏写 ⇒ 守卫静默降级**），只是换了个地方：
// `30-docs.mjs` 调那 12 个守卫时若不传 `--strict`，它们的 **WARNING 级判定永远不红**
// ——只剩 ERROR 会拦，而 WARNING 恰恰是「需要人看一眼」的那一类。于是它天天被打印、
// 天天没人看，门禁却一路绿。
//
// 这不是假设：`doc-link-audit` 的文件头至今写着「2026-09-20 前失效链接只在 `--strict`
// 下失败，而门禁从不带该参数 ⇒ **从未真的红过**」——同一个坑已经踩过一次。而参数漏写
// 同样不报错、不变慢、本地复现不出来，只能靠断言钉住。
test('gate：调 12 个守卫必须传 --strict，否则 WARNING 级判定永不红', () => {
  const src = fs.readFileSync(path.join(repoRoot, 'scripts/gate.d', '30-docs.mjs'), 'utf8')
  const m = src.match(/for \(const name of GUARDS\) \{[\s\S]*?args:\s*\[([^\]]*)\]/)
  assert.ok(m, '没找到 GUARDS 的调用处——接线变了，这条断言要跟着改')
  assert.match(
    m[1],
    /--strict/,
    `GUARDS 调用没带 --strict ⇒ WARNING 只打印不判红（已踩过一次的坑）：args: [${m[1].trim()}]`,
  )
})

// ── 首次真实 CI（2026-10-07，run 37629977724）暴露的四个配置缺口 ──────────────
//
// 这四条的共同点是：**缺口只在 CI 环境里存在，本机永远复现不出来**——本地 Windows
// 不编 GTK、本地 `tauri/node_modules` 一直在、本地 1.93.1 编 cargo-audit 时依赖还没
// 跑到需要 1.96、本地日志文件直接能打开。所以它们不会以"跑得慢"或"报错"的形式被人
// 发现，只会以**红了但没人看得懂**或**绿了但根本没跑**的形式存在。四个都是实测，
// 不是推演；断言钉住的是「改回去也照样红」。
//
// 取 job 块的方式：按两个 job 标记之间切片——ci.yml 的 job 键是两空格缩进，
// 块内所有行都更深，因此 `^  <name>:` 是可靠的切点。
// ⚠️ 必须**逐行**匹配而不是拼 `'\n  ' + name + ':\n'`：工作区是 CRLF
// （`.gitattributes` 只钉了 `*.md/*.mjs/*.ts/*.vue`，`*.yml` 走 core.autocrlf），
// 拼出来的模式在 `\r\n` 上**一个都匹配不到**，断言会以「找不到 job」的形式假红。
function jobBlock(yml, fromJob, toJob) {
  const idx = (name, from) => {
    const m = yml.slice(from).match(new RegExp(`^ {2}${name}:\\s*$`, 'm'))
    return m ? from + m.index : -1
  }
  const a = idx(fromJob, 0)
  assert.ok(a >= 0, `ci.yml 里没有 job \`${fromJob}\`——守卫失效了`)
  if (toJob == null) return yml.slice(a) // 末位 job（如 security-check）之后没有切点
  const b = idx(toJob, a + 1)
  assert.ok(b > a, `找不到 \`${fromJob}\` 之后的 job \`${toJob}\`——守卫失效了`)
  return yml.slice(a, b)
}
function ciYml() {
  return fs.readFileSync(path.join(repoRoot, '.github/workflows/ci.yml'), 'utf8')
}

test('ci.yml：e2e job 必须装 tauri 的 node 依赖，否则 42 例在 import 期全灭', () => {
  const block = jobBlock(ciYml(), 'e2e-check', 'docs-validation')
  assert.match(
    block,
    /working-directory:\s*tauri\s*\n\s*run:\s*npm ci\s*$/m,
    // 缺了它：`e2e/helpers.mjs` 第 16 行 `import '../tauri/node_modules/ws/index.js'`
    // 解析失败 ⇒ 每个用例 import 期 exit=1 ⇒ 实测「42/42 全红、每例 0s」，
    // 与"用例本身坏了"同形。`frontend-checks` 有 `npm ci` 不代表这个 job 有。
    'e2e job 没有在 tauri/ 跑 npm ci ⇒ 42 个用例全部在 import 期失败（与用例坏了同形）',
  )
})

test('ci.yml：rust-checks 必须装 GTK 系统库，否则 workspace 编不过', () => {
  const block = jobBlock(ciYml(), 'rust-checks', 'msrv-check')
  assert.match(
    block,
    /libwebkit2gtk|libgtk-3-dev/,
    // workspace 的 members 含 `tauri/src-tauri` ⇒ Linux 上 `gobject-sys` / `gtk-sys` /
    // `webkit2gtk-sys` 都要跑 pkg-config。实测首红是
    // 「Package gobject-2.0 was not found in the pkg-config search path」。
    // 只装 openssl 是不够的：openssl 那步当时是绿的，红的是它后面的 GTK 链。
    'rust-checks 没装 GTK/WebKit 开发库 ⇒ `cargo check --tests --workspace` 在第一个 sys crate 就红',
  )
})

test('ci.yml：cargo-audit 不能用项目 pin 的工具链编（会红在装工具那一步）', () => {
  const block = jobBlock(ciYml(), 'security-check', null)
  assert.match(
    block,
    /cargo \+stable install cargo-audit/,
    // `rust-toolchain.toml` 钉 1.93.1，裸 `cargo install` 会拿它去编 cargo-audit 的
    // 依赖。实测：`kstring@2.0.5 requires rustc 1.96.0` ⇒ 安装 101 退出，
    // 这个 job 红在**装工具**，一条漏洞都没扫到。
    'cargo-audit 安装没显式指定项目之外的工具链 ⇒ 被 rust-toolchain.toml 的 pin 拖住，job 红在装工具',
  )
  assert.doesNotMatch(
    block,
    /^\s*run:\s*cargo install cargo-audit\s*$/m,
    '裸 `cargo install cargo-audit` 会被仓库根的 rust-toolchain.toml 拿去编，实测失败',
  )
})

// 第二轮 CI（2026-10-07，run 37635738557）暴露的两处：同样是「前置只存在于开发者的
// 机器上」，且同样是剥掉上一层壳之后才露出来的。
test('ci.yml：rust-checks 必须自己产出 tauri/dist，否则 generate_context! 编译期炸', () => {
  const block = jobBlock(ciYml(), 'rust-checks', 'msrv-check')
  assert.match(
    block,
    /working-directory:\s*tauri\s*\n\s*run:\s*npm ci && npm run build/,
    // `tauri::generate_context!()` 是编译期宏，读 `frontendDist: "../dist"` 且要求该
    // 路径存在；`tauri/dist` 被 tauri/.gitignore 忽略、**只在开发者本机存在**。
    // 缺了它：`error: proc macro panicked` ⇒ `--workspace` 的 check/test/clippy/doc
    // 四步**全红**，而报错指向的是一段完全正常的 Rust 代码。
    'rust-checks 没有构建前端产物 ⇒ symbio-tauri 编译期 panic，四步全红且报错指向正常代码',
  )
})

test('ci.yml：cargo audit 必须在仓库根跑（symbio/ 下没有 Cargo.lock）', () => {
  const block = jobBlock(ciYml(), 'security-check', null)
  // 本仓是单 workspace：`Cargo.lock` 只有根目录那一份，`symbio/Cargo.lock` 不存在。
  // cargo-audit 只在 cwd 找 Cargo.lock、不向上找 workspace 根 ⇒ `working-directory:
  // symbio` 必然失败（`Couldn't load Cargo.lock`）。此前从未暴露是因为它连装都装不上。
  assert.doesNotMatch(
    block,
    /^\s*working-directory:\s*symbio\s*$/m,
    'cargo audit 的工作目录被指到 symbio/ ⇒ 那里没有 Cargo.lock，审计必然失败',
  )
  assert.match(
    block,
    /run:\s*cargo audit\b/,
    '没有 cargo audit 调用——守卫失效了',
  )
  assert.ok(
    fs.existsSync(path.join(repoRoot, 'Cargo.lock')),
    '仓库根没有 Cargo.lock —— 上面这条断言的前提变了，跟着改',
  )
  assert.ok(
    !fs.existsSync(path.join(repoRoot, 'symbio', 'Cargo.lock')),
    'symbio/Cargo.lock 现在存在了 ⇒ 工作目录断言的前提变了，跟着改',
  )
})

// 第三轮 CI（2026-10-07，run 37642902455）暴露：`cargo test --workspace` **第一次**
// 真正跑到，三个「断言 stdout 内容」的 hook 用例立刻在 Linux 上全红，`left: ""`。
// 本机没有 sh，POSIX 侧只能结构钉（行为由 CI 在 Linux 上钉，WSL sh 已实测两边字节数）。
test('hook 测试：POSIX 分支必须引用 $1，否则裸 cat 去读被置空的 stdin', () => {
  const src = fs.readFileSync(
    path.join(repoRoot, 'symbio/src/plugins/hook/executor.test.rs'),
    'utf8',
  )
  const fn = src.match(/^fn cat_last_arg_command[\s\S]*?^\}/m)
  assert.ok(fn, '找不到 cat_last_arg_command —— 契约换了地方，跟着改这条钉')
  const posix = fn[0].match(/else\s*\{\s*("(?:[^"\\]|\\.)*")\s*\}/)
  assert.ok(posix, '找不到 POSIX 分支 —— 结构变了，跟着改这条钉')
  const cmd = JSON.parse(posix[1])
  assert.match(
    cmd,
    /\$1/,
    // `executor.rs` 在 POSIX 走 `sh -c <command> <name> <path>`：路径落在 `$1`，
    // **命令串不引用它就看不见**——裸 `cat` 没有操作数，去读 `Stdio::null()` 的
    // stdin ⇒ 恒输出空。实测（WSL sh）：`sh -c cat n p` = 0 字节，
    // `sh -c 'cat "$1"' n p` = 23 字节。Windows 侧 `cmd /C <cmd> <path>` 会把路径
    // 并进命令行，`type` 因此不必引用——**这个平台差异正是它只在 Linux 红的原因**，
    // 也是本地门禁永远抓不到它的原因。
    `POSIX 分支是 ${cmd}，没引用 $1 ⇒ Linux 上钩子读到空 stdin，断言 stdout 的用例必红`,
  )
})

// 本地偶发红（2026-10-07，gate-full-4）：`cargo test -p symbio` 偶发 6 例 `Duplicate`、
// 单跑全绿。根因是临时目录按 **pid** 命名却**从不清理**——Windows 复用 pid 时旧
// `v2-events.wal` 还在，而事件 id 是写死的，首个 append 就撞 Duplicate。
test('v2 测试的临时目录必须用前先删（pid 复用会把残留 WAL 撞成 Duplicate）', () => {
  for (const n of ['v2_bridge', 'v2_memory', 'v2_skills', 'v2_tasks']) {
    const p = `symbio/src/plugins/session/${n}.test.rs`
    const src = fs.readFileSync(path.join(repoRoot, p), 'utf8')
    const fn = src.match(/^fn tmp_\w+\([\s\S]*?^\}/m)
    assert.ok(fn, `${p}：找不到 tmp_* 助手 —— 结构变了，跟着改这条钉`)
    const body = fn[0]
    assert.match(body, /std::process::id\(\)/, `${p}：目录名不再带 pid ⇒ 这条钉的前提变了`)
    const wipe = body.indexOf('remove_dir_all')
    const create = body.indexOf('create_dir_all')
    assert.ok(wipe !== -1, `${p}：tmp 助手没有 remove_dir_all ⇒ 残留 WAL 会撞 Duplicate`)
    assert.ok(
      wipe < create,
      `${p}：remove_dir_all 不在 create_dir_all 之前 ⇒ 先建后删等于没删`,
    )
  }
})

test('gate：e2e 失败详情不能按 ci 模式关掉（CI 上日志文件看不到）', () => {
  const src = fs.readFileSync(path.join(repoRoot, 'scripts/gate.d', '40-e2e.mjs'), 'utf8')
  // 判据是**语句**不是字样：注释里可以（应当）记着这行代码曾经长什么样，
  // 所以按 `ctx.ci !== true` 这种裸字样去 doesNotMatch 会被自己的注释打中。
  assert.doesNotMatch(
    src,
    /^\s*if\s*\(\s*ctx\.ci/m,
    // 本地有 `.workbuddy-ai/gate-logs/`，CI 上那个目录在 runner 里、没人上传 ⇒
    // 「（详见日志）+ 一条本机路径」是一条零信息的红。方向恰好反了。
    '40-e2e.mjs 又用 ctx.ci 做条件 ⇒ CI 红了但零信息',
  )
  // 关掉条件后打印语句必须还在，否则「整个删掉」也能让上一条通过。
  assert.match(
    src,
    /console\.log\(dim\(r\.output\.split/,
    '失败时没有把末 6 行打出来——断言要跟着改',
  )
})

// ── cargoTestRatchet：两个独立 workspace 共用的「跑测试 + 通过数棘轮」 ──────
//
// 抽成共享函数就是为了让 `symbio` 与 `cli` 是**同一套判据**，所以这里钉的是
// **判据本身**，不是某一侧的调用：三条分支（低于 / 等于 / 高于基线）加上两条
// 退化路径（解析不到 / 命令失败）。其中「解析不到不判红」尤其要钉住——那是
// 刻意的：宁可漏报，也不因为**解析**失败把门禁变红（必然红的门禁比没有门禁更糟）。
const fakeCtx = (output, { ci = false, ok = true } = {}) => ({
  ci,
  run: async () => ({ ok, output, code: ok ? 0 : 1, timedOut: false }),
})
const ratchet = (output, opts, { baseline = 10, ciBaseline } = {}) =>
  cargoTestRatchet(fakeCtx(output, opts), {
    label: 'cargo test',
    cwd: '.',
    args: ['test'],
    baseline,
    baselineName: 'rustTests',
    // 关键：**不传就把这个键整个省掉**（而不是传 undefined 占位）——被测函数判的是
    // `ciBaseline === undefined`，两种「没给」必须走同一条路径。
    ...(ciBaseline === undefined ? {} : { ciBaseline, ciBaselineName: 'ciRustTestsTotal' }),
  })

test('cargoTestRatchet：通过数等于基线 ⇒ 通过且无 note', async () => {
  const r = await ratchet('test result: ok. 10 passed; 0 failed').run()
  assert.equal(r.ok, true)
  assert.equal(r.note, undefined)
})
test('cargoTestRatchet：通过数低于基线 ⇒ 判红（有测试被删或失败）', async () => {
  const r = await ratchet('test result: ok. 9 passed; 0 failed').run()
  assert.equal(r.ok, false)
  assert.match(r.note, /9 < 基线 10/)
})
test('cargoTestRatchet：通过数高于基线 ⇒ 通过但提示同步基线', async () => {
  const r = await ratchet('test result: ok. 11 passed; 0 failed').run()
  assert.equal(r.ok, true)
  assert.match(r.note, /基线待更新/)
})
test('cargoTestRatchet：解析不到通过数 ⇒ 通过并标注，不因解析失败判红', async () => {
  const r = await ratchet('no summary here').run()
  assert.equal(r.ok, true)
  assert.match(r.note, /未能解析/)
})
test('cargoTestRatchet：命令失败 ⇒ 判红并带退出码', async () => {
  const r = await ratchet('', { ok: false }).run()
  assert.equal(r.ok, false)
  assert.match(r.note, /exit=1/)
})
// ── CI 分支：2026-10-07 从「只信退出码」改为与 `ciBaseline` 比对 ────────────
//
// 原实现是 `if (ctx.ci) return { ok: true }`（注释还写着「数字仅作信息展示」）——
// **CI 这一侧从来没有棘轮**。本地删掉测试会被分包基线拦下，推上去 CI 却只信退出码，
// 照样绿。这正是「测试失败」与「测试消失」的分界：后者**不留任何痕迹**，退出码恒为 0。
// 而两条分支判据必须一致（抽成共享函数就是为了这个），否则本地与 CI 各自演化成两条规范。
test('cargoTestRatchet：CI 全量等于基线 ⇒ 通过且无 note', async () => {
  const r = await ratchet('test result: ok. 10 passed; 0 failed', { ci: true }, { ciBaseline: 10 }).run()
  assert.equal(r.ok, true)
  assert.equal(r.note, undefined)
})
test('cargoTestRatchet：CI 全量低于基线 ⇒ 判红（测试消失在 CI 也拦得住）', async () => {
  const r = await ratchet('test result: ok. 10 passed; 0 failed', { ci: true }, { ciBaseline: 12 }).run()
  assert.equal(r.ok, false)
  assert.match(r.note, /10 < 基线 12/)
})
test('cargoTestRatchet：CI 全量高于基线 ⇒ 通过但提示同步基线', async () => {
  const r = await ratchet('test result: ok. 11 passed; 0 failed', { ci: true }, { ciBaseline: 10 }).run()
  assert.equal(r.ok, true)
  assert.match(r.note, /基线待更新/)
})
// ⚠️ 这一条是本组的重点：**少写一个参数不该让守卫消失**。
// 「退回只信退出码」是这条守卫唯一会**静默**失效的方式——调用方漏写 `ciBaseline`，
// 不报错、不变慢、日志里只少一行字，CI 就此变回抓不住「测试消失」的状态。
test('cargoTestRatchet：CI 模式缺 ciBaseline ⇒ 判红（省略参数不让棘轮消失）', async () => {
  const r = await ratchet('test result: ok. 10 passed; 0 failed', { ci: true }).run()
  assert.equal(r.ok, false)
  assert.match(r.note, /缺 ciBaseline/)
})
// 解析不到 ⇒ 仍不判红，与本地同一口径（宁可漏报，也不因**解析**失败变红）。
test('cargoTestRatchet：CI 解析不到通过数 ⇒ 通过并标注', async () => {
  const r = await ratchet('no summary here', { ci: true }, { ciBaseline: 10 }).run()
  assert.equal(r.ok, true)
  assert.match(r.note, /未能解析/)
})
// CI 的实际输出形状是**多个测试目标各一行**，判据按求和而非首行——口径不同于本地
// 分包（那条取首个 result 行）。求和算错会让 1347 这类基线判在错误的数上。
test('cargoTestRatchet：CI 全量按各行求和（不是首行）', async () => {
  const out = 'test result: ok. 7 passed; 0 failed\ntest result: ok. 3 passed; 0 failed'
  const sum = await ratchet(out, { ci: true }, { ciBaseline: 10 }).run()
  assert.equal(sum.ok, true)
  assert.equal(sum.note, undefined)
  const low = await ratchet(out, { ci: true }, { ciBaseline: 11 }).run()
  assert.equal(low.ok, false)
  assert.match(low.note, /10 < 基线 11/)
})
test('cargoTestRatchet：高出基线但**超出回填容差** ⇒ 判红（不回填就没有棘轮）', async () => {
  const r = await ratchet(`test result: ok. ${10 + BASELINE_GRACE + 1} passed; 0 failed`).run()
  assert.equal(r.ok, false)
  assert.match(r.note, /超出回填容差/)
})

// ================= 棘轮三态判定的唯一定义：`ratchetVerdict` =================
//
// 本地分包、CI 全量、verify 的程序数与反例档**必须同一条判据**——各自写一遍
// 「低于基线怎么办」必然演化成两套口径。四态各钉一条，外加容差边界：
// 差一格就从黄变红，边界不钉住则容差会在无人察觉时被改动。

const v = (actual, baseline) => ratchetVerdict({ actual, baseline, name: 'rustTests' })

test('ratchetVerdict：低于 ⇒ 红', () => {
  assert.equal(v(9, 10).ok, false)
})
test('ratchetVerdict：等于 ⇒ 绿且无 note', () => {
  const r = v(10, 10)
  assert.equal(r.ok, true)
  assert.equal(r.note, undefined)
  assert.equal(r.warn, undefined)
})
test('ratchetVerdict：高出但在容差内 ⇒ 绿 + warn（给「先加测试再回填」留余量）', () => {
  const r = v(10 + BASELINE_GRACE, 10)
  assert.equal(r.ok, true)
  assert.equal(r.warn, true)
  assert.match(r.note, /基线待更新/)
})
test('ratchetVerdict：高出且越过容差 ⇒ 红，且点名要改哪一格', () => {
  const r = v(10 + BASELINE_GRACE + 1, 10)
  assert.equal(r.ok, false)
  assert.match(r.note, /BASELINE\.rustTests/)
})

// ================= BASELINE 只增：`parseBaselineCells` / `baselineErosion` =================
//
// 守的是**判据自己**。这一族的失效形态是「解析不到 ⇒ 恒绿」，不是「解析错了 ⇒ 变红」，
// 所以第一条用**运行时导出的常量**做对照：解析口径一旦与 `BASELINE` 的形状脱钩就立刻红。

const sharedSrc = fs.readFileSync(path.join(repoRoot, 'scripts/gate.d', '_shared.mjs'), 'utf8')
/** 最小 BASELINE 文本（只认 `  键: 数字,` 一种形状） */
const baselineText = (pairs, extra = '') =>
  `export const BASELINE = {\n${extra}${pairs.map(([k, n]) => `  ${k}: ${n},`).join('\n')}\n}\n`

test('parseBaselineCells：真实源码每格都取到，且与导出的 BASELINE 逐格相等', () => {
  const cells = parseBaselineCells(sharedSrc)
  assert.ok(cells.size > 0, '真实文件里一格都没取到 ⇒ 解析口径已与 BASELINE 的形状一起失效')
  for (const [key, value] of Object.entries(BASELINE)) {
    assert.equal(cells.get(key), value, `${key} 的解析值与运行时常量不一致`)
  }
})

test('parseBaselineCells：只认对象体内的格——注释里的历史数字不是基线', () => {
  const text = baselineText(
    [['rustTests', 5]],
    '  // 回填 `1339 → 1341`（2026-10-06 实测）\n  // verifyPrograms: 14,\n',
  )
  const cells = parseBaselineCells(text)
  assert.equal(cells.size, 1, '把注释里的数字当一格 ⇒ 历史值冒充基线')
  assert.equal(cells.get('rustTests'), 5)
})

test('parseBaselineCells：对象体在 `\\n}` 处结束，体外不扫', () => {
  const cells = parseBaselineCells(`${baselineText([['a', 1]])}\nfunction f() {\n  b: 2,\n}`)
  assert.deepEqual([...cells.keys()], ['a'])
})

test('parseBaselineCells：CRLF 工作区与 LF 取到同一份（`git show` 给的是 LF）', () => {
  assert.deepEqual([...parseBaselineCells(baselineText([['a', 1]]).replace(/\n/g, '\r\n'))], [['a', 1]])
})

test('parseBaselineCells：没有 BASELINE 对象 ⇒ 空集，不抛', () => {
  assert.equal(parseBaselineCells('// 这个文件里没有基线').size, 0)
})

test('baselineErosion：下调 ⇒ 报出该格；持平或上调 ⇒ 空', () => {
  const base = [{ ref: 'b', text: baselineText([['a', 10], ['c', 4]]) }]
  assert.deepEqual(baselineErosion(baselineText([['a', 9], ['c', 4]]), base), [
    { key: 'a', from: 10, to: 9, waived: false, source: 'b' },
  ])
  assert.equal(baselineErosion(baselineText([['a', 10], ['c', 5]]), base).length, 0)
})

test('baselineErosion：整格被删也是削判据（`to` 为 null，不是漏掉）', () => {
  const [hit] = baselineErosion(baselineText([['a', 10]]), [
    { ref: 'b', text: baselineText([['a', 10], ['verifyNegatives', 3]]) },
  ])
  assert.equal(hit.key, 'verifyNegatives')
  assert.equal(hit.to, null)
})

test('baselineErosion：基准取**并集里的最高值**——只比最近一版会放过「已提交的那次下调」', () => {
  const [hit] = baselineErosion(baselineText([['a', 11]]), [
    { ref: 'new', text: baselineText([['a', 10]]) },
    { ref: 'old', text: baselineText([['a', 12]]) },
  ])
  assert.equal(hit.from, 12)
  assert.equal(hit.source, 'old')
})

test('baselineWaivers：理由非空才算豁免，且按格生效', () => {
  const text = baselineText([['a', 1]]) + '// baseline-allow a: 压测豁免通道\n// baseline-allow b:\n'
  assert.deepEqual([...baselineWaivers(text)], [['a', '压测豁免通道']])
  assert.equal(baselineWaivers(text).has('b'), false, '空理由的豁免等于没有判据')
})

test('baselineErosion：已登记的豁免仍报出该格，只标 waived（黄字还是红由阶段决定）', () => {
  const current = baselineText([['a', 9]]) + '// baseline-allow a: 一次性压测\n'
  assert.equal(baselineErosion(current, [{ ref: 'b', text: baselineText([['a', 10]]) }])[0].waived, true)
})

// ⚠️ 与 `verify 阶段必须上 CI` 同族：**守卫写在仓库里 ≠ 守卫在 CI 上跑**。
// `35-baseline.mjs` 靠 `git show` 读基准版本，而 `actions/checkout` 默认浅克隆只有一个
// 提交 ⇒ 它在 CI 上走「诚实跳过」。跳过不报错、不变慢、日志一片绿，所以这里同时钉两件事：
// 阶段进了 `--only`，且所在 job 拉了历史。
test('ci.yml：baseline 阶段必须上 CI，且所在 job 要拉全历史', () => {
  const block = jobBlock(ciYml(), 'docs-validation', 'security-check')
  const calls = block.split('\n').filter((l) => l.includes('gate.mjs') && !l.trim().startsWith('#'))
  assert.ok(
    calls.some((l) => /only=[^\s]*baseline/.test(l)),
    `baseline 不在 docs-validation 的 --only 里 ⇒「基线只增」只存在于开发者本机：${calls.join(' | ').trim()}`,
  )
  assert.match(
    block,
    /actions\/checkout@v5[\s\S]{0,200}?fetch-depth:\s*0/,
    '没拉 git 历史 ⇒ baseline 阶段在 CI 上恒为「诚实跳过」，日志里看不出来',
  )
})
