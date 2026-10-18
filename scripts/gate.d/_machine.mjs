#!/usr/bin/env node
/**
 * **这台机器能开多大并发**——所有并发度与超时预算的唯一来源。
 *
 * ## 它治的病：并发度是「某台机器上试出来的数」，却被写成了常量
 *
 * 门禁提速那一轮（2026-10-18）之后，仓里留下了这样一批数：
 *
 *   `30-docs.mjs   concurrency: 8`        审计脚本，IO 密集
 *   `55-verify.mjs concurrency: 8`        每个验证程序一次 rustc
 *   `58-e2e.mjs    min(6, 核数)`          e2e 用例
 *   `windows-restart.test.mjs  = 2`       PowerShell 场景
 *   `VITEST_TIMEOUT_MS = 300_000`
 *   `grep-audit.test.mjs: timeout 120_000`
 *   `windows-restart.test.mjs: timeout 480_000 / invoke 150_000 / waitFor 120_000`
 *
 * 它们**全都正确**——在写它们的那台机器上。而写下它们的人（我）连那台机器几核
 * 都记错了（注释里写「20 核」，实际 `os.cpus().length` 是 8，i7-6700HQ 4C8T）。
 * 这恰好说明问题：**这些数字从来没有跟机器绑定过**，它们只是一次实测的快照。
 *
 * 后果按机器分两种，都很糟：
 *
 * - **更慢的机器**（4 核 / 8GB / 机械盘）：8 路并发互相抢核，8 个 rustc 把内存
 *   吃到 swap，`vitest` 与 `windows-restart` 撞上按强机拍的预算 ⇒ **假红**。
 *   假红比慢更贵：它教人「门禁红了不一定是真坏了」，于是真红也会被忽略。
 * - **更快的机器**（16/32 核）：仍然只开 8 路，多出来的核空转。门禁时间不会
 *   因为换机而下降，用户看到的是「我升级了电脑，门禁一点没快」。
 *
 * ## 两类量，两种来源（别混）
 *
 * **① 并发度**：由**当下**的机器决定（核数 / 可用内存 / 启动时负载）。
 *    立即算出，零成本，不需要历史。换机器立刻生效。
 *
 * **② 超时预算**：由**上一轮实测**决定（`budget()`），不是由核数决定。
 *    「这台机器跑这一步要多久」没有任何静态指标能答得比「上次跑了多久」更准；
 *    核数只是「能同时干几件事」，不是「一件事干多快」。首次运行（无历史）才退化为
 *    `静态值 × cpuScale`，其中 `cpuScale` 来自单核探针。
 *
 * 这两条分开，是因为它们失效的方式不同：并发度算错只是慢/争抢；预算算错会**假红**。
 *
 * ## 探测成本必须可忽略
 *
 * 门禁一轮 4–5 分钟，探针 43ms + 负载采样 126ms ≈ **170ms**（本机实测）。超过
 * 300ms 就该被质疑——**为省时间而花的时间**必须远小于省下的时间，否则它自己
 * 就是新的性能问题。`machine.test.mjs` 有一条判据钉住这个上限。
 *
 * ## 惰性：`--help` / `--list` 不许付这笔钱
 *
 * 探针不在模块加载时跑。`--help`、`--list`、被拒绝的未知参数都只读元信息，
 * 不该为了打印一行帮助先花 170ms（同理见 `gate.mjs` 里「日志目录懒建」那段）。
 * 首次调用 `machine()` 才算。
 */

import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const REPO = path.resolve(HERE, '..', '..')

/** 校准文件（per-machine，落点被 `.gitignore` 的 `.workbuddy*` 规则覆盖）。 */
export const CALIBRATION_FILE = path.join(REPO, '.workbuddy-ai', 'gate-calibration.json')

