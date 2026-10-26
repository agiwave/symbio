/**
 * plugin-special-case-audit 回归测试 —— 证明它**会红**，且**只在该红的地方红**
 *
 * 一个只会亮绿灯的守卫等于没有守卫。但「写宽了」同样是坏守卫：误报一片 ⇒ 人把豁免写到
 * 满屏 ⇒ 判据被喂到失效。故每个码都钉两头：注入违规必须红，真实形态的**近邻**不得误报。
 *
 *   ① 干净夹具 → exit 0；
 *   ② SC-001 `if name == "agent_run"` → 红；
 *   ③ SC-001 不误报：`unwrap_or("ask_user")`（缺省值）与 `.get("heartbeat")`（键名）→ 绿
 *      ——这两处是**真实仓库里的现成实例**，把它们算进来豁免就写不完了；
 *   ④ SC-001 不误报：注释里的比较、`.test.rs` 里的比较 → 绿；
 *   ⑤ SC-002 预算键直接跟数字 → 红；读档案表 → 绿；
 *   ⑥ SC-003 绕开档案表直接读思考字段 → 红；走 `first_str` + 表键 → 绿；
 *   ⑦ SC-004 供应商名进 `contains` 判据 → 红；`match` 臂（别名登记表）与
 *      `"anthropic-version"` 头名 → 绿；
 *   ⑧ SC-005 豁免悬空（登记了而那一行没命中）→ 红；
 *   ⑨ 带理由的豁免 → 绿；**空理由**不算豁免 → 红；
 *   ⑩ 工具名集合读不出来 ⇒ 判据失效，exit 1（不许「读空即全绿」）。
 *
 * 用法：node --test scripts/plugin-special-case-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./plugin-special-case-audit.mjs', import.meta.url))

/** 删临时目录 —— 逐个删，不用 rmSync(recursive)（沙箱会拦截批量删除，见 test-layout-audit.test.mjs）。 */
function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

/**
 * 工具名的**最小可提取**夹具。
 *
 * 为什么不直接用本仓真实的工具名：那样这条判据会随一次无关改名一起红（`agent_run`
 * 若被改名，红的理由却是「守卫坏了」）。工具名从代码提取这件事本身，由 `facts/_capability`
 * 的测试守；这里只要求「提取接口给得出一个名字」。
 */
const TOOLS_FIXTURE = {
  'crates/symbio-plugin-fake/src/tools.rs': [
    'pub fn table() -> Vec<ToolSpec> {',
    '    vec![tool("agent_run", "跑子智能体", schema(), CapabilityRiskLevel::High)]',
    '}',
    '',
  ].join('\n'),
}

/** 跑一次守卫：`session` / `protocols` 是相对路径 → 内容的夹具，`tools` 为工具名夹具（缺省用最小夹具）。 */
function run({ session = {}, protocols = {}, tools = TOOLS_FIXTURE } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'plugin-special-case-'))
  try {
    const write = (base, files) => {
      for (const [rel, content] of Object.entries(files)) {
        const abs = path.join(root, base, rel)
        fs.mkdirSync(path.dirname(abs), { recursive: true })
        fs.writeFileSync(abs, content)
      }
    }
    write('session', session)
    write('protocols', protocols)
    write('tools', tools)
    for (const dir of ['session', 'protocols', 'tools']) {
      fs.mkdirSync(path.join(root, dir), { recursive: true })
    }
    const result = spawnSync(
      process.execPath,
      [
        script,
        `SESSION_ROOT=${path.join(root, 'session')}`,
        `PROTOCOLS_ROOT=${path.join(root, 'protocols')}`,
        `TOOLS_ROOT=${path.join(root, 'tools')}`,
      ],
      { cwd: root, env: { ...process.env, NO_COLOR: '1' }, encoding: 'utf8', timeout: 30_000 },
    )
    assert.ifError(result.error)
    return result
  } finally {
    rmTree(root)
  }
}

/** 档案表夹具：SC-003 的名单来源就是它里面的字符串字面量。 */
const PROFILE_FIXTURE = {
  'profile.rs': [
    'pub(crate) const OPENAI_CHAT_REASONING_KEYS: &[&str] = &["reasoning_content", "reasoning"];',
    'pub(crate) fn first_str(v: &Value, keys: &[&str]) -> Option<&str> {',
    '    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str))',
    '}',
    '',
  ].join('\n'),
}

test('干净夹具：无违规时退出 0', () => {
  const r = run({
    session: { 'executor.rs': 'fn f(name: &str) -> bool { name.starts_with("local/") }\n' },
    protocols: { 'profile.rs': PROFILE_FIXTURE['profile.rs'], 'chat.rs': 'let x = 1;\n' },
  })
  assert.equal(r.status, 0, r.stderr)
  assert.match(r.stdout, /✓ 插件层特判审计/)
})

