// gate.d 共享库：基线、输出解析、环境探针。任务模块按需引入。
//
// ## 基线棘轮：向上自动、向下才红
//
// 三态判定只有**一处**定义（`ratchetVerdict`，纯函数）：低于基线红、高出但在
// `BASELINE_GRACE` 内黄字提示、超出容差红。`BASELINE` 本身被下调由
// `gate.d/35-baseline.mjs`（元守卫）判红——那是向下方向唯一的放宽口，且必须配
// `// baseline-allow <key>: <理由>` 一行豁免。
//
// 但**应用到门禁时**走 `ratchetOrBump`（带副作用的网关版）：实测 > 基线就**就地把数字
// 写回本文件**（只增、不写注释、本地顺手 `git add`），判绿；实测 < 基线才判红。
// 理由：棘轮的真实价值只在向下方向（测试被删 ⇒ 红，逼人查）。向上是预期内的正常增长，
// 不该拦人、更不该每次手写一段论证——那论证正是被本仓规则禁止的「源码内变更史」。
// 故本文件里 `BASELINE` 对象**不应再堆各批次的变更注释**；要追变更看提交历史。

import fs from 'node:fs'
import path from 'node:path'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { stripAnsi, yellow, green, red } from '../color.mjs'

// 本文件自身路径：自动上调 `BASELINE` 时就地重写这里（`acceptBaseline`）。
const SHARED_MJS = fileURLToPath(import.meta.url)
const SHARED_DIR = path.dirname(SHARED_MJS)
const REPO_ROOT = path.resolve(SHARED_DIR, '..', '..')

/**
 * 通过数基线（**只增不减**；跑高了请更新这里并说明理由；**跑低了要说明理由**）
 *
 * S27（916）：VdfsChange 信封 = `{path, data?}`、操作枚举整个退役——信封构造
 *      守卫（`bare` 无载荷 / `with_data` 带载荷 / 无 `change` 键）、门面原样透传、
 *      transcript 四帧生命周期（首帧全量副本 / delta 窄载荷 / removed 状态帧 /
 *      运行态随节点视图）。`transcript_stream` 已随 D-025 退役，908 中相关
 *      用例随之删除，由 VDFS 变更通道的新用例接替。
 * 908：流式链路评审 A–D 批的回归锚点——`event_bus` 满通道语义（不摘除订阅 /
 *      `Closed` 才摘除 / resync 标记形状 / 满通道后标记必达）与**扇出不复制载荷**
 *      （`PluginPayload::Data(Arc<Value>)` 的 `Arc::ptr_eq` 断言），外加
 *      `transcript_stream` 的扇出共享与信封可解回事件（该模块已退役）。逐条见
 *      `docs/archive/streaming-chain-review-2026-09-22.md` §0.1。
 * 901：转写帧日志分级——`FrameLogLevel`（骨架 INFO / 细节 DEBUG）与
 *      `frame_log_of` 的相位判据，折行器新增 `foldable`（骨架帧与首帧不可折）。
 *      新增「骨架帧不可折」「相位分界」「行尾不落空格」用例（3）。
 * 898：控制台日志降噪——新增 `logger` 级别闸门用例（2）与 telegram 配置「缺键不是错误」
 *      用例（2）。闸门：无 subscriber 路径默认 INFO，debug 静默（`--verbose` / `SYMBIO_LOG`
 *      放开）；telegram 结构级 `#[serde(default)]` 使「只有身份键的 PLUGIN.yml」解析为默认值
 *      而非 Err（此前每次启动一条假 WARN）。
 * 894：S24 收口——转写流帧收成一条 `ChatNode`：删除 `NodeOp` / `NodeChange`，
 *      `delta`（增量，与 `content` 互斥）与 `status = removed`（删除状态迁移）落到
 *      `ChatNode` 上，告警下沉为 `TranscriptWriter::warn`；转写核心日志的「纯增量」
 *      折行（`DeltaLogCoalescer`）与请求级会话快照的回退判据一并落地。相应新增/重排了
 *      消息合并、删除帧、压缩终态、转写往返、折行边界（`seq`/图不受影响）、快照回退
 *      判据等用例（含一处 `llm_state_frame` → `llm_message_frame` 回归修复的锁定用例）。
 * 881：v1→v2 迁移 + 节点状态流 S20~S23 + 压缩消息流化 + 中止收口终态化 + 协议
 *      增量提取器逐字节回归 + tool_name 线上名投影 + MCP camelCase/载荷语义网 +
 *      extract_result 判定顺序。逐批明细见对应提交（`git log --grep=<批次/主题>`；
 *      本仓库不维护变更日志，变更历史即提交历史）。
 */