/**
 * 单核探针的**基准读数**：`43ms`。
 *
 * 取自 2026-10-19 的 i7-6700HQ（4C8T / 16GB / Windows）——也就是把这些数写死
 * 的那台机器。选它当 1.0 不是因为它是标准，而是因为**仓里现存的所有静态预算
 * 都是在它上面拍的**：以它为基准，`cpuScale ≈ 1.0` 时新旧预算等价，改动可回退。
 *
 * 更新的机器会读到 < 43ms（`cpuScale < 1`），更老的 > 43ms。**`cpuScale` 只在
 * 首次运行（无实测历史）时用来放大预算，且只放大不缩小**（见 `budget()`）——
 * 缩小预算没有任何收益，只会多一种假红的可能。
 */
const REFERENCE_CPU_MS = 43

/** 单核探针的工作量。固定值，跨机器可比；改它就要同步改 `REFERENCE_CPU_MS`。 */
const PROBE_ITERATIONS = 6_000_000

/**
 * 每路并发的大致常驻内存（GB）。用于**内存钳**：核多但内存小的机器（如 8 核
 * 8GB 的轻薄本）开满并发会 swap，而 swap 比串行还慢——这是「并发度只看核数」
 * 最典型的翻车方式。
 *
 * 估值偏保守（宁可少开一路）：
 * - `rustc`/`cargo` 单进程峰值实测 0.3–1.2GB（debug 编译带 debuginfo 更肥）
 * - `node --test` 单进程 200–600MB（vitest worker 带 coverage 更高）
 */
const GB_PER_CPU_SLOT = 1.2
const GB_PER_IO_SLOT = 0.5

/**
 * 可用内存 = `max(freemem, 总量 × 这个比例)`。
 *
 * **不能用 `os.freemem()` 直接当可用内存**：Windows 上它只统计「完全空闲」的页，
 * 把文件缓存（standby list）全排除在外，而缓存是**可回收**的——一编译就会被让出来。
 * 本机实测 `freemem` 5GB / 总量 16GB，若按 5GB 算，`cpu` 只算出 3 路，
 * 而 `55-verify` 在同一台机器上并发 8 个 rustc 早已实测稳定。**低估的代价是白白
 * 丢掉一半并行度**，比高估更隐蔽（没有报错，只是慢）。
 *
 * 取 60% 是个折中：留 40% 给 OS、桌面、IDE 与不可回收的缓存。宁可保守，
 * 但不要保守到把并发砍半。
 */
const MEM_USABLE_RATIO = 0.6

/** 各类并发度的上限。理由见 `derive()` 里逐条注释。 */
const MAX_CPU_SLOTS = 16
const MAX_IO_SLOTS = 16
const MAX_E2E_SLOTS = 6
const MAX_PROCESS_SLOTS = 3

/** 校准历史保留多少轮（中位数窗口）。太少被一次异常拖走，太多记不住机器的变化。 */
const HISTORY_WINDOW = 5

/** 预算系数：实测中位数 × 这个倍数。见 `budget()`。 */
const BUDGET_FACTOR = 2.5
/** 预算相对静态基准的放大上限。防止「越超时 → 预算越大 → 更久才发现挂死」的正反馈。 */
const BUDGET_MAX_GROWTH = 6

// ── 探针 ────────────────────────────────────────────────────────────────

/**
 * 单核速度探针：固定工作量的整数混合运算，返回**最快一次**的毫秒数。
 *
 * 取最小值而不是平均值，是因为要测的是**能力**不是**当刻状态**：跑的时候机器上
 * 可能有别的东西在抢核（门禁自己也可能在跑），平均值会被污染，而最小值最接近
 * 「没有干扰时这核能多快」。预热一次是必要的——首次执行走解释器，会读成慢 3 倍。
 *
 * 用整数运算而不是 `Math`/字符串，是为了避开不同 V8 版本对浮点/SIMD 的优化差异：
 * 这里要的是「这核多快」，不是「这个 V8 多聪明」。
 */
function cpuProbe() {
  const work = (n) => {
    let x = 0
    for (let i = 0; i < n; i++) x = (x * 1103515245 + 12345) & 0x7fffffff
    return x
  }
  work(PROBE_ITERATIONS / 30) // 预热 JIT
  let best = Infinity
  for (let round = 0; round < 3; round++) {
    const t0 = process.hrtime.bigint()
    work(PROBE_ITERATIONS)
    const ms = Number(process.hrtime.bigint() - t0) / 1e6
    if (ms < best) best = ms
  }
  return best
}

