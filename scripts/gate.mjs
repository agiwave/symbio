#!/usr/bin/env node
/**
 * gate — 改动后的**统一检查入口**
 *
 * 用途：把「改动后必跑」的那一串命令与它们的坑**收进代码**，而不是记在
 * 文档 / 记忆里靠人背。跑法只有一条：
 *
 *   node scripts/gate.mjs              # 全量（后端 → 前端 → 审计 → MSRV → 事实文件）
 *   node scripts/gate.mjs --only=frontend
 *   node scripts/gate.mjs --only=msrv   # 用 `rust-version` 声明的最低工具链真跑一次 cargo check
 *   node scripts/gate.mjs --skip=backend
 *   node scripts/gate.mjs --fix        # 先自动格式化 / 重生成，再检查
 *   node scripts/gate.mjs --ci         # CI 对齐：cargo test --workspace（含集成测试）
 *   node scripts/gate.mjs --profile=release   # 额外跑 cargo build（本地默认不跑）
 *
 * ## 与 .github/workflows/ci.yml 的关系
 *
 * CI 目前仍是手写命令（backend / frontend 两个 job 并行）。本脚本的 `--ci` 与
 * `--profile=` 就是为对齐它准备的，后续可把 CI 改成
 * `node scripts/gate.mjs --only=backend --ci --profile=release` 与
 * `node scripts/gate.mjs --only=frontend`，让「检查什么」只有一处真相。
 * ⚠️ 改 CI 前请在 CI 环境验证过再合并（本地无法证明 Actions 上跑得通）。
 *
 * ## 阶段与顺序（顺序有语义，不要随意调）
 *
 *   1. backend   cargo check --tests / test --lib / clippy / fmt --check（在 `symbio/`）
 *   2. frontend  vue-tsc --noEmit / vitest run（在 `tauri/`）
 *   3. docs      grep-audit / mechanism-audit / style-audit / doc-link-audit
 *                / test-layout-audit / dead-code-audit（均判定型）
 *                + schema-audit（报告型，仅防崩溃）
 *                每个判定型守卫都先跑**自己的回归测试**（证明它能变红）
 *   4. msrv      用 `rust-version` 声明的**最低**工具链跑 cargo check（symbio/ 与 cli/）。
 *                本机没装该工具链时**跳过并提示**（要真跑需 `rustup toolchain install`）；
 *                CI 里装了 ⇒ 一定跑，故「MSRV 写了但没人验证」这条不再成立。
 *   5. facts     gen-current-facts --check（**必须最后**：它由代码生成，
 *                前面任何自动修复都可能改动代码）
 *
 * ## 封装进去的坑（改本脚本前请先读这些，它们都是踩出来的）
 *
 * - **不接管道**：cargo / git 的输出一旦接 `| tail` 就缓冲到 EOF ⇒ 全程零输出，
 *   与卡死无法区分；且管道会**吞掉错误** ⇒ 失败的命令看起来成功。本脚本用
 *   spawn 实时读流：既逐行转发（有进度），又把全文落到日志文件（可回溯），
 *   判定**只信退出码**。
 * - **沙箱误报**：clippy 之后常见的 `[sandbox] target/… 拒绝` / `os error 5`
 *   是沙箱拦截，**不是失败**——看退出码，不要grep 文本。
 * - **rustfmt 只用 `cargo fmt`**：工具链锁 1.93.1（rustfmt 1.8.0），裸 `rustfmt`
 *   走 rustup default ⇒ 格式漂移。CI 与本脚本都用 `cargo fmt --all -- --check`。
 * - **vitest 不能后台跑**：本脚本前台跑 + 超时 kill。总结和通过数只是附加检查，
 *   不能覆盖非零退出码、超时或信号终止；缓存 / 并发异常也必须排查后重跑。
 * - **MSRV 阶段要单独的工具链**：`rust-version` 声明的是 1.91，而本机与 CI 默认锁
 *   1.93.1（`rust-toolchain.toml`）。故该阶段用 `RUSTUP_TOOLCHAIN` **覆盖**工具链文件
 *   （环境变量优先级高于 `rust-toolchain.toml`）换编译器跑。两个附带约束：
 *   ① `RUSTUP_TOOLCHAIN` **只对 rustup 装的 cargo 生效**（发行版包 / homebrew 的 cargo
 *   静默忽略它）⇒ 该阶段先探 `rustc --version`，版本 ≠ 声明值就跳过，绝不拿默认编译器
 *   冒充 MSRV 结论；② 换编译器会让 target 缓存整体失效 ⇒ 用独立 `CARGO_TARGET_DIR`
 *   （`.workbuddy-ai/msrv-target/`），否则每跑一次门禁就触发一次整树重编。
 *   整段逻辑只看版本号与退出码，不依赖 OS / shell，三端一致。
 * - **基线只增不减**：`cargo test --lib` / vitest 的通过数低于基线即失败。
 *   高于基线时提示更新本文件顶部的 `BASELINE`——那是刻意要人看一眼的地方。
 *
 * 退出码：0 = 全通过；1 = 有阶段失败。
 *
 * 平台无关（Windows / macOS / Linux 通用）：不依赖 bash / npx，子进程一律
 * `shell: false` + 参数数组；node 侧脚本用 `process.execPath` 启动。
 */