test('SC-001：工具名进比较位置 ⇒ 红', () => {
  const r = run({ session: { 'executor.rs': 'if name == "agent_run" {\n    go();\n}\n' } })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /SC-001/)
})

test('SC-001 不误报：缺省值与键名不是「按名分流」', () => {
  // 这两处是真实仓库里的现成写法（resume.rs 的 `unwrap_or("ask_user")`、
  // autonomous.rs 的 `.get("heartbeat")`）——判据把它们算进来的话，豁免会写满整个仓。
  const r = run({
    session: {
      'resume.rs': 'let tool_name = prompt.get("tool_name").and_then(|v| v.as_str()).unwrap_or("ask_user");\n',
      'autonomous.rs': 'let h = meta.get("heartbeat").and_then(|v| v.get("include_history"));\n',
    },
  })
  assert.equal(r.status, 0, r.stderr)
})

test('SC-001 不误报：注释与 .test.rs 里的比较不算', () => {
  const r = run({
    session: {
      'executor.rs': '// 判据同批首读一次：这里曾经有 `if name == "agent_run"` 的特判\nfn f() {}\n',
      'executor.test.rs': 'if name == "agent_run" {\n    assert!(true);\n}\n',
    },
  })
  assert.equal(r.status, 0, r.stderr)
})

test('SC-002：预算键直接跟数字 ⇒ 红', () => {
  const r = run({
    protocols: { 'chat.rs': 'req["max_tokens"] = json!(4096);\n' },
  })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /SC-002/)
})

test('SC-002 不误报：读档案表 ⇒ 绿', () => {
  const r = run({
    protocols: {
      'chat.rs': 'req["max_tokens"] = json!(cfg.max_tokens.unwrap_or(profile::OPENAI_CHAT.default));\n',
    },
  })
  assert.equal(r.status, 0, r.stderr)
})

test('SC-003：绕开档案表直接读思考字段 ⇒ 红', () => {
  const r = run({
    protocols: {
      'profile.rs': PROFILE_FIXTURE['profile.rs'],
      'chat.rs': 'if let Some(r) = delta.get("reasoning_content").and_then(|v| v.as_str()) {}\n',
    },
  })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /SC-003/)
})

test('SC-003 不误报：走 first_str + 表键 ⇒ 绿', () => {
  const r = run({
    protocols: {
      'profile.rs': PROFILE_FIXTURE['profile.rs'],
      'chat.rs': 'let r = first_str(delta, OPENAI_CHAT_REASONING_KEYS);\n',
    },
  })
  assert.equal(r.status, 0, r.stderr)
})

test('SC-004：供应商名进 if 式判据 ⇒ 红', () => {
  const r = run({ protocols: { 'chat.rs': 'if cfg.model.contains("glm") {\n    clamp();\n}\n' } })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /SC-004/)
})

test('SC-004 不误报：match 臂（别名登记表）与头名不算分支', () => {
  const r = run({
    protocols: {
      'mod.rs': 'let id = match alias {\n    "anthropic_messages" | "anthropic" => MODEL_PROTOCOL_ANTHROPIC_MESSAGES,\n    _ => MODEL_PROTOCOL_OPENAI_CHAT,\n};\n',
      'messages.rs': 'h.insert("anthropic-version", v);\n',
    },
  })
  assert.equal(r.status, 0, r.stderr)
})

test('SC-005：豁免悬空 ⇒ 红（登记了而那一行没命中）', () => {
  const r = run({
    session: { 'executor.rs': 'fn f() {}\n// special-case-allow SC-001: 这一行早就没有比较了\n' },
  })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /SC-005/)
})

test('带理由的豁免放行；空理由不算豁免', () => {
  const waived = run({
    session: {
      'executor.rs': [
        'if name == "agent_run" {',
        '    // special-case-allow SC-001: 这里的名字来自外部清单，不是注册表里的工具',
        '    go();',
        '}',
        '',
      ].join('\n'),
    },
  })
  assert.equal(waived.status, 0, waived.stderr)
  assert.match(waived.stdout, /豁免 1 处/)

  const empty = run({
    session: {
      'executor.rs': [
        'if name == "agent_run" {',
        '    // special-case-allow SC-001:',
        '    go();',
        '}',
        '',
      ].join('\n'),
    },
  })
  assert.equal(empty.status, 1, '空理由视为未承认')
  assert.match(empty.stderr, /SC-001/)
})

test('工具名集合读不出来 ⇒ 判据失效，不许「读空即全绿」', () => {
  const r = run({ session: { 'executor.rs': 'fn f() {}\n' }, tools: {} })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /失去判据/)
})