/**
 * 启动时的 CPU 忙碌比（0..1）。**同一台机器不同时段也不同**——用户说「每天不同
 * 性能的电脑」，其中一半情况其实是同一台机器在别的时候（后台编译、杀软全盘扫描、
 * 视频会议）本来就忙。此时开满并发只会让所有任务一起变慢，并且把预算撑破。
 *
 * Windows 上 `os.loadavg()` 恒返回 `[0,0,0]`（POSIX 专有），所以只能靠
 * `os.cpus()` 的 `times` 两次差分。采样窗口 120ms：短到无感，长到能跨过一次
 * 调度周期。**探针本身会在采样前跑掉 40ms**，对结果的影响是「略微偏高」——
 * 方向安全（偏保守）。
 */
function loadSample() {
  return new Promise((resolve) => {
    const before = os.cpus().map((c) => ({ ...c.times }))
    setTimeout(() => {
      const after = os.cpus().map((c) => ({ ...c.times }))
      let total = 0
      let idle = 0
      for (let i = 0; i < before.length && i < after.length; i++) {
        const a = before[i]
        const b = after[i]
        total += (b.user - a.user) + (b.nice - a.nice) + (b.sys - a.sys) + (b.idle - a.idle) + (b.irq - a.irq)
        idle += b.idle - a.idle
      }
      resolve(total > 0 ? Math.min(1, Math.max(0, 1 - idle / total)) : 0)
    }, 120)
  })
}

// ── 派生：核数 / 内存 / 负载 → 各类并发度 ──────────────────────────────

const clamp = (n, lo, hi) => Math.min(hi, Math.max(lo, Math.floor(n)))

/**
 * 从原始读数派生并发度。**纯函数**（无 IO、无环境），所以可以被测试直接喂假读数
 * ——「8 核 8GB 该开几路」「32 核该开几路」这类问题不该靠换一台真机器来验。
 *
 * 三类任务的瓶颈不同，不能共用一个数：
 *
 * - **cpu**：`rustc` / `cargo` / `vitest` worker。吃满每个核，所以按**逻辑核 - 1**
 *   （留一个给门禁主进程做日志落盘与状态机）。上限 16：同一 `target/` 目录的
 *   cargo 命令本来就要抢 `.cargo-lock`，再多也只是排队；而验证程序那批虽是独立
 *   rustc，16 路已足够吃满任何消费级机器。
 * - **io**：审计脚本（`grep-audit` / `doc-link-audit` / 各种 `*.test.mjs`）。绝大部分
 *   时间在 `readdir` / `readFile` / `spawn node`，CPU 占用低，**超订有益**。
 *   所以按逻辑核给满，上限 16（再多就是给文件系统添堵）。
 * - **e2e**：上限 6 不是机器限制，是**用例结构**限制——理论最优
 *   `总工作量 / 最慢单例 ≈ 5.7`（见 `58-e2e.mjs` 与 `scripts/e2e-concurrency.mjs`）。
 *   机器再强也不该超过它，所以这里只做「低配往下收」，不做「高配往上加」。
 * - **process**：`windows-restart` 的 PowerShell 场景。瓶颈是 **Windows 进程创建**
 *   （`CreateProcess` 走全局锁），不是 CPU——本机实测并发 3 会假红、并发 4 更差，
 *   并发 2 才是拐点。拐点位置随机器变（强机单次冷启动更短），所以按 `cpuScale`
 *   与核数分档；历史上记录过假红的档位再封顶（见 `processCap()`）。
 */