import fs from 'node:fs'
import path from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')
const backendDir = path.join(repoRoot, 'symbio')
const frontendDir = path.join(repoRoot, 'tauri')
const logDir = path.join(repoRoot, '.workbuddy-ai', 'gate-logs')
/**
 * MSRV 阶段专用的 target 目录。
 *
 * 为什么必须隔离：MSRV（1.91）与默认工具链（1.93.1）是**两个编译器**，而 cargo 的
 * fingerprint 含 rustc 版本 —— 共用 target 会让两边互相作废，本地每跑一次门禁就等于
 * 触发一次整树重编（切回去再重编一次）。独立目录后：MSRV 检查不影响日常构建缓存，
 * 且它自己第二次起是增量的。路径在 `.workbuddy-ai/` 下（已 gitignore）。
 */
const msrvTargetDir = path.join(repoRoot, '.workbuddy-ai', 'msrv-target')

/**
 * 通过数基线（**只增不减**；跑高了请更新这里并说明理由；**跑低了要说明理由**）
 *
 * 639：v1→v2 迁移 + v2 manifest 校验 +6（migrate 2 / manifest 4）。
 *
 * 638：删除 OAB v1 装配实现后 **-18**——删的是 v1 的用例本身
 * （`core/spec/*` 协议核心、`host/prompt.rs` 人格片段、`host/capability.rs`
 * 身份工具、`BundleStore` 的 prompt/skill/mcp 分类扫描），补的是 v2 主链路
 * 「不合规目录拒绝接入且写明双侧版本」+1。v1 的装配语义已不存在，它的测试
 * 不该留下——留着就是在给一段已删的实现作证。
 */
