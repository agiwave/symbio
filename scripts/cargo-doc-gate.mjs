#!/usr/bin/env node
/**
 * cargo-doc-gate — `cargo doc` 的**内容指纹闸门**：源码没变就不重跑。
 *
 * ## 为什么需要它（一次被拆 crate 推翻的成本主张）
 *
 * `gate.d/10-backend.mjs` 里那段注释（2026-10-14）说 `cargo doc` 只有 15.7s，理由
 * 是「依赖的 `rmeta` 由同阶段的 clippy / test 编出来、直接复用」，因此它便宜到
 * 不必为它设计跳过机制。那个数字在**一个 crate** 的树里成立。
 *
 * D-191 拆成 33 个 crate 之后实测：`cargo doc --no-deps --workspace` **1m36s**。
 * 原因不是「rmeta 不复用了」，而是 rustdoc 要为**每个 crate** 各跑一遍，crate 数
 * 从 1 变 33，这一步就按 crate 数线性放大。同日实测 backend 阶段 3m1s（其中 doc
 * 1m36s）、docs 阶段 49s ⇒ **backend 成了关键路径**，与那段注释记录的
 * 「并发批墙钟由 docs 决定、backend 整阶段 25s」正好相反。
 *
 * 于是「本地也跑 doc」这条判据仍然值得留（断链要留在开发者回路里），但代价必须
 * 重新摊掉：**判据的输入没变，上一轮的通过结论就还成立**。
 *
 * ## 这一档代价到底是多少（2026-10-28 本机 20 核复测）
 *
 * | 情形 | 实测 |
 * |---|---|
 * | 戳命中（内容逐字未变） | **0.09s**（本脚本，含 `rustc -V` 与全树哈希） |
 * | 内容未变但 mtime 变了（切分支、`touch`） | cargo 自己判脏：动一个叶子 crate 的 `lib.rs` ⇒ **17.9s**（4 个 crate 重新 Documenting）；动 core/kernel ⇒ 全树 35 个 crate，**148s** |
 * | 产物全 fresh、无戳（cargo 自己扫一遍） | 0.5s |
 *
 * 所以这个戳买的是中间那一档：**内容没变而 mtime 变了**——`git checkout` 会重写全部
 * mtime，那一轮没有这个戳就要白付 148s。它**买不到**「真的改了 core」那一档：
 * 一次 core 改动确实要让 35 个 crate 的文档各渲染一遍（插件的文档会 re-export core 的
 * 项，链接判据只能在各 crate 自己的 rustdoc 跑里成立），那是判据的固有成本，不是浪费。
 *
 * 已评估并**否决**的一条（记在这里免得再试）：把 doc 挪进独立泳道、给它单独的
 * `CARGO_TARGET_DIR`，让它与 test / clippy 重叠——关键路径能省约 60s（backend 现为
 * test 33.6s + clippy 9.5s + doc + cli 16.7s 串在一条 `.cargo-lock` 上）。否决理由：
 * 独立 target 要从零编一遍依赖（含 `ort` / ONNX Runtime 的原生依赖与构建脚本），
 * 冷启动一次的成本高于它每轮省下的，而它还要长期多占一份磁盘。
 *
 * ## 判据
 *
 * 指纹 = 全部 rustdoc 输入的内容哈希：
 *   - `crates/**` 下所有 `.rs`（含内联 `//!` / `///` / doc 注释）
 *   - `tauri/src-tauri/src/**` 下所有 `.rs`
 *   - 根 `Cargo.toml` / `Cargo.lock` 与各 crate 的 `Cargo.toml`（feature 会改变
 *     被文档化的项）
 *   - `rustc -V` 与本次的 `RUSTDOCFLAGS`（工具链或 lint 档位变了，上次的绿灯不适用）
 *
 * 戳写在 `target/.symbio-doc-gate.stamp`：与产物同处（gitignore 天然覆盖），
 * 且**只在 `cargo doc` 退出码 0 之后**写。失败不写戳 ⇒ 下一轮必然重跑，
 * 不存在「红过之后被缓存成绿」。
 *
 * ## 与 `cli-binary.mjs` 同一条纪律
 *
 * 二者都是「产物/结论 → 输入」的判据，都靠内容而不是 mtime，都只在成功时落戳。
 * 区别只在于被缓存的是二进制还是一次通过。
 *
 * ## 用法
 *
 *   node scripts/cargo-doc-gate.mjs          # 需要就跑，不需要就跳过
 *   node scripts/cargo-doc-gate.mjs --force  # 无视戳（排查 rustdoc 问题时）
 */

