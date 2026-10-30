/**
 * extension-vocab-audit 回归测试 —— 证明它**会红**（EX-008，D-252）
 *
 * 一个只会亮绿灯的守卫等于没有守卫。注入真实违规并断言脚本变红：
 *   ① 干净夹具 → exit 0；
 *   ② 端口形状上加一格 `turn: u64` / `turn_counter` → exit 1 —— 容器词进端口；
 *   ③ `use` 里出现 `TurnSettings` → exit 1 —— 配置类型不入能力面；
 *   ④ `return` 关键字与注释里的「这一轮」→ exit 0 —— 判形状不是判措辞；
 *   ⑤ 生成表（`events.rs` 的 `EVENT_TURN_*` / `Entity::Turn`）与 `*.test.rs` → exit 0 —— 有名豁免①；
 *   ⑥ CJK 容器字进了标识符（`轮次`）→ exit 1 —— 换一种写法的同一个病；
 *   ⑦ 豁免清单被写成通配 / 空理由 → 豁免清单自身判红；
 *   ⑧ 精确豁免只豁免同名同格位的那一个 token，别的 token 照样红；
 *   ⑨ 范围读不出（缺 `plugin-api/src`）→ exit 2。
 *
 * 用法：node --test scripts/extension-vocab-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

import { stripRustCode, findContainerWordTokens, validateExemptions } from './extension-vocab-audit.mjs'

const script = fileURLToPath(new URL('./extension-vocab-audit.mjs', import.meta.url))

/** 删临时目录 —— 逐个删，不用 rmSync(recursive)（沙箱会拦截批量删除，见 no-direct-call-audit.test.mjs）。 */
function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

/** 干净夹具：一个端口文件，形状里没有容器词（注释里可以说「这一轮」）。 */
const CLEAN = {
  'crates/symbio-plugin-api/src/effects.rs': `//! 唯一写口 —— 宿主在落格那一刻盖章（这一轮的轮号由宿主写）。
pub struct EffectEnv<'a> {
    pub anchor: &'a str,
    pub user_seq: u64,
    pub applied: Vec<crate::Event>,
}
pub fn run_recall(env: &EffectEnv) -> bool {
    if env.applied.is_empty() {
        return false;
    }
    true
}
`,
  'crates/symbio-plugin-api/src/settle.rs': `pub trait Contributor {
    fn contribute(&self, at: &Settling) -> Vec<Contribution>;
}
`,
}

function audit(files, env = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ext-vocab-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const result = spawnSync(process.execPath, [script, `--root=${root}`], {
      cwd: root,
      env: { ...process.env, NO_COLOR: '1', ...env },
      encoding: 'utf8',
      timeout: 30_000,
    })
    assert.ifError(result.error)
    return result
  } finally {
    rmTree(root)
  }
}

test('① 干净夹具 ⇒ 绿', () => {
  const r = audit(CLEAN)
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /✓ EX-008/)
})