const BASELINE = {
  // 657 → 660：嵌入 2 个真实推理回归测试 + `codebase_search` 注册护栏测试
  // 660 → 666：节点状态流 S20——会话运行态投影（`SessionRuntime` /
  //   `session_node` / `session_change`）与 `MessageStatus` 词表对齐的回归测试
  // 666 → 668：S20.1——中止批次必然收口（无节点停在 `Streaming`）、
  //   未执行的终态是 `Completed` 而非 `Failed`
  // 668 → 671：S20.2——级联删除只发一条 `truncated`（不是 N 条 `deleted`）、
  //   删末尾一条走同一语义、目标不存在时一条变更都不发
  // 671 → 687：索引落盘 + 按 mtime 增量重建——增量判据（未变零嵌入 / 只重嵌改动 /
  //   删文件不触发嵌入 / 新增只嵌自己 / rebuild 全量）、二进制格式往返与四种损坏输入
  //   （魔数 / 版本 / 截断 / 荒谬长度）不 panic、生成物闸门
  // 687 → 700：会话前后端消息节点一致性——中止收口 `converge_inflight`（在途集合
  //   唯一定义 / 存储与在途两个数据源 / 幂等）、`wait_tool_abort` 的四个出口
  //   （已置位 / abort 帧 / 取消令牌 / 通道关闭且忽略无关帧）、中止出口 `aborted`
  //   与 `completed` 结局可区分、会话叶子读含在途、resume 中止后父节点定稿
  // 700 → 702：压缩期「阶段」`phase`——只在运行中投影到节点属性、非运行分支一律
  //   丢掉（`from_state` 是唯一入口，两条规则各一例）
  // 702 → 704：修复压缩后消息顺序倒挂——`assign_seq` 必须让 `seq` 沿数组单调不减
  //   （快照先于保留区），以及"正常路径不得触发重排"这条反面保险
  // 704 → 704：压缩改走消息流节点——新增 `compression_request` ×2 + `flatten` 跳过
  //   压缩节点 ×1，与移除的会话级 `phase` 后端 ×2 相抵，净零
  // 704 → 712：会话 id 改短 GUID（8 位十六进制）×2 + 自动压缩熔断状态机 ×3 +
  //   压缩失败原因可诊断（kind / message / Display）×3
  // 712 → 714：中止收口终态化——`abort_terminal_of` 让根 Turn 一律定稿 Aborted、
  //   `converge_inflight` 与 `persist_failure` 的终态不再混用 Completed（均经回退验证确认会红）
  // 714 → 721：会话级写入并入 VDFS——`Session::merge_metadata_object` 的 5 例
  //   （浅合并保留未提到的键 / 只给 title 不动 metadata / 非对象整体替换 /
  //   空 title 原样写入 / 空对象是 no-op）+ 2 例跨路径断言
  //   （`session/update` 与 `vdfs/write` 产出**逐字相同**的 metadata、
  //   `session/clear` 落到默认分支）。两条均经回退验证确认会红：前者注入
  //   "invoke_update 丢 title" 即红，后者注入"把 clear 路由加回来"即红。
  // 721 → 732：会话消息的三条旧路由迁 VDFS——`vdfs_provider.test.rs` 新增 13 例
  //   （改写：只覆盖提供的字段 / 不带 id / id 冲突 / `create` 被拒 / 目标不存在；
  //   截断：区间 + **一条** `truncated` + 目标不存在不发变更；清空：会话本体保留 +
  //   目录上 `deleted`；`delete` 对区段被拒；动作不认识 / 放错地址），
  //   `handlers.test.rs` 删掉随 `invoke_delete_message` 退役的 3 例（契约已搬到
  //   provider 测试，见该文件头对照表）并新增 `migrated_session_routes_stay_retired`
  //   ×1（5 条退役路由不得被加回来）。净 +11。
  //   经回退验证：去掉消息 `write` 的 id 补齐逻辑，`message_write_accepts_a_patch_without_id`
  //   即红——该例正是首轮跑测试抓出的真实缺陷（`ChatMessage::id` 必填让「字段子集」不成立）。
  rustTests: 732,
  vitestFiles: 24,
  // 156 → 160：S20——`sessionRouteOf` 地址分派、节点载荷就地收敛（零回读）、
  //   状态迁移驱动的提示音、`failed` 作为独立会话状态
  // 160 → 164：工具调用运行态——「运行中」标签 + 动效点 + 已运行时长、
  //   `waiting_user_action` 的「待确认」标签、终态不给标签
  // 164 → 172：S20.2——`removeFrom` 的级联范围与边界（删末尾 / 锚点缺失 /
  //   缺 seq 的旧数据）、`deleteMessage` 的失败回滚与权威列表对齐、
  //   `truncated` 与 `deleted` 两种删除语义不互相污染
  // 172 → 177：历史水合改为**合并**语义——快照里没有的在途节点保留（正在跑的那一轮
  //   不消失）、保留节点重排到历史之后、终态本地节点被丢弃、快照整条覆盖同 id 节点
  // 177 → 181：会话节点阶段 `phase`——运行中采信 / 非运行不采信 / 未知阶段当作
  //   常规处理 / 压缩结束回到空
  // 181 → 177：压缩改走消息流节点（新增 `Compression` 类型 + 请求形态改造），
  //   会话级 `phase` 机制随之移除（4 例前端 `phase` 测试删除，2 例后端 `phase`
  //   测试删除）；净增 `compression_request` ×2 + `flatten_chat_messages` 跳过
  //   压缩节点 ×1（均经回退验证确认会红）
  // 177 → 179：中止收口终态化——`MessageNode` 对 `aborted` 终态的渲染：组级交代条
  //   + 重试入口；`completed` 终态的"无角标无重试"反向断言（均经回退验证确认会红）
  // 179 → 308（文件 19 → 24）：消息域机制化改造。
  //   · `MessageNode` 从 1549 行拆成「facets → 渲染器标识 → 组件」的分派器，
  //     8 个子渲染器 + 装配点各自补测（新增 messageRenderers.spec.ts）；
  //   · 消息级业务规则（重试 / 补参 / 重试路由）抽成纯函数后可直接单测；
  //   · 会话 store 拆出 `sessionTranscript` / `sessionLive` 两个纯模块，
  //     原属 store 的用例随之可脱离 Pinia 单测（新增两个 spec）；
  //   · 新增 `messageContent.spec.ts`——含**类名契约**断言（`.json-*` 规则必须
  //     存在于**全局**样式表）：scoped 编译会补 `[data-v-*]`，而 `v-html` 注入的
  //     元素拿不到它 ⇒ 着色静默失效，这是回归防线；
  //   · 本次新增 4 个 spec：`messageRenderers` / `messageContent` /
  //     `sessionTranscript` / `sessionLive`（19 → 24 里其余增量来自同一轮改造
  //     中更早完成的拆分，已计入上一版基线口径）。
  //   注：`scripts/mechanism-audit.test.mjs` 走 `node --test`（阶段 3），不计入此处。
  // 308 → 313：会话级写入并入 VDFS——`session.spec.ts` 新增 5 例，锁定
  //   删除走 `vdfs/delete(.vdfs/session/<id>)`、metadata 走
  //   `vdfs/write(.vdfs/session/<id>)`，并断言**地址**（地址拼错在真实环境里
  //   表现为删错会话，是灾难级）。经回退验证：改成挂载根地址 + 丢 title 两项皆红。
  // 313 → 321：会话消息的三条旧路由迁 VDFS——`session.spec.ts` 新增 8 例：
  //   `updateMessage` 写**单条消息**地址（含载荷形状与「地址取自 message.id」）、
  //   `deleteMessage` 走 `action("truncate")` 且回执映射为 `deleted_ids`
  //   （含非字符串过滤 / `data` 缺失回落空列表）、`clearMessages` 走
  //   `action("clear")` 且地址是**消息列表目录**。三组都断言**地址**——
  //   少一层就清到会话本体，是灾难级。
  vitestTests: 321,
}