import fs from 'node:fs'
import path from 'node:path'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')
const STAMP = path.join(repoRoot, 'target', '.symbio-doc-gate.stamp')

/** 参与指纹的源码根（rustdoc 的全部输入都在这几棵树里） */
const SRC_ROOTS = ['crates', path.join('tauri', 'src-tauri', 'src')]

/** 跳过构建产物与依赖目录（`target/` 里就放着本文件的戳，收进来会自锁） */
const SKIP_DIRS = new Set(['target', 'node_modules', '.git', 'dist', '.cache'])

function walk(dir, out = []) {
  let entries
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return out
  }
  for (const e of entries) {
    if (e.isDirectory() && SKIP_DIRS.has(e.name)) continue
    const p = path.join(dir, e.name)
    if (e.isDirectory()) walk(p, out)
    else if (e.isFile()) out.push(p)
  }
  return out
}

/** `rustc -V`：工具链换了，上一轮的通过结论不适用 */
function rustcVersion() {
  const r = spawnSync('rustc', ['-V'], { encoding: 'utf8' })
  return r.status === 0 ? r.stdout.trim() : '<rustc 不可用>'
}

/**
 * 当前 rustdoc 输入的指纹。
 *
 * 只哈希 `.rs` + 清单：rustdoc 的断链判据不依赖其它文件，把 `docs/` 下的 md 收进来
 * 会让「改一段文档说明就重跑一次」——那些 md 不是 rustdoc 的输入（`include_str!`
 * 进来的资源会变 `.rs` 的字节吗？不会，但那种资源一变，链接判据也不变，故不必收）。
 *
 * `crates/` 只走**一遍**：`.rs` 与 `Cargo.toml` 是同一次遍历的产物（原先分两趟，
 * 而这一层的开销全在目录遍历 + 读文件上，实测一次指纹 92ms）。
 */
export function docFingerprint(root = repoRoot) {
  const h = createHash('sha256')
  h.update(`rustc\0${rustcVersion()}\n`)
  h.update(`flags\0${process.env.RUSTDOCFLAGS ?? ''}\n`)
  const files = []
  for (const rel of SRC_ROOTS) {
    for (const f of walk(path.join(root, rel))) {
      if (f.endsWith('.rs') || path.basename(f) === 'Cargo.toml') files.push(f)
    }
  }
  for (const rel of ['Cargo.toml', 'Cargo.lock']) {
    const p = path.join(root, rel)
    if (fs.existsSync(p)) files.push(p)
  }
  const rels = [...new Set(files.map((f) => path.resolve(f)))]
    .map((f) => path.relative(root, f).split(path.sep).join('/'))
    .sort()
  for (const rel of rels) {
    h.update(`T\0${rel}\0`)
    h.update(fs.readFileSync(path.join(root, ...rel.split('/'))))
    h.update('\0\n')
  }
  return h.digest('hex')
}

function readStamp() {
  try {
    return fs.readFileSync(STAMP, 'utf8').trim()
  } catch {
    return null
  }
}

function writeStamp(fp) {
  fs.mkdirSync(path.dirname(STAMP), { recursive: true })
  fs.writeFileSync(STAMP, fp, 'utf8')
}

function main() {
  const force = process.argv.includes('--force')
  const fp = docFingerprint()
  if (!force && readStamp() === fp) {
    console.log(`  doc 输入未变（指纹 ${fp.slice(0, 8)}）⇒ 跳过 cargo doc（上一轮的通过仍然成立）`)
    return 0
  }
  const r = spawnSync('cargo', ['doc', '--no-deps', '--workspace'], {
    cwd: repoRoot,
    stdio: 'inherit',
    env: process.env,
  })
  if (r.status !== 0) {
    // 不落戳：失败必须让下一轮重跑，不能把一次红缓存成绿
    console.error(`  cargo doc 失败（exit=${r.status ?? 'spawn 失败'}）⇒ 不更新指纹戳`)
    return r.status ?? 1
  }
  writeStamp(fp)
  console.log(`  cargo doc 通过 ⇒ 记指纹 ${fp.slice(0, 8)}`)
  return 0
}

// ==================== CLI 入口（门禁的 doc 任务就调它） ====================

const invokedDirectly =
  process.argv[1] != null &&
  path.basename(process.argv[1]) === path.basename(fileURLToPath(import.meta.url))

if (invokedDirectly) process.exit(main())
