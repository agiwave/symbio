import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

// `dead-code-audit.mjs` 的 R-001（Rust 声明级）是**判定型**守卫，按本仓教条必须
// 有「注入真实违规并断言脚本变红」的回归测试——否则规则写错了也不会有人发现。
//
// 脚本的仓库根由 `--root=` 注入，故这里在临时目录造一棵**最小仓库**：
//   <root>/tauri/index.html  → /src/main.ts   （前端侧需要一个可达入口，否则
//   <root>/tauri/src/main.ts                   前端分支会把 fixture 当死代码）
//   <root>/crates/*.rs                    （Rust 侧被检对象）
const script = fileURLToPath(new URL('./dead-code-audit.mjs', import.meta.url))

function audit(rustFiles, { waiver = null, env: extraEnv = {}, ts = {} } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'dead-code-audit-'))
  try {
    fs.mkdirSync(path.join(root, 'tauri/src'), { recursive: true })
    fs.writeFileSync(
      path.join(root, 'tauri/index.html'),
      '<script type="module" src="/src/main.ts"></script>\n',
    )
    // 前端夹具：`ts` 里每份文件都从 `main.ts` 静态 import 一次，好让它们进**可达集**
    // （L1 依赖图只扫可达文件——没被 import 的文件根本不会进 R-003 的判定面）。
    const imports = Object.keys(ts).map((name) => `import './${name}'`)
    fs.writeFileSync(
      path.join(root, 'tauri/src/main.ts'),
      [...imports, 'const app = 1', 'console.log(app)'].join('\n') + '\n',
    )
    for (const [name, src] of Object.entries(ts)) {
      fs.writeFileSync(path.join(root, 'tauri/src', name), src)
    }
    fs.mkdirSync(path.join(root, 'crates'), { recursive: true })
    for (const [name, src] of Object.entries(rustFiles)) {
      const text = waiver === null ? src : src.replace('WAIVER', waiver)
      fs.writeFileSync(path.join(root, 'crates', name), text)
    }
    const env = { ...process.env, NO_COLOR: '1', ...extraEnv }
    const result = spawnSync(process.execPath, [script, `--root=${root}`], {
      cwd: root,
      env,
      encoding: 'utf8',
      timeout: 20000,
    })
    assert.ifError(result.error)
    return result
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

test('R-001 fires on a pub declaration nobody mentions', () => {
  const r = audit({ 'a.rs': 'pub fn orphan_helper() -> i32 {\n    1\n}\n' })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /R-001/)
  assert.match(r.stdout, /a\.rs :: fn orphan_helper/)
})

test('R-001 stays silent when the name is mentioned outside its own line', () => {
  // `pub use` 不是 DECL_RE 认的声明形态，因此参照文件只贡献"名字出现过一次"
  const r = audit({
    'a.rs': 'pub fn shared_helper() -> i32 {\n    1\n}\n',
    'b.rs': 'pub use a::shared_helper;\n',
  })
  assert.equal(r.status, 0)
  assert.match(r.stdout, /无全仓零引用的 pub 声明/)
})

test('R-001 counts a same-file mention as a real use', () => {
  // 同一文件内的引用同样是真引用（`#[serde(default = "…")]` 那类只在声明文件里出现）
  const r = audit({
    'a.rs': 'pub fn local_default() -> i32 {\n    1\n}\npub fn other() -> i32 {\n    local_default()\n}\n',
  })
  assert.equal(r.status, 1, 'other 自身零引用，仍应被抓——用它反证 local_default 没被误报')
  assert.doesNotMatch(r.stdout, /local_default/)
})

test('R-001 waiver requires a non-empty reason', () => {
  const src = 'pub const KEPT_THING: &str = "x"; WAIVER\n'
  assert.equal(
    audit({ 'a.rs': src }, { waiver: '// dead-code-allow R-001: 消费方在前端' }).status,
    0,
  )
  assert.equal(
    audit({ 'a.rs': src }, { waiver: '// dead-code-allow R-001:   ' }).status,
    1,
    '空理由视为未承认——否则加个注释就能过，守卫退化成橡皮图章',
  )
})

test('R-001 reports waived items separately from violations', () => {
  const src = 'pub const KEPT_THING: &str = "x"; WAIVER\n'
  const r = audit({ 'a.rs': src }, { waiver: '// dead-code-allow R-001: 闭集成员' })
  assert.equal(r.status, 0)
  assert.match(r.stdout, /已承认保留/)
  assert.match(r.stdout, /闭集成员/)
})

test('an empty fixture passes (guard is not vacuously red)', async () => {
  const r = audit({})
  assert.equal(r.status, 0)
})

// ── R-002：`dead_code` 承认标记必须带理由 + 数量棘轮 ──────────────────────
// 起因：C-003 收窄根导出后，只被测试引用的冻结契约名会被 `dead_code` 逐个点名，
// 它们按契约要留到接线那天 ⇒ 逐项承认。承认若无理由、无数量约束，守卫就自废了。

test('R-002 fires on an allow(dead_code) with no reason', () => {
  const r = audit({
    'a.rs': '#[allow(dead_code)]\nfn contract_thing() {}\n',
  })
  assert.equal(r.status, 1, '没有理由的承认 = 未承认，必须判红')
  assert.match(r.stdout, /R-002/)
  assert.match(r.stdout, /a\.rs:1/)
})