/** vitest 前台最长等待（毫秒）——超时即 kill 并失败 */
const VITEST_TIMEOUT_MS = 180_000

// ── 参数 ───────────────────────────────────────────────────────────────
const argv = process.argv.slice(2)
const FIX = argv.includes('--fix')
/** CI 对齐模式：`cargo test --workspace`（含集成测试）而非 `--lib`，且不做基线比较 */
const CI = argv.includes('--ci')
/** `cargo build --profile <p>`：给了就跑构建（本地默认不跑，省几分钟） */
const profileArg = argv.find((a) => a.startsWith('--profile='))
const profile = profileArg ? profileArg.slice(10).trim() : null
const onlyArg = argv.find((a) => a.startsWith('--only='))
const skipArg = argv.find((a) => a.startsWith('--skip='))
const only = onlyArg ? onlyArg.slice(7).split(',').map((s) => s.trim()) : null
const skip = skipArg ? skipArg.slice(7).split(',').map((s) => s.trim()) : []

// ── 输出 ───────────────────────────────────────────────────────────────
const useColor = Boolean(process.stdout.isTTY) && process.env.NO_COLOR === undefined
const paint = (code) => (s) => (useColor ? `\x1b[${code}m${s}\x1b[0m` : s)
const red = paint('0;31')
const green = paint('0;32')
const yellow = paint('0;33')
const dim = paint('2')
const bold = paint('1')

const enabled = (id) => (only ? only.includes(id) : true) && !skip.includes(id)

/**
 * 阶段顺序 —— **必须与主流程里的调用顺序一致**（`enabled()` 的 id 也从这里取）。
 * 序号由它推导，免得手写「3/4」之后插了新阶段忘了改数字。
 */
const STAGE_ORDER = ['backend', 'frontend', 'docs', 'msrv', 'facts']

function stageHeader(id, title) {
  const n = STAGE_ORDER.indexOf(id) + 1
  console.log()
  console.log(bold(`── 阶段 ${n}/${STAGE_ORDER.length} · ${title} ${'─'.repeat(Math.max(0, 44 - title.length))}`))
}