test('② 端口形状加一格 `turn: u64` ⇒ 红', () => {
  const r = audit({
    'crates/symbio-plugin-api/src/effects.rs':
      CLEAN['crates/symbio-plugin-api/src/effects.rs'] +
      'pub struct Extra { turn: u64 }\n',
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /\[ERROR\] EX-008/)
  assert.match(r.stdout, /effects\.rs/)
  assert.match(r.stdout, /turn/)
})

test('② 形参带 `turn_counter` ⇒ 红（大小写不敏感、子串也算）', () => {
  const r = audit({
    'crates/symbio-plugin-api/src/settle.rs': 'pub fn fold(state: u64, turn_counter: u64) -> u64 { state }\n',
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /turn_counter/)
})

test('③ `use` 里出现 `TurnSettings` ⇒ 红：配置类型不入能力面', () => {
  const r = audit({
    'crates/symbio-plugin-api/src/effects.rs': 'use crate::TurnSettings;\npub struct X;\n',
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /TurnSettings/)
})

test('④ `return` 关键字与注释里的「这一轮」⇒ 绿：判形状不是判措辞', () => {
  const r = audit({
    'crates/symbio-plugin-api/src/effects.rs':
      '// 返回值说明：returns 0 或 returned 1（这一轮的轮号宿主盖）\n' +
      'pub fn f() -> u8 {\n    return 0;\n}\n',
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('⑤ 生成表与 *.test.rs 不判：有名豁免①（事实名不是端口形状）', () => {
  const r = audit({
    ...CLEAN,
    'crates/symbio-plugin-api/src/events.rs': `// 由 scripts/facts/events-rs.mjs 生成，请勿手改
pub const EVENT_TURN_DISPATCH: &str = "turn.dispatch";
pub const CELLS: &[EventDecl] = &[EventDecl::new(EVENT_TURN_DISPATCH, (Entity::Turn, Verb::Asserted))];
`,
    'crates/symbio-plugin-api/src/effects.test.rs': 'pub const X: &str = "turn.dispatch";\n',
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('⑥ CJK 容器字进了标识符 ⇒ 红（换一种写法的同一个病）', () => {
  const r = audit({
    'crates/symbio-plugin-api/src/effects.rs': 'pub struct Extra { 轮次: u64, 幕: u8 }\n',
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /轮/)
  assert.match(r.stdout, /幕/)
})

test('⑦ 豁免写成通配 ⇒ 豁免清单自身判红', () => {
  const errs = validateExemptions([{ path: 'crates/**/*.rs', token: 'turn', why: '存量' }])
  assert.equal(errs.length, 1)
  assert.match(errs[0], /不许用通配/)
})

test('⑦ 豁免没有理由正文 ⇒ 判红', () => {
  const errs = validateExemptions([{ path: 'crates/symbio-plugin-api/src/effects.rs', token: 'turn', why: '  ' }])
  assert.equal(errs.length, 1)
  assert.match(errs[0], /没有理由正文/)
})

test('⑧ 精确豁免只豁免同名同格位的那一个 token，别的 token 照样红', () => {
  const r = audit(
    {
      'crates/symbio-plugin-api/src/effects.rs': 'pub struct Extra { turn: u64, turn_counter: u64 }\n',
    },
    {
      EXTENSION_VOCAB_EXEMPTIONS: JSON.stringify([
        { path: 'crates/symbio-plugin-api/src/effects.rs', token: 'turn', why: 'D-252 认下的存量' },
      ]),
    },
  )
  assert.equal(r.status, 1)
  assert.doesNotMatch(r.stdout, /effects\.rs:\d+\s+标识符 `turn` `/)
  assert.match(r.stdout, /turn_counter/)
})

test('⑧ 豁免不匹配路径时完全不生效（精确等值，不是前缀）', () => {
  const r = audit(
    { 'crates/symbio-plugin-api/src/effects.rs': 'pub struct Extra { turn: u64 }\n' },
    {
      EXTENSION_VOCAB_EXEMPTIONS: JSON.stringify([
        { path: 'crates/symbio-plugin-api/src/other.rs', token: 'turn', why: '指错文件' },
      ]),
    },
  )
  assert.equal(r.status, 1)
})

test('⑨ 范围读不出（缺 plugin-api/src）⇒ exit 2', () => {
  const r = audit({})
  assert.equal(r.status, 2)
})

test('纯函数：stripRustCode 剥块注释、字符串与原始字符串', () => {
  const stripped = stripRustCode('a /* turn /* nested */ still */ b "turn" r#"turn"# c')
  assert.deepEqual(stripped.split(/\s+/).filter(Boolean), ['a', 'b', 'c'])
  assert.equal(stripRustCode("let c = 'x'; let l = 'a;"), "let c =  ; let l = 'a;")
})

test('纯函数：findContainerWordTokens 认 turn 族与 CJK、放行 return 族', () => {
  assert.deepEqual(findContainerWordTokens('return TurnSettings turn_counter 轮次'), [
    'TurnSettings',
    'turn_counter',
    '轮',
  ])
  assert.deepEqual(findContainerWordTokens('return true;'), [])
})