export const BASELINE = {
  // 各测试套件的计数基线（**只增不减**）。
  // 向下（删测试）必须配 `// baseline-allow <key>: <理由>`（见 `35-baseline.mjs` 元守卫，防『悄悄下调基线』）。
  // 向上：门禁实测超过即自动把数字写回本文件，不在此堆各批次变更注释——变更原因看提交历史。
  // baseline-allow rustTests: 三次已归因的下修非侵蚀——批 P3 退役 `bridge` 转写路径（`v2_bridge.test.rs` 15 条缩到 `derived_facts.test.rs` 5 条 + 11 条迁与新增，逐条归属见上方注释）、批 删 `v2_mode_defaults_to_full_and_roundtrips` 1 条（被测对象 `V2Mode` 随 D-051 退役）、批 删 `v2_reconcile.test.rs` 9 条（被测对象 `v2_reconcile` 双源对账随单源切换退役，判据随之消失；`worker_progress.test.rs` 由「两读法对照」改写为「直接钉 WAL 读法」，条数仍 5）。
  // baseline-allow rustTests: 批 DE-014 5a 删 6 条（1764→1758）——被测对象整条退役：`context_compact` 工具、55% 水位提醒与其开关 `enable_compact_tool`（裁决与被否替代见 D-072）。逐条：`should_emit_context_nudge_threshold`、`build_request_view_injects_nudge_at_tail`、`build_request_view_nudge_comes_after_skeletonization`、`nudge_requires_all_three_conditions`（nudge 四条）、`turn_split_idx_starts_at_user_instruction`、`turn_split_idx_fallback_and_abort`（`find_turn_user_split_idx` 两条，切分点已并入 `auto_compress_process`）。覆盖没有丢：压缩仍只有一个入口，它的两种触发（阈值自动 / 失败节点重试 force）由 `context/pipeline.test.rs` 与 e2e `t11-compression-node` / `t18-compression-streaming` 守着。
  // baseline-allow rustTests: 批 DE-014 5b-1 删 2 条（1758→1756）——被测对象整体退场：闸门的「等工具齐」态与它判的那份在途工具簿记（裁决与否决替代见 D-073）。逐条：`in_flight_tools_block_wakeup`、`wait_for_tools_takes_priority_over_exit_conditions`。覆盖没有丢：那条不变量「不完整不唤醒」换了承担者——运行器环体只在把本批结果接进 exchange 之后才发下一次请求，由 e2e `t26-v2-tool-round`（批序与续轮）和 `t27-approval-resume`（本批留空 ToolCall 是合法状态、恢复后照续）守着；`gate_turn` 的两条真实退出条件（软上限优先于中止）与放行仍由 `chat_loop.test.rs` 其余 7 条钉住。
  // baseline-allow rustTests: 批 DE-014 5b-2 净删 4 条（1756→1752）——被测对象整体退场：`max_tool_rounds` 工具轮软上限整条链（它判的是 `TurnState` 上一个恒 0 的计数，所以那条上限从未命中过；裁决与被否替代见 D-074）。逐条：`chat_loop.test.rs` 删 5 增 2（7→4）——删 `max_tool_rounds_reached_exits_with_reason`、`below_max_tool_rounds_proceeds`、`none_means_unbounded_rounds`、`max_tool_rounds_zero_is_unbounded_not_instant_break`（软上限四条）、`max_rounds_is_judged_before_abort`（上限与中止的优先序），新增 `request_count_is_not_an_exit_condition`、`abort_wins_over_request_count`，把那条「优先序」的形状换成等价且现在真的成立的「中止优先于请求数」；`plugin.test.rs` 删 2 条 `max_tool_rounds_zero_maps_to_unlimited`、`default_max_tool_rounds_is_effectively_unlimited`（配置→请求的契约翻译点随字段消失；同族的 wire 契约「缺键 ≠ 零值」由 `model_chat.test.rs::absent_fields_deserialize_as_none_not_zero` 以 `auto_compress` / `load_history` 继续钉住，没有丢）。**新增** 1 条补上被删分支留下的洞：`chat_loop/progress.test.rs::a_user_supplement_resets_the_clock_without_spending_the_quota`——「抽到补充 ⇒ 静默时钟归零」这句纪律躺在恒假的抽干分支里，随分支失效；现在它是 `RoundHookFn` 的入参 `supplemented_at` + `RoundProgress::stirred`，有测试可钉（D-074 裁决二）。覆盖没有丢：工具轮数的真源在 `turn_runner` 环体的 `tool_round` 与 `RoundProgress::rounds`，由 e2e `t26-v2-tool-round`、`t27-approval-resume` 守着。
  // baseline-allow rustTests: 批 DE-014 5c 净删 31 条（1778→1747）——被测对象整族退役：请求视图层的三层裁剪（L1 内容淡化 / L2 轮次淡化 / L3 分层滑窗骨架化），裁决与被否替代见 D-078。不是「测试跟不上」，是**那三层与「请求体 = 事实网格的纯投影」结构上互斥**：运行器每请求跑 `verify_prompt_fidelity`，配对判据是正文精确相等（`projection/prompt_fidelity.rs`），而三层改的正是正文 ⇒ 接线后同一节点同时报两条缺口。逐条归属：`context/window.test.rs` 整文件 19 条（L3 骨架化的全部判据，模块一并删除）、`context/view.test.rs` 15→5（−10；旧的 L1/L2 判据与 11 参入口一起退场，换成五条注入段判据 `memory_section_is_prepended` / `readyset_section_sits_after_memory` / `delegate_section_lands_at_index_two` / `empty_sections_produce_nothing` / `every_section_carries_its_kind`，钉次序与 `meta.kind`）、`symbio_core/capability/tests.rs` 6→5（−1；`context_retention_serde_roundtrip` + `keep_count_semantics` 随 `CapabilityToolContextRetention` 退场，同族的「声明式枚举字段 serde 往返 + 缺键不产默认值」机制由新增的 `declared_enum_field_serde_roundtrip` 接手，载体换成仍在册的 `risk: CapabilityRiskLevel`——载体换掉、机制不许跟着死）、`tokenizer.test.rs` 10→9（−1；`conditional_truncation_only_when_over` 随死函数 `truncate_head_chars_if_over` / `truncate_tokens` 退场）。覆盖没有丢：投影完整性由 `prompt_fidelity` 每轮复核 + 上述五条注入段判据承担，端到端由 e2e `t47`（视图`role=user` 条数被轮次窗口按住）与 `t29` / `t31`（记忆段、调度段在包里且 `messages.json` 一个字不多）守着。
  // baseline-allow rustTests: 批 新增 2 条判据（1747→1749）——`message_events.test.rs::an_unopenable_fact_source_is_a_failure_not_an_exemption`
  // （事实源打不开 ⇒ `Failed`，不再与「无锚」的 `NoAnchor` 豁免混成一格）与
  // `chat_session.test.rs::apply_reports_a_failed_fact_write_and_writes_nothing`（`apply` 报错 ∧ 缓存里没有那个节点）。
  // 这两条钉的是 D-053 补记里那张结果表的**两个新分叉**，原实现里它们同属 `Discarded`、无法分别断言。非放宽。
  // baseline-allow rustTests: 批 新增 1 条判据（1749→1750）——`chat_loop/state.test.rs::the_session_actor_is_a_reasoner_which_is_why_pattern_cannot_dispatch`，
  // 钉「会跑模型的主体派生 Reasoner」这个反证（D-079 裁决一的地基）。非放宽。
  // baseline-allow rustTests: 批 新增 3 条判据（1750→1753）——`protocols/profile.test.rs` 的三条：
  // `every_protocol_row_keeps_its_measured_budget`（四行两数各是今天的实测值，且探测值必须过 GLM 那条 `> 2` 的下限）、
  // `the_chain_skips_nulls_and_follows_the_table_order`（MiMo 的 `reasoning_content: null` + `reasoning` 形态、顺序由表定）、
  // `the_anthropic_keys_use_the_same_extractor`（Anthropic 三组键与类型名同样落表）。档案表本身是把散落魔法数搬成数据，
  // 这三条钉的是「搬家后值不变、链序不变」——原先是三处字面量，没有可断言的单点。非放宽。
  // baseline-allow rustTests: 批 （02be8fc5）净删 1 条判据（1834→1833）——该批删掉
  // `adapters/mod.test.rs::tier_capabilities_match_the_budget_table`：`LatentCapability`
  // 表的读方随 ClassifyOnly 退役而消失，删的是持续维护税，不是判据。基线随之下调，非放宽。
  // 段一 新增 6 条（1924→1930，非放宽）：`event_bus/tests.rs` 的
  // `resync_of_recognizes_the_markers_this_module_produces` / `resync_of_unwraps_the_bus_envelope` /
  // `a_real_change_is_not_a_resync_marker`（两条解包入口互斥且成对——消费方「先试变更再试指令」的顺序靠它成立），
  // `subagent.test.rs` 的 `turn_end_of` 三格（working / 正常收尾 / 带错误）。
  // 段二 新增 4 条（1930→1934，非放宽）：`projection/mod.test.rs` 的
  // `read_all_is_the_named_form_of_the_full_reading_dialect` / `read_all_never_degrades_even_on_an_empty_slice`
  //（具名口径 ≡ 那两个参数的旧写法，逐格相等）、`governance/mod.test.rs` 的
  // `vis_scope_of_event_only_ever_widens_on_an_exact_spelling`（内容侧取值口只在精确拼写上变宽）、
  // `authz.test.rs` 的 `the_read_gate_is_owner_or_membership_in_one_judgement`（属主 ∨ 矩阵，一条判据）。
  // 段四 新增 5 条（1934→1939，非放宽）：`event/grid.test.rs` 的
  // `the_table_is_one_row_per_name_and_covers_every_declared_name`（一行一名，条数 ≡ 事件名常量数）、
  // `event_cell_answers_only_the_exact_spelling`（拼错 / 大小写 / 前半段命中一律「不在表上」）、
  // `the_table_agrees_with_the_read_side_about_which_cell_carries_a_node`（表 ≡ 读侧三目 `match` 的格位，
  // 挪一格就认不出——那条 `None` 就是原先的静默失效现场）；`store/wal.test.rs` 的
  // `the_grid_gate_rejects_a_name_that_is_not_on_the_table` / `the_grid_gate_rejects_a_name_claiming_another_cell`
  // （唯一写入口拒两种错位形态，且不消耗 seq、不落盘；同名字回到同格照落）。
  // 段二 新增 5 条（1957→1962，非放宽）：`message_events.test.rs` 的
  // `a_parked_change_is_on_disk_and_lands_for_a_fresh_token_holder`（寄存件在盘上 ⇒
  // 从未参与寄存的新令牌句柄补落得下去——寄存区从进程内静态表搬到文件的承重判据，旧实现这条必红）、
  // `parked_changes_land_in_park_order`（落格顺序 = 寄存顺序，倒放会静默给出错的末态）、
  // `a_staged_half_piece_never_lands_and_is_cleaned_up`（`.staged` 半件不是「半条变更」）、
  // `a_full_park_drops_the_oldest_entry`（有界：停在上界且让位的是最旧的一件）、
  // `a_park_that_cannot_be_written_reports_failure`（写不进去 = 无副本 ⇒ `Failed`，不许回 `Parked`）。
  // 新增 7 条（1969→1976，非放宽）：`invariants/mod.test.rs` 的
  // `provenance_to_earlier_existing_event_passes`（片内指向更早且存在 ⇒ 绿，正向基线）、
  // `provenance_pointing_forward_is_detected`（前向边）、`provenance_pointing_at_itself_is_detected`（自指）、
  // `provenance_into_a_hole_is_dangling`（片内查无此号）、
  // `provenance_across_the_slice_head_is_graced` / `provenance_beyond_the_slice_tail_is_graced_too`
  // （切片两头的跨片边头尾对称宽限——只宽限头部等于假设"切片总是从头切"）、
  // `pending_events_are_skipped_by_dangling_check`（未入库事件跳过）。
  // 前四条是 `produced_by_coverage` 查不出的那一半：它只问"有没有边"，不问"指向谁"。
  // 新增 5 条（1976→1981，非放宽）：`turn_runner.test.rs` 的
  // `denial_fact_tests` 一族——`a_denied_closure_lands_a_denial_fact`（拒绝落
  // `authz.denied`，载荷答得出「拒了什么 / 谁 / 因何」，且落它不变量仍全绿）、
  // `a_denied_turn_stays_unresolved`（`denied` **不是**收束格——本任务最容易做反的一处：
  // 为了让日志好看而把它算成收束，等于把「静默中断」一起掩盖）、
  // `a_denial_is_recorded_once_per_turn_and_kind`（幂等：产物臂在工具结果循环里会重复到闸前）、
  // `speech_and_artifact_denials_do_not_collide`（两臂各自成键）、
  // `denial_names_the_capability_the_gate_judges`（**对拍**：拒绝原因里的能力名拼写
  // 必须真的是生产表里那一项——手写一份没有对拍就只是又一处「表授予 A、闸门判 B」）。
  // baseline-allow rustTests: 批 净增 6 条判据（1981→1987）——停机原因闭集落地。
  // 逐条：`invariants/mod.test.rs` 新增 5 条（`closure_reason_in_the_closed_set_passes`
  // 五个取值全过 / `closure_reason_missing_is_reported_when_not_graced` 严判档红、读侧宽限档绿 /
  // `closure_reason_outside_the_closed_set_is_reported_either_way` 越界在两种档位下都红 /
  // `closure_reason_covers_both_closure_cells_and_the_denial_fact` 两种收束格 + `authz.denied`
  // 一个都逃不掉 / `closure_reason_denied_on_a_denial_fact_passes`）；`turn_runner.test.rs`
  // 的 `final_round_with_neither_text_nor_reasoning_still_falls_back` 补一条 `reason` 断言
  // （兜底必须是闭集里的取值，不是自由文本）；`turn_exec.test.rs` 的
  // `tool_free_turn_failure_leaves_fallback_event_and_bubbles` 同补一条。净增为正：
  // 只改了载荷键名（`why` → `detail`）与两条断言的目标字段，没有删判据。
  // baseline-allow rustTests: 批 b 净删 44 条（2038→1994）——被测对象整族退役：旧控制面
  // `loop_registry`（31 条：五格对拍、命中臂归属、档位逐格对表、同格加行零改主循环……）与
  // `chat_loop/progress`（10 条：`RoundProgress` 的静默时钟与配额），连同 `stage_events.test.rs`
  // 的 3 条旧 D-107 用例（`executed_*_turns_the_next_ask_into_declined`，被测对象是
  // `Verdict::Declined`）。判据没有丢：行表的开账 / 关账、全局 `prio`、装载期缺口由
  // `loop_rows.test.rs` 十条钉住；阶段折叠与 `report_due` 的纯函数判据在
  // `decl/mod.test.rs`；旧对拍的另一半（同一份日志切片的「已做」判定）由 `turn_head`
  // 视图的用例与 `stage_events.test.rs` 保留的 7 条承担。
  // baseline-allow rustTests: 批 步 5d 净删 3 条（2097→2094）——被测对象整条退役：
  // ① `resume.rs` / `resume.test.rs` 整份删除（6 条），被测的是**恢复路径的私有旁路**
  // （自己写协议事实、自己跑执行体）；旁路退役后唤醒 / 续写只能从事实进环。判据没有丢：
  // 唤醒那一笔的落格形状、读侧还原、拒绝还原、关账后读不回，由新增的
  // `tool_dispatch.test.rs` 四条钉住；「续写不重开用户格、收束记在原轮上」由
  // `turn_runner.test.rs` 的 `a_wake_continues_the_same_turn_without_reopening_the_user_cell`
  // 接手（它走生产 `ToolPort::land_wake` / `wake_batch`）；「恢复路径零旁路」由门禁
  // `S-017`（`scripts/grep-audit.mjs`）钉住。
  // ② `tool_executor.test.rs` 删 1 条（`not_executed_patch_is_completed_not_failed`）——
  // 被测的 `apply_not_executed` 是父节点终态的**第二个写方**，随它退役，父节点终态改由
  // `close_unfinished` 一处写；该形状改由同文件保留的
  // `interactive_break_leaves_result_for_skipped_calls` 断言（状态 Completed / 无 error /
  // `meta.failure_kind = not_executed`）。
  // 新增 4 条（唤醒路径：落格 / 读侧 / 拒绝 / 关账后读不回）。
  // 新增 5 条（2108 → 2113）：`await-*` 行「在等」与「没事」的可观测后果两形状用例
  // （D-126，`waiting_means_a_sent_request_without_a_terminal` 与对偶形状那条）+
  // 判据 7 / 8 / 12 三条机制判据的可执行证据（空表试金石 / 任意前缀 / 输入闭包，
  // 各经一次变异验证确认能红——`stale_sent` 恒假即红）。
  // 新增 1 条（2126 → 2127）：`port.test.rs::a_repeated_fact_write_collapses_on_its_idempotency_key`
  // ——D-136 之后「重复派发」被行拦下（`dispatch-*` 的 `sent` 出口已关），幂等键那条
  // 判据换由同一形状的**重复落格**直接钉（`request` 同 id、`response` 同 `(port,id,attempt)`）。
  // 新增 6 条（2142 → 2148）：S04 调度者与租约落地（`D-139`）——`cap` 是参数不是结构
  // （cap=1 串行 / cap=4 放开 / 0 与 1 同义）、活租约退出候选集且到点即回、调度者规格
  // （Decider / 80ms / 持 `assign.work`）、100 次注入只发一笔租约、到期判死并重派、
  // 缺 `assign.work` 零事实。
  // 新增 2 条（2148 → 2150）：读史窗口落地（`D-151`）——`the_history_window_keeps_only_the_newest_facts`
  // （切法是留最新、切点在下标 0、刚好一窗不切）与 `memories_older_than_the_window_wait_for_a_later_round`
  // （窗外更旧的记忆本轮不压缩，反向：同样四条放回窗口里立刻合并）。
  // 新增 31 条（2150 → 2181）：Agent 权限四字段执行面落地（`D-153`）——
  // ① `symbio_core/agent/tests.rs` 17 条：glob 段语义（`*` 不跨段 / `**` 跨段 /
  //   `?` 单字符 / 空路径只被 `**` 与 `.` 命中）、空清单即拒、读 / 写两清单独立判、
  //   域名标签边界（`evil-example.com` 不中 `*.example.com`）、limits 缺省值恒判、
  //   作用域只管自己的包（兄弟目录 `<root>-evil` 不算里面）、虚拟与物理两种路径形态同案、
  //   ctx 共享槽经 fork 往返、重复发布被忽略、无槽 = 宿主态；
  // ② `manifest.test.rs` 3 条：空 `permissions` 块合法且等于零授权、越包 glob 装载期拒、
  //   `host_apis` 往返不丢（改名 `declared_permissions` → `permissions` 那条是重命名，0 净增）；
  // ③ `web/net_guard.test.rs` 3 条：宿主态恒过、空清单恒拒、清单内放行 / 清单外拒；
  // ④ `vdfs/provider.test.rs` 5 条：越界必拒且 provider 零收到、读 / 写两清单分别判、
  //   空清单全拒、无 scope = 宿主态、物理绝对路径同案；
  // ⑤ `agent/host/tests.rs` 3 条：`max_prompt_bytes` 超限即拒且作用域未发布、
  //   `max_tools` 遍历后才判（子树真贡献了工具才超）、限额达标静默放行。
  rustTests: 2191,

  // baseline-allow ciRustTestsTotal: 批 的同一增量在 CI 口径下修一格（1425 → 1424，删 `v2_mode_defaults_to_full_and_roundtrips`），非侵蚀。
  // baseline-allow ciRustTestsTotal: 批 DE-014 5c 同一净删 31 条（1462 → 1431）在 CI 口径下的折算——`--workspace` 求和含 symbio lib 目标，删掉的 31 条在两种口径里是同一批，逐条归属见上方 `rustTests` 的注（D-078 三层淡化整族退役）。非侵蚀。
  // baseline-allow ciRustTestsTotal: 批 的同一增量在 CI 口径下修一格（1431→1433），逐条归属见上方 `rustTests` 的注。
  // baseline-allow ciRustTestsTotal: 批 的同一增量在 CI 口径下修一格（1433→1434），逐条归属见上方 `rustTests` 的注。
  // baseline-allow ciRustTestsTotal: 批 的同一增量在 CI 口径下修一格（1434→1437），逐条归属见上方 `rustTests` 的注。
  // baseline-allow ciRustTestsTotal: 批 （02be8fc5）的同一净删在 CI 口径下修一格（1437→1436）——删掉的
  // `adapters/mod.test.rs::tier_capabilities_match_the_budget_table` 无 `cfg` 门，Linux 那一跑的求和同样少它一条。逐条归属见上方 `rustTests` 的注。
  // baseline-allow ciRustTestsTotal: 批 步 5d 的同一净删 3 条在 CI 口径下的折算（1436→1433）——
  // `--workspace` 求和含 symbio lib 目标，删掉的 3 条在两种口径里是同一批，逐条归属见上方
  // `rustTests` 的注。非侵蚀。
  // baseline-allow ciRustTestsTotal: S04 调度者与租约的同一增量在 CI 口径下修一格（1437→1443），逐条归属见上方 `rustTests` 的注。非侵蚀。
  // baseline-allow ciRustTestsTotal: 读史窗口的同一增量在 CI 口径下修一格（1443→1445），逐条归属见上方 `rustTests` 的注。非侵蚀。
  // baseline-allow ciRustTestsTotal: Agent 权限四字段执行面的同一增量在 CI 口径下修一格（1445→1476）——31 条判据全落在 symbio lib 目标（`symbio_core/agent/tests.rs` 与四个 `*.test.rs`），`--workspace` 求和同样 +31。逐条归属见上方 `rustTests` 的注。非侵蚀。
  ciRustTestsTotal: 1476,
  cliRustTests: 21,
  tauriRustTests: 4,
  // doc-link-allow D-010: 下一行是退役文件的历史叙述（已把这两个文件删掉），不是入链
  // baseline-allow vitestFiles: 批 （`76324817`，D-063 会话呈现合流）退役 `tauri/src/schemas/conversation_line.ts` 与其 185 行 spec `tauri/src/schemas/__tests__/conversation_line.spec.ts`（整文件删除 ⇒ 测试文件 55→54）。被测对象随「对话/工作分栏退役」消失，判据随之下沉：新形态由 `e2e/cases/t24-panel-split.mjs` 与 `symbio/src/plugins/session/context/conversation_view.rs` 侧覆盖，不是丢掉覆盖。
  // baseline-allow vitestTests: 同一批 ：上述 spec 的用例 + `tauri/src/stores/__tests__/appearance.spec.ts` 随分栏退役收窄的那部分，合计 856→838（-18）。`appearance` 的其余行为仍由该 spec 的保留用例守着（只删了分栏相关的那一段），非侵蚀。
  // baseline-allow vitestTests: 批 段一（D-111）净删 6 条（894→888）——删的是**已不存在的被测对象**：`registry/messageTypes` 的 `messageCompactionStats` / `messageMemoryStats` / `compactionTriggerLabel` / `formatTokens` 四条前端口径随裁决迁到交付出口（那 6 条逐个断言的正是这四个函数的取值与格式化）。覆盖没有丢，是换了承担者：同一口径现在由 Rust 侧 `plugins/session/stats_reading.test.rs` 7 条判据钉（四行齐全 / 失败路径报「当前水位」不补 0 / 无数字出行即消失 / 压缩产物两行 / 非压缩的 notice 不误读 / token 单位只有一处 / 未知触发词原样透出），前端只留「有行就逐行排版」的渲染判据（`MessageNode.spec.ts` 那三条改吃 `stats` 行，条数不变）。非侵蚀。
  // baseline-allow vitestTests: 1002 → 1003（+1，D-147 / D-149）——新增的是**判据用例**，不是补回覆盖：
  // 折叠顺序反转后「主体优先 / 残留格兜底 / 旧形态不再归一」三组各要一条断言，
  // 由 8 条变 9 条（原「历史裸字符串一路通吃」那条拆成地址归一 + 旧形态不归一，净 +1）。
  // baseline-allow vitestTests: 1003 → 1024（+21，折叠骨架按需重取）——新增的是判据用例：
  // `skeletonLazy.spec.ts` 钉本地释放与按需重取的折叠状态，`MessageNode` / 会话 store 补
  // 重取路径的渲染与竞态断言。不是补回覆盖。
  vitestFiles: 68,
  vitestTests: 1024,
  verifyPrograms: 17,
  verifyNegatives: 4,
  // baseline-allow e2eCases: 批 读通道禁令（e2e 不得读任何内部存储文件——`messages.json` / `v2-events.wal` / `session.json` 一并禁）下删 3 个用例：t33 熔断（存储层判据，已由 `wal.test.rs` / `active.test.rs` 覆盖）、t40（断言全为 WAL 白盒入格，可见效果已由 t29 记忆召回 / t35 技能编译覆盖）、t43（messages-json-is-rebuildable，判据「真源已换、缓存可弃」已下沉 `store/tests.rs`，且 messages.json 已停写）。45 → 42，非侵蚀：三条的判据都下沉到了 Rust 侧，不是丢掉覆盖。
  // 增 t76（召回可见域）。
  e2eCases: 82,
  // 换述读数（D-097）：`recall_eval.rs` 的第四个数字带着一组新断言——指纹锁、
  // 每条换述必须真的换了措辞、依赖度定义域、54 条换述逐条验「承重」（换成无关问句
  // 读数必须精确上抬 1/(n·m)）、以及时间兜底案上读数不动。224 → 235，全是**新增判据**，
  // 没有一条是为了让数字变大而写的：删掉任何一条，对应那半句结论就没有守卫了。
  // 挂生产内核：`invariants.rs` 的 seq 重号在「严格单调且无跳号」语义下
  // 报两条（重号位 + 其后错位），`termination.rs` 加了内核判定的边界自证
  // （恰在上界合法 / 越界一轮判否）。235 → 237。
  // 增 1 条（243→244）：`invariants.rs` 在**真实日志**上注入一处悬空溯源
  // （把一条事件的 `produced_by` 指到片内一个被删掉的号），断言 `I2DanglingProvenance`
  // 必须响。它同时钉住新判据的反向纪律——`enable_i2_dangling: false` 时那条必须消失。
  // 增 3 条（244→247）：停机原因闭集。① 自造 `violated_log` 注入一处越界
  // `reason`（`"timeout"`，用 `authz.denied` 而不是再塞一条 final——后者会把那一轮
  // 收束掉，于是「无收束」那条判据反而消失，注入一条违规弄丢另一条是自假绿）；
  // ② ③ 在**真实数据形状**上喂合法值 / 越界值各一次：合法值必须全绿（否则判据在误杀）、
  // 越界必须红。真实日志的**回放**不跑这条——夹具只留信封、`reason` 在载荷里、且它采于
  // 之前；硬跑只会得到「每条收束格都缺 reason」的假红，那不是判据错了。
  verifyAsserts: 256,
  // 唯一一处**向上**的 panic 面（实测 77）：段归档（S2.7-b2）与它的读侧——
  // `WalStore::archive_before`、归档段并集的缓存（`archive_events` 拿读锁 + 写回写锁）、
  // `range` 拼接时的读锁，都要拿同一个 `RwLock`，沿用本模块既有的「锁当无中毒用」约定
  // （同 `len`/`head` 的 `expect("WalStore 锁中毒")`）。P6-④ 的 `SessionStore` 事实源
  // 句柄表（`facts` / `snapshot` / `delete_session`）重复同一条约定：表锁只在持锁期间
  // panic 时中毒，本仓不把中毒当可恢复状态。每一处均已按 PN-001 登记，理由同构：
  // 改成毒后恢复是行为变更，需单独 ADR。这不是放宽棘轮——新增的都是既有约定的复制，
  // 不是新引入的健壮性缺口。（P6-③ 把 `len` 的 expect 收进 `cfg(test)` 后实测曾降到 72；
  // 那一轮按 floor 口径保留 73，本批一并落定为实际值 77。）
  // 6b-②' P5 再加两处同构的复制：`runtime_facts`（读事实源表锁）与 `runtime_cache`
  // （运行态投影缓存表锁）。两处都只是既有「锁当无中毒用」约定的又一个持有者，
  // 不引入新的健壮性缺口；基线随之 77 → 79。
  // 抽内核：`invariants/mod.rs` 的四个检查改成「解析层构造内核输入 + 判定走
  // kernel.rs」后，各加一处 `expect` 把「内核返回的下标/id 必来自本切片」钉成显式
  // 约定（解析层与内核的输入约定若破，这里当场响而不是静默错位）——四条理由同构、
  // 均为穷尽性不变量，已按 PN-001 登记在 mod.rs。基线随之 79 → 83。
  panicSites: 83,
}


