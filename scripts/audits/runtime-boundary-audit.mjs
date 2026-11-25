#!/usr/bin/env node
/**
 * runtime-boundary-audit — 运行时与装配根的依赖方向（RB-001）
 *
 * 运行时只接收异步 RootFactory；具体插件链接属于 CLI / Tauri 的组合边界。
 * `symbio-runtime` 一旦在生产、构建或开发清单里依赖 `symbio`，运行时就会重新
 * 认识装配入口，方案 H 的服务组合也会沿旧方向回流。开发依赖同样禁止：测试
 * 可以用最小假根，发布二进制由入口提供完整装配工厂。
 *
 * 生产源码也不得引用 `symbio` crate；注释只用于说明方向，不构成依赖。
 *
 * 用法：node scripts/audits/runtime-boundary-audit.mjs [--root=<目录>]
 * 退出码：0 = 通过；1 = 有违规；2 = 判据范围读不出。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成）
// @ns RB 运行时组合边界
// @codes RB-001
// @gate guard

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green } from '../color.mjs'
import { stripComments } from '../rust-scan.mjs'

const script = fileURLToPath(import.meta.url)
const rootArg = process.argv.find((arg) => arg.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(script), '..', '..')
const MANIFEST = 'crates/symbio-runtime/Cargo.toml'
const SRC = 'crates/symbio-runtime/src'

let errors = 0
const report = (where, message) => {
  console.log(`${red('[ERROR]')} RB-001 ${where}  ${message}`)
  errors++
}

function rustFiles(dir) {
  const out = []
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const abs = path.join(dir, entry.name)
    if (entry.isDirectory()) out.push(...rustFiles(abs))
    else if (entry.isFile() && entry.name.endsWith('.rs') && !entry.name.endsWith('.test.rs')) out.push(abs)
  }
  return out
}

try {
  const manifest = fs.readFileSync(path.join(ROOT, MANIFEST), 'utf8')
  const dependency = /^\s*(?:symbio|"symbio"|'symbio')\s*=\s*(?=\{|"|true|false)/m
  if (dependency.test(manifest.replace(/^\s*#.*$/gm, ''))) {
    report(MANIFEST, '运行时不得依赖装配根 symbio；请由 CLI / Tauri 注入 RootFactory')
  }

  for (const abs of rustFiles(path.join(ROOT, SRC))) {
    const rel = path.relative(ROOT, abs).replace(/\\/g, '/')
    const code = stripComments(fs.readFileSync(abs, 'utf8'))
    if (/\bsymbio\s*::/.test(code)) report(rel, '生产源码不得引用装配根 symbio')
  }
} catch (error) {
  console.error(red(`✗ 范围读不出：${MANIFEST} 或 ${SRC} 无法读取（D-280）: ${error.message}`))
  process.exit(2)
}

if (errors > 0) {
  console.log(red(`  ${errors} 个违规`))
  process.exit(1)
}
console.log(green('✓ RB-001  运行时清单与生产源码均不依赖装配根 symbio'))
