/**
 * mechanism-audit 回归测试
 *
 * 一个只会亮绿灯的守卫等于没有守卫。这里对每条规则都注入一个**真实违规**，
 * 断言脚本确实变红（exit 1）；再对同一段文本放进注释 / 放进允许的目录，
 * 断言它不再报（否则守卫会因为误报被人用豁免注释喂到失效）。
 *
 * 跑法：node --test scripts/mechanism-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./mechanism-audit.mjs', import.meta.url))

/**
 * 在临时目录里搭一棵最小 `tauri/src` 树并跑审计。
 *
 * @param {Record<string, string>} files 相对 `tauri/src` 的路径 → 内容
 * @param {{strict?: boolean}} [opts]
 */
function audit(files, { strict = false } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'mechanism-audit-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, 'tauri', 'src', rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const result = spawnSync(
      process.execPath,
      [script, `--root=${root}`, ...(strict ? ['--strict'] : [])],
      { env: { ...process.env, NO_COLOR: '1' }, encoding: 'utf8', timeout: 20000 },
    )
    assert.ifError(result.error)
    return result
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

/** 一个干净的最小树（每条规则各自只放一个违规文件） */
const CLEAN = {
  'components/Clean.vue': '<template><div /></template>\n<script setup lang="ts">\n</script>\n',
}

test('干净树通过（exit 0）', () => {
  const r = audit(CLEAN)
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /六条规则全部通过/)
})

