#!/usr/bin/env node
/**
 * cli-binary — CLI release 二进制的**路径解析**与**新鲜度机制**。
 *
 * 门禁（`gate.d/58-e2e.mjs`）与 e2e（`e2e/helpers.mjs`、`e2e/run-tests.mjs`）
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
 * 内容指纹只问一件事：**构建输入变了吗**。代价是哈希整个 `crates/` 树的源码
 * 与根清单（数千文件 / 约数十 MB），约几十毫秒 —— 每次检查都付得起。
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
 * CLI 二进制的绝对路径。
 *
 * 只有一个去处：`cli` 是根 workspace 的成员，cargo 把所有成员的产物写进**同一个**
 * 根 `target/`。路径是目录结构的投影，没有第二个候选——探测别处只会把过期产物
 * 当成「找到了」，而那正是本机制要防的假失败。
 */
export function cliBinaryPath(repoRoot, profile = 'release') {
  return path.join(repoRoot, 'target', profile, EXE)
}

/**
 * 同一路径的**相对**写法（正斜杠）——文档指路只认这一条。
 *
 * `doc-symbol-audit` 的 D-012 拿它当唯一标准：文档里写的产物路径必须解析回这里。
 */
export function cliArtifactRelPath(profile = 'debug') {
  return `target/${profile}/${EXE}`
}

/**
 * 递归列出目录下的全部文件（目录不存在 ⇒ 空，不抛）。
 *
 * 跳过构建产物与依赖目录：本函数现在的扫描根是**整个 `crates/`**，若把
 * `crates/<成员>/target/` 收进指纹，写构建戳这个动作本身（戳就写在 `target/release/`
 * 里）会改变指纹 ⇒ 「刚写完戳就不新鲜」，机制自锁。
 */
const SKIP_DIRS = new Set(['target', 'node_modules', '.git', 'dist', '.cache'])

function walkFiles(dir, out = []) {
  let entries
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return out
  }
  for (const e of entries) {
    if (e.isDirectory() && SKIP_DIRS.has(e.name)) continue
    const p = path.join(dir, e.name)
    if (e.isDirectory()) walkFiles(p, out)
    else if (e.isFile()) out.push(p)
  }
  return out
}

/**
 * 构建输入清单 = `crates/` 整棵树 + 根清单 + 可能存在的根 `.cargo/config.toml`。
 *
 * 用「列目录」而不是「读 Cargo 的依赖图」：依赖图要跑 cargo metadata（几百毫秒且
 * 需要网络语义），而这里只需要**宁可多算**——多纳入一个文件只会多触发一次无害的
 * cargo 增量构建，漏掉一个才会让过期产物蒙混过关。
 *
 * ## 为什么只有一个源码根
 *
 * CLI 复用整棵插件树，构建输入就是整个 `crates/`。写成 `crates/symbio-cli/src` 与
 * `crates/` 两个根时，**后者包含前者** ⇒ 同一文件收两遍，清单里出现重复项（指纹仍然
 * 可用，但清单本身不再是一份集合）。
 *
 * 故只走一个根，并用 `Set` 去重：清单是**文件集合**这件事由数据结构保证，
 * 而不是靠调用方小心不要写出包含关系的子根。
 */
export function cliBuildInputs(repoRoot) {
  const seen = new Set()
  const files = []
  const push = (p) => {
    const abs = path.resolve(p)
    if (seen.has(abs)) return
    seen.add(abs)
    files.push(abs)
  }
  for (const p of walkFiles(path.join(repoRoot, 'crates'))) push(p)
  for (const rel of ['Cargo.toml', 'Cargo.lock', '.cargo/config.toml']) {
    const p = path.join(repoRoot, rel)
    if (fs.existsSync(p)) push(p)
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
        `请先构建：node scripts/cli-binary.mjs（等价于 cargo build -p symbio-cli --release）`,
    )
  }

  if (log) log(`${before.staleReason} ⇒ cargo build --release`)

  const r = spawnSync('cargo', ['build', '--release'], {
    cwd: path.join(repoRoot, 'crates', 'symbio-cli'),
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
      `cargo build --release 成功但找不到二进制（预期：${cliBinaryPath(repoRoot)}）`,
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
