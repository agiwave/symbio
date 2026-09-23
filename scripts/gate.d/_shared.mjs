// gate.d 共享库：基线、输出解析、环境探针。任务模块按需引入。
//
// ## 基线只增不减
//
// 通过数低于基线即失败；高于基线时提示更新本表 —— 那是刻意要人看一眼的地方。
// 每次上调都要在这里留一句「为什么」。

import fs from 'node:fs'
import path from 'node:path'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { stripAnsi, yellow } from '../color.mjs'

/**
 * 通过数基线（**只增不减**；跑高了请更新这里并说明理由；**跑低了要说明理由**）
 *
 * 908：流式链路评审 A–D 批的回归锚点——`event_bus` 满通道语义（不摘除订阅 /
 *      `Closed` 才摘除 / resync 标记形状 / 满通道后标记必达）与**扇出不复制载荷**
 *      （`PluginPayload::Data(Arc<Value>)` 的 `Arc::ptr_eq` 断言），外加
 *      `transcript_stream` 的扇出共享与信封可解回事件。逐条见
 *      `docs/archive/streaming-chain-review-2026-09-22.md` §0.1。
 * 901：转写帧日志分级——`FrameLogLevel`（骨架 INFO / 细节 DEBUG）与
 *      `frame_log_of` 的相位判据，折行器新增 `foldable`（骨架帧与首帧不可折）。
 *      新增「骨架帧不可折」「相位分界」「行尾不落空格」用例（3）。
 * 898：控制台日志降噪——新增 `logger` 级别闸门用例（2）与 telegram 配置「缺键不是错误」
 *      用例（2）。闸门：无 subscriber 路径默认 INFO，debug 静默（`--verbose` / `SYMBIO_LOG`
 *      放开）；telegram 结构级 `#[serde(default)]` 使「只有身份键的 PLUGIN.yml」解析为默认值
 *      而非 Err（此前每次启动一条假 WARN）。
 * 894：S24 收口——转写流帧收成一条 `ChatMessage`：删除 `NodeOp` / `NodeChange`，
 *      `delta`（增量，与 `content` 互斥）与 `status = removed`（删除状态迁移）落到
 *      `ChatMessage` 上，告警下沉为 `TranscriptWriter::warn`；转写核心日志的「纯增量」
 *      折行（`DeltaLogCoalescer`）与请求级会话快照的回退判据一并落地。相应新增/重排了
 *      消息合并、删除帧、压缩终态、转写往返、折行边界（`seq`/图不受影响）、快照回退
 *      判据等用例（含一处 `state_frame` → `message_frame` 回归修复的锁定用例）。
 * 881：v1→v2 迁移 + 节点状态流 S20~S23 + 压缩消息流化 + 中止收口终态化 + 协议
 *      增量提取器逐字节回归 + tool_name 线上名投影 + MCP camelCase/载荷语义网 +
 *      extract_result 判定顺序。逐批明细见对应提交（`git log --grep=<批次/主题>`；
 *      本仓库不维护变更日志，变更历史即提交历史）。
 */