export function derive({ logicalCores, memGB, freeMemGB, cpuMs, busy }) {
  // 见 `MEM_USABLE_RATIO`：freemem 在 Windows 上不计可回收缓存，直接用它会把并发砍半。
  const availableMemGB = Math.max(freeMemGB, memGB * MEM_USABLE_RATIO)

  // 负载钳：已经有别的东西在抢核时，按剩余空间给。**不做「-1 个核」的精细扣除**
  // ——门禁自己就是最大的负载源，精细扣除会在门禁跑到一半时自我降级，反而不稳。
  //
  // 阈值 50% / 75% 而不是 40% / 70%：开发机上开着 IDE + 浏览器 + 聊天工具，
  // 稳态负载经常就是 30–45%，那不是「机器忙」，是「正常在用」。用 40% 当门槛，
  // 会把**绝大多数本地运行**都判成繁忙而白降一档并发——自适应降级只该对
  // 「真的有重活」生效（另一个 cargo 编译、杀软全盘扫描、视频会议）。
  const loadFactor = busy > 0.75 ? 0.5 : busy > 0.5 ? 0.75 : 1

  const cpuByCores = logicalCores - 1
  const cpuByMem = availableMemGB / GB_PER_CPU_SLOT
  const cpu = clamp(Math.min(cpuByCores, cpuByMem) * loadFactor, 1, MAX_CPU_SLOTS)

  const ioByCores = logicalCores
  const ioByMem = availableMemGB / GB_PER_IO_SLOT
  const io = clamp(Math.min(ioByCores, ioByMem) * loadFactor, 1, MAX_IO_SLOTS)

  // e2e：**不能沿用 `cpu`**。e2e 用例是「一个 node 进程 + 一个小服务 + 一个端口」，
  // 单个占用远小于一次 cargo 编译，原实现 `min(6, 核数)` 在本机实测 6 路稳定。
  // 若跟着 `cpu`（按 rustc 的胃口算）走，本机会被压到 3 路，e2e 阶段直接慢一倍
  // ——那是把「按最重的任务定并发」这个错误又犯一遍。
  // 所以它有自己的内存口径（每路 ~0.8GB），且上限 6 是**用例结构**决定的
  // （理论最优 = 总工作量/最慢单例 ≈ 5.7，见 `scripts/e2e-concurrency.mjs`），
  // 不是机器决定的：机器再强也不该超过它，只做「低配往下收」。
  const e2e = clamp(Math.min(MAX_E2E_SLOTS, logicalCores - 1, availableMemGB / 0.8) * loadFactor, 1, MAX_E2E_SLOTS)

  // process：进程创建拐点。强机（探针快 + 核多）能到 3，中档 2，弱机 1。
  // 阈值 30ms / 8 核取自本机对照：本机 43ms / 8 核 → 2，与实测拐点一致。
  const process = clamp(cpuMs <= 30 && logicalCores >= 8 ? 3 : logicalCores >= 4 ? 2 : 1, 1, MAX_PROCESS_SLOTS)

  return { cpu, io, e2e, process, loadFactor, availableMemGB }
}

/** 分档，只用于**打印与人读**，不参与任何判定（避免「档位」变成第二个真相来源）。 */
export function tierOf({ logicalCores, memGB }) {
  if (logicalCores <= 4 || memGB <= 8) return 'low'
  if (logicalCores <= 8 || memGB <= 16) return 'mid'
  return 'high'
}

/**
 * 环境变量覆盖：`GATE_CPU_SLOTS` / `GATE_IO_SLOTS` / `GATE_E2E_SLOTS` / `GATE_PROCESS_SLOTS`。
 *
 * **为什么探测必须可覆盖**：探测拿的是 `os` 的报告，而它有三种已知的失真——
 *
 * 1. **容器 / cgroup**：`os.cpus()` 报告的是**宿主**的核数，实际 CPU 配额可能只有
 *    2 核。此时探测会开满宿主核数，然后在被限流的 cgroup 里互相拖。
 * 2. **CI runner**：`availableParallelism()` 尊重 affinity，某些 runner 上会低估。
 * 3. **共享机器**：跑门禁时别人也在跑，探测那一刻的负载不代表全程。
 *
 * 这三种情况没有一种能靠「更聪明的探测」解决——它们要的是**人知道机器实际给了多少**。
 * 所以探测给的是**起点**，不是终局；能覆盖，才敢让探测自动生效。
 *
 * 非法值（0 / 负数 / 非数）**忽略而不是报错**：覆盖是应急口，不该因为手滑打错
 * 就让整轮门禁起不来——那会把「想调并发」变成「门禁坏了」。
 */