/**
 * 棘轮的**回填容差**：实测高出基线、差值在此格数以内只提示，超出即判红。
 *
 * 为什么要有上限：「高于基线」不是无害的——基线不跟着涨，删掉同样多的测试仍然全绿，
 * 棘轮就被削掉了同样的格数。而提示是可以被无限忽略的（日志天天有、门禁天天绿）。
 *
 * **适用边界（2026-10-10 调整）**：测试数 / 断言数这类「只增」棘轮已改为
 * **向上自动写回**（见 `ratchetOrBump`），不再靠本容差逼人回填，故本常量对它们
 * 实际不再触发红。它仍服务于 `35-baseline.mjs` 里的**其他**棘轮（前端覆盖率阈值、
 * 死码存量、core 单消费方符号数），以及 `20-frontend-static` 的覆盖率阈值提示。
 */
export const BASELINE_GRACE = 5

/**
 * 棘轮三态判定的**唯一定义**——所有棘轮（本地 / CI / verify）共用一条判据，
 * 各自决定红不红必然演化成两套口径。
 *
 * | 实测 vs 基线 | 结论 |
 * |---|---|
 * | 低于 | 红：有测试或证据被移除（棘轮存在的唯一理由） |
 * | 高出但在 `BASELINE_GRACE` 内 | 绿 + `warn`：提示回填 |
 * | 高出且超出容差 | 红：没回填的基线等于没有棘轮 |
 * | 等于 | 绿，无 note |
 *
 * `kind` / `unit` 只影响措辞，不影响判定。
 */
