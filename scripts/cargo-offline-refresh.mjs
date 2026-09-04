#!/usr/bin/env node
/**
 * 重新预热 Cargo 本地依赖缓存。
 *
 * 为什么需要它：
 *   symbio/.cargo/config.toml 里固定了 `net.offline = true`，构建全程不触网、
 *   只读 ~/.cargo 缓存。因此**新增或升级依赖后**，必须跑一次本脚本把新 crate
 *   拉进缓存，否则离线构建会直接报「缺少 crate」。
 *
 * 用法（需要网络）：
 *   node scripts/cargo-offline-refresh.mjs
 *
 * 只校验离线可解析、不联网：
 *   node scripts/cargo-offline-refresh.mjs --check
 *
 * 说明：本脚本只做下载与解析校验，不编译，因此不需要 MSVC 环境变量。
 * 用 Node 写成，Windows / macOS / Linux 通用（仓库不使用 .sh / .ps1 平台脚本）。
 */

import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const crateDir = path.resolve(scriptDir, '..', 'symbio')

/** 只校验离线可解析，不联网 */
const checkOnly = process.argv.includes('--check')

/**
 * @param {string[]} args
 * @param {{quiet?: boolean}} [options] quiet=true 时丢弃 stdout（cargo metadata 会吐出
 *        数 MB 的 JSON，这里只关心退出码），stderr 始终继承以便看到报错。
 */
function runCargo(args, options = {}) {
  const { quiet = false } = options
  const result = spawnSync('cargo', args, {
    cwd: crateDir,
    stdio: quiet ? ['ignore', 'ignore', 'inherit'] : 'inherit',
    // shell:false + 参数数组 —— 跨平台无引号转义问题
    shell: false,
  })

  if (result.error) {
    console.error(`[cargo-offline-refresh] 无法启动 cargo：${result.error.message}`)
    console.error('请确认 Rust 工具链已安装并且 cargo 在 PATH 中。')
    process.exit(1)
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1)
  }
}

if (checkOnly) {
  console.log('[check] 校验离线可解析（不联网）...')
  // 输出丢弃：只要退出码为 0 即代表离线依赖完整
  const result = spawnSync('cargo', ['metadata', '--offline', '--format-version', '1'], {
    cwd: crateDir,
    stdio: ['ignore', 'ignore', 'pipe'],
    shell: false,
  })

  if (result.status === 0) {
    console.log('OK：离线依赖完整，cargo check / cargo test 可离线运行。')
    process.exit(0)
  }

  console.error('离线依赖不完整。请联网后重新运行本脚本（不带 --check）预热缓存。')
  if (result.stderr?.toString().trim()) {
    console.error(result.stderr.toString().trim())
  }
  process.exit(1)
}

console.log('[1/2] 拉取全部依赖（含 dev-dependencies）到本机缓存...')
runCargo(['--config', 'net.offline=false', 'fetch'])

console.log('[2/2] 校验离线可解析...')
// quiet：metadata 的 JSON 输出无关紧要，只看退出码
runCargo(['metadata', '--offline', '--format-version', '1'], { quiet: true })

console.log('OK：离线缓存已就绪。cargo check / cargo test 现在可完全离线运行。')