function fmtDuration(ms) {
  const s = Math.round(ms / 1000)
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`
}

/** cargo 的进度噪音（刷屏且无信息量） */
const NOISE = /^(?:\s*$|.*\r$|\s*(Compiling|Checking|Downloading|Downloaded|Updating|Locking|Adding|Removing|Finished|Blocking|Waiting|Fresh|Documenting|Building)\b)/

/**
 * 跑一条命令：**实时逐行转发 + 全文落日志 + 只信退出码**。
 *
 * @param {object} o
 * @param {string} o.label      控制台显示名
 * @param {string} o.cmd        可执行文件
 * @param {string[]} o.args
 * @param {string} [o.cwd]
 * @param {number} [o.timeoutMs]
 * @param {'filtered'|'all'|'none'} [o.echo] 控制台转发策略：`filtered` = 过滤 cargo
 *        进度噪音后转发（默认，长任务用它），`all` = 全转发（短任务用它），
 *        `none` = 不转发（只看结果）。**无论哪种，全文都会落日志。**
 * @param {Record<string,string>} [o.env] 追加/覆盖的环境变量（MSRV 阶段用它换工具链）
 * @returns {Promise<{ok: boolean, code: number|null, signal: string|null, output: string, timedOut: boolean}>}
 */
function run(o) {
  const { label, cmd, args, cwd = repoRoot, timeoutMs = 0, echo = 'filtered', env } = o
  process.stdout.write(`  ▸ ${label} … `)

  return new Promise((resolve) => {
    const started = Date.now()
    let output = ''
    let timedOut = false

    const child = spawn(cmd, args, { cwd, shell: false, env: { ...process.env, ...env } })
    let timer = null
    if (timeoutMs > 0) {
      timer = setTimeout(() => {
        timedOut = true
        child.kill('SIGKILL')
      }, timeoutMs)
    }

    const consume = (chunk) => {
      const text = chunk.toString()
      output += text
      if (echo !== 'none') {
        for (const raw of text.split('\n')) {
          const line = raw.replace(/\r/g, '').trimEnd()
          if (!line) continue
          if (echo === 'filtered' && NOISE.test(line)) continue
          console.log(`      ${dim(line)}`)
        }
      }
    }
    child.stdout?.on('data', consume)
    child.stderr?.on('data', consume)

    child.on('error', (err) => {
      if (timer) clearTimeout(timer)
      console.log(red('启动失败'))
      console.log(red(`      ${err.message}`))
      resolve({ ok: false, code: null, signal: null, output: output + err.message, timedOut })
    })

    child.on('close', (code, signal) => {
      if (timer) clearTimeout(timer)
      const ok = code === 0 && signal === null && !timedOut
      const ms = Date.now() - started
      fs.mkdirSync(logDir, { recursive: true })
      fs.writeFileSync(path.join(logDir, `${slug(label)}.log`), output, 'utf8')
      if (timedOut) {
        console.log(yellow(`超时（已 kill，${fmtDuration(ms)}）`))
      } else if (ok) {
        console.log(green(`ok (${fmtDuration(ms)})`))
      } else {
        console.log(red(`失败 (exit=${code}${signal ? `, ${signal}` : ''}, ${fmtDuration(ms)})`))
      }
      resolve({ ok, code, signal, output, timedOut })
    })
  })
}

function slug(s) {
  return s.replace(/[^a-zA-Z0-9]+/g, '-').replace(/^-|-$/g, '').toLowerCase() || 'step'
}

/** ANSI 转义序列（子进程带颜色输出时，正则会被转义码打断，必须先剥离） */
const ANSI_RE = /\x1b\[[0-9;]*[A-Za-z]/g
const stripAnsi = (s) => s.replace(ANSI_RE, '')

/** 从输出里抓一个整数（第一个捕获组） */
function grabInt(output, re) {
  const m = stripAnsi(output).match(re)
  return m ? Number(m[1]) : null
}

/**
 * 把**所有**匹配的捕获组相加。
 *
 * `cargo test --workspace` 会为每个测试目标各打一行 `test result:`，
 * 只抓第一行会拿到某个小目标的数字（甚至是 0），必须求和才是有意义的通过数。
 */
function sumInt(output, re) {
  const text = stripAnsi(output)
  let total = 0
  let found = false
  for (const m of text.matchAll(new RegExp(re.source, re.flags.includes('g') ? re.flags : `${re.flags}g`))) {
    total += Number(m[1])
    found = true
  }
  return found ? total : null
}

// ── 阶段 ───────────────────────────────────────────────────────────────
const results = []
function record(stage, label, pass, note) {
  results.push({ stage, label, pass, note })
}

async function stageBackend() {
  stageHeader('backend', '后端（cargo）')
  if (!fs.existsSync(path.join(backendDir, 'Cargo.toml'))) {
    console.log(yellow(`  跳过：未找到 ${backendDir}/Cargo.toml`))
    return
  }
  if (FIX) {
    const f = await run({ label: 'cargo fmt --all（--fix）', cmd: 'cargo', args: ['fmt', '--all'], cwd: backendDir })
    record('backend', 'cargo fmt --all（--fix）', f.ok)
  }

  const check = await run({
    label: 'cargo check --tests',
    cmd: 'cargo',
    args: ['check', '--tests'],
    cwd: backendDir,
  })
  record('backend', 'cargo check --tests', check.ok)

  const testArgs = CI ? ['test', '--workspace'] : ['test', '--lib']
  const test = await run({
    label: `cargo ${testArgs.join(' ')}`,
    cmd: 'cargo',
    args: testArgs,
    cwd: backendDir,
  })
  const passed = grabInt(test.output, /test result: ok\. (\d+) passed/)
  if (CI) {
    // CI 跑全量（含集成测试）：每个测试目标各打一行 ⇒ 求和；通过数与 `--lib`
    // 基线不是一回事，故不比基线，只信退出码（数字仅作信息展示）
    const total = sumInt(test.output, /test result: ok\. (\d+) passed/)
    if (total !== null) console.log(dim(`      ${total} passed（--workspace 全量；只信退出码）`))
    record('backend', `cargo ${testArgs.join(' ')}`, test.ok)
  } else if (passed === null) {
    record('backend', 'cargo test --lib', test.ok, '未能解析通过数')
  } else if (passed < BASELINE.rustTests) {
    record('backend', 'cargo test --lib', false, `通过数 ${passed} < 基线 ${BASELINE.rustTests}（有测试被删或失败）`)
  } else {
    if (passed > BASELINE.rustTests) {
      console.log(yellow(`      ⚠ 通过数 ${passed} > 基线 ${BASELINE.rustTests}：请更新 scripts/gate.mjs 的 BASELINE.rustTests`))
    } else {
      console.log(dim(`      ${passed} passed（基线 ${BASELINE.rustTests}）`))
    }
    record('backend', 'cargo test --lib', test.ok, passed > BASELINE.rustTests ? `通过数 ${passed}（基线待更新）` : '')
  }

  const clippy = await run({
    label: 'cargo clippy --all-targets -- -D warnings',
    cmd: 'cargo',
    args: ['clippy', '--all-targets', '--', '-D', 'warnings'],
    cwd: backendDir,
  })
  if (!clippy.ok && /\[sandbox\]|os error 5/.test(clippy.output)) {
    console.log(yellow('      ⚠ 输出含沙箱拦截字样——那是环境限制，不是 lint 失败；以退出码为准'))
  }
  record('backend', 'cargo clippy', clippy.ok)

  const fmt = await run({
    label: 'cargo fmt --all -- --check',
    cmd: 'cargo',
    args: ['fmt', '--all', '--', '--check'],
    cwd: backendDir,
  })
  if (!fmt.ok) console.log(yellow('      ↳ 格式不符：跑 `node scripts/gate.mjs --fix` 自动格式化'))
  record('backend', 'cargo fmt --check', fmt.ok)

  if (profile) {
    const build = await run({
      label: `cargo build --profile ${profile}`,
      cmd: 'cargo',
      args: ['build', '--profile', profile],
      cwd: backendDir,
    })
    record('backend', `cargo build --profile ${profile}`, build.ok)
  }

  // `cli/` 是**独立 workspace**（仓库根没有 Cargo.toml）：存在就一并检查，
  // 免得手工只跑 `symbio/` 而漏掉它。
  const cliDir = path.join(repoRoot, 'cli')
  if (fs.existsSync(path.join(cliDir, 'Cargo.toml'))) {
    if (FIX) {
      await run({ label: 'cli: cargo fmt --all（--fix）', cmd: 'cargo', args: ['fmt', '--all'], cwd: cliDir })
    }
    for (const [label, args] of [
      ['cli: cargo check --tests', ['check', '--tests']],
      ['cli: cargo clippy --all-targets -- -D warnings', ['clippy', '--all-targets', '--', '-D', 'warnings']],
      ['cli: cargo fmt --all -- --check', ['fmt', '--all', '--', '--check']],
    ]) {
      const r = await run({ label, cmd: 'cargo', args, cwd: cliDir })
      record('backend', label, r.ok)
    }
  }
}

async function stageFrontend() {
  stageHeader('frontend', '前端（vue-tsc / vitest）')
  const pkg = path.join(frontendDir, 'package.json')
  if (!fs.existsSync(pkg)) {
    console.log(yellow(`  跳过：未找到 ${frontendDir}/package.json`))
    return
  }

  const tsc = await run({
    label: 'vue-tsc --noEmit',
    cmd: process.execPath,
    args: [path.join(frontendDir, 'node_modules', 'vue-tsc', 'bin', 'vue-tsc.js'), '--noEmit', '-p', 'tsconfig.json'],
    cwd: frontendDir,
  })
  record('frontend', 'vue-tsc --noEmit', tsc.ok)

  const vitest = await run({
    label: 'vitest run',
    cmd: process.execPath,
    args: [path.join(frontendDir, 'node_modules', 'vitest', 'vitest.mjs'), 'run'],
    cwd: frontendDir,
    timeoutMs: VITEST_TIMEOUT_MS,
  })
  const files = grabInt(vitest.output, /Test Files\s+(\d+) passed/)
  const tests = grabInt(vitest.output, /Tests\s+(\d+) passed/)
  const enough =
    files !== null && tests !== null && files >= BASELINE.vitestFiles && tests >= BASELINE.vitestTests

  if (!vitest.ok) {
    record('frontend', 'vitest run', false, vitest.timedOut ? '超时终止' : `exit=${vitest.code}, signal=${vitest.signal}`)
  } else if (!enough) {
    record('frontend', 'vitest run', false, `文件/用例数未达基线或无法解析：${files}/${tests}（基线 ${BASELINE.vitestFiles}/${BASELINE.vitestTests}）`)
  } else {
    console.log(dim(`      ${files} 文件 / ${tests} 用例（基线 ${BASELINE.vitestFiles}/${BASELINE.vitestTests}）`))
    record('frontend', 'vitest run', true)
  }
}

async function stageDocs() {
  stageHeader('docs', '静态审计')
  // 判定型守卫的**回归测试**必须先跑：一个只会亮绿灯的守卫等于没有守卫，
  // 而它腐烂的方式恰恰是「规则写错了所以永远不命中」——只有注入真实违规
  // 并断言脚本变红，才能把「通过」和「没在工作」区分开。
  for (const name of ['grep-audit', 'mechanism-audit']) {
    const t = await run({
      label: `${name} 回归测试`,
      cmd: process.execPath,
      args: ['--test', path.join(scriptDir, `${name}.test.mjs`)],
      cwd: repoRoot,
    })
    record('docs', `${name} 回归测试`, t.ok)
  }
  // 判定型：有发现即以非零退出码失败。
  for (const name of [
    'grep-audit',
    'mechanism-audit',
    'style-audit',
    'doc-link-audit',
    'test-layout-audit',
    'dead-code-audit',
  ]) {
    const r = await run({
      label: `scripts/${name}.mjs`,
      cmd: process.execPath,
      args: [path.join(scriptDir, `${name}.mjs`)],
      cwd: repoRoot,
      echo: 'all',
    })
    record('docs', `${name}`, r.ok)
  }
  // 报告型：schema-audit 只出「可删 / 可下放」候选清单，退出码恒为 0（判定需人工
  // grep 复核）。故**只有它崩溃才会让门禁红**——这正是防它腐烂的机制。
  // 报告正文长且含人工判断项，走日志不刷屏。
  const schemaAudit = await run({
    label: 'scripts/schema-audit.mjs（报告型）',
    cmd: process.execPath,
    args: [path.join(scriptDir, 'schema-audit.mjs')],
    cwd: repoRoot,
    echo: 'none',
  })
  record('docs', 'schema-audit（仅防崩溃）', schemaAudit.ok)
  console.log(dim('      schema-audit / dead-code-audit 报告正文见 .workbuddy-ai/gate-logs/'))
  console.log(dim('      doc-link-audit 已豁免 docs/archive/（归档记录当时形态，改写即篡改历史）'))
}

/**
 * 从 `Cargo.toml` 读 `rust-version` —— MSRV 的**唯一真相源**（`clippy.toml` 的 `msrv`
 * 只是给 clippy 看的副本，取值必须与它一致）。读不到返回 null。
 *
 * 返回值补全成 `x.y.z`：rustup 的工具链名带 patch（`1.91.0-x86_64-…`），而
 * `rust-version` 允许只写 `1.91`，两者对齐才能命中同一个已安装的工具链。
 */
function readMsrv(dir) {
  const toml = path.join(dir, 'Cargo.toml')
  if (!fs.existsSync(toml)) return null
  const m = fs.readFileSync(toml, 'utf8').match(/^\s*rust-version\s*=\s*"([^"]+)"/m)
  if (!m) return null
  const parts = m[1].trim().split('.')
  while (parts.length < 3) parts.push('0')
  return parts.join('.')
}

/**
 * MSRV 阶段：让 `rust-version` 从「注释里的一个数字」变成**真被编译验证过的约束**。
 *
 * 本机与 CI 都锁 `rust-toolchain.toml` 的 1.93.1 ⇒ 平时用的**不是** MSRV；声明 1.91
 * 却从没在 1.91 上跑过，等于没声明。这里用 `RUSTUP_TOOLCHAIN` **覆盖**工具链文件
 * （环境变量优先级高于 `rust-toolchain.toml`）换编译器真跑一次
 * `cargo check --locked --all-targets`。
 *
 * 没装该工具链时**跳过并提示**，不判失败 —— 否则没装 1.91 的人跑全量门禁必红。
 * CI 的 msrv job 会先 `rustup toolchain install`，所以那里一定真跑。
 */
async function stageMsrv() {
  stageHeader('msrv', 'MSRV（rust-version 实编译校验）')
  const jobs = [
    ['symbio', backendDir],
    ['cli', path.join(repoRoot, 'cli')],
  ]
    .map(([name, dir]) => [name, dir, readMsrv(dir)])
    .filter(([, dir, msrv]) => msrv && fs.existsSync(path.join(dir, 'Cargo.toml')))

  if (jobs.length === 0) {
    console.log(yellow('  跳过：两个 workspace 都没有可读的 rust-version 声明'))
    return
  }

  for (const [name, dir, msrv] of jobs) {
    // 先用**同一个环境**探一次 rustc 版本。关键：`RUSTUP_TOOLCHAIN` 只有在 rustup
    // 安装的 cargo 上才生效（发行版包 / homebrew 的 cargo 会**静默忽略**它）。若不先
    // 核对版本，那些环境会拿默认编译器跑完并报告「MSRV 通过」——假阳性比不检查更糟。
    // 故：实际编译器 ≠ 声明值，一律跳过并写明原因。平台无关（不看 OS，只看版本号）。
    const probe = await run({
      label: `${name}: rustc --version（要求 ${msrv}）`,
      cmd: 'rustc',
      args: ['--version'],
      cwd: dir,
      env: { RUSTUP_TOOLCHAIN: msrv },
      echo: 'none',
    })
    const actual = stripAnsi(probe.output).match(/^rustc (\d+\.\d+\.\d+)/m)?.[1] ?? null
    if (actual !== msrv) {
      const why = actual ? `当前生效的是 ${actual}` : '取不到 rustc 版本'
      console.log(yellow(`      ↳ ${name}: 跳过（${why}）。要真验证需恰好装 ${msrv}：`))
      console.log(yellow(`         rustup toolchain install ${msrv}    # 更新的 patch 不能代替，证明不了下限`))
      record('msrv', `${name} @ ${msrv}`, true, `跳过：无 ${msrv} 工具链`)
      continue
    }

    const r = await run({
      label: `${name}: cargo check --all-targets @ ${msrv}`,
      cmd: 'cargo',
      args: ['check', '--locked', '--all-targets'],
      cwd: dir,
      // CARGO_TARGET_DIR 隔离：不让换编译器的检查作废日常构建缓存（见 msrvTargetDir 注释）
      env: { RUSTUP_TOOLCHAIN: msrv, CARGO_TARGET_DIR: path.join(msrvTargetDir, name) },
    })
    if (!r.ok) console.log(red(`      ↳ ${name} 在 ${msrv} 上编译失败 ⇒ rust-version 声明与实际不符`))
    record('msrv', `${name} @ ${msrv}`, r.ok)
  }
  console.log(dim(`      独立 target：${path.relative(repoRoot, msrvTargetDir) || '.'}/（不污染日常构建缓存）`))
}