export function ratchetVerdict({ actual, baseline, name, kind = '通过数', unit = '测试' }) {
  if (actual < baseline) {
    return { ok: false, note: `${kind} ${actual} < 基线 ${baseline}（有${unit}被删或失败）` }
  }
  if (actual > baseline + BASELINE_GRACE) {
    return {
      ok: false,
      note: `${kind} ${actual} > 基线 ${baseline}：超出回填容差 ${BASELINE_GRACE} ⇒ 更新 BASELINE.${name}`,
    }
  }
  if (actual > baseline) return { ok: true, warn: true, note: `${kind} ${actual}（基线待更新）` }
  return { ok: true }
}

/**
 * BASELINE 的**只增**判据：当前值只要低于任一基准版本里的值，就是削判据。
 *
 * `BASELINE` 是源码常量，而它是「有测试被删就红」这条判据本身——把它改小不需要动
 * 任何测试、任何守卫代码，门禁照样全绿，且**不留痕迹**（与「测试消失」同形）。
 *
 * **基准是「触碰过该文件的最近两个版本」，不是分支根**：远历史里基线本来就低，拿它比
 * 会让每一跑都红，于是豁免被喂到失效；只比最近一版又漏掉「已经提交的那次下调」。
 * 两版足够让**下调发生的当次**变红，无论它还在工作区还是已进 HEAD。
 *
 * ⚠️ 只判 `BASELINE` 一个对象是**覆盖面**问题而非正确性问题——plan/13 批 M12 把它
 * 泛化成 {@link ratchetErosion}（多落点 + 两种方向），本函数保留为 `dir: 'floor'`
 * 的那一格，语义一字未改。
 */
