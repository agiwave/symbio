// gate.d 共享库：基线、输出解析、环境探针。任务模块按需引入。
//
// ## 基线只增不减
//
// 通过数低于基线即失败；高于基线时提示更新本表 —— 那是刻意要人看一眼的地方。
// 每次上调都要在这里留一句「为什么」。

import fs from 'node:fs'
import path from 'node:path'
import { stripAnsi } from '../color.mjs'

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
 *      extract_result 判定顺序。逐批明细见 docs/CHANGELOG.md 的对应条目。
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
  rustTests: 915,
  // 46 spec 文件 / 661 → 683 → 687 → 689 用例。文件数与用例数均与平台无关（全仓 spec
  // 零平台分支、it.each 只遍历静态常量数组），照实测值钉死；逐批明细见 docs/CHANGELOG.md。
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