export function applyOverrides(slots, env = process.env) {
  const pick = (key) => {
    const v = Number(env[key])
    return Number.isFinite(v) && v >= 1 ? Math.floor(v) : null
  }
  return {
    ...slots,
    cpu: pick('GATE_CPU_SLOTS') ?? slots.cpu,
    io: pick('GATE_IO_SLOTS') ?? slots.io,
    e2e: pick('GATE_E2E_SLOTS') ?? slots.e2e,
    process: pick('GATE_PROCESS_SLOTS') ?? slots.process,
  }
}

// ── 校准文件：上一轮实测 ────────────────────────────────────────────────

/**
 * 机器指纹：核数 / 内存 / 平台 / CPU 型号。
 *
 * **指纹不符就丢弃全部历史**，这是「不同电脑」这件事唯一的正确处置。若沿用
 * 另一台机器的耗时，就会在快机上把预算撑得过大（挂死发现得晚），或在慢机上
 * 把预算压得过小（假红）——两种都比没有历史更糟。
 *
 * 故意**不含** CPU 当前频率、可用内存等会漂的量：它们一变就把历史清了，
 * 于是永远攒不到 5 轮中位数。只认「换机器」这件大事。
 */
export function fingerprintOf({ platform, arch, logicalCores, memGB, cpuModel }) {
  return `${platform}-${arch}|${logicalCores}c|${Math.round(memGB)}g|${cpuModel}`
}

/** 读校准文件；不存在 / 损坏 / 指纹不符 一律当「没有历史」，不抛。 */
function readCalibration(fingerprint) {
  try {
    const raw = JSON.parse(fs.readFileSync(CALIBRATION_FILE, 'utf8'))
    if (raw?.version !== 1 || raw.fingerprint !== fingerprint) {
      return { version: 1, fingerprint, tasks: {}, stages: {}, process: {}, discarded: raw?.fingerprint ?? null }
    }
    return { version: 1, fingerprint, tasks: {}, stages: {}, process: {}, ...raw }
  } catch {
    return { version: 1, fingerprint, tasks: {}, stages: {}, process: {} }
  }
}

/** 取中位数（窗口内）。比均值抗一次异常，比 p90 省样本。 */
function median(xs) {
  if (!xs.length) return null
  const s = [...xs].sort((a, b) => a - b)
  const m = s.length >> 1
  return s.length % 2 ? s[m] : Math.round((s[m - 1] + s[m]) / 2)
}

// ── 惰性单例 ────────────────────────────────────────────────────────────

let cached = null

/**
 * 探测本机能力。**进程内只算一次**（170ms 不该被每个阶段各付一遍）。
 *
 * 返回的对象是**只读约定**（不 freeze，因为 `budget()` 要往里挂计数），字段：
 *
 *   fingerprint   机器指纹（换机即变）
 *   logicalCores  逻辑核（`availableParallelism`，尊重 affinity）
 *   memGB         物理内存总量
 *   freeMemGB     探测时的可用内存
 *   cpuMs         单核探针读数（越小越快）
 *   cpuScale      `cpuMs / 43`，1.0 = 基准机
 *   busy          启动时 CPU 忙碌比
 *   tier          'low' | 'mid' | 'high'（仅供人读）
 *   slots         { cpu, io, e2e, process }
 *   probeMs       探测本身花掉的毫秒（判据用）
 *   calibration   历史实测（`tasks` / `stages` / `process`）
 */