export function baselineErosion(currentText, baseTexts) {
  return ratchetErosion({ dir: 'floor' }, parseBaselineCells, currentText, baseTexts, baselineWaivers(currentText))
}

/**
 * **棘轮元判据**：把「当前值不得比基准版本更松」抽成一处。
 *
 * 两种方向，都是「只许往紧的方向改」：
 * - `floor`（**只增**，下限）：当前值不得**小于**基准值。用于「测试数 / 覆盖率阈值」
 *   这类**越多越严**的判据——调低就是削判据。
 * - `ceiling`（**只减**，上限）：当前值不得**大于**基准值。用于「死码存量 / 未跨出
 *   core 的符号数」这类**越少越严**的判据——调高就是削判据。
 *
 * 两者共用这一处，是为了让「什么算削判据」只定义一次：`35-baseline.mjs` 的
 * `RATCHETS` 清单每加一条落点，不必重新决定一次红不红——那正是 M2 把
 * `ratchetVerdict` 收进单点的同一条理由。
 *
 * `cellsOf(text)` 是**提取函数**，由调用方给（各落点的形状不同：`BASELINE` 是对象
 * 字面量、`CORE_EXPORT_BASELINE` 是 `zero:one` 对、`vitest.config.ts` 是 `thresholds`
 * 嵌套块）。取不到任何格时返回空 Map，**由调用方决定**是红还是跳过——本函数不替它猜。
 *
 * ⚠️ **契约：`cellsOf` 必须返回 `Number`，不能返回字符串。** 两个方向都靠 `<` / `>`
 * 判定，而 JS 的字符串比较是**字典序**：`'9' < '10'` 为 **false**（`'9'` > `'1'`）。
 * 一个返回字符串的提取器会让 `9 → 10` 这类**下调**静默通过——方向恰好是最危险的那一侧。
 */
