// schema-audit 回归测试
//
// 本脚本是**报告型**（退出码恒 0），所以它的失效形态不是「假绿灯」而是**说假话**：
// 报告把在用的模块列成「下放候选 / 删除候选」，读的人顺着去改本来没坏的东西。
// 实测事故正是这样：`schemas/hook` 有 3 个消费文件（hook 插件自己 3 个 + session），
// 报告却写「仅 1 个外部消费文件」——因为 hook 插件写的是
// `use crate::symbio_core::schemas::{HookEvent, HookOutput}`（走 `schemas/mod.rs` 的
// 顶层再导出名），路径里**没有子模块名**，只按「最长模块前缀」匹配会得到 null，
// 该消费方整个丢失。
//
// 因此这里钉两件事：
//   1. 顶层再导出名（`schemas::X`）必须归到 X 的定义模块；
//   2. 修复不能顺手把「真的只有一个消费方」也放过——那是这条报告存在的理由。
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const script = path.join(path.dirname(fileURLToPath(import.meta.url)), 'schema-audit.mjs')

/**
 * 造一棵最小仓库并跑 schema-audit。
 * @param {Record<string,string>} files 相对路径 -> 内容
 */
function audit(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'schema-audit-'))
  try {
    // 前端那一半无条件 walk `tauri/src`，缺了会抛 ⇒ 给个空目录占位
    fs.mkdirSync(path.join(root, 'tauri', 'src'), { recursive: true })
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
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

/** 最小 schemas：`mod.rs` 把 `hook::HookEvent` 顶层再导出 */
const SCHEMAS_MOD = `pub mod hook;\npub use hook::HookEvent;\n`
const HOOK_SCHEMA = `pub struct HookEvent { pub kind: String }\n`

test('顶层再导出名（schemas::X）归属到 X 的定义模块', () => {
  // hook 插件走**顶层名**导入；session 走**子模块路径**导入
  const out = audit({
    'symbio/src/symbio_core/schemas/mod.rs': SCHEMAS_MOD,
    'symbio/src/symbio_core/schemas/hook.rs': HOOK_SCHEMA,
    'symbio/src/plugins/hook/executor.rs': 'use crate::symbio_core::schemas::{HookEvent};\n',
    'symbio/src/plugins/hook/plugin.rs': 'use crate::symbio_core::schemas::{HookEvent};\n',
    'symbio/src/plugins/session/tool_executor.rs':
      'use crate::symbio_core::schemas::hook::{HookEvent};\n',
  })
  // hook 有 3 个消费文件 ⇒ 不是下放候选
  assert.match(out, /^\s*hook\s+\[3\]/m, out)
  // 只断言**该段**的内容（`[\s\S]*` 会一路吞到后面的文件清单，那里也印着 hook）
  assert.match(out, /--- 下放候选[^\n]*\n\s*（无）/, out)
})

test('真的只有一个消费方时，仍如实报为下放候选', () => {
  // 反例守卫：修好「漏算消费方」不能变成「一律不算候选」——
  // 那样这条报告就只剩下「无」，等于没有。
  const out = audit({
    'symbio/src/symbio_core/schemas/mod.rs': SCHEMAS_MOD,
    'symbio/src/symbio_core/schemas/hook.rs': HOOK_SCHEMA,
    'symbio/src/plugins/session/tool_executor.rs':
      'use crate::symbio_core::schemas::hook::{HookEvent};\n',
  })
  assert.match(out, /^\s*hook\s+\[1\]/m, out)
  // 下放候选段印的是 `schemas/hook`（`display(top)`），不是裸 `hook`
  assert.match(out, /--- 下放候选[^\n]*\n\s*schemas\/hook\b/, out)
})