export async function machine() {
  if (cached) return cached

  const started = Date.now()
  const logicalCores = os.availableParallelism?.() ?? os.cpus().length
  const memGB = os.totalmem() / 2 ** 30
  const freeMemGB = os.freemem() / 2 ** 30
  const cpuModel = (os.cpus()[0]?.model ?? 'unknown').trim()

  const cpuMs = cpuProbe()
  const busy = await loadSample()

  const fingerprint = fingerprintOf({ platform: os.platform(), arch: os.arch(), logicalCores, memGB, cpuModel })
  const calibration = readCalibration(fingerprint)
  const derived = derive({ logicalCores, memGB, freeMemGB, cpuMs, busy })
  const slots = applyOverrides(derived)
  const overridden = ['cpu', 'io', 'e2e', 'process'].filter((k) => slots[k] !== derived[k])

  // 历史里记过假红的档位要封顶：机器参数说能开 3，但上次真开 3 就红过，就退回 2。
  // 这不是「猜」，是**记住上一次试的结果**——比任何静态规则都可靠。
  const falseRedAt = Array.isArray(calibration.process?.falseRedAt) ? calibration.process.falseRedAt : []
  if (falseRedAt.length) {
    const cap = Math.max(1, Math.min(...falseRedAt) - 1)
    slots.process = Math.min(slots.process, cap)
  }

  cached = {
    fingerprint,
    logicalCores,
    memGB,
    freeMemGB,
    cpuModel,
    cpuMs,
    cpuScale: cpuMs / REFERENCE_CPU_MS,
    busy,
    tier: tierOf({ logicalCores, memGB }),
    slots,
    derivedSlots: derived,
    overridden,
    probeMs: Date.now() - started,
    calibration,
    discardedFingerprint: calibration.discarded ?? null,
  }
  return cached
}

// ── 超时预算 ────────────────────────────────────────────────────────────

/**
 * 给某个任务算超时预算（毫秒）。`baseMs` 是**静态基准**——仓里原来写死的那个数。
 *
 * 规则：
 *
 * 1. 有历史（≥1 轮通过）：`中位数 × 2.5`，钳在 `[baseMs, baseMs × 6]`。
 *    下限取 `baseMs` 而不是历史值，是因为**预算只该因机器变慢而变大，不该因
 *    机器变快而缩小**：缩小唯一的效果是把「偶发慢」变成「偶发红」。
 * 2. 无历史：`baseMs × max(1, cpuScale)`。慢机首跑不至于一上来就假红。
 * 3. 上限 `baseMs × 6`：防止「超时 → 记更大的数 → 更久才发现挂死」的正反馈。
 *    一轮门禁最坏多等几分钟可以接受，等半小时不行。
 *
 * ⚠️ **预算不是判据**。它只决定「等多久算挂死」。改它不改变任何一条判定，
 * 只改变「挂死被发现得多晚」。这正是它允许自校准的原因——判据值不能自校准。
 */
export function budget(m, key, baseMs) {
  const hist = m.calibration.tasks?.[key]?.ms ?? []
  const measured = median(hist.slice(-HISTORY_WINDOW))
  const floor = baseMs
  const ceil = baseMs * BUDGET_MAX_GROWTH
  if (measured != null) return clamp(Math.round(measured * BUDGET_FACTOR), floor, ceil)
  return clamp(Math.round(baseMs * Math.max(1, m.cpuScale)), floor, ceil)
}

/**
 * 把本轮的实测耗时并进校准文件（**在 `report()` 之前调用一次**）。
 *
 * 只记 `ms > 0` 的项。跳过的（`skipped`）没有耗时，`run` 型任务可能是 0ms，
 * 都不进历史——它们会把中位数拉低，而中位数是用来定预算的。
 *
 * 写失败**不抛**：门禁的结论不该因为「记不下这次多快」而改变（同 `_logs.mjs`
 * 里 `pruneOldRuns` 的处置）。
 */