export function ratchetErosion({ dir = 'floor' } = {}, cellsOf, currentText, baseTexts, waivers) {
  const current = cellsOf(currentText)
  const waivedKeys = waivers ? new Set(waivers.keys()) : new Set()
  const loosened = (now, then) => (dir === 'ceiling' ? now > then : now < then)
  const worst = new Map()
  for (const { ref, text } of baseTexts) {
    for (const [key, then] of cellsOf(text)) {
      const now = current.get(key)
      if (now !== undefined && !loosened(now, then)) continue
      const hit = worst.get(key)
      if (hit && hit.from >= then) continue
      worst.set(key, { key, from: then, to: now ?? null, waived: waivedKeys.has(key), source: ref })
    }
  }
  return [...worst.values()]
}

/**
 * 已登记的基线豁免（键 → 理由）。豁免形态与 `dead-code-allow R-002: 理由` 同一条
 * 约定：**理由非空**，否则不算豁免——空理由的豁免等于没有判据。
 */
/**
 * 自动上调 `BASELINE.<key>` 的数值并重写本文件（**只增不减，不写注释**）。
 *
 * 棘轮的真实价值只在向下方向（实测掉下去 ⇒ 红，逼人查「是不是丢了测试」）。
 * 向上是预期内的正常增长：门禁检测到「实测 > 基线」就**就地写回新数**，不写任何
 * 注释——自动发生的，无需解释。这把原设计里「每次加测试都要手写一段论证再回填」
 * 的仪式整个去掉（那段论证就是被本仓规则禁止的「源码内变更史」）。
 *
 * 仅当 `value > 当前值` 才写，绝不下调（下沉由 `35-baseline.mjs` 的元守卫拦）。
 * 本进程单线程，同步读写不会与同批并发的 `35-baseline` 读产生撕裂。
 *
 * @param stage 仅本地模式传 true：写回后一并 `git add` 本文件，使上调随本次提交走
 *              （与 `autoWork` 对 fmt / 事实文件生成的处理同一条约定）。
 * @returns true 表示确实写回（数值上调）；false 表示无需改或写入失败。
 */
export function acceptBaseline(key, value, { stage = false } = {}) {
  let text
  try {
    text = fs.readFileSync(SHARED_MJS, 'utf8')
  } catch {
    return false
  }
  const re = new RegExp(`^  ${key}: (-?\\d+),$`, 'm')
  const m = re.exec(text)
  if (!m) return false
  const current = Number(m[1])
  if (!(value > current)) return false
  const next = text.replace(re, `  ${key}: ${value},`)
  try {
    fs.writeFileSync(SHARED_MJS, next, 'utf8')
  } catch {
    return false
  }
  if (stage) {
    try {
      spawnSync('git', ['add', '--', SHARED_MJS], { cwd: REPO_ROOT })
    } catch {
      /* 暂存失败不阻断门禁 */
    }
  }
  return true
}

/**
 * 棘轮判定 + 向上自动上调（网关版）。`ratchetVerdict` 保持纯函数（供单测），
 * 本函数在其上叠加「向上即写回」的副作用：
 * - 实测 > 基线：自动上调（只增，无注释），判绿；
 * - 实测 < 基线：判红（向下是唯一要守的方向，对应「测试被删」）；
 * - 相等：判绿。
 *
 * 仅当 `ctx.autoBump` 为真（本地门禁）才真的写回；CI 只亮绿灯、不落盘（CI 无法提交，
 * 且上调让它变绿正是预期）。`ctx.autoBump` 由 `gate.mjs` 设为 `!ctx.ci`。
 */
export function ratchetOrBump({ actual, baseline, name, kind = '通过数', unit = '测试', ctx }) {
  if (actual > baseline) {
    const bump = Boolean(ctx?.autoBump)
    if (bump) acceptBaseline(name, actual, { stage: true })
    return { ok: true, note: bump ? `基线自动上调至 ${actual}（无注释）` : '基线待更新（本地门禁会自动上调）' }
  }
  return ratchetVerdict({ actual, baseline, name, kind, unit })
}

export function baselineWaivers(text) {
  const out = new Map()
  for (const m of text.matchAll(/^ {0,2}\/\/ baseline-allow ([A-Za-z][A-Za-z0-9_]*): (\S[^\n]*)$/gm)) {
    out.set(m[1], m[2].trim())
  }
  return out
}

/**
 * 从 `_shared.mjs` 源码文本里取出 BASELINE 的数值格。
 *
 * 只认对象体内的 `  键: 数字,` 一种形状，且**只扫对象体**——不然注释里那句
 * 「回填 `1339 → 1341`」这类文本会被当成一格，而它描述的恰恰是历史值。
 */
export function parseBaselineCells(text) {
  const norm = text.replace(/\r\n/g, '\n')
  const start = norm.indexOf('export const BASELINE = {')
  const cells = new Map()
  if (start < 0) return cells
  const body = norm.slice(start)
  const end = body.indexOf('\n}')
  const lines = (end < 0 ? body : body.slice(0, end)).split('\n')
  for (const line of lines) {
    const m = /^ {2}([A-Za-z][A-Za-z0-9_]*): (-?\d+),$/.exec(line)
    if (m) cells.set(m[1], Number(m[2]))
  }
  return cells
}

/**
 * 解析脚本里的判据码声明（`@ns` 命名空间 / `@codes` 本脚本判的号）。
 *
 * 声明**只认行注释形态**，写在文件头之后——登记表的唯一真源是这些行，
 * `docs/004-ref/GATE_CODES.md` 由它们生成（`facts/gate-codes.mjs`）。
 */
export function parseGateCodeDecls(text) {
  const ns = new Map()
  const codes = new Map()
  for (const m of text.matchAll(/^ *\/\/ @ns ([A-Z]{1,4}) (\S[^\n]*)$/gm)) {
    ns.set(m[1], m[2].trim())
  }
  for (const m of text.matchAll(/^ *\/\/ @codes ((?:[A-Z]{1,4}-\d{3}\s*)+)$/gm)) {
    for (const code of m[1].trim().split(/\s+/)) codes.set(code, code.slice(0, code.indexOf('-')))
  }
  return { ns, codes }
}

/** 正则字面量可以出现在这些字符之后（其余情况下 `/` 是除号，不是正则的开头） */
const REGEX_PRECEDERS = new Set([
  '', '(', ',', '=', ':', '[', '!', '&', '|', '?', '{', '}', ';', '+', '-', '*', '%', '~', '^', '<', '>',
])