export const BASELINE = {
  // 915：批次 E（会话运行态并入转写流）的**核心不变量**——两种帧共用一个 `seq` 计数器
  //      是「会话不忙 ⇒ 本轮已终态」的全部依据，因此它必须有测试锚点，而不是靠读代码：
  //      `session_state_frames_share_the_message_seq_counter`（取号）、
  //      `session_state_frame_does_not_touch_the_message_graph`（不碰消息图）、
  //      `session_state_frame_is_an_envelope_that_decodes_back_to_the_node`（信封往返）、
  //      `the_two_frame_kinds_are_not_confusable`（按 type 分派，不靠猜）。
  //      −1：`session_change`（带节点视图的 VDFS `updated`）已删除 ⇒ 其单测随之删除。
  // 912：帧解包收敛到 `symbio_core` 的公共入口（`transcript_stream::event_of` /
  //      `is_resync`、`vdfs_provider::vdfs_change_of`）后补的契约用例——
  //      `vdfs_change_of` 三条（解信封 / 拒异 kind / 非 Data 帧不 panic）+
  //      背压标记一条（`event_of` 解不出、`is_resync` 认出）。
  // 915：批次 G（变更词汇收窄）**净增 0**——删掉「载荷按类型可选」的用例，换成三条
  //      **形状守卫**（`change_carries_no_payload_and_the_vocabulary_is_closed`：
  //      线上形状恰好 `["change","path"]`；`map_paths_is_the_single_translation_point`；
  //      `change_event_wire_shape_is_exactly_path_and_change`）。数量相抵，但断言的性质
  //      从「载荷怎么映射」变成「词汇表是闭集」——后者才是这次收窄要锁的东西。
  // 916：`770a9ea`（工具调用参数以字符串透传）补的单测
  //      `tool_call_args_passthrough_as_string_without_reserialize`——该次提交只跑了
  //      `cargo test --lib message_builder`，基线因此滞后一格；本轮门禁全量实测 916 对齐。
  rustTests: 916,
  // 46 spec 文件 / 661 → 683 → 687 → 689 用例。文件数与用例数均与平台无关（全仓 spec
  // 零平台分支、it.each 只遍历静态常量数组），照实测值钉死；逐批明细见对应提交
  // （`git log --grep=<批次/主题>`；本仓库不维护变更日志，变更历史即提交历史）。
  // 689：批次 G——前端侧收窄 `VdfsChange`（删四个载荷字段与 `appended` / `truncated`
  //      两个取值）。`useVdfs` 那 4 条 `appended` 用例换成 4 条**变更收敛**用例（三个
  //      取值同走重拉 / 影响判定收窄 / 已废除取值不再被静默吞掉）；`schemas` 侧新增
  //      「导出的 `VDFS_CHANGE_*` 恰好三个」闭集断言 + 「`RESYNC` 不并进变更词汇」。
  // 687：批次 E——转写流的会话运行态帧协议用例（7）：到达时**先冲刷**同会话待落地帧 /
  //      按会话冲刷（别的会话留在队列）/ 两种帧共用一个游标不触发跳号 / 运行态帧跳号同样
  //      重读 / 重复帧丢弃 / 缺节点视图仍推进水位 / 未接线不抛错；
  //      `sessions` store 的运行态收敛用例（5）：就地落 status 与标题零回读 / `failed`
  //      独立成态 / 新一轮清空上一轮 error / 提示音只在迁移上响 / 状态未变不覆盖 activity；
  //      以及 VDFS 侧新增两条（`updated` 防抖重拉、`updated` **不改运行态**）。
  //      −4：`reconcileTranscript` 的触发端用例（宽限复查整条机制已删除）。
  // 683：`sessions` store 补 `reconcileTranscript` 的触发端用例（4）——宽限期内不动作 /
  //      仍不收敛才回读 / 已收敛不回读 / 未发生 `working → 非 working` 迁移不安排；
  //      以及转写流合帧的提交批量化用例（3）。
  // 672：收起态摘要跟「流式末端」走——`messagePreviewFollowsLiveEdge` 判据用例（2）+
  //      摘要取端（末端 / 开头 / 短内容 / 空内容，4）+ 渲染层两条（思考、工具行）。
  vitestFiles: 46,
  vitestTests: 689,
}

export const VITEST_TIMEOUT_MS = 180_000

/** cargo 的进度噪音（刷屏且无信息量） */
export const cargoNoiseRe =
  /^(?:\s*$|.*\r$|\s*(Compiling|Checking|Downloading|Downloaded|Updating|Locking|Adding|Removing|Finished|Blocking|Waiting|Fresh|Documenting|Building)\b)/

/** 从输出里抓一个整数（第一个捕获组） */
export function grabInt(output, re) {
  const m = stripAnsi(output).match(re)
  return m ? Number(m[1]) : null
}

/** 把所有匹配的捕获组相加（`cargo test --workspace` 每个目标各打一行 `test result:`） */
export function sumInt(output, re) {
  const text = stripAnsi(output)
  let total = 0
  let found = false
  for (const m of text.matchAll(new RegExp(re.source, re.flags.includes('g') ? re.flags : `${re.flags}g`))) {
    total += Number(m[1])
    found = true
  }
  return found ? total : null
}

/** vitest --coverage 表格里「All files」行的行覆盖率（第 4 列，%） */
export function coverageLinesPct(output) {
  const m = stripAnsi(output).match(
    /^All files\s*\|\s*([\d.]+)\s*\|\s*([\d.]+)\s*\|\s*([\d.]+)\s*\|\s*([\d.]+)\s*\|/m,
  )
  return m ? Number(m[4]) : null
}

/** `tauri/vitest.config.ts` 里 `thresholds.lines` 的当前取值 */
export function coverageThreshold(frontendDir) {
  const cfg = path.join(frontendDir, 'vitest.config.ts')
  if (!fs.existsSync(cfg)) return null
  const m = fs.readFileSync(cfg, 'utf8').match(/thresholds:\s*\{[^}]*?\blines:\s*(\d+)/)
  return m ? Number(m[1]) : null
}