test('R-002 accepts the reason on the same line and on the line above', () => {
  const sameLine = audit({
    'a.rs': '#[allow(dead_code)] // dead-code-allow R-002: 协议契约键（消费方=004-PROTOCOLS.md）\nfn k() {}\n',
  })
  assert.equal(sameLine.status, 0, '同行注理由应被认')
  const above = audit({
    'a.rs': '// dead-code-allow R-002: 冻结契约名（消费方=前端镜像）\n#[allow(dead_code)]\nfn c() {}\n',
  })
  assert.equal(above.status, 0, '紧邻上一行注理由也应被认（与 R-001 同一回看口径）')
  const empty = audit({
    'a.rs': '// dead-code-allow R-002:   \n#[allow(dead_code)]\nfn c() {}\n',
  })
  assert.equal(empty.status, 1, '空理由视同未承认——否则加个注释就能过')
})

test('R-002 ignores `#[allow(dead_code)]` mentioned inside doc comments', () => {
  // `plugin/route.rs` 的文档段就写过一句「会带上 `#[allow(dead_code)]`」——
  // 举例的写法若被当成标记，就会凭空多出一条「没有理由的标记」。
  const r = audit({
    'a.rs': '/// 这里会带 `#[allow(dead_code)]` 同行注理由\nfn documented() {}\n',
  })
  assert.equal(r.status, 0)
  assert.match(r.stdout, /0 处 `\[allow\(dead_code\)\]`/)
})

test('R-002 ratchet: marks above the baseline turn red', () => {
  const r = audit(
    { 'a.rs': '// dead-code-allow R-002: 契约名（消费方=协议文档）\n#[allow(dead_code)]\nfn c() {}\n' },
    { env: { DEAD_CODE_R002_BASELINE: '0' } },
  )
  assert.equal(r.status, 1, '数量超过基线必须判红——棘轮只许降')
  assert.match(r.stdout, /棘轮/)
})

test('R-002 counts the module-level marker too', () => {
  const r = audit({ 'a.rs': '#![allow(dead_code)]\nfn anything() {}\n' })
  assert.equal(r.status, 1, '模块级 `#![allow(dead_code)]` 同样要带理由')
  assert.match(r.stdout, /a\.rs:1/)
})

// R-002 ③：**承认必须指名消费方**。消费方不在 Rust 侧是这类标记存在的唯一理由，
// 判据只能是理由里那个非空名字——否则「其实有人用」只是一句自述，门禁查不出。
test('R-002 ③ 承认理由必须指名消费方（`消费方=` 非空）', () => {
  const missing = audit({
    'a.rs': '#[allow(dead_code)] // dead-code-allow R-002: 契约名先于接线\nfn c() {}\n',
  })
  assert.equal(missing.status, 1, '理由里没有消费方 = 没人用，判红')
  assert.match(missing.stdout, /指名消费方/)

  const empty = audit({
    'a.rs': '#[allow(dead_code)] // dead-code-allow R-002: 契约名（消费方=  ）\nfn c() {}\n',
  })
  assert.equal(empty.status, 1, '空消费方视同没写')

  const ok = audit({
    'a.rs': '#[allow(dead_code)] // dead-code-allow R-002: 前端镜像词（消费方=schemas/vdfs.ts）\nfn c() {}\n',
  })
  assert.equal(ok.status, 0, '理由点名消费方则放行')
})

// ================= R-003：前端未用导出必须二态 =================
//
// 这一条把「说不清该不该用」的导出（既不删也不承认）从降级提示升成判定。
// 三个用例分别钉：未承认 ⇒ 红；文件头承认（契约面）⇒ 绿；空理由 ⇒ 仍红（承认不成立）。
// 第三个最重要——给了「写一行注释就过关」的口子，就必须同时钉住那条注释要有内容。

test('R-003 fires on an unused export with no acknowledgment', () => {
  const r = audit(
    {},
    { ts: { 'dead.ts': 'export const NOBODY_CALLS_ME = 1\nexport const USED = 2\nconsole.log(USED)\n' } },
  )
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /R-003/)
  assert.match(r.stdout, /dead\.ts :: NOBODY_CALLS_ME/)
  assert.doesNotMatch(r.stdout, /dead\.ts :: USED/, '有人用的那个不报（判据不误伤）')
})

test('R-003 accepts a file-header acknowledgment with a reason (契约面)', () => {
  const r = audit(
    {},
    {
      ts: {
        'routes.gen.ts':
          '// dead-code-allow R-003: 前后端路由契约的全量镜像，未用常量是契约的另一半\nexport const ROUTE_UNUSED = "x/y"\n',
      },
    },
  )
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /全部已承认/)
})

test('R-003 waiver needs a non-empty reason（写了标记不等于承认）', () => {
  const r = audit(
    {},
    {
      ts: {
        'routes.gen.ts': '// dead-code-allow R-003:\nexport const ROUTE_UNUSED = "x/y"\n',
      },
    },
  )
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /R-003/)
  assert.match(r.stdout, /ROUTE_UNUSED/)
})

test('R-003 承认是**文件作用域**：同文件里新出现的未用导出一并被承认（生成物才有这个待遇）', () => {
  const r = audit(
    {},
    {
      ts: {
        'routes.gen.ts':
          '// dead-code-allow R-003: 契约面全量镜像\nexport const A = 1\nexport const B = 2\n',
      },
    },
  )
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /2 个导出全库无人用，全部已承认/)
})