// ── M-001：组件不得解释后端 meta 字段 ────────────────────────────────────
test('M-001 命中：组件直接取 meta 字段', () => {
  const r = audit({
    ...CLEAN,
    'components/Bad.vue': `<script setup lang="ts">\nconst ok = node.meta.recoverable\n</script>\n`,
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /M-001 .*components\/Bad\.vue:2/)
})

test('M-001 命中：先断言类型再取字段（`as` 形态）', () => {
  const r = audit({
    ...CLEAN,
    'components/Bad.vue': `<script setup lang="ts">\nconst ok = !(m.meta as { ephemeral?: boolean } | undefined)?.ephemeral\n</script>\n`,
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /M-001/)
})

test('M-001 不误报：整体透传 meta（展开 / 原样序列化）不算解释字段', () => {
  const r = audit({
    ...CLEAN,
    'components/Ok.vue': `<script setup lang="ts">\nconst next = { ...(base.meta || {}), flag: true }\nconst dump = JSON.stringify(node.meta)\n</script>\n`,
  })
  assert.equal(r.status, 0, r.stdout)
})

// ── M-002：不得硬编码 .vdfs 地址 ─────────────────────────────────────────
test('M-002 命中：组件里的 .vdfs 字符串字面量', () => {
  const r = audit({
    ...CLEAN,
    'components/Bad.vue': `<script setup lang="ts">\nconst p = '.vdfs/session'\nconst q = \`.vdfs/\${id}\`\n</script>\n`,
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /M-002 .*components\/Bad\.vue:2/)
})

test('M-002 不误报：注释里的地址举例 / CSS 里的 .vdfs- 类名', () => {
  const r = audit({
    ...CLEAN,
    'components/Ok.vue': [
      '<!-- 地址形如 `.vdfs/session/<id>`，说明用 -->',
      '<template><div class="vdfs-card" /></template>',
      '<script setup lang="ts">',
      '// 清单来自 `.vdfs/session` 的目录内容',
      '/** 绑定地址（如 `.vdfs` 或 `.vdfs/session/<id>`） */',
      '</script>',
      '<style scoped>',
      '.vdfs-card { color: red; }',
      '</style>',
      '',
    ].join('\n'),
  })
  assert.equal(r.status, 0, r.stdout)
})

test('M-002 不误报：字符串里的 // 不吞掉本行（漏报比误报更危险）', () => {
  const r = audit({
    ...CLEAN,
    'components/Ok.vue': `<script setup lang="ts">\nconst url = 'https://example.com/x'\nconst p = '.vdfs/session'\n</script>\n`,
  })
  assert.equal(r.status, 1, 'URL 里的 // 不应把后一行的违规吞掉')
  assert.match(r.stdout, /M-002/)
})

// ── M-003：不得直接 invoke ───────────────────────────────────────────────
test('M-003 命中：组件里直接 invoke / 直接引 @tauri-apps/api/core', () => {
  const r = audit({
    ...CLEAN,
    'components/Bad.vue': `<script setup lang="ts">\nimport { invoke } from '@tauri-apps/api/core'\nconst x = invoke('cmd')\n</script>\n`,
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /M-003/)
})

test('M-003 放行：services/ 里可以 invoke', () => {
  const r = audit({
    ...CLEAN,
    'services/ok.ts': `import { invoke } from '@tauri-apps/api/core'\nexport const x = () => invoke('cmd')\n`,
  })
  assert.equal(r.status, 0, r.stdout)
})

// ── M-004：registry / schemas 不得 import 组件 ───────────────────────────
test('M-004 命中：registry 里 import .vue', () => {
  const r = audit({
    ...CLEAN,
    'registry/bad.ts': `import X from '@/components/X.vue'\nexport default X\n`,
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /M-004 .*registry\/bad\.ts:1/)
})

test('M-004 放行：*Renderers.ts 是刻意的组件装配点', () => {
  const r = audit({
    ...CLEAN,
    'registry/vdfsRenderers.ts': `import X from '@/components/X.vue'\nexport default X\n`,
    'registry/messageRenderers.ts': `import Y from '@/components/Y.vue'\nexport default Y\n`,
  })
  assert.equal(r.status, 0, r.stdout)
})

// ── M-005：schemas 不得反向依赖 ──────────────────────────────────────────
test('M-005 命中：schemas 引 registry', () => {
  const r = audit({
    ...CLEAN,
    'schemas/bad.ts': `import { x } from '@/registry/messageTypes'\nexport const y = x\n`,
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /M-005 .*schemas\/bad\.ts:1/)
})

test('M-005 放行：schemas 内部互相引用 / 引第三方', () => {
  const r = audit({
    ...CLEAN,
    'schemas/ok.ts': `import { z } from './other'\nimport { computed } from 'vue'\nexport const y = [z, computed]\n`,
  })
  assert.equal(r.status, 0, r.stdout)
})

// ── M-006：组件不得用字面量比较消息词表 ──────────────────────────────────
test('M-006 命中：组件里用字面量比较 status / role / type', () => {
  const r = audit({
    ...CLEAN,
    'components/Bad.vue': `<script setup lang="ts">\nconst a = m.status === 'failed'\nconst b = m.role !== 'user'\nconst c = m.type === 'tool_call'\n</script>\n`,
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /M-006 .*components\/Bad\.vue:2/)
  assert.match(r.stdout, /M-006 .*components\/Bad\.vue:4/)
})

test('M-006 不误报：本地 prop 取值 / 内容形状判定 / 非消息域的词表', () => {
  const r = audit({
    ...CLEAN,
    'components/Ok.vue': [
      '<script setup lang="ts">',
      `const cls = variant === 'turn'`,
      `const hasText = typeof c === 'object' && 'text' in c`,
      `const r = renderer === 'json' || renderer === 'markdown'`,
      `const local = status === 'completed'`,
      `const bad = msg.status === 'completed'`,
      '</script>',
      '',
    ].join('\n'),
  })
  // 第 4 行是**本地 prop**（没有接收者），第 5 行才是真违规（`msg.status` 是消息字段）。
  // 规则要求 `.status|.role|.type` 前导点，正是为了放过前者、抓住后者——
  // 一个把 `status === 'x'` 也判违规的守卫会在真实代码里误报，然后被豁免注释喂死。
  // 只数 `[ERROR]` 行：章节标题与汇总行里也含 "M-006" 字样。
  assert.equal(r.status, 1)
  const hits = (r.stdout.match(/\[ERROR\] M-006/g) ?? []).length
  assert.equal(hits, 1, r.stdout)
  assert.match(r.stdout, /Ok\.vue:6/)
  assert.doesNotMatch(r.stdout, /Ok\.vue:5/)
})

// ── 豁免注释 ─────────────────────────────────────────────────────────────
test('豁免注释要求**非空理由**（空理由视为未豁免）', () => {
  const bad = `<script setup lang="ts">\nconst ok = node.meta.recoverable // mechanism-allow M-001: 已复核，属临时节点判定\n</script>\n`
  assert.equal(audit({ ...CLEAN, 'components/W.vue': bad }).status, 0)

  const empty = `<script setup lang="ts">\nconst ok = node.meta.recoverable // mechanism-allow M-001:   \n</script>\n`
  assert.equal(audit({ ...CLEAN, 'components/W.vue': empty }).status, 1)
})

test('豁免只作用于同一条规则，不掩盖其它规则', () => {
  const src = [
    '<script setup lang="ts">',
    `const a = node.meta.recoverable // mechanism-allow M-001: 已复核`,
    `const b = '.vdfs/session'`,
    '</script>',
    '',
  ].join('\n')
  const r = audit({ ...CLEAN, 'components/W.vue': src })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /\[ERROR\] M-002/)
  assert.doesNotMatch(r.stdout, /\[ERROR\] M-001/)
})