/** 沙箱「批量删除守卫」特征（vite 清 dist/ / vitest 清 coverage/ 会触发） */
export const sandboxDeleteRe = /safe-delete|SAFE_DELETE/

/**
 * 沙箱拦截导致的「未判定」：不记失败，记 skipped。
 * 一个**必然红**的门禁比没有门禁更糟 —— 人会学会忽略它。
 */
export function blockedBySandboxDelete(output) {
  return sandboxDeleteRe.test(output)
}

export function maybeSandboxDeleteHint(output) {
  if (!sandboxDeleteRe.test(output)) return
  console.log(
    '      ⚠ 输出含「批量删除被拦」字样：这是沙箱限制，不是构建/测试失败。'.padStart(0),
  )
  console.log('        确认方法：在沙箱外跑同一条命令，或先手工清掉 dist/ 与 coverage/。')
}

/** 从 `Cargo.toml` 读 `rust-version`，补全成 x.y.z（rustup 工具链名带 patch） */
export function readMsrv(dir) {
  const toml = path.join(dir, 'Cargo.toml')
  if (!fs.existsSync(toml)) return null
  const m = fs.readFileSync(toml, 'utf8').match(/^\s*rust-version\s*=\s*"([^"]+)"/m)
  if (!m) return null
  const parts = m[1].trim().split('.')
  while (parts.length < 3) parts.push('0')
  return parts.join('.')
}

/** 检测 cli 是否已有 release 构建（e2e 阶段的前置条件） */
/**
 * CLI release 二进制的**唯一**路径算出处。
 *
 * ⚠️ 产物落在哪个 target 目录**取决于本机 `cli/.cargo/config.toml`**——它被
 * `cli/.gitignore` 忽略（内容是机器相关的绝对路径），作用是把 `build.target-dir`
 * 指到 `../symbio/target` 以共享 symbio 已预热的依赖缓存（离线环境下没有第二次
 * 机会重新编译全部 C 依赖）。**因此不能写死任一位置**：有该配置时产物在
 * `symbio/target/`，没有时在 `cli/target/`——两个候选都探，取实际存在者。
 *
 * 写死单一位置会让「二进制找不到 ⇒ 每次都判定缺失」与「e2e 拿不到二进制 ⇒ 全用例
 * 失败（且失败形态是 -1 + 空 stderr，与崩溃无法区分）」同时发生。
 */
export function cliBinaryPath(repoRoot) {
  const exe = `symbio-cli${process.platform === 'win32' ? '.exe' : ''}`
  const candidates = [
    path.join(repoRoot, 'symbio', 'target', 'release', exe),
    path.join(repoRoot, 'cli', 'target', 'release', exe),
  ]
  return candidates.find((p) => fs.existsSync(p)) ?? candidates[0]
}

export function cliBinaryExists(repoRoot) {
  return fs.existsSync(cliBinaryPath(repoRoot))
}

// ==================== 「自动执行的工作」（不是检查项） ====================

/**
 * `git status --porcelain` → 脏路径集合（含已暂存与未暂存）。
 *
 * 只取路径，不区分状态：本模块关心的是「这个路径的内容在修复前后有没有变」。
 *
 * 用 `-z`（NUL 分隔）而非默认的行分隔：默认输出会把非 ASCII 路径**转义**成
 * `"\346\226\207.md"`（`core.quotepath=true` 是默认值），那样的串既读不了文件
 * （`contentHashes` 拿到 null），也不能直接喂给 `git add`（暂存失败）。
 * `-z` 下路径原样给出、不加引号，这两处一并消失。
 */
function dirtyPaths(repoRoot) {
  const r = spawnSync('git', ['status', '--porcelain', '-z'], { cwd: repoRoot, encoding: 'utf8' })
  const out = new Set()
  if (r.status !== 0) return out
  const tokens = r.stdout.split('\0')
  for (let i = 0; i < tokens.length; i++) {
    const t = tokens[i]
    if (!t) continue
    // 形如 `XY path`；重命名/复制时**紧随其后还有一个「旧路径」token**，跳过它。
    const xy = t.slice(0, 2)
    out.add(t.slice(3))
    if (xy[0] === 'R' || xy[0] === 'C' || xy[1] === 'R' || xy[1] === 'C') i++
  }
  return out
}

