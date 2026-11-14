// schema-audit 回归测试
//
// 本脚本是**报告型**（退出码恒 0），所以它的失效形态不是「假绿灯」而是**说假话**：
// 报告把在用的东西列成「死文件 / 死导出」，读的人顺着去改本来没坏的东西。
//
// 被测面只剩**前端镜像**（`tauri/src/schemas`）。后端半边（`symbio_sdk/schemas`）已随
// D-087 退役、其机器与三条夹具测试于 2026-11-13 删除——那三条测试守的是一个真仓里
// **不存在**的目录，夹具非空、被测物扫 0 个文件，测试全绿而判据空转。
//
// 这里钉三件事（都是「报告不许说假话」的具体形态）：
//   1. 相对路径导入（`from './vdfs-form'`）不许被报成死文件；
//   2. 只在自己文件里被用的导出（`OUTCOME_*` 词表常量）**不是**死导出；
//   3. 真的全库无人用的导出要列出来，但带 `// dead-code-allow R-001:` 的要标成
//      「已承认保留」——那是刻意留的跨栈契约半边，删掉不会让任何测试变红。
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { withTempTree } from '../fs-walk.mjs'

const script = path.join(path.dirname(fileURLToPath(import.meta.url)), 'schema-audit.mjs')

/**
 * 造一棵最小仓库并跑 schema-audit。
 * @param {Record<string,string>} files 相对路径 -> 内容
 */
function audit(files) {
  return withTempTree({}, (root) => {
    // 脚本无条件 walk `tauri/src`，缺了会抛 ⇒ 给个空目录占位
    fs.mkdirSync(path.join(root, 'tauri', 'src', 'schemas'), { recursive: true })
    for (const [rel, content] of Object.entries(files)) {
      const p = path.join(root, rel)
      fs.mkdirSync(path.dirname(p), { recursive: true })
      fs.writeFileSync(p, content)
    }
    const r = spawnSync(process.execPath, [script, `--root=${root}`], {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 10000,
    })
    assert.equal(r.status, 0, `脚本应恒为 0（报告型）：\n${r.stderr}`)
    return r.stdout
  }, { prefix: 'schema-audit-' })
}

/** 抠出「前端 schemas 审计」之后的正文：后端那一段只印一句退役说明，不该混进断言。 */
function frontHalf(out) {
  const i = out.indexOf('==== 前端 schemas 审计')
  return i < 0 ? out : out.slice(i)
}

test('相对路径导入构成引用 ⇒ 不被报成死文件', () => {
  const out = frontHalf(
    audit({
      'tauri/src/schemas/vdfs-form.ts': 'export interface FormField { id: string }\n',
      // `export * from` 也是引用（统一出口再导出一份的形态）
      'tauri/src/schemas/vdfs.ts': "export * from './vdfs-form'\nexport const a = 1\n",
      'tauri/src/components/Detail.vue': "<script setup lang=\"ts\">\nimport { a } from '../schemas/vdfs'\n</script>\n",
    }),
  )
  assert.match(out, /vdfs-form\.ts\s+【单消费方】/, out)
  assert.doesNotMatch(out, /vdfs-form\.ts\s+【死文件/, out)
})

test('只在定义文件内被用的导出不是死导出（自引用计数）', () => {
  // 实测事故：`OUTCOME_COMPLETED`（定义在 `schemas/vdfs.ts`、也在同文件里被比较）
  // 曾被列进「死导出」——那不是保守，是报告在说假话。
  const out = frontHalf(
    audit({
      'tauri/src/schemas/vdfs.ts': [
        "export const OUTCOME_COMPLETED = 'completed'",
        'export function isDone(x: string) {',
        '  return x === OUTCOME_COMPLETED',
        '}',
      ].join('\n'),
      'tauri/src/services/vdfs.ts': "import { isDone } from '../schemas/vdfs'\nexport const y = isDone('a')\n",
    }),
  )
  assert.doesNotMatch(out, /OUTCOME_COMPLETED/, out)
})

test('真死导出要列出来；带豁免理由的标「已承认保留」', () => {
  const out = frontHalf(
    audit({
      'tauri/src/schemas/home.ts': [
        '// dead-code-allow R-001: 与后端 schemas.rs 的请求半边对应，前端只传字面量',
        'export type home_reload = { path: string }',
        'export interface NobodyUses { x: number }',
      ].join('\n'),
    }),
  )
  assert.match(out, /NobodyUses/, out)
  assert.match(out, /home_reload（已承认保留：与后端 schemas\.rs 的请求半边对应/, out)
})