/**
 * 扫一段源码，分出两类区间：
 * - `literals`：字符串字面量的**内容**区间（不含引号）；
 * - `blanks`：应当从「代码标识符」里剔除的区间——字符串内容 **与注释**。
 *
 * 为什么只要字符串：判据码被「用到」的唯一硬形态是它出现在输出语句里；注释里的
 * 「原 E-010 已退役」讲的是历史，要求它进登记表就把叙述当成了事实——而这类叙述
 * 在头注释里成段存在，误报会直接把守卫喂成豁免。
 *
 * 为什么注释要一起剔：注释是**文档的另一处**，它跟着代码一起腐烂。按标识符统计时
 * 留下注释，等于让一个早已删除的常量继续替它那一族「作证」，把合法的改名判成假红。
 */
export function codeSpansOf(src) {
  const literals = []
  const blanks = []
  let i = 0
  let last = ''
  while (i < src.length) {
    const c = src[i]
    const d = src[i + 1]
    if (c === '/' && d === '/') {
      const nl = src.indexOf('\n', i)
      const stop = nl < 0 ? src.length : nl
      blanks.push({ start: i + 2, end: stop })
      i = stop < src.length ? stop + 1 : src.length
      continue
    }
    if (c === '/' && d === '*') {
      const end = src.indexOf('*/', i + 2)
      blanks.push({ start: i + 2, end: end < 0 ? src.length : end })
      i = end < 0 ? src.length : end + 2
      continue
    }
    if (c === '/' && REGEX_PRECEDERS.has(last)) {
      i++
      let inClass = false
      while (i < src.length && src[i] !== '\n') {
        if (src[i] === '\\') {
          i += 2
          continue
        }
        if (src[i] === '[') inClass = true
        else if (src[i] === ']') inClass = false
        else if (src[i] === '/' && !inClass) break
        i++
      }
      last = '/'
      i++
      continue
    }
    if (c === '"' || c === "'" || c === '`') {
      let j = i + 1
      while (j < src.length) {
        if (src[j] === '\\') {
          j += 2
          continue
        }
        if (src[j] === c) break
        if (c !== '`' && src[j] === '\n') break
        j++
      }
      literals.push({ start: i + 1, end: j })
      blanks.push({ start: i + 1, end: j })
      last = c
      i = j + 1
      continue
    }
    if (!/\s/.test(c)) last = c
    i++
  }
  return { literals, blanks }
}

export function stringLiteralsOf(src) {
  return codeSpansOf(src).literals.map(({ start, end }) => src.slice(start, end))
}

/**
 * 取「代码自己写下的标识符」那一份文本：字符串内容与注释整段抹成空格。
 *
 * 为什么需要：`env!("CARGO_PKG_VERSION")` 里的 `CARGO_PKG_VERSION` 是**外部约定**的
 * 名字，不是本仓声明的常量；一条讲历史改名注释里的 `ROUTE_OLD` 也只是叙述。按标识符
 * 统计时留下它们，文档里合法的 `CARGO_TARGET_DIR` 就成了假红，而早已删除的常量还能
 * 替自己那一族「作证」。判据的「这一族存在」只能由代码本体回答。
 */
export function codeTextOf(src) {
  let out = ''
  let at = 0
  for (const { start, end } of codeSpansOf(src).blanks) {
    out += src.slice(at, start) + ' '
    at = end
  }
  return out + src.slice(at)
}

/**
 * `vitest run --coverage` 的**静态基准**墙钟预算（真正用哪个数由 `_machine.mjs::budget` 定）。
 *
 * 它**不是判据**——判定在覆盖率阈值（`tauri/vitest.config.ts` 的 `thresholds`）与
 * 用例通过数上，这里只防「跑飞了还不肯停」。
 *
 * 原值 180s。2026-10-19 冷全量（基座 rebase 了 25 个提交 ⇒ backend 全量重编 10m58s，
 * 8 逻辑核全压满）实测 coverage 跑了 **>180s** ⇒ 报「超时终止」，与真的卡死长得一样。
 * `docs` 与 `backend` 同批之后，前端这三项与 cargo 争核是**常态**而不是意外，
 * 所以基准按「最坏负载」给，不按「空闲时多快」给：空闲时 40–60s，最坏 ~4m。
 *
 * ⚠️ 这个数**只在首跑**（本机没有实测历史）时直接生效；之后由
 * `budget()` 用上一轮实测覆盖。它是**基准**，不是「本机跑多快」的记录——
 * 把某次实测值写死在这里，就又会变成「只对那台机器正确」。
 */
export const VITEST_TIMEOUT_MS = 300_000

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

/**
 * `cargo test` + **通过数棘轮**（`symbio` 与 `cli` 共用）。
 *
 * 抽成共享函数而不是各写一遍：`symbio` 与 `cli` 是**两个独立 workspace**
 * （仓库根没有 `Cargo.toml`），而「跑测试并比对通过数基线」这件事必须对两者是
 * **同一套判据**——各写一份的结果是两边各自演化，最后变成两条不同的规范
 * （`check-commit-msg.mjs` 把「单条」与「一段范围」抽成同一个 `validate()` 就是为此）。
 *
 * 棘轮语义（与 `BASELINE` 头部的说明一致）：
 * - 低于基线 ⇒ **失败**（有测试被删，或有测试失败而退出码没反映出来）；
 * - 高于基线 ⇒ 通过，但打印提示要求同步基线（那是刻意要人看一眼的地方）；
 * - 解析不到通过数 ⇒ 通过但标注 —— 宁可漏报，也不因为**解析**失败把门禁变红。
 *
 * **CI 分支同样有棘轮**（2026-10-07 补上，此前是 `return { ok: true }`）：靠 `ciBaseline`
 * 这一格，口径是 `--workspace` **各行求和**，与下面的本地口径不同。调用方在 CI 模式
 * **必须**传它——不传不降级、直接判红，理由见函数体内注释：省略参数不该让守卫消失。
 *
 * **口径（本地分包时）**：取输出里**第一个** `test result: ok. N passed`——那是 lib
 * 目标的**通过数**（`cargo test -p <pkg>` 先跑 lib，再跑集成测试 / doctest）。所以基线
 * 是「通过数」**不是「用例总数」**：`ignored` 的不计入（实测 `symbio` lib 总 1299、
 * `2 ignored` ⇒ 口径数 1297）。拿「总用例数」去定基线会差掉 ignored 那几个。
 *
 * 自定义任务而非声明式命令：同一次运行既判退出码又解析通过数，
 * 避免为了拿输出再跑一遍测试。
 */