/** 工作区内容的哈希（文件不存在 → null）。用于判定「修复是否真的改写了它」。 */
function contentHashes(repoRoot, paths) {
  const m = new Map()
  for (const p of paths) {
    try {
      m.set(p, createHash('sha256').update(fs.readFileSync(path.join(repoRoot, p))).digest('hex'))
    } catch {
      m.set(p, null)
    }
  }
  return m
}

/**
 * 把一件**确定性的机械工作**交给门禁自己做完，而不是判它「有没有做过」。
 *
 * ## 为什么不判「有没有做过」
 *
 * 格式化与事实文件生成是**函数**，不是判断：`fmt(code) → code'`、`gen(code) → facts`
 * 对同一份输入永远给同一个输出。把它们写成 `--check` 等于让门禁因为
 * **人忘了按一次按钮**而红——它报的不是代码有问题，是流程有问题。而修复动作
 * 完全确定、零风险，没有任何理由等人来按。
 *
 * ## 语义
 *
 * - 命令**跑成功** ⇒ 通过（`ok: true`）。产物被改写不算失败，那是它该做的事。
 * - 命令**本身报错** ⇒ 不通过（真失败：工具坏了 / 输入不可解析）。
 * - 本地：被改写的路径**当场暂存**，使修复与「本次提交」是同一份内容。
 * - CI：CI 不能提交，所以「跑完仍有差异」只能报红——那是唯一能保住不变量的信号。
 *
 * ⚠️ **「CI 报红」靠 `ctx.ci`，而 `ctx.ci` 来自 `--ci`。** 调用本原语的任务
 * （`10-backend` 的两处 fmt、`60-facts` 的生成）必须在 CI 侧被以 `--ci` 调用，
 * 否则会退化成「自动修复 + 暂存」而**静默放过漂移**——一个只亮绿灯的检查项。
 * 回归测试里有一条专门断言 `.github/workflows/ci.yml` 的对应步骤传了 `--ci`。
 *
 * ## 怎么认出「被修复改写的路径」（⚠️ 这里错过一次）
 *
 * 第一版按「修复前干净、修复后变脏」判定，**方向反了**：日常流程是
 * 「改文件 → `git add` → 提交」，所以修复前就已脏（甚至已暂存）才是**常态**，
 * 而那样会漏掉它们 ⇒ 提交里留下**未格式化**的那一版。
 *
 * 正确判据是**内容哈希**：修复前给所有脏路径记哈希，修复后重算，
 * 哈希变了就是被改写过（无论它此前是干净、已暂存、还是已有未提交改动）。
 * 只比较脏路径即可——干净路径修复后若变脏，它自然进入「修复后」这一侧。
 *
 * 这样「修复前就脏、修复没碰」的路径**不会被暂存**：门禁没有立场替人决定
 * 「那些改动该不该进本次提交」（`commit.mjs` 的模型是「先 git add 你要的，
 * 再提交索引」）。
 */
export async function autoWork(ctx, { label, cmd, args = [], cwd }) {
  const before = dirtyPaths(ctx.repoRoot)
  const beforeHash = contentHashes(ctx.repoRoot, before)

  const r = await ctx.run({ label, cmd, args, cwd })
  if (!r.ok) {
    return {
      ok: false,
      note: r.timedOut ? '超时终止' : `exit=${r.code}${r.signal ? `, ${r.signal}` : ''}`,
    }
  }

  const after = dirtyPaths(ctx.repoRoot)
  const afterHash = contentHashes(ctx.repoRoot, after)
  const touched = [...after].filter(
    (p) => !before.has(p) || beforeHash.get(p) !== afterHash.get(p),
  )

  if (touched.length === 0) return { ok: true }

  const shown = touched.slice(0, 5).join('、') + (touched.length > 5 ? ` 等 ${touched.length} 个` : '')
  if (ctx.ci) {
    return {
      ok: false,
      note: `已执行但有 ${touched.length} 处差异（${shown}）—— CI 无法提交，请在本地跑一次门禁（会自动修复并暂存）`,
    }
  }

  const add = spawnSync('git', ['add', '--', ...touched], { cwd: ctx.repoRoot, encoding: 'utf8' })
  if (add.status !== 0) {
    return { ok: false, note: `已修复 ${touched.length} 个文件但暂存失败：${(add.stderr || '').trim()}` }
  }
  console.log(yellow(`      ⚠ 门禁已自动修复并暂存 ${touched.length} 个文件：${shown}`))
  return { ok: true, note: `自动修复 ${touched.length} 个文件（已暂存）` }
}