export function recordRun(m, results, { stageWall = {} } = {}) {
  if (!results?.length) return
  const next = {
    version: 1,
    fingerprint: m.fingerprint,
    updatedAt: new Date().toISOString(),
    machine: {
      logicalCores: m.logicalCores,
      memGB: Math.round(m.memGB * 10) / 10,
      cpuModel: m.cpuModel,
      cpuMs: Math.round(m.cpuMs),
      busy: Math.round(m.busy * 100) / 100,
    },
    tasks: { ...(m.calibration.tasks ?? {}) },
    stages: { ...(m.calibration.stages ?? {}) },
    process: { ...(m.calibration.process ?? {}) },
  }

  for (const r of results) {
    if (!r?.label || !(r.ms > 0)) continue
    // 失败项也记：`超时终止` 的那条 ms 恰好是「至少要这么久」，丢掉它下次还会超时。
    const key = `${r.stage}::${r.label}`
    const prev = Array.isArray(next.tasks[key]?.ms) ? next.tasks[key].ms : []
    next.tasks[key] = { ms: [...prev, Math.round(r.ms)].slice(-HISTORY_WINDOW) }
  }
  for (const [id, ms] of Object.entries(stageWall)) {
    if (!(ms > 0)) continue
    const prev = Array.isArray(next.stages[id]?.ms) ? next.stages[id].ms : []
    next.stages[id] = { ms: [...prev, Math.round(ms)].slice(-HISTORY_WINDOW) }
  }

  try {
    fs.mkdirSync(path.dirname(CALIBRATION_FILE), { recursive: true })
    const tmp = `${CALIBRATION_FILE}.tmp`
    fs.writeFileSync(tmp, JSON.stringify(next, null, 2))
    fs.renameSync(tmp, CALIBRATION_FILE)
  } catch {
    /* 见函数头：记不下不影响结论 */
  }
}

/**
 * 记一次「这个并发档位出现假红」。下次 `machine()` 会据此封顶。
 *
 * **只由测试自己调用**（它知道失败信息长什么样），门禁框架不猜——把「超时」
 * 自动当成「假红」会把真 bug 一起吞掉。
 */
export function recordFalseRed(m, processConcurrency) {
  const prev = Array.isArray(m.calibration.process?.falseRedAt) ? m.calibration.process.falseRedAt : []
  if (prev.includes(processConcurrency)) return
  m.calibration.process = { ...(m.calibration.process ?? {}), falseRedAt: [...prev, processConcurrency] }
  try {
    const onDisk = JSON.parse(fs.readFileSync(CALIBRATION_FILE, 'utf8'))
    if (onDisk?.fingerprint !== m.fingerprint) return
    onDisk.process = { ...(onDisk.process ?? {}), falseRedAt: m.calibration.process.falseRedAt }
    fs.writeFileSync(CALIBRATION_FILE, JSON.stringify(onDisk, null, 2))
  } catch {
    /* 见 `recordRun` */
  }
}

/** 上一轮实测的耗时（毫秒）；没有历史返回 `null`。LPT 排序用。 */
export function measuredMs(m, stageId, label) {
  const ms = m.calibration.tasks?.[`${stageId}::${label}`]?.ms
  return Array.isArray(ms) && ms.length ? median(ms.slice(-HISTORY_WINDOW)) : null
}

// ── 打印 ────────────────────────────────────────────────────────────────

/**
 * 一行说明「它认为这台机器什么水平、因此选了多大的并发」。
 *
 * **必须打印**：自适应机制最大的风险不是算错，而是**没人知道它在算**。打印之后，
 * 用户在慢机上看到 `并发 2` 就知道门禁确实降级了（而不是「还是那么慢，优化没用」），
 * 在快机上看到 `并发 12` 就知道升级没白升。判据 = `machine.test.mjs`「档案必须
 * 能打印成人读的一行」。
 *
 * 刻意不打印 `fingerprint` 全文（一长串机器型号，刷屏且没人看）。
 */
export function describe(m) {
  const pct = Math.round(m.busy * 100)
  const parts = [
    `${m.logicalCores} 逻辑核`,
    `${m.memGB.toFixed(1)}GB`,
    `单核 ${m.cpuMs.toFixed(0)}ms（${m.cpuScale.toFixed(2)}× 基准）`,
  ]
  if (pct >= 10) parts.push(`启动负载 ${pct}%`)
  const manual = m.overridden?.length ? `（手工覆盖 ${m.overridden.join('/')}）` : ''
  return `${parts.join(' / ')} ⇒ 并发 cpu ${m.slots.cpu} · io ${m.slots.io} · e2e ${m.slots.e2e} · process ${m.slots.process}${manual}`
}
