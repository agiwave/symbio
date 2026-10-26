//! `core-change-audit` 的判据回归：两条规则各配正例与反例。
//!
//! 判据本身是**纯函数**（`judge(sha, files, message)`），所以这里不起临时仓库、
//! 不跑 git——上面那层只负责「把 git 的输出喂进来」，喂进来的形状由这些用例钉住。
//! 端到端那半（`--range` / 默认 merge-base）在 `main()` 里，靠门禁实跑覆盖。

import { test } from 'node:test'
import assert from 'node:assert/strict'
import { judge } from './core-change-audit.mjs'

const SHA = 'deadbeefdeadbeefdeadbeefdeadbeefdeadbeef'
const CORE = 'crates/symbio-core/src/lib.rs'
const KERNEL = 'crates/symbio-kernel/src/event/mod.rs'
const PLUGIN = 'crates/symbio-plugin-web/src/plugin.rs'

/** 一条合规的提交信息（标题 + 门禁段），用来隔离出「trailer 有无」这一件事 */
function msg(extra = '') {
  return `feat(x): 标题\n\n1. 改了什么\n\n门禁：全绿\n${extra}`
}

test('CC-001 不触发：没动 core/kernel 的提交不需要任何说明', () => {
  const r = judge(SHA, [PLUGIN], msg())
  assert.deepEqual(r.errors, [])
})

test('CC-001 触发：动 core 却没有 `Core-Change:`', () => {
  const r = judge(SHA, [CORE], msg())
  assert.equal(r.errors.length, 1)
  assert.match(r.errors[0], /^CC-001/)
})

test('CC-001 触发：动 kernel 同样算 core', () => {
  const r = judge(SHA, [KERNEL], msg())
  assert.equal(r.errors.length, 1)
  assert.match(r.errors[0], /^CC-001/)
})

test('CC-002 触发：既动插件又动 core，理由里没有决策号', () => {
  const r = judge(SHA, [CORE, PLUGIN], msg('\nCore-Change: 顺手补了个分支\n'))
  assert.equal(r.errors.length, 1)
  assert.match(r.errors[0], /^CC-002/)
})

test('通过：既动插件又动 core，理由挂决策号', () => {
  const r = judge(SHA, [CORE, PLUGIN], msg('\nCore-Change: D-195 共享类型只能住机制层\n'))
  assert.deepEqual(r.errors, [])
})

test('通过：只动 core，理由可以是自由文本（决策号只在「扩能力」这一档强制）', () => {
  const r = judge(SHA, [CORE], msg('\nCore-Change: 纯注释整理，无语义变化\n'))
  assert.deepEqual(r.errors, [])
})

test('认全角冒号：`Core-Change：D-195` 不是漏网的另一种写法', () => {
  const r = judge(SHA, [CORE, PLUGIN], msg('\nCore-Change：D-195 理由\n'))
  assert.deepEqual(r.errors, [])
})

test('决策号允许三位以上（D-1000 不该被当成没写）', () => {
  const r = judge(SHA, [CORE, PLUGIN], msg('\nCore-Change: D-1000 理由\n'))
  assert.deepEqual(r.errors, [])
})

test('provider / testkit 也算「扩能力」侧的证据', () => {
  for (const f of ['crates/symbio-provider-vdfs_service/src/lib.rs', 'crates/symbio-testkit/src/lib.rs']) {
    const r = judge(SHA, [CORE, f], msg('\nCore-Change: 随手改\n'))
    assert.equal(r.errors.length, 1, f)
    assert.match(r.errors[0], /^CC-002/)
  }
})

test('reason 里只写「D-」不算：必须是完整决策号形态', () => {
  const r = judge(SHA, [CORE, PLUGIN], msg('\nCore-Change: D- 理由\n'))
  assert.equal(r.errors.length, 1)
  assert.match(r.errors[0], /^CC-002/)
})