export function cargoTestRatchet(ctx, { label, cwd, args, baseline, baselineName, ciBaseline, ciBaselineName }) {
  return {
    label,
    run: async () => {
      const r = await ctx.run({ label, cmd: 'cargo', args, cwd })
      if (!r.ok) {
        const note = r.timedOut ? '超时终止' : `exit=${r.code}${r.signal ? `, ${r.signal}` : ''}`
        return { ok: false, note, logFile: r.logFile }
      }
      if (ctx.ci) {
        // ⚠️ **没给 `ciBaseline` ⇒ 判红**，而不是退回「只信退出码」。
        //
        // 退回是这条守卫唯一会**静默**失效的方式：调用方少写一个参数，CI 就变回
        // 只抓「测试失败」、抓不住「测试消失」——后者不留任何痕迹（本地 1339 与基线
        // 比，CI 却只是求和打印一行数字）。宁可让漏写的人当场看见红，也不要一个
        // 只亮绿灯的检查项。真要「这个 crate 在 CI 没有可比基线」，显式传 `ciBaseline: 0`，
        // 那是**声明**，不是省略。
        if (ciBaseline === undefined) {
          return {
            ok: false,
            note: 'CI 模式缺 ciBaseline ⇒ 本轮次没有棘轮（调用方少写了一个参数）',
            logFile: r.logFile,
          }
        }
        // CI 跑一次 `--workspace`：输出是**所有测试目标各行求和**，与本地分包「各取
        // 首个 result 行」不是同一个口径（见 `BASELINE.ciRustTestsTotal`）。
        const total = sumInt(r.output, /test result: ok\. (\d+) passed/)
        if (total === null) return { ok: true, note: '未能解析通过数（--workspace 全量；只信退出码）' }
        const v = ratchetOrBump({
          actual: total,
          baseline: ciBaseline,
          name: ciBaselineName ?? baselineName,
          kind: 'CI 全量',
          ctx,
        })
        if (v.ok) {
          console.log(green(`      ${total} passed（CI 全量，基线 ${ciBaseline}）${v.note ? ` · ${v.note}` : ''}`))
        } else {
          console.log(red(`      ↳ ${v.note}`))
        }
        return { ...v, logFile: r.logFile }
      }
      const passed = grabInt(r.output, /test result: ok\. (\d+) passed/)
      if (passed === null) return { ok: true, note: '未能解析通过数' }
      const v = ratchetOrBump({ actual: passed, baseline, name: baselineName, ctx })
      if (v.ok) {
        console.log(green(`      ${passed} passed（基线 ${baseline}）${v.note ? ` · ${v.note}` : ''}`))
      } else {
        console.log(red(`      ↳ ${v.note}`))
      }
      return { ...v, logFile: r.logFile }
    },
  }
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

// CLI release 二进制的路径与新鲜度判定**不在这里**——统一在
// `scripts/cli-binary.mjs`（门禁与 e2e 共用的唯一真相）。曾经这里只有
// 「文件在不在」两个函数，而「在」不等于「对应当前源码」，于是过期产物被一直用下去。

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
 * （`10-backend` 的两处 fmt、`06-facts-write` 的生成链）必须在 CI 侧被以 `--ci` 调用，
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

/**
 * `autoWork` 的**整链版**：把一串生成器当一个原子工作跑，只在**末态**与跑之前比差异。
 *
 * ## 为什么 `autoWork` 逐格跑不了整条链（2026-10-23 实测恒红）
 *
 * `autoWork` 每跑完一步就判一次「这一步有没有改写文件」。单步生成器这样是对的
 * （它的产物就是末态）。但「事实文件 → 登记汇总页 → 引用地址 → verify 期望数据」是一条
 * **有中间态的链**：`registry-audit --write` 把汇总页写成 `[[编号]]` 记号，
 * 下一格 `gen-doc-refs` 才展开成地址——逐格判差异时，第一格的产物**本身就是差异**，
 * 于是 `--ci` 侧无论树新树旧都报红。CI 要的是末态判据，不是生成器的自述。
 *
 * ## 语义（与 `autoWork` 同一条约定，只是判据挪到链尾）
 *
 * - 任一步**命令本身报错** ⇒ 不通过（真失败：工具坏了 / 输入不可解析）；CI 侧顺带还原工作区。
 * - 本地：末态有差异 ⇒ 差异路径**当场暂存**，修复与本次提交同一份内容；无差异即绿。
 * - CI：末态有差异 ⇒ **还原工作区**后报红。还原是这条相对 `autoWork` 多出来的一件事：
 *   链跑完树是脏的，而 CI 的红只该说明「生成物没跟上代码」，不该把树留在半成品形态
 *   给后续步骤（`git diff --exit-code`、产物上传）当输入。
 *   还原口径：跑之前已脏的路径按**内容快照**写回（那是人自己的改动，不能用 `git restore`
 *   ——它恢复到索引，会把未暂存改动抹掉）；跑之前干净的路径恢复到索引内容即正确；
 *   跑之前不存在的（生成器新建）删掉自己写出来的东西。
 */
export async function autoChain(ctx, { label, steps }) {
  const before = dirtyPaths(ctx.repoRoot)
  const beforeHash = contentHashes(ctx.repoRoot, before)
  const snapshot = new Map()
  if (ctx.ci) {
    for (const p of before) {
      try {
        snapshot.set(p, fs.readFileSync(path.join(ctx.repoRoot, p)))
      } catch {
        snapshot.set(p, null)
      }
    }
  }
  const touchedNow = () => {
    const after = dirtyPaths(ctx.repoRoot)
    const afterHash = contentHashes(ctx.repoRoot, after)
    return [...after].filter((p) => !before.has(p) || beforeHash.get(p) !== afterHash.get(p))
  }
  const restore = () => {
    // 本地模式不还原：那侧的约定是「生成并暂存」，而 `git restore` 会把人**未提交的改动**
    // 一起抹掉。链某一步真失败时把现状留给人看，比替他决定该留哪一版更安全。
    if (!ctx.ci) return null
    const toIndex = []
    for (const p of touchedNow()) {
      // 跑之前已脏：那是人自己的改动，按**内容**写回（`git restore` 恢复到索引会抹掉未暂存改动）。
      if (snapshot.has(p)) {
        const bytes = snapshot.get(p)
        if (bytes === null) fs.rmSync(path.join(ctx.repoRoot, p), { force: true })
        else fs.writeFileSync(path.join(ctx.repoRoot, p), bytes)
      } else toIndex.push(p)
    }
    if (!toIndex.length) return null
    // `toIndex` 里可能混着「跑之前不存在」的新文件（生成器新建）：`git restore` 不认它们，删掉。
    const ls = spawnSync('git', ['ls-files', '--', ...toIndex], { cwd: ctx.repoRoot, encoding: 'utf8' })
    const tracked = new Set((ls.stdout || '').split('\n').filter(Boolean))
    for (const p of toIndex) if (!tracked.has(p)) fs.rmSync(path.join(ctx.repoRoot, p), { force: true })
    const keep = toIndex.filter((p) => tracked.has(p))
    if (!keep.length) return null
    const r = spawnSync('git', ['restore', '--worktree', '--', ...keep], {
      cwd: ctx.repoRoot,
      encoding: 'utf8',
    })
    return r.status === 0 ? null : `还原失败：${(r.stderr || '').trim()}`
  }

  for (const s of steps) {
    const r = await ctx.run({
      label: s.label,
      cmd: s.cmd,
      args: s.args ?? [],
      cwd: s.cwd ?? ctx.repoRoot,
    })
    if (!r.ok) {
      const why = r.timedOut ? '超时终止' : `exit=${r.code}${r.signal ? `, ${r.signal}` : ''}`
      const back = restore()
      return {
        ok: false,
        note: `${s.label} 失败（${why}）${back ? `，且${back}` : ctx.ci ? '，工作区已还原' : ''}`,
      }
    }
  }

  const touched = touchedNow()
  if (touched.length === 0) return { ok: true }
  const shown = touched.slice(0, 5).join('、') + (touched.length > 5 ? ` 等 ${touched.length} 个` : '')
  if (ctx.ci) {
    const back = restore()
    return {
      ok: false,
      note:
        `${label} 跑完末态有 ${touched.length} 处差异（${shown}）——生成物没跟上代码，CI 无法提交` +
        (back ? `，且${back}` : '，工作区已还原') +
        '；请在本地跑一次该链（会自动生成并暂存）',
    }
  }
  const add = spawnSync('git', ['add', '--', ...touched], { cwd: ctx.repoRoot, encoding: 'utf8' })
  if (add.status !== 0) {
    return { ok: false, note: `已生成 ${touched.length} 个文件但暂存失败：${(add.stderr || '').trim()}` }
  }
  console.log(yellow(`      ⚠ 门禁已自动生成并暂存 ${touched.length} 个文件：${shown}`))
  return { ok: true, note: `自动生成 ${touched.length} 个文件（已暂存）` }
}
