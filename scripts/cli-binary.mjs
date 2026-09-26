#!/usr/bin/env node
/**
 * cli-binary — CLI release 二进制的**路径解析**与**新鲜度机制**。
 *
 * 门禁（`gate.d/40-e2e.mjs`）与 e2e（`e2e/helpers.mjs`、`e2e/run-tests.mjs`）
 * 都只从这里取二进制，本文件是这条知识的**唯一真相**。
 *
 * ## 为什么要有这个文件：一个已发生的假失败
 *
 * 原来的判据是「文件在不在」——`when: () => ctx.ci || !cliBinaryExists(repoRoot)`。
 * 于是本机那份 9/22 21:37 的 exe 被一直用下去，而 9/23 的源码已经加了
 * 「新建类型携带选项定义（schema）」的能力 ⇒ e2e T13 断言失败，**失败形态与眼前的
 * 源码直接矛盾**（源码里明明有的字段，运行时是 `undefined`）。排查方向被误导到
 * 「是不是 schema 没接上」，实际是**产物过期**。
 *
 * 结论：二进制是构建产物的函数，**产物比输入旧就是不可信的**。
 * 判据不能是「存在」，只能是「这份产物是否对应当前源码」。
 *
 * ## 判据为什么用内容指纹而不是 mtime
 *
 * mtime 会被 git checkout / 解压 / 复制 / 恢复备份改写，方向还不确定——
 * 既可能假旧（多一次无害的重建），也可能**假新**（把过期产物当最新，正是要防的）。
 * 内容指纹只问一件事：**构建输入变了吗**。代价是哈希 `cli/src` + `symbio/src`
 * 的全部 `.rs`（本仓 272 文件 / 约 3 MB）与四个清单，约 10 ms —— 每次检查都付得起。
 *
 * 大文件（`include_bytes!` 进来的内嵌资产，如 24 MB 的 `model.onnx`）改用
 * **(大小, mtime)**：它每次变动大小或时间必变，而每次为它读 24 MB 纯属浪费。
 * 阈值见 `LARGE_INPUT_BYTES`。
 *
 * ## 机制
 *
 * - 构建成功后，把当次输入的指纹写成**构建戳**（`.symbio-cli.build-stamp`，
 *   与二进制同目录，即 `target/` 内、天然被 gitignore）。
 * - 使用前比对：戳 == 当前指纹 ⇒ 直接用；不等 ⇒ 先 `cargo build --release`
 *   再写戳。**cargo 自己会判增量**，源码没真变时它 0.3 s 就结束。
 * - 不许构建时（`build: false`）指纹对不上就**抛错**，绝不静默用旧产物。
 *
 * 手工跑过 `cargo build --release` 时戳不会更新 ⇒ 下次使用会多一次
 * 「重建」（cargo 秒回）。方向是安全的：宁可多跑一次 cargo，不可少跑一次。
 *
 * ## 用法
 *
 *   node scripts/cli-binary.mjs            # 需要就重建；打印一行结论
 *   node scripts/cli-binary.mjs --check    # 只判不建；不可信则退出码 1
 */

import fs from 'node:fs'
import path from 'node:path'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const EXE = `symbio-cli${process.platform === 'win32' ? '.exe' : ''}`

/** 超过此体积的构建输入按 (大小, mtime) 判，不读内容（内嵌资产） */
const LARGE_INPUT_BYTES = 1024 * 1024

/** 戳文件名（与二进制同目录） */
const STAMP_NAME = '.symbio-cli.build-stamp'

/**
 * 三个候选位置（按优先级：根 workspace 的 target 在前）。
 *
 * 根 workspace 统一后，cli 的 release 二进制默认落在仓库根 `target/release/`
 * （所有成员共用一个 target/）。旧位置的 `symbio/target/`、`cli/target/` 只在
 * 过渡期（尚未清理的旧产物）可能存在，一并探测以避免「找不到即判定缺失」。
 *
 * 写死单一位置会让「二进制找不到 ⇒ 每次都判定缺失」与「e2e 拿不到二进制 ⇒
 * 全用例失败（且失败形态是 -1 + 空 stderr，与崩溃无法区分）」同时发生。
 */
export function cliBinaryCandidates(repoRoot) {
  return [
    path.join(repoRoot, 'target', 'release', EXE),
    path.join(repoRoot, 'symbio', 'target', 'release', EXE),
    path.join(repoRoot, 'cli', 'target', 'release', EXE),
  ]
}

/**
 * 取**最新**的那个已存在候选，而不是第一个。
 *
 * 换过 `cli/.cargo/config.toml`（或从别的机器拷来 target/）之后，两个位置可能
 * 各躺着一份。此时「第一个」可能恰是几个月前那份——正是要防的过期产物。
 * 构建产物里最新的一份才对应最近一次构建。
 */
export function cliBinaryPath(repoRoot) {
  const candidates = cliBinaryCandidates(repoRoot)
  let best = null
  for (const p of candidates) {
    let st
    try {
      st = fs.statSync(p)
    } catch {
      continue
    }
    if (!best || st.mtimeMs > best.mtimeMs) best = { p, mtimeMs: st.mtimeMs }
  }
  return best ? best.p : candidates[0]
}

/** 递归列出目录下的全部文件（目录不存在 ⇒ 空，不抛） */
function walkFiles(dir, out = []) {
  let entries
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return out
  }
  for (const e of entries) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) walkFiles(p, out)
    else if (e.isFile()) out.push(p)
  }
  return out
}

/**
 * 构建输入清单 = 两个 crate 的源码树 + 四个清单 + 可能存在的 build.rs / .cargo 配置。
 *
 * 用「列目录」而不是「读 Cargo 的依赖图」：依赖图要跑 cargo metadata（几百毫秒且
 * 需要网络语义），而这里只需要**宁可多算**——多纳入一个文件只会多触发一次无害的
 * cargo 增量构建，漏掉一个才会让过期产物蒙混过关。
 */