async function stageFacts() {
  stageHeader('facts', '事实文件（必须最后）')
  if (FIX) {
    const gen = await run({
      label: 'gen-current-facts.mjs（--fix 写入）',
      cmd: process.execPath,
      args: [path.join(scriptDir, 'gen-current-facts.mjs')],
      cwd: repoRoot,
    })
    record('facts', 'gen-current-facts（写入）', gen.ok)
  }
  const chk = await run({
    label: 'gen-current-facts.mjs --check',
    cmd: process.execPath,
    args: [path.join(scriptDir, 'gen-current-facts.mjs'), '--check'],
    cwd: repoRoot,
  })
  if (!chk.ok) console.log(yellow('      ↳ 漂移：跑 `node scripts/gate.mjs --fix` 重新生成'))
  record('facts', 'gen-current-facts --check', chk.ok)
}

// ── 主流程 ─────────────────────────────────────────────────────────────
console.log(bold('══ 门禁 ══'))
console.log(dim(`  仓库根：${repoRoot}`))
console.log(dim(`  模式：${FIX ? '--fix（先格式化 / 重生成，再检查）' : '只读检查'}`))
console.log(dim(`  完整日志：${path.relative(repoRoot, logDir) || '.'}/`))

if (enabled('backend')) await stageBackend()
if (enabled('frontend')) await stageFrontend()
if (enabled('docs')) await stageDocs()
if (enabled('msrv')) await stageMsrv()
if (enabled('facts')) await stageFacts()

// ── 汇总 ───────────────────────────────────────────────────────────────
const failed = results.filter((r) => !r.pass)
console.log()
console.log(bold('══ 汇总 ══'))
for (const r of results) {
  const mark = r.pass ? green('✓') : red('✗')
  console.log(`  ${mark} ${r.label}${r.note ? yellow(` — ${r.note}`) : ''}`)
}
console.log()
console.log(`  通过 ${results.length - failed.length} / ${results.length}`)

if (failed.length > 0) {
  console.log(red(`  失败 ${failed.length} 项，逐项日志见 ${path.relative(repoRoot, logDir) || '.'}/`))
  process.exit(1)
}
console.log(green('  全部通过'))
process.exit(0)