export function cliBuildInputs(repoRoot) {
  const files = []
  for (const sub of ['cli/src', 'symbio/src']) {
    files.push(...walkFiles(path.join(repoRoot, sub)))
  }
  for (const rel of [
    'Cargo.toml',
    'Cargo.lock',
    'cli/Cargo.toml',
    'cli/build.rs',
    'cli/.cargo/config.toml',
    'symbio/Cargo.toml',
    'symbio/build.rs',
    'symbio/.cargo/config.toml',
  ]) {
    const p = path.join(repoRoot, rel)
    if (fs.existsSync(p)) files.push(p)
  }
  return files.sort()
}

/** 构建输入的 sha256 指纹（同一份输入 ⇒ 同一个值） */
export function cliBuildFingerprint(repoRoot) {
  const h = createHash('sha256')
  for (const p of cliBuildInputs(repoRoot)) {
    const rel = path.relative(repoRoot, p).split(path.sep).join('/')
    let st
    try {
      st = fs.statSync(p)
    } catch {
      continue
    }
    if (st.size > LARGE_INPUT_BYTES) {
      h.update(`L\0${rel}\0${st.size}\0${Math.trunc(st.mtimeMs)}\n`)
    } else {
      h.update(`T\0${rel}\0`)
      h.update(fs.readFileSync(p))
      h.update('\0\n')
    }
  }
  return h.digest('hex')
}

/** 二进制旁的构建戳路径 */
export function cliStampPath(binaryPath) {
  return path.join(path.dirname(binaryPath), STAMP_NAME)
}

function readStamp(binaryPath) {
  try {
    return fs.readFileSync(cliStampPath(binaryPath), 'utf8').trim()
  } catch {
    return null
  }
}

/**
 * 二进制当前状态：`fresh` 为真当且仅当「存在」且「戳 == 当前指纹」。
 *
 * `staleReason` 是给人看的一句话——e2e 假失败最贵的是排查方向，把原因直接说出来。
 */
export function cliBinaryState(repoRoot) {
  const binaryPath = cliBinaryPath(repoRoot)
  const exists = fs.existsSync(binaryPath)
  const fingerprint = cliBuildFingerprint(repoRoot)
  const stamped = exists ? readStamp(binaryPath) : null
  const fresh = exists && stamped === fingerprint
  let staleReason = ''
  if (!exists) staleReason = '二进制不存在'
  else if (stamped == null) staleReason = '没有构建戳（手工构建或从别处拷来的，无法确认对应哪份源码）'
  else if (stamped !== fingerprint) staleReason = '构建输入自上次构建后已改变'
  return { binaryPath, exists, fingerprint, stamped, fresh, staleReason }
}

/**
 * 保证「接下来用的二进制对应当前源码」。
 *
 * - 已新鲜 ⇒ 直接返回（不启动 cargo）。
 * - 不新鲜且 `build` 为真 ⇒ `cargo build --release`，成功后写戳。
 * - 不新鲜且 `build` 为假 ⇒ **抛错**。这是本机制的全部要点：宁可停，
 *   也不拿过期产物跑出一堆与源码矛盾的断言。
 *
 * 同步实现（`spawnSync`）：调用方 `runCli` 本身就是同步的，异步化没有收益。
 */
export function ensureCliBinary(repoRoot, { build = true, log = null } = {}) {
  const before = cliBinaryState(repoRoot)
  if (before.fresh) return { ...before, rebuilt: false }

  if (!build) {
    throw new Error(
      `CLI release 二进制不可信（${before.staleReason}）：${before.binaryPath}\n` +
        `请先构建：node scripts/cli-binary.mjs（等价于 cd cli && cargo build --release）`,
    )
  }

  if (log) log(`${before.staleReason} ⇒ cargo build --release`)

  const r = spawnSync('cargo', ['build', '--release'], {
    cwd: path.join(repoRoot, 'cli'),
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
  })
  if (r.error) {
    throw new Error(`无法启动 cargo（${r.error.message}）——构建 CLI 需要本机 Rust 工具链`)
  }
  if (r.status !== 0) {
    const tail = `${r.stderr || ''}\n${r.stdout || ''}`.trim().split('\n').slice(-12).join('\n')
    throw new Error(`cargo build --release 失败（exit=${r.status}）\n${tail}`)
  }

  const after = cliBinaryState(repoRoot)
  if (!after.exists) {
    throw new Error(
      `cargo build --release 成功但找不到二进制（预期其一：${cliBinaryCandidates(repoRoot).join(' | ')}）`,
    )
  }
  fs.writeFileSync(cliStampPath(after.binaryPath), after.fingerprint, 'utf8')
  if (log) log(`已重建：${after.binaryPath}`)
  return { ...after, fresh: true, rebuilt: true }
}

// ==================== CLI 入口（门禁的构建任务就调它） ====================

const invokedDirectly =
  process.argv[1] != null && path.basename(process.argv[1]) === path.basename(fileURLToPath(import.meta.url))

if (invokedDirectly) {
  const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
  const checkOnly = process.argv.includes('--check')
  try {
    const st = ensureCliBinary(repoRoot, {
      build: !checkOnly,
      log: (m) => console.log(`      ${m}`),
    })
    console.log(
      st.rebuilt
        ? `      ✓ CLI release 二进制已重建（${path.relative(repoRoot, st.binaryPath)}）`
        : `      ✓ CLI release 二进制与当前源码一致（${path.relative(repoRoot, st.binaryPath)}）`,
    )
    process.exit(0)
  } catch (e) {
    console.error(`      ✗ ${e.message}`)
    process.exit(1)
  }
}
