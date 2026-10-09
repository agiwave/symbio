// gate.d 共享库：基线、输出解析、环境探针。任务模块按需引入。
//
// ## 基线只增不减
//
// 三态判定只有**一处**定义（`ratchetVerdict`）：低于基线红、高出但在 `BASELINE_GRACE`
// 内黄字提示、超出容差红；`BASELINE` 本身被下调由 `gate.d/35-baseline.mjs` 判红。
// 每次动基线都要在这里留一句「为什么」。

import fs from 'node:fs'
import path from 'node:path'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { stripAnsi, yellow } from '../color.mjs'

/**
 * 通过数基线（**只增不减**；跑高了请更新这里并说明理由；**跑低了要说明理由**）
 *
 * S27（916）：VdfsChange 信封 = `{path, data?}`、操作枚举整个退役——信封构造
 *      守卫（`bare` 无载荷 / `with_data` 带载荷 / 无 `change` 键）、门面原样透传、
 *      transcript 四帧生命周期（首帧全量副本 / delta 窄载荷 / removed 状态帧 /
 *      运行态随节点视图）。`transcript_stream` 已随 ADR-025 退役，908 中相关
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
 * 894：S24 收口——转写流帧收成一条 `ChatMessage`：删除 `NodeOp` / `NodeChange`，
 *      `delta`（增量，与 `content` 互斥）与 `status = removed`（删除状态迁移）落到
 *      `ChatMessage` 上，告警下沉为 `TranscriptWriter::warn`；转写核心日志的「纯增量」
 *      折行（`DeltaLogCoalescer`）与请求级会话快照的回退判据一并落地。相应新增/重排了
 *      消息合并、删除帧、压缩终态、转写往返、折行边界（`seq`/图不受影响）、快照回退
 *      判据等用例（含一处 `llm_state_frame` → `llm_message_frame` 回归修复的锁定用例）。
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
  //      `is_resync`、`vdfs::vdfs_change_of`）后补的契约用例——
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
  // 925：会话选项并入详情方言 + `session/update` 退役（2026-09-23）——**净 +9**
  //      （916 → 925）。逐文件核对（`git diff HEAD -- symbio/` 数 `#[test]` /
  //      `#[tokio::test]`，含新建的侧车文件），不是估算：
  //        +4  `plugins/local/policy/policy_tracker.test.rs`（新建）——限流窗口
  //            `checked_sub` 下溢修复的用例（`window_longer_than_clock_origin_…`）
  //        +4  `plugins/session/plugin/vdfs_provider.test.rs`——1 条属 S2
  //            （`session_list_carries_the_option_definition`），3 条属 S6
  //            （具名新建地址末段即 id / 命中已存在则覆盖 / 覆盖分支浅合并）
  //        +3  `plugins/session/options/tests.rs`——产物换成 `DetailField` 后的锚点
  //            （字段 key 就是解析链读的 metadata 键；心跳子表单缺省值与子对象同源）
  //        +2  `symbio_core/schemas/detail.rs`——`DETAIL_PICKS` 与方言补充
  //        -1  `plugins/session/handlers.test.rs`——「两条路径一致性」随路由退役删除，
  //            契约搬到 `write_merges_metadata_shallowly`
  //        -3  `symbio_core/schemas/options.rs`（整文件删除）——旧 `OptionNode` 产物用例
  //      合计 +9。删的是机制不是覆盖：新产物那侧由上面几条接管。
  // 916：S27（变更信封 = `{path, data?}`，操作枚举退役）——**净 −9**（925 → 916）。
  //      `transcript_stream` 退役删掉它那批帧协议用例；`delta` 从 `updated` 的
  //      可选字段变成 `ChatMessage.delta` 字段本身，随「按类型分派」一起消失的用例
  //      由 VDFS 变更通道的新用例接替（信封构造守卫 / 首帧全量副本 / removed 状态帧 /
  //      运行态随节点视图）。⚠️ 上一批（356ba9d）在提交信息里写了「916 通过」、
  //      也加了上面那条 S27 注记，却**忘了把这里的数字从 925 改下来**——棘轮基线
  //      只增不减，漏改就是门禁常红（同一批还漏了 `vitestFiles` / `vitestTests`）。
  // 919：S27 收口补齐（2026-09-23）——**+3 用例**（916 → 919），全部围绕「在途号
  //      永不落库」这条不变式（它是两个独立递增的计数器能共存的前提）：
  //      ① `transcript.test.rs`：`is_inflight_seq` 的号段边界（含「存量泄漏水位仍是
  //         权威号」——这正是 `INFLIGHT_SEQ_BASE` 从 `1 << 40` 抬到 `1 << 50` 的理由）；
  //      ② `chat_session.test.rs`：`append_messages` 摘掉在途占位号并重新分配；
  //      ③ 同文件：`replace_messages` 同样摘号，且**既有序号一个不动**。
  //      背景：`CompressionEmitter::finish` 把在途节点原样交给落库，存储水位被抬进
  //      在途号段，两个计数器在同一区间各自递增 ⇒ 撞号（同一会话里用户消息与压缩
  //      节点各持 `1099511627781`，e2e T8 的「seq 严格递增」当场失败）。
  // 924：会话实时投递合帧 + `VdfsChangeSubscriptions::notify` 语义澄清（2026-09-23）
  //      ——实测 924。**+4 用例**为本批新增：`session/transcript.test.rs` 三条
  //      （相邻同节点纯增量合成一帧 / 切换节点即开新窗口 / 运行态帧先冲出待投增量）
  //      + `symbio_core/vdfs/host.rs` 一条（**无关键订阅**一条都收不到）；
  //      两处改名不计数（`coalescing_…_loses_no_text`、`…_is_node_address_…`）。
  //      余下 1 格是上批**基线滞后**：HEAD 实测 920、基线记 919（同 `916 → 925`
  //      那次漏改的口径）。
  // 930：会话收件箱（`<sid>/inbox` 集合 + 空间自驱动消费者，2026-09-24）
  //      ——实测 930。**+6 用例**为本批新增：`session/inbox.test.rs` 四条
  //      （FIFO 顺序与地址身份 / 两种取消的分界 / **忙则排队** / 请求参数与 workdir
  //      随条目带走）+ `plugin/nodes.test.rs` 两条（收件箱写入的两种形状判别 /
  //      条目节点与消息节点同源）；另有两条既有用例随 `internal_dirs` 多一段而改数值。
  // 926：**VDFS 收敛 + `Move` 整条下线**（2026-09-24）——实测 926，**净 −4**。
  //      逐文件 diff（`git show HEAD:<f>` 数 `#[test]` / `#[tokio::test]`，不是估算）：
  //        −2  `plugins/vdfs/fs.test.rs`——跨半移动被拒 / 半内移动转发两条随
  //            `UnifiedFs::dispatch` 不再分流删除（**跨半在结构上不可能发生**：
  //            `VdfsRequest` 载荷里已没有第二个地址字段）
  //        −2  `plugins/vdfs/host.test.rs`——三条 move 用例（同类别转发 / 跨类别拒绝 /
  //            跨半拒绝）删除，补回一条 `unimplemented_ops_surface_as_not_implemented`
  //            把「部分实现的 provider 未实现操作原样穿出」这条**原有不变式**重新钉住
  //            （不提高要求，只是换载体——原载体 `Bare` 只被那三条用到）
  //        −1  `plugins/vdfs/physical.test.rs`——`do_move` 的 rename 用例
  //        −2  `plugins/vdfs/provider.test.rs`——`ToolVdfs::move_item` 的两条
  //        +1  `plugins/composite/vdfs.test.rs`——`rejects_unknown_dir`（原用例删掉
  //            跨目录 move 断言后，未知名一条留作纯拒绝面）
  //        +1  `symbio_core/vdfs/contract.test.rs`——新增
  //            `new_type_carries_optional_import_entry`（类型内挂可选导入入口）；
  //            另有两处改名不计数（`request_variant_set_is_the_operation_surface`、
  //            `new_type_is_single_dir_scoped_and_omitted_when_absent`）
  //        +1  `plugins/mcp/plugin.test.rs`——原「新建类型」一条拆成「主入口」与
  //            「导入是同一类型的第二条入口」两条
  //      ⚠️ 删的是**形态**不是覆盖：被删守卫所守的形态（跨半 / 跨子目录移动）
  //      已随 `VdfsRequest::Move` 消失而**结构上不可能**，比运行时判前缀再拒绝更强。
  //      930 → 926 的缺口全部由上面这批解释，没有「测试被悄悄跳过」。
  // 928：**导入改走详情页动作 + `VdfsNewType` 收成纯呈现定义**（2026-09-24）
  //      ——实测 928，**净 +2**。逐文件 diff（`git show HEAD:<f>` 数 `#[test]` /
  //      `#[tokio::test]`）：
  //        +2  `providers/vdfs_service/pack.test.rs`——新增 `unpack_mirrors_pack`
  //            （出向 `VdfsPack` 序列化后去掉 `id` 即入向载荷，钉住往返契约）与
  //            `unpack_rejects_malformed_payload`（`None` / 字符串 / 缺 `filename` /
  //            缺 `b64` 一律报同一句）
  //        +1  `plugins/agent/host/detail.test.rs`——新增 `import_is_offered_in_draft_only`
  //            （同一份定义服务两种态：`when: {is_existing: false}` 成立、`true` 不成立）
  //        −1  `plugins/mcp/plugin.test.rs`——`import_is_a_second_entry_of_the_same_type`
  //            随「类型内不再有第二种入口」删除
  //        0   `symbio_core/vdfs/contract.test.rs`——`new_type_carries_optional_import_entry`
  //            改写为 `new_type_is_pure_presentation`（钉住序列化键集合恰好是呈现字段）
  //        0   `plugins/skill/plugin.test.rs`——`import_name_of_strips_zip` 改写为
  //            `unpack_name_comes_from_the_filename`（换成 `VdfsUnpack::name_of` 的载体）
  //        0   `plugins/model/plugin.test.rs` / `plugins/session/plugin/vdfs_provider.test.rs`
  //            / `plugins/mcp/detail.test.rs` / `plugins/skill/detail.test.rs`——只在既有
  //            用例里删/加断言，条数不变
  //      ⚠️ 净增是预期的：删掉的是「导入是一种入口形态」的用例，补上的是「导入是
  //      动作」与「解包载荷往返」的用例——**换的是形态，不是覆盖**。
  // 957（2026-09-25）：**两级一并补齐**，因为上一批漏了同步。
  //      ① 上批 `b191963`（插件体系卫生改进）实测 **952**、基线仍记 928 ⇒ 滞后 24，
  //        提交信息写了「952 通过」却没改这里的数字（与 `916 → 925` 那次同型）。
  //      ② 本批（ADR-032：插件身份归 manifest）**+5**，逐条：
  //        +4  `symbio_core/plugin/dir.test.rs`——身份四则
  //            （从未落位 ⇒ 空 / 投影**只补缺失键**、用户改过的不覆盖 /
  //             键都在 ⇒ 不碰文件 / 写配置不得冲掉身份与装配位）
  //        +2  `plugins/composite/registry.test.rs`——
  //            `identity_comes_from_manifest_even_when_not_mounted`
  //            （这是 ADR-032 要锁的那条：**停用插件也有身份**）+
  //            `mounting_seeds_the_identity_into_the_manifest`（走**真实** `mount_all`：
  //            真目录 → 真工厂 → 真构造，锁住「装配期真的投影了」——只测
  //            `seed_identity` 本身挡不住那条调用链断掉）
  //      另有两条改名不计数：`unconstructed_entries_have_no_meta` ⇒
  //      `identity_is_empty_for_never_seeded_dirs`（语义从「未构造」收窄为「从未落位」）。
  // 952：删「为兼容旧历史而存在」的代码（2026-09-26）——**净 −6 用例**（958 → 952）。
  //      删掉的生产面：agent 的 `migrate.rs` 模块、model 的 `load_with_legacy` 一族
  //      迁移方法、mcp 的 `migrate_from_legacy_config`、home 的 `migrate_legacy_config`、
  //      skill 的旧标题回落、`PluginDir::remove_keys`；判据面则把「合法 Agent 目录」
  //      收口到 `store::load_record` 一地（`manifest::load` + `manifest::validate`），
  //      并据此删掉 vdfs 上 6 处「列不出来却能浏览」的回退分支。
  //      −10：agent `migrate.test.rs` 整文件（2）、`write_item_rejects_oversized…`（1）、
  //      model 6 条迁移保全、`remove_keys_drops_legacy_fields_but_keeps_identity`（1）。
  //      +4：agent 新增「过不了 §10 门槛的整包拒收」「非 v2 目录一处也浏览不到」
  //      「挂载根只列通过门槛者」，model 新增「启动加载填满注册表与镜像」。
  // 958：把上一条删掉的那 6 条**补回来**（2026-09-26）——952 → **958**，全部来自
  //      `3a25987`（三处根因修复）：`plugins/local/policy/mod.test.rs` **+4**（限流
  //      窗口 `checked_sub` 下溢的两条路径 × 正反例）与 `plugins/session/active.test.rs`
  //      **+2**（压缩永久失败熔断）。逐文件核对方式：`git diff 45466fc..HEAD -- symbio/src`
  //      数 `#[test]` / `#[tokio::test]` 的增删（+6 / −0），不是估算。
  //      本格此前滞后一格：那批修复只跑了定向测试，基线没跟着改；本次门禁全量实测对齐。
  // 960：共享内核收口 P2（2026-09-26）——**净 +2**（958 → 960），工具调用累积器
  //      `TurnToolCallAccumulator` 从 `symbio_core/llm/turn.rs` 迁到
  //      `plugins/model/tool_accumulator.rs`，测试**净迁移 0**（10 条随迁、4 条留 core），
  //      新增两条锁住本次收口的形状不变式：
  //        +1  `plugins/model/tool_accumulator.test.rs` 的 `finish_sorts_by_wire_index`
  //            ——`finish` 按 wire index 升序收口（`HashMap` 迭代顺序随进程随机，
  //            不排序则同一批工具的执行顺序在两次运行间漂移）。
  //        +1  `symbio_core/llm/turn.test.rs` 的
  //            `into_messages_carries_tool_calls_from_the_result_field`
  //            ——`TurnOutput.tool_calls`（结果形态）是落库的唯一来源，节点 id 必须
  //            原样落成 ToolCall 子节点（此前 id 由 `get_completed()` 二次产生）。
  // 959：共享内核收口 P3（2026-09-26）——**净 −1**（960 → 959），`max_seq` / `assign_seq`
  //      从 `symbio_core/schemas/session/chat_message.rs` 迁到
  //      `plugins/session/chat_session.rs`（`seq` **字段**是跨栈 schema、留 core；
  //      「怎么补号」是存储写入边界的策略，消费者只有 `chat_session.rs` 内部）。
  //      逐文件核对（`git show HEAD:… | grep -c`，不是估算）：
  //        −7  `symbio_core/schemas/session/chat_message.test.rs`（10 → 3）
  //            —— 7 条 `assign_seq_*` 随实现迁出，core 只剩 3 条契约用例。
  //        +6  `plugins/session/chat_session.test.rs`（19 → 25）
  //            —— 5 条迁入（顺序 / 稳定 / base 语义 / 开头段 / 末尾段）
  //               + 1 条新增 `assign_seq_on_empty_list_returns_base`。
  //      同批还删了两处死代码（不增减用例）：`assign_seq` 的「夹缝无号项 → 整表重排」
  //      兜底（输入不可达，且修法本身破坏「既有序号原样保留」不变式，改由末尾
  //      `debug_assert!` 守前置条件）；`close_turn` 的 `finish.is_length() && had_tool`
  //      分支与 `TurnResult.had_tool` 字段（该意图已由 `tools/tool_executor.rs` 的
  //      `parse_error` 分支承担，且更靠前——拒绝执行 + 以协议错误回报模型）。
  // 961：ADR-038 单消费方符号下沉（2026-09-26）——**净 0**（959 → 959），逐文件核对：
  //        −5  `symbio_core/llm/turn.test.rs` 整文件删除（构造器与 `impl TurnOutput`
  //            随迁 `plugins/session/message_build.rs`）。
  //        −7  `plugins/model/message_builder.test.rs`（21 → 14）——用例 A / A2 / B /
  //            C / D 与两条流式 id 复用回归迁出；E / F 两条合同测试改本地同形状 fixture。
  //        +12 `plugins/session/message_build.test.rs`（新建）——上两条之和。
  //            计数口径不变（ADR-023 不豁免测试），迁的是**消费方那一侧**的 fixture。
  //      同批 +2（959 → **961**）：`plugins/session/compression.test.rs`（37 → 39）——
  //        压缩快照模板新增 `<next_step>` 节与 `Decision:` 前缀约定（含渲染映射回归），
  //        与本次下沉无关，一并实测入基线。
  // 2026-09-28（同日第二笔）：+4（961 → **965**）。`plugins/session/context/pipeline.test.rs`
  //      新增两条：压缩节点 `meta` 的结构化交代（`compression_stats`）——字段齐全时
  //      五项都要在，且**成功才有「压缩后水位」**（失败 / 未触发时编一个 0，界面会
  //      显示成「水位已降到 0」）。
  // 969：2026-09-28（同日第三笔）：+4（965 → **969**）。压缩摘要的**增量改道**与其
  //      约束位序列化——四条各钉一处：
  //        +2 `plugins/session/context/pipeline.test.rs`：`gate_passes_only_pure_delta_frames`
  //           / `gate_keeps_delta_only_and_strips_meta`——白名单只放行纯增量帧
  //           （Turn 骨架与带状态帧一律吞），改道后落点即压缩节点。
  //        +1 `symbio_core/exec/tests.rs`：`sink_filtered_rewrites_and_drops_per_frame`
  //           ——`ExecEventSink::Filtered` 吞掉的帧不进写入点、不记进度；放行的按
  //           **改写后**的帧进（进度只计放行帧）。
  //        +1 `symbio_core/schemas/detail.test.rs`：`absent_constraints_do_not_serialize_as_null`
  //           ——三个约束位缺席时不写 `null`（写成 `null` ⇒ 整条恒假、界面静默
  //           少一个动作，T13 清单节「进入下一级」曾整条消失）。
  // 2026-09-28：969 → **976**（+7）。`work` 并入 `memory`（ADR-040）：memory 插件
  //           重写/新增 4 件测试（plugin 9 例 / vdfs 11 例 / workspace 8 例 / config 4 例），
  //           work 的旧测试随目录删除（净变化 +7）；agent 侧穿越用例改指工作区腿、
  //           config.test 补 `workspace_enabled` 字段。
  // 2026-09-29：976 → **989**（+13）。补充整合（G-A）：用户在会话运行中连发的多条消息
  //           抽干后合并成**一条**用户消息（不再"一条消息 = 一轮"）。
  //           `transcript/supplements.test.rs` 新增 8 例（`merge_supplements` 的纯函数形状：
  //           空批次 / **n=1 原样返回且一个字段都不多** / 按入队序空行拼接 / 三个 meta 标记
  //           / 不改写用户原文 / 缺 content 当空串 / 非文本片段走 `Parts` 无损 / 全文本仍为文本）；
  //           `transcript/inbox.test.rs` 新增 5 例（抽干**整队** / 上界截断且余者留队 /
  //           平凡值退化为一条 / 取走的每条都在**自己的地址**上发 `bare` 变更 / 上界 0 夹到 1）。
  // 2026-09-29：989 → **1000**（+11）。对话面插件拆分 S1（骨架 + 契约）：新增
  //           `plugins/triage`（6 例：平凡判决恒 Escalate / `Verdict` 带判别键的枚举形态 /
  //           缺载荷报错 / 未知子命令 NotFound / traverse 不贡献工具 / meta 首参 == 目录名）
  //           与 `plugins/reply`（5 例：三个判决变体都走同一条平凡路径 / 缺载荷报错 /
  //           未知子命令 NotFound / traverse 不贡献工具 / meta 首参 == 目录名）。
  // 2026-09-29：1000 → **1034**（+34）。对话面插件拆分 S2（判决：规则短路 + 快速档）。
  //           `plugins/triage` 6 → **29**（+23）：`rules.test` 7 例（含**反例**——"你好，
  //           帮我读一下 README" 不得命中问候，误判会静默吞掉真实请求）、`classify.test` 9 例
  //           （四选一映射 / 仅 work 升级 / 噪声容忍 / 不可解析 ⇒ None / 空节点剔除）、
  //           `config.test` 3 例、`plugin.test` 4 → 10（**全部不依赖模型**：规则命中与
  //           无能力访问器时的升级方向，即"零 LLM 往返"的等价证明）。
  //           `session/context/conversation_view.test` **7** 例（对话线投影：只留
  //           `role=user` 与根级文本 assistant；工具结果 / Turn / reasoning 一律不进）。
  //           `model/message_builder` 14 → **16**（+2：`meta.exclude_from_context` 剔除，
  //           **C-D3**）。`session/config` 4 → **6**（+2：`triage_enabled` 出厂默认 `false`、
  //           缺键落默认）。
  // 2026-09-29：1034 → **1062**（+28）。对话面插件拆分 S3（措辞：模板 + 答话生成）。
  //           `plugins/reply` 5 → **22**（+17）：`templates.test` 8 例（抄本一致性逐字比对
  //           `triage::reasons` / 每个模板码都有行 / `from_context` **不得**有行 / 表无空行
  //           无重复 / 两变体各查各的 / **兜底随变体而不同** / `Report` 无模板 / 空输入是
  //           正常行）、`compose.test` 5 例（投影原样转发 / 空正文剔除 / `non_empty` /
  //           无模型服务 ⇒ None / 无用户发言 ⇒ None）、`plugin.test` 4 → 10（模板理由码
  //           零 LLM 往返 / `Answered` 恒有话说 / 未知码按变体兜底 / `Report` ⇒ 空串）。
  //           `session/chat_loop/compose.test` **10** 例（落点与两个标记：根级 / 被
  //           `conversation_view` 认下 / Turn 子文本**不**算对话线 / `Answered` 不设
  //           `exclude_from_context` / `Escalate` 设 / 两标记相互独立 / 理由码原样透传 /
  //           正文原样 / `surface` 字面量是跨端契约 / id 唯一）。
  //           `session/config` 6 → **7**（+1 净增：两个开关出厂默认翻 `true` + 相互独立，
  //           替换掉 S2 的 `triage_is_off_by_default`）。
  // 2026-09-29：1062 → **1078**（+16）。对话面插件拆分 S4（中途汇报：轮边界判定 + `reply` 填表）。
  //           `session/chat_loop/progress.test` **7** 例（`ProgressPolicy::due` 的逐条件钉法：
  //           四条件全满足 / 关掉总开关 / 差 1ms 不报 / 轮次不够 / 配额用完 / 配额为 0 /
  //           负静默时长不报——只验"全满足时汇报"在"恒返回 true"的实现下也通过）。
  //           `session/chat_loop/compose.test` 10 → **12**（+2：`Report` 节点被对话线认下且带
  //           剔除标记 / `reason = "progress"` 字面量是跨端契约）。
  //           `session/config` 7 → **10**（+3：汇报出厂默认开启且三个上界都有意义 /
  //           缺键落默认 / 汇报开关与另两个独立）。
  //           `plugins/reply/templates.test` 8 → **11**（+3：`progress_text` 把现状说进
  //           句子 / `tool_rounds=0` 不说"已完成 0 轮" / `humanize_ms` 的分档与边界；
  //           另有 1 例改名：`report_has_no_template_yet` → `report_has_no_template_row`）。
  //           `plugins/reply/plugin.test` 10 → **11**（+1 净增：`Report` 的填表产线零 LLM
  //           往返 + 恒有话说，替换掉 S3 的 `report_yields_no_dialog_text_yet`）。
  // 1079（S5，2026-09-29）：**对话线分界改判据**——`1078 → 1079`，**+1**。
  //      改动本身很小，但它是 S5 的地基：`conversation_view` 原先按「根级」切，
  //      把**助手正文**（`Turn` 的子 `Text`，用户真正读到的那段回答）挡在对话线外
  //      ⇒ 插件看不到自己上一轮答过什么，前端「对话」面板也看不到回答本身。
  //      判据改为**只看角色与类型、不看 `parent_id`**（位置不参与分界）。
  //      `conversation_view.test` 8 → **9**：新增
  //      `turn_child_answer_text_is_on_the_conversation_line`（正面边界，与
  //      `tool_and_turn_and_reasoning_are_excluded` 的逐类排除一正一反）；
  //      另有 2 例按新规则改写（`projection_keeps_only_the_conversation_line` 期望
  //      由 `["u1","f1"]` 变 `["u1","t1","f1"]`、`limit_keeps_the_tail` 的 `all.len()` 3）。
  //      `chat_loop/compose.test` 12 不变：`dialog_node_lands_on_the_conversation_line`
  //      改名并加 `parent_id.is_none()` 字面断言、`a_turn_child_text_is_not_...`
  //      反转为 `..._is_...`，**净增 0**（改的是判据不是数量）。
  // 1089（S6，2026-09-29）：**模型槽单槽 → 生效者 + 目录**——`1079 → 1089`，**+10**。
  //      原以为 S6「架构改动：无」，核对后发现 `CapabilityVisitor` 的模型槽是**单槽**
  //      （`provider: Arc<RwLock<Option<...>>>`，重复注册即覆盖），插件只能拿到会话选定的
  //      那一个 ⇒「各插件独立模型」在代码里不成立。改法：`register_model_providers(active_id,
  //      available)` 一次注册合成（**拆两次注册**会允许"生效者不在目录里"这个没有错误信号的
  //      非法状态）；`get_model_provider_by_id` 严格查找；`resolve_model_provider(preferred)`
  //      带默认实现 ⇒ 降级方向只有一处定义。温度**不新增字段**（它本就是 provider 条目
  //      `<根>/model/<id>/provider.json` 的参数，换条目就是换温度）。
  //      `providers/collectors/tool_visitor.test` 3 → **6**：目录按 id 严格查 /
  //      生效者不在目录里 ⇒ None（不是"降级到别的"）/ 空注册 ⇒ None / 重复注册替换目录。
  //      `plugins/triage/config.test` 3 → **5**（+2：`model` 与 `system_prompt` 从配置来 /
  //      空串与缺席同义——设置页清空输入框得到的是 `""`，不当成"配了个空提示词"）。
  //      `plugins/triage/plugin.test` 8 → **9**（+1：traverse 声明自己的配置，
  //      条目名 = 目录名、`path == "triage/PLUGIN.yml"`、字段键顺序
  //      `["rule_shortcut","model","system_prompt"]`）。
  //      `plugins/reply/config.test`（**新**）**3** 例：两个键从配置来 / 空串与缺席同义 /
  //      默认全 `None`；`plugins/reply/plugin.test` 11 → **12**（+1：traverse 声明配置，
  //      字段键顺序 `["model","instruction"]`）。**只覆盖指令段**：注册段（`build_system_prompt`
  //      里排在前面那段）永远在，否则 `instruction` 认空串时答话会失去全部约束且日志正常。
  // 1166（S5–S7，2026-09-30）：**core 事件网格三阶落地**——`1089 → 1166`，**+77**。
  //      S5 记忆：事件格子 memory.encoded/consolidated/forgotten/recalled；
  //      ③ recall 投影（as-of / 排除式遗忘 / 预算降级 / thread_private 跨主体隔离）
  //      + consolidate 接受边界（保真度下界 N8）；② RecallTranslator。S06 §6 四条
  //      验收 + 溯源覆盖 + 全链路绿。
  //      S6 多主体：事件格子 commitment.opened/released/broken/asserted；
  //      ③ reputation 投影（as-of 与 recall 同口径、打分是参数）；② CommitmentKeeper。
  //      S08 §6 验收 1/3/4 落地（成对拒绝在 S3 区）。
  //      S7 任务树：事件格子 task.opened/progress/held/asserted/rework_created；
  //      ③ readyset 投影（纯函数就绪集，N1 双跑一致）；C14 acyclic_deps（Kahn +
  //      悬空）；C15 SelfVerifier 构造期拒绝；rework_bounded（终止性前提 2）。
  //      S03 §6 验收 1–4 全落地。
  // 1175（S8，2026-09-30）：**时延与插话**——`1166 → 1175`，**+9**。
  //      事件格子 control.opened（打断与熔断共用一格，载荷 reason 区分）；
  //      ② PreemptionDecider（80ms 反射档：Proceed/Suspend/Queue + 超时默认继续）
  //      + CircuitBreaker（Refuse 零事件 / Break 必须落事件 / Allow）；
  //      ③ readyset 补挂起排除（held 不进就绪集——二次调度 0）与 cost_ledger
  //      投影（熔断判据来源，N1 双跑一致）；governance 高风险组合
  //      （ProduceArtifact+AssertVerification）构造期拒绝。S07/S09 §6 验收落地。
  // 1181（S9，2026-09-30）：**自治与学习（22 步收官）**——`1175 → 1181`，**+6**。
  //      事件格子 system.triggered / system.health / conation.expressed；
  //      ② AutonomousInitiator（自主层 = budget_ms 第四取值 86.4M，非新架构层）+
  //      IntentGate（欲→行的 ZST 闸门：CandidateIntent / GateWarrant / 关停开关）+
  //      SkillCompiler（技能=memory.encoded{tag:skill}，溯源 100%）+ SkillRouter
  //      （反自动化回退）；③ calibration 投影。S12/S11 §6 验收逐条落地。
  // 1184（真实模型接线，2026-09-30）：**ProviderLlmAdapter 全链路**——`1181 → 1184`，**+3**。
  //      ⑤ LlmAdapter 端口的真实实现（包 ModelProvider，S2 预留接线点兑现）：
  //      真实 TCP/HTTP/SSE 全链路彩排（Reasoner 持令牌 → execute_turn → final 落
  //      事件 → 不变量绿）+ 空流按失败（I3）+ 不可达端点映射。
  // 1185（SLO 校准，2026-09-30）：**埋点 + 首测**——`1184 → 1185`，**+1**。
  //      埋点：LlmAdapter::generate_timed（trait 默认，adapter 边界实测）+
  //      Reasoner::reply_timed；final 事件带实测 cost_ms → cost_ledger 累计 →
  //      熔断判据闭环（G3 输入接实测）。校准：真实 HTTP/SSE 12 轮实测系统
  //      自身开销 P50=1ms / max=3ms（G2 首测，四层预算初值保留为正式值），
  //      守卫 reflex_tier_system_overhead_is_measured_and_bounded 钉住两档下界。
  // 1187（兜底率统计，2026-09-30）：**SLO §1.2 兜底率列的口径落地**——`1185 → 1187`，**+2**。
  //      ③ fallback_rate 投影（按档统计兜底率；turn 档位 = 用户消息载荷 tier，
  //      缺失进 unspecified 桶可观测）+ LatencyTier::name/from_name 往返。
  // 1191（turn 运行器 + WAL seq 修复，2026-09-30）：**chat_loop 切换的第一块**——`1187 → 1191`，**+4**。
  //      ② TurnRunner（actors 域）：单轮 = 用户消息入格（tier 随载荷）→ Reasoner
  //      生成（实测耗时）→ final/fallback 落格——N3/N5/实测成本靠构造成立；
  //      I3 失败也落事件。真锅：WalStore 落盘行 seq 全 null（assign_seq 在序列化
  //      之后）——重开恢复丢序，N2 静默破裂；修复 = 赋 seq 先于序列化，
  //      重开 check_all 绿（此前 S4 测试没走到这条所以没炸）。
  // 1193（转写投影，2026-09-30）：**多轮对话的历史侧**——，**+2**。
  //      ③ transcript 投影（user.message→user 行 / final→assistant 行 /
  //      fallback→assistant+why——模型知道自己说过兜底话术）；
  //      Reasoner::reply_timed 的 prompt 改从转写出（多轮带历史，
  //      单轮裸文本等价）——历史来自同一事实源，不另存副本（ADR-044 同族）。
  // 1198（v2 事实桥，2026-09-30）：**session 插件接线**——`1193 → 1198`，**+5**。
  //      v2_bridge：chat_loop 收束点（finish_turn）把每轮事实（用户发言/助手
  //      答复/实测耗时）转写进 per-session 的 v2 WAL——v1 行为零变化，纯增量
  //      记录；P99/兜底率从此有生产数据源。Aborted/ResumeDone 不转写（诚实
  //      缺口）；桥故障只 plugin_warn 不冒泡（不得拖垮 v1 对话）。
  // 1201（slo_report 投影 + 阶段三扫描口，2026-09-30）——`1198 → 1201`，**+3**。
  //      时延列的正式口径：final 实测 cost_ms 按 turn 归档（declared_tier
  //      单源共享，兜底轮不混样本）；slo_scan_wal_roots（opt-in）walk 会话
  //      存储根的 v2-events.wal，跨会话合并 P50/P95/P99 + 兜底率——
  //      v2 事实桥写、扫描口读，三列同源闭环（ADR-044/045）。
  // 1203（v2_mode 总开关，2026-09-30）——`1201 → 1203`，**+2**。
  //      ADR-045 过渡期的迁移总开关：off（纯 v1）/ bridge（默认，v1 运行
  //      + 事实累积）；full 档待 v2 引擎切换落地时在同一枚举增设——
  //      「切到 v2 的哪一步」一个问题一个旋钮，不拆多个开关。
  // 1205（流式生成，2026-09-30）——`1203 → 1205`，**+2**。
  //      `full` 档的硬缺口补上：LlmAdapter::generate_streaming（默认 =
  //      一次性全文的诚实降级）+ DeltaSink/SilentDeltas；Reasoner/TurnRunner
  //      收成单路径（run = run_streaming + SilentDeltas）；ProviderLlmAdapter
  //      帧桥（快照记账 + 窄帧转发，reasoning/工具增量不进 v2 文本面）——
  //      SSE mock 验收增量拼接 = 聚合全文。流式只是帧的形态，收束语义不变。
  // 1207（v2 执行器，2026-09-30）——`1205 → 1207`，**+2**。
  //      切换日本体：`v2_mode = full` 且无工具挂载的轮次经 v2 运行器执行
  //      （事实原生入格 + prompt 从转写出 + 流式经 UiBridge 回同一出口）；
  //      验收 = v2_exec.test 成功轮（网格两格 + UI 帧同构）/ 失败轮（I3 兜底格）。
  //      适配器搬家：ProviderLlmAdapter → symbio_core/adapters（session 直引
  //      兄弟插件违反 E-009）；全链路测试留在 plugins/model。
  // 1209（中止语义，2026-10-02）——`1207 → 1209`，**+2**。
  //      v2 路径的中止不再是「失败」：`AdapterError::Aborted` 与
  //      `GenerationFailed` 在类型上分开（压成一个变体，消费方只能靠错误
  //      文本猜）；`TurnOutcome.aborted` 让运行器**不落收束格**（网格少一格
  //      是诚实缺口，ADR-045 同源），适配器保真映射 `PluginError::Aborted`
  //      → `AdapterError::Aborted`，v2_exec 上抛 `PluginError::Aborted`，
  //      chat_loop 走独立 Aborted 出口。验收 = core 中止轮（只剩用户格 +
  //      C4 判得出缺口）/ session 中止轮（Aborted 上抛 + 网格一格）。
  // 1213（工具轮 v2 化，2026-10-02）——`1209 → 1213`，**+4**。
  //      工具通道 / 分发通道 / `artifact.added` 格子三处契约补齐，v2 运行器
  //      在类型上做得了工具轮：core 新增 LlmTurn + DispatchPort/DispatchOutcome +
  //      EVENT_ARTIFACT_ADDED；`run_with_tools` 工具循环（产物落格带溯源 /
  //      中途正文定格切节点 / 等待用户不落收束格）；session 侧
  //      `SessionDispatchPort` 复用 `process_tool_calls_async`（工具节点与结果
  //      仍写进同一份对话图），full 档放开 `tools.is_empty()`。
  //      （端口名不带 `Tool` 前缀：`Tool*` 是 `capability` 域的子命名空间，
  //      两只端口与 `LlmAdapter` 同域 —— core-naming-audit N-003 的判据。）
  //      验收 = core 工具轮 3 例（产物格 + 溯源 / 等待用户不落格 / 无分发方兜底）
  //      + e2e t26（MCP echo → `/_requests` 回读结果进下一次请求 + WAL 溯源）。
  // 1214（审批与恢复·机制，2026-10-02）——`1213 → 1214`，**+1**。
  //      等待轮的恢复**续写同一轮**（不新开用户格、收束仍记在原 turn 上）：
  //      core 新增 `TurnResume` + `TurnInput.resume`（`run_with_tools` 续写分支：
  //      复用既有用户格 seq / 续编号产物格 / 种子交换段）；插件侧
  //      `ResumeOutcome::Continue{resumed}` 把恢复产生的工具交换交回 chat_loop，
  //      `v2_exec` 定位已开未收束轮次并落 `artifact.added`。判据是 C4
  //      （`unresolved_turns` 按 turn 号配对）：另开新轮会把原轮变成永久假缺口。
  //      验收 = core `resume_continues_same_turn_without_reopening_user_cell`
  //      （缺口被填 + 用户格只一格 + 恢复交换进 prompt）。
  //      注：e2e `t27` 待 CLI 恢复入口（`cli/src/client.rs` 现写死 `resume: None`），
  //      属批 2b，不在本次基线内。
  // 1220（读数口 `session/stats`，2026-10-03）——`1214 → 1220`，**+6**。
  //      读侧出口接进生产路由：core 根导出 `slo_report` / `checkpoint` +
  //      `WalStore::open_readonly`（只读开档：不创建、不截断撕裂尾行），
  //      `plugins/session/stats.rs` 一次给四列（时延/兜底/成本/断点）。
  //      写侧判据同步订正：`v2_facts::first_user_utterance` 此前只认
  //      `status = Completed`，而 `chat/send` 的用户消息不填该字段 ⇒ 转写恒
  //      不发生且**无告警**，事实源根本不曾存在（详见 04 §3.1 S2 行）。
  //      验收 = stats 3 例（复算 / 反向手术 / 缺源不创建）+ wal 只读 2 例
  //      + `first_user_utterance` 真实形状 1 例 + e2e t28（四列对账 + 两刀反向）。
  // 1223（不变量进读出口，2026-10-03）——`1220 → 1223`，**+3**。
  //      `check_all` 从「只被测试调」变成**读出口的第五列**（`session/stats`
  //      的 `invariants`，空 = 全部断言通过）：C4 未收束 / C5 超预算并进
  //      `check_all`，两条 `dead-code-allow` 摘除（04 §3.1 批④）。判据为
  //      **读侧口径**的宽限——首日不假红：C4 放行切片尾轮在途（切片无
  //      wall-clock，「在途」与「卡死」无从分辨；被后续轮越过的照样报），
  //      C5 只在声明过档位时判、取最宽一档预算。严判档走形参
  //      （`unresolved_turns(_, false)` / `budget_exceeded(_, Some(n))`）。
  //      验收 = `read_side_graces_the_trailing_open_turn_but_flags_the_overtaken_one`
  //      + `read_side_budget_follows_declared_tiers_and_skips_when_none_declared`
  //      + stats `invariants_move_when_the_wal_changes`（两刀后清单跟着红/回落）。
  // 1222（Decider 族退役，2026-10-03）——`1223 → 1222`，**−1**。
  //      规则应答器（`Decider` / `DeciderMiss` / `new` / `rehearsal` /
  //      `respond`）生产零调用 ⇒ 整族删除（04 §3.1 批⑤）。专属用例
  //      `decider_reads_the_last_user_message_from_events` /
  //      `decider_miss_reports_the_utterance` 随之退役，**输入契约改由生产形态
  //      承接**（`reasoner_reply_reads_the_user_message_from_events`，+1）：
  //      净降 1 不是「删测试放行」，而是删掉的两条只测已删除的类型。
  //      `rehearse_turn` 的规则表换成彩排内定值，链路形状断言一条不少。
  // 1232（授权矩阵接线，2026-10-03）——`1222 → 1232`，**+10**。
  //      读写两道闸进生产（04 §3.1 批⑥）：矩阵**生产构造**住宿主表
  //      `symbio/src/authz.rs`（能力名字符串 ⇒ `PermissionMatrix::from_names`
  //      按 7 项闭集校验、失败降级空矩阵；`PRINCIPAL_MAIN` / `PRINCIPAL_USER`
  //      与事件 actor 同源，判的对象 = 写的对象）；写侧
  //      `v2_facts::authorize_close`（收束入格前 `can_reply`，轮次→能力映射
  //      只在 core）；读侧 `session/stats` 载荷声明 `principal` 才 `can_see`
  //      （属主全量、矩阵外读数为空，不声明 = 今天行为逐字不变）。
  //      验收 = governance 3 例（`from_names` 成功 / 认不出的能力名**拒绝整体**
  //      构造 / `can_reply` 映射）+ authz 4 例（表形状 / 写读两侧 / 降级空矩阵）
  //      + stats 2 例（属主全量 / 非属主全零）+ v2_bridge 写闸 1 例 + e2e t28
  //      读方三态（`principal` = user / agent:main / 未知 ⇒ 4 格 / 0 格 / 0 格）。
  // 1243（记忆三段接线，2026-10-04）——`1232 → 1243`，**+11**。
  //      步 11–13 全部接入（04 §3.1 批⑦）：写方 `v2_facts::record_to_wal` 收束时
  //      编码本轮用户发言（同文去重判定方 = core 的 `contains_content`）、把本轮
  //      检索落成 `memory.recalled`、再过 `consolidate::accept` 才巩固；读方
  //      `prepare_turn_inputs` 首轮扫跨会话事实源、`build_request_view` 置顶注入。
  //      验收 = v2_memory 8 例（编码溯源/去重/截断、巩固三态、跨会话召回与排版、
  //      检索事实幂等与 actor 对齐、合并算法两条边界）+ 桥三段接线 1 例 +
  //      请求视图置顶注入 1 例 + e2e `t29`（记忆进 prompt、跨会话召回、巩固可见）。
  // 1266（任务表，2026-10-04）——`1252 → 1266`，**+14**。
  //      04 §3.1 批⑨ 全量接线：`local/todo_write` 输入 schema 增 `depends_on`（任务图
  //      是数据）、`note_tasks` 任务表出参 → `TurnState.task_decls` → `v2_facts::record`
  //      → `v2_tasks::write` 逐批入格（opened / progress / asserted / 返工）、
  //      `readyset` 投影进 `stats` 读列、调度段 `prompt_section` → `build_request_view`
  //      置顶注入、`acyclic_deps` / `rework_bounded` 进 `check_all`（五条 → 七条）。
  //      验收 = `v2_tasks` 9 例（就绪判定 / 成环与悬空反向 / 降级不产事实 / 返工轮与
  //      上界反向 / 调度段门控）+ `note_tasks` 3 例（短名判据、只认成功、id 兜底）+
  //      声誉式复算与读侧闸的 `readyset` 列 1 例 + 调度段注入位置 1 例 + e2e `t31`。
  // 1252（身份 / 承诺 / 声誉，2026-10-04）——`1243 → 1252`，**+9**。
  //      04 §3.1 批⑧ 全量接线：身份（`TurnInput.actor` 入参、`authz::principal_of` /
  //      `matrix_for`、`view::visible_to`、`build_request_view(viewer)`、
  //      `ChatMessage.principal` 落库单点 `append_and_publish::attributed`）、
  //      承诺写方（`note_delegation` 四个终态分支 → `TurnState.delegations` →
  //      `v2_facts::record_to_wal` 逐条入格）、声誉读列（`stats::read` 会话主体
  //      第 4 参 + `reputation` 列全有全无）。
  //      验收 = core 可见域 2 例 + 视图按主体过滤 1 例 + `note_delegation` 2 例 +
  //      桥承诺入格 2 例 + 声誉列 1 例 + `attributed` 身份补齐 1 例 + e2e `t30`
  //      （父/子两会话请求包不串主体、代际立约入格、出口读声誉）。
  // 1270（外部执行闸门，2026-10-04）——`1266 → 1270`，**+4**。
  //      04 §3.1 批⑩ 子批 A（S8 步 20）接线：`process_tool_calls_async` 批首读一次
  //      判据（`BreakerInputs`：授权走 `PermissionMatrix::can_write_name` 按能力名
  //      判 `produce.artifact`、已耗走 `cost_ledger` 台账〔ADR-044〕、本次申请 =
  //      深度档预算、总预算 = 自主层预算），随后**每个调用点各判一次**
  //      `CircuitBreaker::gate`；`Refuse` 零事件、`Break` 经 `TurnState.gate_breaks`
  //      出参交 `v2_facts::record` 落 `control × opened`（与承诺 / 任务同锚
  //      `user_seq`）、`Allow` 照旧开窗。判据在批首读而非逐点重放：一次工具批几十
  //      个调用点 × 全量 WAL 重放会让闸门自己变成时延源。
  //      验收 = 判据读矩阵 1 例（非 agent 主体 fail-closed）+ 台账吃满即拒且走出参
  //      1 例 + 放宽预算翻面 1 例（S09 §6.4 反向：预算在生效而非常量）+
  //      桥入格与空出参静默 1 例（S09 §6 验收 1 与 2 两条相反判据）。
  // 1274（插话抢占，2026-10-04）——`1270 → 1274`，**+4**。
  //      04 §3.1 批⑩ 子批 B（S8 步 19）接线：`transcript/inbox.rs` 忙窗**只判不落**
  //      （队列非空 + 一忙窗一判，`Instant` 计时读只读事实源 →
  //      `PreemptionDecider::decide(.., LatencyTier::Reflex.budget_ms())`），空闲分支
  //      **先取批、有插话才落格**（`held_event` + `control_event`，锚 = 落格前
  //      `head`），插话轮结束后落 `resume_event`；状态跨忙窗住在
  //      `ActiveSessionStateInner::preempt`，唤醒条件扩成 `has_pending_work`。
  //      判据输入由 `record_to_wal` 保证：收束发言先于承诺 / 任务入格。
  //      验收 = 忙判 → 空闲落格 → 插话轮 → 收恢复全程 1 例（就绪集出 / 回 +
  //      `check_all` 恒空）+ 反向 1 例（收束发言在任务之后 ⇒ 判为排队、零事实，
  //      证明挂起是判出来的）+ 未结清判定不能睡过去 1 例 + 没有事实源时结清走通
  //      1 例（`Ok(None)` 清状态、`Err` 留着重试，两者分界错一边就是 J3 或空转）。
  // 1286（批⑫ B 类零散契约，2026-10-05）——`1284 → 1286`，**+2**。
  //      `TestConnectionResult` 富提示接线（04 §3.1 批⑫）：`success_message()` 把
  //      名字 / 版本 / 工具数 / 协议 / 耗时拼进 `VDFS_ACTION_TEST` 的 `message`，
  //      整段 `instructions` 走同一结果的 `data`（BUG-MR32 取回了就得有去处）。
  //      验收 = 形状 1 例（五样都在、说明文本不挤进来）+ 缺身份不留空洞 1 例。
  //      同批另三处不产用例——接入面分别是**两处日志**（`TRACE_ID`）、**删除**
  //      （`ROUTE_SESSION_CHAT_ABORT`）与**构造签名**（local 两处 `security` 字段）。
  // 1289（批 0-A 跨进程写者令牌，2026-10-05）——`1286 → 1289`，**+3**。
  //      [plan/11 §3 批 0](../../docs/plan/11-多执行器与多主体加固实施方案.md) 的 ①③
  //      合并子批：`store = wal` 的单写者从**进程内** `RwLock` 升级为**跨进程**写者
  //      令牌。`WalStore::open` 对旁挂锁文件（`<wal>.lock`）取一次非阻塞独占锁
  //      （std `File::try_lock`，Unix `flock` / Windows `LockFileEx`），拿不到即降级成
  //      「非写者」——读照常，`append` 恒返回 `AppendError::NotTheWriter`；截断（撕裂
  //      尾行恢复）也只在持令牌时发生。trait 一行未改（F1 冻结锚点）。
  //      验收 = 两写者交错 1 例（第二个拿 `NotTheWriter` 且盘上零丢失、无重号）+
  //      只读档写入口关闭 1 例 + 令牌随 drop 释放 1 例（接手者的 `head` 来自重放，
  //      不是从 0 重新计数——否则接手即重号）。端到端另在 e2e `t32`（两个真进程写
  //      同一会话），按目录发现式加载，不进本格计数。
  // 1293（批 0-B 单写者观测面，2026-10-05）——`1289 → 1293`，**+4**。
  //      [plan/11 §3 批 0](../../docs/plan/11-多执行器与多主体加固实施方案.md) 的 ②：
  //      初稿「`append` 追加期裁决同一 turn 的 `user.message`」被 `EventEnvelope` 的
  //      边界判死（只暴露 `event_id` / `seq` / `assign_seq`，刻意不暴露 `turn`），而它的
  //      目的已由 0-A 达成（turn 号本就派生自事实源，且这次读现在发生在写者令牌之下）。
  //      残留的是信号缺口：不变量层有 `seq` 轴（`seq_monotonic`）与收束轴
  //      （`final_unique_per_turn`），缺对称的开轮轴。补 `open_unique_per_turn`
  //      （每 turn 至多一条 `u-{turn}`）并入 `check_all`（七条 → 八条）。
  //      验收 = 每轮只开一次不报警 1 例 + 一轮两条开轮被看见 1 例（锚在后落那条、
  //      指出先落那条）+ 同坐标别的 kind 不算用户开轮 1 例 + 经 `check_all` 合跑只红
  //      这一条 1 例。端到端另在 e2e `t28` 加第三刀（复制一格开轮事件 ⇒ `invariants`
  //      列报「每 turn 至多 1 条开轮」），按目录发现式加载，不进本格计数。
  // 1295（plan/12 批 1 校准读列 + 并入的 actor 归位，2026-10-05）——`1293 → 1295`，**+2**。
  //      [plan/12 §批 1](../../docs/plan/12-价值验收与基线埋点.md)：把 `calibration`
  //      投影接进 `plugins/session/stats.rs` 的 `SessionStats` 成第五列（四列 + 校准 /
  //      声誉 / 就绪集），与四列同一份切片、同一个 as-of，`may_read = false` 时同样走空
  //      切片（`{ by_skill: {} }`）。判据 = `calibration_column_recomputes_and_moves_
  //      with_the_wal` 1 例（追加两条 `memory.recalled` ⇒ 复算相等 + `uses` / `fallbacks`
  //      随切片动；非属主 ⇒ 空视图）。端到端另在 e2e `t28` 加第四注（照抄真实行形状注入
  //      `memory.recalled` ⇒ 逐字段断言 `uses` / `fallbacks`），不进本格计数。
  //      同批并入**另一处会话**的 SkillCompiler `actor` 归位修复（写侧前提：技能落格前
  //      归位成属主，否则生产召回视图看不见技能 ⇒ 校准列永远空），其用例
  //      `the_production_recall_view_sees_the_compiled_skill` 1 例计入本格。
  // 1296（plan/11 批 2 ③ 熔断台账读列修路径，2026-10-05）——`1295 → 1296`，**+1**。
  //      补 e2e `t33`（熔断接线）时抓到的**生产 bug**：`BreakerInputs::of` 把
  //      `session_dir` 当会话目录用，而它实际是**插件目录**（`<homedir>/session`）⇒
  //      `session_dir.join("v2-events.wal")` 读的是插件根下不存在的文件 ⇒ `spent_ms`
  //      恒 0 ⇒ 熔断的 `budget-exhausted` 分支永远走不到，且无任何告警（静默失效）。
  //      修法 = 走 `paths::session_dir(session_dir, session_id)`（session_id 取自
  //      `ctx[SESSION_ID]`）。验收 = 台账从**会话目录**读而非插件目录 1 例
  //      （三侧断言：插件目录同名文件不得被读走 / 会话目录读得出账 / 无会话 id ⇒ 零账），
  //      同文件另两条既有用例改夹具（`test_ctx` → `gate_ctx`）不增计数。端到端见 e2e `t33`。
  // 1297（plan/12 批 2 转写读列，2026-10-05）——`1296 → 1297`，**+1**。
  //      [plan/12 §批 2](../../docs/plan/12-价值验收与基线埋点.md)：把 `transcript`
  //      投影接进 `plugins/session/stats.rs` 的 `SessionStats` 成最后一列（七个投影
  //      里最后一个接上生产读出口的），与四列同一份切片、同一个 as-of，
  //      `may_read = false` 时同样走空切片（`{ entries: [] }`，四列形态）。判据 =
  //      `transcript_column_reads_the_conversation_back_from_the_wal` 1 例（复算相等 +
  //      期望值钉死 + 删一格 final ⇒ 那一句从转写里消失 + 非属主 ⇒ 空表）。端到端
  //      见 e2e `t37`（两轮真实对话 ⇒ 出口转写与 WAL 的三格逐条对账，按目录发现式
  //      加载，不进本格计数）。**纯读**：`transcript` 投影早已被 `actors::Reasoner`
  //      消费，无 `#[allow(dead_code)]` 可摘，故不进 04 §3.1 的清偿批次表。
  // 1299（plan/10 批 3 工具事实进转写投影，2026-10-06）——`1297 → 1299`，**+2**。
  //      [plan/10 §3 批 3](../../docs/plan/10-工具轮v2化实施方案.md)：`artifact.added`
  //      （`artifact × asserted`）进 `transcript` 投影，成 `role = "tool"` 行
  //      （另带 `tool` = 工具名；非工具行 `skip_serializing_if` 保持 `{role, text}` 旧形状）。
  //      口径从「三格 → 两角色」变成「四格 → 三角色」——跨轮的 prompt 因此能重建
  //      **含工具**的对话（此前只有轮内交换 `render_tool_exchange`，跨轮就丢了工具结果
  //      ⇒ 模型会重复调用同一个工具）。判据 2 例：① `transcript_includes_artifact_as_tool_line`
  //      （四格 → 四行 + 工具名/正文 + prompt 里渲染成 `工具结果(<tool>): <text>`，与轮内
  //      交换同形 + 线格式：非工具行仍是 `{role, text}`）；② `transcript_column_surfaces_tool_rows`
  //      （补一格工具产物 ⇒ `session/stats` 的转写列浮出工具行）。端到端见 e2e `t38`
  //      （两轮真实工具轮 ⇒ 第三处请求带第一轮的工具结果，按目录发现式加载，不进本格计数）。
  // 1303（plan/11 批 2 ②「子 Agent 注册不覆盖父」判据化，2026-10-06）——`1299 → 1303`，**+4**。
  //      [plan/11 §3 批 2 ②](../../docs/plan/11-多执行器与多主体加固实施方案.md)：该批原写
  //      「`register_*` 单槽改多槽」，核实后**前提不成立**（每次 `collect_capabilities`
  //      各建一个 `DefaultToolVisitor` ⇒ 槽位按收集隔离；同一次收集里子树这两项被
  //      `SubAgentVisitor` 丢弃）⇒ 改为**判据化**：把「子 Agent 不得劫持父会话的根与模型」
  //      从"只有模块文档在说"变成用例钉住。判据 4 例（`plugins/agent/host/scope.test.rs`）：
  //      ① `sub_agent_root_registration_is_discarded`（内层根不变）；②
  //      `sub_agent_model_registration_is_discarded`（内层生效者与**目录**都不变）；
  //      ③ `sub_agent_multi_slot_registrations_join_parent_with_prefix`（工具 / 提示词段 /
  //      挂载点**确实**带前缀并集——挡住「代理什么都不转发」那种假绿）；④
  //      `read_side_forwards_to_inner_unchanged`（读侧透传，防劫持改从读侧发生）。
  //      端到端见 e2e `t39`（钉**根**那一半：`vdfs_read <根>/memory/AGENTS.md` 读到的必须是
  //      `{homedir}/AGENTS.md`；模型那一半端到端钉不住——注册序来自 `HashMap`，见 11 §3）。
  // 1304（会话配置面补齐 6/24 → 24/24，2026-10-06）——`1303 → 1304`，**+1**。
  //      `session/plugin.rs::config_definition()` 此前只登记 6 个字段，而 `SessionConfig`
  //      有 24 个 ⇒ `skill_compile_enabled`（S11 技能编译）/ `conation_enabled`（S12 自主层）
  //      / `v2_mode` 的 `full` 档这些**高阶能力开关在产品里根本开不了**（后端存在、
  //      设置页无控件 = 导航层不可达的虚假实现）。本批把 24 个字段全量登记（5 分区：
  //      基础 / 上下文与工具 / 记忆 / 对话面机制 / 自主与学习），并新增覆盖判据
  //      `config_definition_covers_every_session_config_field`：schema 字段键集合（跨**全部**
  //      sections）必须与 `SessionConfig` 序列化键集合**完全相等**（少 = 开不了，多 = 保存必失败，
  //      重复 = 两控件互相覆盖）。既有 `config_definition_defaults_come_from_session_config`
  //      同步改为跨分区遍历（原只看 `sections[0]`，分区化后后四区无人看守）。
  //      连带：数值字段刻意**不声明** `min`/`max`（边界是策略不是事实，`DetailField::check`
  //      会据此拒收 ⇒ 编边界即造假约束），故 `vdfs_provider.test.rs::config_write_validates_before_applying`
  //      的坏值从「`max_messages: 1`（靠旧 min 边界）」改成「`max_messages: "不是数字"`（形状错误）」。
  // 1305（full 档补上收束派生事实里的记忆与学习，2026-10-06）——`1304 → 1305`，**+1**。
  //      `v2_mode = full` 档下 `chat_loop` 以 `TurnState::v2_executed` 拦下整段
  //      `v2_facts::record`（「同一轮两份记账是假象」），但记忆三段（S5 步 11–13）与
  //      技能观测 / 编译（S11 步 22）**不是轮次事实**、而是派生副作用——`v2_exec` 一处
  //      都不写 ⇒ 这个档位的长期记忆与自我改进**静默全丢**，而档位名还自称「整体切换」。
  //      本批把「记忆 + 学习」从 `record_to_wal` 抽成 `v2_facts::record_learning`
  //      （同一函数、两个调用点：bridge 档经 `record_to_wal`，full 档在 `v2_exec` 轮末
  //      直接调，锚点分别是转写的 `v2u-*` 格与原生写的 `u-{turn}` 格）。仍只走 bridge 档
  //      的（承诺 / 任务表 / 熔断 / 写侧授权闸）已在 `v2_exec` 模块文档**诚实划界**。
  //      +1 = `v2_exec.test.rs::full_turn_lands_memory_and_learning_facts`；端到端判据 =
  //      e2e `t40`（两轮不同发言 ⇒ 逐轮编码 + 逐轮编译 + 第 2 轮判第 1 轮编的技能）。
  // 1309（S11 快路的**执行半边**，2026-10-06）——`1305 → 1309`，**+4**。
  //      在此之前 `SkillRoute::SkillFastPath` 只作用在**提示词**上：判定被算出来
  //      （`route` 逐条推观测），却**从不产生后果**——没有消费方，`issue_reflex()` 在
  //      生产里零消费者。本批把这条判定接出第二个后果：命中一条已编译技能 ⇒ 本轮以
  //      技能正文收束、**一次模型调用都不发生**。执行侧是 core 的
  //      `TurnRunner::run_reflex`：**没有 `llm` 形参**、只收 `RuleOnly` 令牌 —— 于是
  //      「反射档调模型」不是被检测到，而是**写不出来**（`verify/latency_gate.rs` 的
  //      `assemble(Reflex)` 落地形态：反射档结构上装不进 LLM 字段）。开关
  //      `SessionConfig::skill_fast_path` 默认 off；三条边界 = 只对新开轮 / 只在 full 档 /
  //      关着时候选集恒空。`+4` = 核心 `reflex_turn_tests::reflex_turn_lands_open_and_close_with_reflex_tier`、
  //      `v2_skills.test.rs::the_hit_judgement_is_literal_after_the_writers_normalisation`、
  //      `v2_exec.test.rs::{skill_hit_takes_the_reflex_tier_without_any_model_call,
  //      a_skill_hit_never_takes_over_a_resumed_turn}`；端到端判据 = e2e `t41`
  //      （同一句话第二遍 ⇒ mock-llm 请求数不增、`tier = reflex`、以技能正文收束；
  //      第三轮换一句话 ⇒ 请求数照增，把"命中才跳过"与"整档不调模型"分开）。
  // 1313（S10 核实：读侧过滤的**判据化**，2026-10-06）——`1309 → 1313`，**+4**。
  //      核实结论是「**部分成立**」：S10 §5 的架构结论（通用性过滤必须是读侧投影参数、
  //      与 `memory.forgotten` 同构）成立，机制也在生产里；但 §3 那一族 `recall:<键>=<值>`
  //      今天只有 `tag` 有形参，`min_generality` / `density` 连**值的写方**都没有
  //      （分级器是算法问题）⇒ 照抄落地会得到一个**恒真**的假过滤，故不实现，改为判据化。
  //      真实缺口：`tag` 这个**唯一已落地**的实例在生产里零判据——既有用例一律传正好
  //      匹配的标签，过滤从未排除过任何东西。负向自检（把过滤改成恒真）实测：**只有本批
  //      新增的 4 条变红，其余 1309 条全绿** ⇒ 缺口属实。而它是**承重**的：`consolidate`
  //      靠它只合并同标签的记忆，失效时技能会被当经验合并并排除式遗忘（**无任何报错**）。
  //      `+4` = `projection/recall.test.rs::{a_tag_filter_keeps_only_its_own_tag,
  //      widening_the_filter_recalls_what_it_excluded_without_touching_the_log,
  //      changing_the_tag_changes_the_view}`（纯函数三性质：排除 / 可逆 / 活参数）+
  //      `v2_memory.test.rs::consolidation_never_merges_across_tags`（生产后果）。
  // 1320（S12 核实：一处**假成立** + 一处**装饰判据**，2026-10-06）——`1313 → 1320`，**+7**。
  //      核实 S12 §3 的四行增量，结论：事件格子 / 投影参数 / grants 行**三行成立**，
  //      ActorSpec 行只是目标。但「成立」里有两条是**假**的，本批修掉：
  //      ① **grants 行在生产里被静默推翻**——`authz::ROWS` 根本没有 `agent:autonomous`
  //         那一行，而 `matrix_for` 对一切 `agent:*` 都派生主智能体的整套能力集（**含
  //         `reply.*`**）⇒「自主行为不得冒充用户对话」在生产数据里**不成立**，文档与验收
  //         断言却都写着它成立。修法：加 `PRINCIPAL_AUTONOMOUS` 那一行（只持 `define.work`），
  //         并把 `matrix_for` 改成「**表里登记过的主体走表**（表是权威，派生不覆盖它），
  //         未登记的 `agent:<id>` 才派生」；主体名收成常量（表里那一行与
  //         `AutonomousInitiator` 四个事件的 `actor` 是同一个常量——名字分两处写，
  //         那行就成了一条谁也管不到的死记录）。
  //      ② **验收 2 的判据是装饰**——原用例自造 `PrincipalPolicy::paired("agent:autonomous", …)`
  //         矩阵，再断言「拒绝 ⇒ `store.head()` 不变」：自造矩阵只证明「一个我自己写的策略
  //         拒绝了我自己」，而空 store 上「拒绝 ⇒ 零事件」是**同义反复**。改为查
  //         `authz::matrix_for`（生产表）+ 两条对照（持 `define.work`、主智能体持 `reply.first`）。
  //      ③ **`projection = conation` 不存在**（文档断言了没实现的行为）——判定「这条长目标
  //         声明过没有」原先写在消费方里直接 `range(0).any(…)` 全表扫。新增
  //         `symbio_core::projection::conation`（`ConationView` / `ConationIntent`），升格靠
  //         **溯源**认（`task.opened` 的 `produced_by` 指回欲 seq，不是 goal 字符串相等），
  //         消费方 = `heartbeat::long_goal_declared` 的 `is_declared(goal)`（判定住 core、插件只消费）。
  //      负向自检两条，各数清变红数：把 `matrix_for` 的分支改成恒假 ⇒ **3 条红**
  //      （`autonomous_actor_cannot_write_to_dialog` / `matrix_for_prefers_the_registered_row_over_derivation`
  //      / 既有 `tool_executor::external_execution_capability_is_read_from_the_matrix`）；
  //      把 `is_declared` 的 `task_id.is_some()` 去掉 ⇒ **5 条红**（4 条投影判据 + 集成判据
  //      `heartbeat::gate_opens_the_long_goal_exactly_once_per_goal`——首 tick 就会被误判成
  //      「已声明」而一格都不开，证明消费方真的接上了）。
  //      `+7` = `projection/conation.test.rs` 五条（升格靠溯源 / as-of / 无溯源可读不升格 /
  //      按目标不误伤 / 升格前后两态）+ `authz.test.rs::{the_autonomous_row_is_narrower_than_main,
  //      matrix_for_prefers_the_registered_row_over_derivation}`。
  // 1321（full 档补上承诺 / 任务表 / 熔断三份收束派生事实，2026-10-06）——`1320 → 1321`，**+1**。
  //      `full` 档的轮次事实由 v2 运行器原生记账，`chat_loop` 以 `v2_executed` 拦下整段
  //      `v2_facts::record`——但承诺 / 任务表 / 熔断**不是轮次事实**（数据来源在工具执行层），
  //      那一侧原先既不收集出参也不落格（`v2_tools` 三处 `&mut Vec::new()` 的「看得见的注记」），
  //      ⇒ `full` 档这三样**静默全丢**（与 t40 修的记忆/学习同一个坑）。本批：
  //      ① 把 `record_to_wal` 里三段落格抽成 `v2_facts::record_derived`（两档共用同一份
  //         实现、同一锚 `user_seq`，差别只在 `anchor_id` 词干：桥档 `{user_id}-a{attempt}`、
  //         full 档 `t{turn}`）——抽取保持行为逐字不变（既有 bridge 用例全绿）；
  //      ② `SessionDispatchPort` 新增 `DerivedFacts` + `take_derived` 取件面，`dispatch` 收下
  //         三份出参（原先是临时量）；
  //      ③ `v2_exec::execute_turn` 轮末调 `record_derived` 落格（承诺失败只记日志不冒泡——
  //         轮次已收束，派生事实失败不该把成功的一轮说成失败）。
  //      负向自检数清变红数：把三份出参改回 `&mut Vec::new()` ⇒ **单元 1 条红**
  //      （`v2_exec::full_turn_lands_derived_commitment_facts`，网格里无 `commitment.*`）+
  //      **e2e 1 条红**（`t42`，网格里 `task.opened` 0 格、`commitment.*` / `task.controlled` 全无）。
  //      `+1` = `v2_exec.test.rs::full_turn_lands_derived_commitment_facts`（出参通道 + 写方 +
  //      锚点一次钉死；e2e `t42` 另覆盖真工具成功 / 台账手术两条单测够不到的路径）。
  //
  // 2026-10-07 **回填 `1321 → 1339`（+18）**。这不是「又加了 18 个测试」的登记，而是
  //      **基线欠账的清偿**：实测 `cargo test -p symbio` 首行 1339，而棘轮对「高于基线」
  //      只打黄字（`⚠ 通过数 1339 > 基线 1321：请更新 BASELINE.rustTests`）——那条黄字
  //      自 2026-10-06 起**每天都在打、每天都没人回填**，门禁照旧全绿。后果不是难看，
  //      是**棘轮被削掉了 18 格**：删掉 18 个测试，1339−18 = 1321 = 基线，仍然绿。
  //      溯源（2026-10-08 补齐，方法 = `git diff --name-only 22360cb..0d5e449 -- symbio/src`
  //      **逐文件数测试属性**，而不是只数新增 `fn` 行）：+18 **全部归因，无残差**——
  //      `plugins/hook/executor.test.rs` 整文件首次接线 **+13**（`#[path = "executor.test.rs"]`
  //      与 `#[cfg(test)]` 同批出现）、`symbio_core/capability/tests.rs` **+4**、
  //      `symbio_core/actors/mod.test.rs` **+1**。后者之所以曾被记成「未解释残差」，
  //      是因为它在**已有 fn** 上补了 `#[test]`：`git diff -U0 | grep '+fn'` 那种数法
  //      结构上就看不见它——**溯源方法有盲区，不等于来源不存在**，本条即该教训的存证。
  //
  // 2026-10-08 **回填 `1339 → 1341`（+2）**，仍是欠账清偿、不是新账：+2 全部来自
  //      `0b806c0` 给 `v2_exec.test.rs` 补的两条 `flush_returns_*` 判据，而
  //      `git diff fe612a2..22ed05a -- '*.rs'` 在该区间**零**新增测试 fn ⇒ 归因无残差。
  //      之所以又欠了一次，是同一个结构：**黄字不判红**，只要没人回填，棘轮就持续被削格。
  //
  // 2026-10-08 **回填 `1341 → 1343`（+2）**：+2 = `gateway::server` 非回环安全护栏的
  //      两条判据（谓词四象限 + `start()` 接线断言）。**加了测试就跟着回填**——结构同上，
  //      不再重复解释。
  // 2026-10-08 **回填 `1343 → 1366`（+23）**：+23 = 批 B step 2（ADR-047 委派者三项
  //      真源的 Q1/Q2 落地）——`chat_loop/delegate.test.rs` **首次接线 +22**
  //      （Q1 三层判据 10 条、Q2 能力目录 6 条、合成段落 6 条）、
  //      `context/view.test.rs` 的 **index-2 次序判据 +1**。**加了测试就跟着回填**。
  // 2026-10-08 **回填 `1366 → 1373`（+7）**：+7 = 批 B step 2 的 Q3（worker 进展投影）
  //      落地，`delegate.test.rs` 新增 7 条——`rounds` 数 user 消息不拿 `message_count`
  //      顶、`steps` 无内容节点不产出、`in_flight` 只认见过的终态、步骤截断与上限、
  //      段落字段逐字可见、次序在判定/目录之后、空段一个字节不占。**加了测试就跟着回填**。
  // 2026-10-08 **C1（理由词表归一）：净 +1，1375 → 1376**。删掉的是
  //      `compose/templates.test.rs::the_vocabulary_matches_classify`——它比的是两份抄本，
  //      词表合一之后**没有任何输入能翻红它**（恒真断言，与 M8/M9 抓到的 `extends_domain`
  //      同一类）。补进来的是 `schemas/dialog.test.rs` 两条：九个字面量逐条钉住
  //      （`reason` 会原样写进转写节点 `meta.reason`，改值 = 老对话读不回当时的理由，
  //      与 `progress_reason_literal_is_the_wire_contract` 同一条判据）、`Verdict` 的
  //      serde 形状收下**词表之外**的码（未知码是可达输入，不是事故）。
  // 1378（2026-10-09，批 P3）：1383 → 1378，**−5**，且**分两向**：
  //   −10 随退役的转写路径一起删掉的旧用例（发言抽取 3 / attempt 编号 1 / 轮次事实入格 1 /
  //      `authorize_close` 1 / 记忆三段经 `record_to_wal` 1 / `off` 档 1 / 重开 1 改写后保留）；
  //   +11 迁与新增：迁 4 条改直调存活写方（`record_derived` 3 + `record_learning` 1）、
  //      新增 7 条（`prefix` 位置与不累积 1 / `keep == 0` 不截断 + `Some(1)` 对照 1 /
  //      config 档位取值 1 / **双向完整性防线 3** —— 四类缺口各造一个的反向自检 1 +
  //      平凡值无假红 1 + 工具轮重开复核 1）。
  //   净 +1。**逐条归属已核**：删掉的每一条要么测的是被退役的函数，要么其覆盖面已由
  //   `v2_exec.test.rs`（带反向自检）或新的 `v2_facts.test.rs` 接管。
  //   ⚠️ 这是**下修**，不是回填容差——`ratchetErosion` 判的是「基线相对最近两个碰过本文件
  //   的提交有没有变松」，所以这一格带 `baseline-allow` 豁免（理由见其上）。
  // baseline-allow rustTests: 批 P3 退役 `bridge` 转写路径，`v2_bridge.test.rs` 的 15 条缩到 `v2_facts.test.rs` 的 5 条（净 −10），另加 11 条迁与新增判据，净 +3；逐条归属见上方注释，非侵蚀。
  // 续 · 批 P3c（缺口 3）：**+2** 条新判据（core 层「注入的消息进了本轮的下一次请求」
  //   1 + 插件层「补充落成事实 / 本轮可见 / 重开 WAL 后下一轮仍可见」1），无删除。
  rustTests: 1383,
  /**
   * CI 口径的 Rust 通过数（**只增不减**）——与上面三个分包基线**是不同口径，不能互替**。
   *
   * 为什么要单独一格：本地与 CI 跑的不是同一条命令。本地按 crate 分包
   * `cargo test -p <pkg>`，每个包**各取首个** `test result: ok. N passed`（lib 目标，
   * `ignored` 不计）；CI 跑一次 `cargo test --workspace`，输出是**所有测试目标各行求和**
   * （含 cli / tauri / doctest 那几行）。所以 1384 = 1376 + 8 + 0 + 0，**不等于**三个
   * 分包基线之和的口径语义（那里是「首个 result 行」，这里是「全部 ok 行求和」）。
   *
   * ⚠️ **这个格子是补上来的，不是一直有的**：`cargoTestRatchet` 的 CI 分支原先
   * `return { ok: true }` ——「CI 跑全量……数字仅作信息展示」。于是 CI 这一侧
   * **根本没有棘轮**：本地删测试会被 1321 这个数拦下，推上去 CI 却只信退出码，
   * 照样绿。这与 ci.yml 漏掉 `v2-plan` 是同一课的第三次：**只信退出码**能抓住
   * 「测试失败」，抓不住「测试消失」——后者不留任何痕迹。
   *
   * 1384 = 1376 + 8 + 0 + 0（**推算**：+1 与 `rustTests` 同源，即 C1 词表归一的净增；
   * 下一次 CI 以实测复核）。第 4 行是 doctest `0 passed; 7 ignored`。
   * 口径的出处是 CI run 37647725557：`rust-checks (dev)` 四行依次
   * `1341 / 8 / 0 / 0 passed`，并打黄字 `⚠ CI 全量 1349 > 基线 1347：
   * 请更新 BASELINE.ciRustTestsTotal`。**本地与 CI 是两条独立的格子**，同一个增量
   * 会各欠一次，回填时必须两边一起看。
   */
  ciRustTestsTotal: 1384,
  /**
   * `cli` crate 的通过数（**只增不减**，判据与 `rustTests` 完全相同）。
   *
   * 为什么单独立一格：`symbio` 与 `cli` 是**两个独立 workspace**，`cargo test`
   * 在 `symbio/` 下跑不到 `cli/` 的测试。而门禁此前只对 cli 做了
   * `cargo check --tests` + `clippy --all-targets` —— 两者都**只编译不执行**，
   * 于是 `cli/src/args.rs` 里那 8 条参数解析用例**从来没有被任何门禁跑过**：
   * 它们可以一直失败而门禁全绿。这正是本仓反复栽的那类坑（「漏跑的代价远大于多跑」，
   * 见 `.github/workflows/ci.yml` 把白名单改成黑名单那段），只是这次漏的是**执行**。
   * 8 = 2026-09-23 实测（`cd cli && cargo test`）。
   */
  cliRustTests: 8,
  /**
   * `tauri/src-tauri`（包名 `symbio-tauri`）的通过数（**只增不减**，判据同上）。
   *
   * 为什么单独立一格：这是**第三个独立 workspace**，`cargo test` 在 `symbio/` 或
   * `cli/` 下都跑不到它。而它此前**完全不在门禁的扫描范围内**——不 fmt、不 check、
   * 不 clippy、不 test，449 行 Rust 全靠「没人动它」。它 `use symbio::…`，是
   * `symbio` 公开面的**跨 crate 消费方**：一次 API 改名可以让壳编译失败而门禁全绿。
   * （`scripts/tauri-binary.mjs` 会构建壳，但它只在**手工**跑它时才构建，门禁只跑
   * 它的回归测试，因此不算覆盖。）
   *
   * 0 = 2026-09-26 实测：壳自己没有用例（`cargo test` 打 `0 passed`）。基线取 0
   * 意味着**棘轮此刻是惰性的**——它不拦任何东西，只在「有人加了用例」之后开始生效
   * （那时门禁会提示更新本格）。真正守壳的是同一阶段里的 `check --tests` 与
   * `clippy --all-targets`：抓的是编译期契约，那才是消费方最该被守的东西。
   */
  tauriRustTests: 0,
  // 47 spec 文件 / 661 → 683 → 687 → 689 → 724 → 726 用例。文件数与用例数均与平台无关（全仓 spec
  // 零平台分支、it.each 只遍历静态常量数组），照实测值钉死；逐批明细见对应提交
  // （`git log --grep=<批次/主题>`；本仓库不维护变更日志，变更历史即提交历史）。
  // 726：`delta` 作为 `updated` 的可选传输字段回来（2026-09-23）——**+2 用例**
  //      （724 → 726），全在 `services/__tests__/eventBusWatch.spec.ts`：
  //      ① 带 `delta` 的变更**不被形状判定丢弃**且原样到达消费者——这是「加字段」
  //         最容易被门面悄悄裁掉的地方（后端 `to_change_event` 已有一条同义断言，
  //         两端各锁一次，因为它们是两份独立实现）；
  //      ② 作用域判定只看 `path`，与**是否带 `delta` 无关**——防止有人把
  //         「热路径增量」当成一种需要单独放行的例外，从而在作用域上开出第二套规则。
  //      ⚠️ 取值集合**没变**（仍是三个），所以这次没有像批次 G 那样「数量相抵」：
  //      净增是实打实的。形状守卫（恰好两键 / 三键）在 `schemas` 侧未动，因为
  //      它锁的是「词汇表是闭集」，而 `delta` 是 `updated` 上的字段、不是新取值。
  // 724：会话选项 schema 化（S1–S4，2026-09-23）——**+1 文件 / +35 用例**。
  //      新增 `ChatOptionBar.spec.ts`（21 条）：按 `DetailField.widget` 分派
  //      （`select` 菜单与选中态 / `path` 原生取值含 `disabled_when` / `form` 子表单 /
  //      `toggle` 就地翻转）、草稿态缓冲与即时回显、落库失败只提示不吞错；
  //      并用**虚构字段名** `alpha` / `beta` / `gamma` 反向钉住「前端零业务字段名」。
  //      另在 `vdfs-form.spec.ts` 补 `evalDetailCondition`（含「缺席键上 `truthy` 与
  //      `not_equals: 0` 结论相反」这条最容易踩的坑）与 `compactFieldText`（紧凑形态的
  //      取值规则）两组纯函数用例；`ChatComposer.spec.ts` 扩桩件 props 并加「选项定义
  //      与值由父组件给，本组件不回读」2 条。
  //      ⚠️ 旧机制的 4 个源文件（`services/options.ts` / `schemas/options.ts` /
  //      `composables/useSessionOptions.ts` / `registry/optionIcons.ts`）**都没有 spec**
  //      ——这正是「前端零业务」的反面教材：那份 `OPTION_PICKS` 是硬编码抄本，无人看守。
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
  // 48 文件 / 724：S27 收口（2026-09-23）——`vitestFiles` 47 → **48**、`vitestTests`
  //      726 → **723**。三条修正一次说清（上一批 356ba9d 只改了 Rust 那侧的注记，
  //      前端这两个数**一个都没改**，于是门禁从那天起就红着）：
  //      ① `+1 文件`：新增 `stores/__tests__/sessionTranscriptSync.spec.ts`；
  //      ② `−3 用例`：`delta` 从 `updated` 的可选字段变成 `ChatMessage.delta` 字段本身，
  //         随「按类型分派」一起作废的用例由**信封形状**用例接替（会话节点归
  //         `sessionNodeSync` / 身份取自地址末段 / 全量帧零回读 / 状态帧零回读）；
  //      ③ `+1 用例`（本轮）：`useVdfs` 补「带全量正文的载荷帧**不**触发重拉」与
  //         「孙辈变更**不**重拉当前目录」——后者是这次请求风暴的直接回归锚点
  //         （`affects` 原先把「任意后代」判成「影响我」）。原「非 delta 载荷走通用
  //         重拉」那条把错行为钉死了，已改写而非删除。
  // 49 文件 / 723：回读理由进路由留痕（2026-09-23，origin）——`vitestFiles`
  //      48 → **49**、`vitestTests` 724 → **723**。四笔一次说清：
  //      ① `+1 文件 / +3 用例`：新增 `services/__tests__/pluginEnvelope.spec.ts`
  //         ——**信封**层的守卫。原先没有任何用例断言「送上 IPC 的 metadata 里
  //         有什么」，于是 `buildMetadata` 里那段 `origin` 被删掉也不会红：
  //         机制照旧「实现」着，日志里只是永远少一个字段。三条分别钉住
  //         「给了理由必带 `origin`」/「没给理由一个键都不多」/「来源与路由正交」。
  //      ② `−5 用例`：`services/__tests__/vdfs.spec.ts` 里 `describe('listVdfs /
  //         statVdfs / readVdfs 的失败口径')` **整段（含文档注释）逐字重复了两遍**
  //         ——同一组断言跑两次。删掉第二份，覆盖不减（第一份原样保留）。
  //      ③ `+1 用例`：`sessionTranscriptSync` 补「Turn 组合节点（本身无正文）⇒
  //         零回读」——这是**每轮会话白跑一对 `stat` + `read`** 的回归锚点。
  //      ④ 其余为断言改形：三个回读动词的首参从「路径」变成「理由」
  //         （`READBACK_REASON` 的必填形参），既有断言跟着往后挪一位并**顺便
  //         钉住理由**（`missing-baseline` / `resource-signal` / `list-refresh` /
  //         `bootstrap` / `vdfs-browser`）。
  // 49 文件 / 725：`useVdfs`「带正文 ⇒ 不重拉」的豁免补上边界（2026-09-23）
  //      ——`vitestTests` 723 → **725**（`vitestFiles` 不变：改的是既有 spec，
  //      没有新增文件）。原规则默认了「被改的节点**已经在列表里**」，而新建出来的
  //      那一项首帧就带正文（写入即带内容 / 流式首帧即增量），于是它**永远不出现在
  //      中栏**，要等某次无关的刷新顺手带出来。补的判据是「列表里有没有这条路径」，
  //      **不问帧里带的是 `delta` 还是 `content`**——帧形状是协议的实现细节。
  //      两条用例分别钉住两个方向：「本目录还不认识的直接子项 ⇒ 必重拉」与
  //      「进入列表后 ⇒ 后续帧零重拉」。后一条是防退化的锚点：少了它，一次流式
  //      会话会变成几十次白拉的 `vdfs/list`（这正是当初加那条豁免要防的东西）。
  // 50 文件 / 727：重连自愈的回归锚（2026-09-23）——`vitestFiles` 49 → **50**、
  //      `vitestTests` 725 → **727**（`+1 文件 / +2 用例`：新增
  //      `services/__tests__/eventBusReconnect.spec.ts`）。
  //      锁的是「断开期间的帧」这条**唯一没有自愈路径**的缺口：后端见 `is_closed`
  //      摘订阅、帧静默丢弃，且订阅已不在表里 ⇒ 没人能补 resync。两条用例分别钉住
  //      「重连补、首连不补」（首连白重读是纯浪费）与「一个重读处理器抛错不得吃掉
  //      后面的作用域」（整份重读是唯一一层恢复机制，漏一半等于没恢复）。
  //      配套加了 `_resetEventBusForTest()`：状态挂在 `globalThis` 上会**跨 spec
  //      文件存活**，不复位就会读到别处留下的 `everConnected = true`，把首连误判成
  //      重连——该用例正是靠这个复位才可重复。
  // 50 文件 / 722：**VDFS 收敛 + `Move` 整条下线**（2026-09-24）——`vitestFiles`
  //      50 不变（改的都是既有 spec，没有新增文件）、`vitestTests` 727 → **722**
  //      （**净 −5**，逐文件核对，不是估算）：
  //        −3  `composables/__tests__/useVdfsPrompt.spec.ts`（19 → 16）——
  //            「重命名锚在选中项上」整段（预填 / 成功收起 / 失败保持 / 选中清空即收起 /
  //            没有选中项不进入）随 `startRename` / `submitRename` / `watch(selectedNode)`
  //            一并删除；判别式互斥那组把「entry 态下开 rename」换成
  //            「file 态下再点新建 ⇒ 回到 entry 态且载荷清空」——**互斥这条不变式仍在**，
  //            只是换一个可达的切换路径来钉。
  //        −2  `components/vdfs/__tests__/VdfsWorkbench.spec.ts`（9 → 7）——
  //            「重命名提示预填原名」删除；「提示态占用详情槽时渲染器不挂载」改用
  //            **机制动作 `delete`** 作代表（原来靠 `rename` 触发），
  //            两条断言合成一条（渲染器让位 + 提示动作行就位）。
  //        0   `components/vdfs/__tests__/VdfsDetailActions.spec.ts`——只把注入的
  //            机制动作从 `[rename, delete]` 收成 `[delete]`，用例数与断言面不变。
  //        0   `composables/__tests__/useVdfs.spec.ts` / `services/__tests__/vdfsScheme.spec.ts`
  //            ——只删桩里的 `moveVdfs` 与改 `new_type` 取值。
  // 49 文件 / 706：**「选入口」状态机退役**（2026-09-24）——`vitestFiles` 50 → 49、
  //      `vitestTests` 722 → **706**，**净 −16**。逐文件 diff（跑两次 `vitest run`
  //      取逐文件条数再 `diff`，不是估算）：
  //        −1 文件 / −16 用例  `composables/__tests__/useVdfsPrompt.spec.ts` 整个删除
  //            ——`useVdfsPrompt` 本身退役：新建 = 直接进该类型的详情页（不再有
  //            「选入口」），导入 = 详情页上的一条动作（取文件由渲染器的原生文件
  //            选择器完成，不再有「选文件」提示态）。它测的正是这两个瞬态。
  //        −1  `components/vdfs/__tests__/VdfsWorkbench.spec.ts`（7 → 6）——提示态
  //            三条用例删除，改为四条：新建按钮可见性 / 「点新建 = 一次无参调用」
  //            （没有第二跳）/ 动作按**载荷形状**分流（带 `File` → `runPackAction`）/
  //            机制动作经控件注入渲染器
  //        −2  `schemas/__tests__/vdfs.spec.ts`（31 → 29）——`newFileNameOf` 两条删除
  //            （目标名改由 provider 侧的 `pack_name_of` 推导，前端不再持有这份知识）
  //        −1  `services/__tests__/vdfs.spec.ts`（15 → 14）——`writeVdfsBinary` 一条
  //            删除（二进制写不再有对外入口；`base64ToBytes` / `arrayBufferToBase64`
  //            的互逆与分块边界用例原样保留）
  //        +4  `composables/__tests__/useVdfs.spec.ts`（15 → 19）——新增四条草稿动作
  //            用例（`File` ⇒ `{filename, b64}` / 落点是当前目录自身 / 成功 ⇒ 退出
  //            草稿 / 失败 ⇒ 留在草稿页）；既有三条把 `startNew(type)` 改成无参调用
  //      ⚠️ 净减是预期的：删掉的是「第二种新建入口」的整条交互链，而**新增的四条
  //      钉住了替代它的那条通道**（动作载荷形状 + 落点 + 草稿退出）。三条
  //      `startNew` 调用改形不计数（断言面不变，只是签名收窄）。
  // 49 文件 / 712（2026-09-25）：`vitestFiles` 不变、`vitestTests` 706 → **712**
  //      —— **+6 全部来自上批 `b191963` 的漏同步**（该批提交信息写了「vitest 712」，
  //      却没把这里的数字从 706 改上来）。本批（ADR-032）只动 Rust 侧与文档，
  //      前端一行未改，因此这 6 条不属本批。
  // 50 文件 / 735（2026-09-27）：前端 UI/UX 第一批。`vitestFiles` 49 → **50**
  //      （新增 `stores/__tests__/nav.spec.ts`）、`vitestTests` 712 → **735**，
  //      **+23** 分布在四处：nav 导航记忆 8 条、列表筛选 4 条（`VdfsWorkbench.spec`）、
  //      节点头键盘可达 4 条（`MessageNode.spec`）、「进入下一级」右置 4 条
  //      （`vdfs-form.spec`）+ 空候选去向 3 条（`ChatOptionBar.spec`，含一条**真装 router**
  //      的用例——它专门抓「注入键写成字符串 `'$router'`」这类 `vue-tsc` 看不见的错，
  //      见该文件注释）。
  // 51 文件 / 744（2026-09-27 同日第二笔）：`vitestFiles` 50 → **51**
  //      （新增 `router/__tests__/coldStart.spec.ts`）、`vitestTests` 735 → **744**，
  //      **+9**：冷启动落点 5 条（记忆优先 / 无记忆落会话目录 / 解析失败降级 /
  //      深链不被改写 / 用 replace）+ 进入会话空间自动开草稿 4 条
  //      （会话空间开、非会话空间不开、有选中不开、不可新建不开）。
  // 51 文件 / 753（2026-09-27 同日第三笔，**图标回归护栏**）：`vitestFiles` 不变、
  //      `vitestTests` 744 → **753**，**+9**。全部是「取图标唯一实现」的护栏：
  //      `vdfsIcons.spec` +4（六个挂载点互不相同 / kind 缺失仍按名字 / kind 空串
  //      仍按名字 / 项级键优先于配置键与名字）、`useVdfs.spec` +2（真实挂载点形状
  //      流过真实 navItems ⇒ 六项图标非空且互不相同；cardIconOf 与 navItems 同图）、
  //      其余为该批的重写与合并（`getVdfsIcon` / `getVdfsIconFor` 两个中间层删除后，
  //      原有用例改走 `iconForNode`，净 +3）。**夹具必须用真实 `kind: 'dir'`**：
  //      后端从不发空 kind，用不存在的形状做夹具，真回归来了照样是绿的。
  // 51 文件 / 756（2026-09-27 同日第四笔，**冷启动落点回归护栏**）：`vitestFiles`
  //      不变、`vitestTests` 753 → **756**，**+3**，全部加在 `coldStart.spec`。
  //      原有 5 条**全部走 `router.push('/')`**——那是守卫唯一正确的分支，所以
  //      带着 bug 也全绿。真实现场是打包后的 Tauri webview 用自定义协议加载，
  //      `createWebHistory()` 取到的 `location.pathname` **不是 `/`**（实测
  //      `/index.html` 不匹配任何具名路由），旧判据 `to.path !== '/'` 于是每次
  //      提前 return，位置记忆从未被读。新增 3 条把「首段不是 `/`」这个环境
  //      形状搬进单测：兜底接回首页 / 仍还原记忆 / 按路由名导航也触发落点。
  //      **反证**：连同兜底路由一起还原成原样 → 其中 2 条变红
  //      （`expected '/index.html' to be '/vdfs/model'`），与用户症状一致。
  // 51 文件 / 768（2026-09-27 同日第五笔，**列表为空时收起中栏**）：`vitestFiles`
  //      不变、`vitestTests` 756 → **768**，**+12**。`Workbench.spec` +7（收起
  //      判据四条合取逐条钉：草稿详情才收 / **有 `empty` 插槽照样收**（旧判据
  //      恒假，见下）/ 非草稿不收 / 不可新建不收 / 加载中不收 / 右栏空不收 /
  //      缺省保守），`VdfsWorkbench.spec` +4（控件喂给容器的三个事实端到端：
  //      空列表+草稿 ⇒ 不渲染中栏 / 空列表+已有条目 ⇒ 保留 / 有内容 ⇒ 保留 /
  //      配置型草稿同样收起），另一条是合并进既有用例。
  //      ⚠️ **被否掉的判据**（写进基线，免得下一轮又"优化"回去）：第一版判
  //      「宿主没提供 `empty` 插槽才收起」，而 `VdfsWorkbench` **无条件**声明该
  //      插槽 ⇒ 该判据在本项目**恒为假**、收起永不发生（一个写了却不生效的
  //      分支）。正确判据是 `hasDraft && canCreate`：空态该不该让位，取决于
  //      右栏是不是那张**新建详情**，不取决于宿主声明了哪些插槽。
  // 50 文件 / 760（2026-09-28，**「回上次地址」整条下线**）：`vitestFiles`
  //      51 → **50**、`vitestTests` 768 → **760**，**净 −8** = 删掉的
  //      `stores/__tests__/nav.spec.ts` 那 8 条（`stores/nav.ts` 一并删除：
  //      生产者与两个消费方同时去掉，store 就无人引用了）。
  //      `coldStart.spec` **条数不变**（8 → 8）：删 1 条「有位置记忆 ⇒ 回上次地址」、
  //      加 1 条「★ 盘上残留旧记忆 ⇒ 一律不还原」——后者是本功能的**验收用例**：
  //      老用户 localStorage 里仍有 `symbio.nav`，必须证明它已无人读取；另
  //      2 条原为记忆写的用例改判「落会话目录」（它们真正的价值是钉住
  //      **打包环境下守卫会执行**，那是落点本身的前提，与记忆无关，故保留）。
  //      该级下线的两个理由（判据拿不到真实入口地址 / 修好后反而"正确地做错事"）
  //      写在 `router/index.ts::coldStartPath` 的注释里。
  // 50 文件 / 761（2026-09-28 同日第二笔，**收起中栏改成通用机制**）：
  //      `vitestFiles` 不变、`vitestTests` 760 → **761**，**+1**（`VdfsWorkbench.spec`
  //      19 条：删 1 条「非会话空间不自动开草稿」、加 2 条「MCP 目录同样自动开」
  //      「后端新下发的类型照样开」）。
  //      ⚠️ **被写死、现已去掉的判据**（写进基线，免得下一轮又按类型名分支）：
  //      收栏的上游（进入目录自动备一张草稿）曾写成
  //      `creatableType.ext === 'session'` ⇒ 只有会话页有草稿、其余类型永远
  //      收不了栏——用户报的「MCP / 技能页没启用这个机制」就出自这里。正确判据
  //      只有「目录自己声明可新建什么」（`cwdNode.new_type`，后端下发），
  //      **监听源里也别把 `ext` 放回去**：那等于又把类型拖进判据。
  // 50 文件 / 768（2026-09-28 同日第三笔，**换目录 ⇒ 右栏跟着变**）：`vitestFiles`
  //      不变、`vitestTests` 761 → **768**，**+7**。用户报「切侧边栏时详情页没跟着
  //      变，模型页显示智能体详情、会话页显示模型详情」——**一步滞后**，两个成因
  //      各钉一组：
  //      ① 自动开草稿的监听源曾是**目录地址** `cwd`，而换目录时地址立刻变、目录
  //         自述（`cwdNode`）要等列表回来才更新，中间那一拍 `creatableType` 还是
  //         上一栏的 ⇒ 开出了上一类的草稿。改监听 `cwdNode`（+1 条 `VdfsWorkbench`）。
  //      ② 旧选中项的清理曾只看「还在不在已加载的列表里」：要等列表回来才判得出，
  //         且 `hasMore` 为真时根本判不了 ⇒ 旧选中项一直挂着。改按**归属**
  //         （`schemas/vdfs.ts::isVdfsUnder`，+4 条；`useVdfs.spec` +2 条）。
  //      两组都做了反证（改回旧写法 ⇒ 各自的 ★ 用例变红）。
  // 2026-09-28（同日两笔一并计入）：50 文件 / 768 用例 → **53 / 820**（+3 文件 / +52 用例）。
  //      ① 前端 UI/UX 第三批「抵达与导航」：新增三个 spec（引导 store / 引导组件 /
  //         会话级 UI 状态），并在 `useVdfs.spec`、`VdfsWorkbench.spec`、
  //         `VdfsDetailActions.spec`、`coldStart.spec` 内补用例（挂载层左栏、空态即引导、
  //         动作投影）。
  //      ② 「会话流与压缩可见性」：+26，四组——压缩后的历史记忆不再是用户气泡
  //         （facets / 渲染器 / 图标标题 / 默认收起 / 不挂运行中信号）、压缩字段读取口
  //         与「缺字段不编数字」、记忆节点与压缩节点事实行（真装 `MessageNode`）、
  //         发送键 / 停止键同一个语义点 + 运行中回车不提交（真装 `ChatComposer`）。
  //      ②里两条做了反证：去掉 `facets.compacted` 的判定 ⇒ 记忆节点变回用户气泡；
  //      把回车改回「运行中也提交」⇒ 对应用例变红。
  // 53 文件 / 828（2026-09-28 同日第三笔，**VDFS 检索入口 + 约束位 null 同义**）：
  //      `vitestFiles` 不变、`vitestTests` 820 → **828**，**净 +8**。四组：
  //      ① `useVdfs.spec` **+3**：`search` 三级语义——声明 true 才启用、未表态 /
  //         false 一律不启用；true 时筛选是**纯投影**（只筛已加载条目）；入口从
  //         启用变为不启用时清掉筛选词（否则列表被一个看不见的词过滤）。
  //      ② `VdfsWorkbench.spec` **净 +2**（2 条按新判据改写 + 2 条新增）：筛选框
  //         由**服务端声明**显隐——声明才出现；未表态 ⇒ 不给（没有声明的功能不
  //         自己长出来）；有内容但服务端明确 false ⇒ 仍不给（不是「有东西就可筛」）；
  //         空目录但声明 true ⇒ 框仍在（前端不二次裁决条目数）。2 条改写把旧判据
  //         「有没有内容」整条替换成「服务端怎么说」。
  //      ③ `vdfs-form.spec` **+2**：`DetailCondition` 缺席约束位写成 `null` 与缺席
  //         同义；`false` / `0` / 空串是**合法取值**，不得被当成缺席。
  //      ④ `VdfsDetailActions.spec` **+1**：定义里 `when` 为 `null` 占位时动作照常
  //         出现（线上形状回归）。
  // 54 文件 / 847（S5，2026-09-29，**对话面分栏**）：`vitestFiles` 53 → **54**、
  //      `vitestTests` 828 → **847**，**+1 文件 / +19 用例**。两组：
  //      ① 新增 `schemas/__tests__/conversation_line.spec.ts` **+15**：
  //         `conversation_line.ts` 是 Rust `conversation_view.rs` 的**镜像**，而
  //         `protocol-mirror-audit` 只守常量 / 枚举 / 字段名，**不守谓词** ⇒ 两侧各写
  //         一份穷举测试是这份镜像唯一的防线。15 条分三组：
  //         - `isConversationNode` **5**：两条判据（user/assistant 文本）判真、
  //           `type` 缺省视为文本（后端流式帧可能不带）、`role` 缺省不算（Rust 侧
  //           `None` 落空）、五类非文本节点判假、`role=tool|system` 即便类型是文本也判假；
  //         - `conversationNodesOf` **6**：投影保序扁平、**`turn` 子正文在对话线上**
  //           （`parent_id` 不参与判定）、工具结果正文一个字节都不泄漏、空输入 / 纯工作
  //           转写 ⇒ 空投影、产出是新数组不改入参；
  //         - `workRootsOf` **4**：根级补集、**它不是树遍历器**（入参给什么就过滤什么，
  //           防"顺手改成递归"而让工作面板丢掉 `turn` 里的工具调用）、与对话线互为补集
  //           （同一份根级上二者并集 = 全量）、空输入。
  //         与 Rust 侧逐条对应关系写在 spec 文件头的表里——改规则必须同时改两处。
  //      ② `stores/__tests__/appearance.spec.ts` 4 → **8**（**+4**，新增一组
  //         「会话分栏」）：`dialogPanelSplit` 是**唯一不影响根节点**的外观项（由
  //         `ModelChatPanel` 直接读），`apply()` 里漏写它 / `watch` 里漏监听它 /
  //         恢复时漏读它，原来那 4 条一条都不会红。4 条：出厂 `true` 且入持久化、
  //         平凡值 `false` 能存下来（关掉分栏可回退）、新实例恢复已保存的 `false`
  //         （否则重启就失效）、旧数据缺键 ⇒ 落出厂值 `true`（不因缺字段退化成单列）。
  // 54 文件 / 851（2026-10-08，**清偿欠账、不是新账**）：`vitestTests` 847 → **851**。
  //      +4 全部来自 `MessageNode.spec`（`f982874` 的工具状态行三判据 + 反例），那一次
  //      只留了注没改数字 ⇒ 同一格欠四格，棘轮被静默削掉。加了测试就回填。
  vitestFiles: 54,
  vitestTests: 851,
  /**
   * `docs/plan/verify/` 的**验证程序数**（棘轮，只许涨）——见 `gate.d/55-verify.mjs`。
   *
   * 为什么这个数也要棘轮：这些程序是 v2 方案的"硬证据"（`docs/plan/README.md §4`），
   * 而**少跑一跑在日志里与"全绿"长得一模一样**。程序被删掉时没有编译错误、没有
   * 测试失败、没有任何红灯——证据凭空消失而门禁照旧 51/51。数量是唯一能看见它的信号。
   *
   * 14 = `docs/plan/README.md §4` 的表（逐个核对过文件名），2026-10-07 实测 14/14 通过。
   */
  verifyPrograms: 14,
  /**
   * 带 `should_not_compile` 的**反例程序数**（棘轮，只许涨）——同上。
   *
   * 比 `verifyPrograms` 更要紧的一条：反例能力写在**源码里**（`55-verify.mjs` 读
   * `hasNegative` 自行发现，不维护名单）。把某个文件里的 `should_not_compile` 摘走，
   * 对应的 C29 那一跑就**凭空消失**，日志变短、其余全绿——这正是 C29 存在的理由
   * 所要防的那件事（`04 §4`：无法区分"真的强制"与"恰好没写错"）。
   *
   * 3 = `projection_purity` / `latency_gate` / `conation_minimal`（`README §4` 与
   * `03 §5.1` 同口径），2026-10-07 实测三个反例档均以 `error[E…]` 非 0 退出。
   */
  verifyNegatives: 3,
  /**
   * e2e 用例**数**（棘轮，只许涨）——见 `gate.d/40-e2e.mjs`。
   *
   * 为什么必需：`40-e2e` 按目录**发现**用例、只聚合 pass/fail，删掉一份
   * `e2e/cases/tNN.mjs`，那一跑仍是「41/41 通过」、门禁照旧全绿——「少跑一跑」
   * 在日志里与「全绿」长得一模一样。而 `plan/13` 的 S5 批次正文正引用着「现 42 例」，
   * 也就是说这个数字**已经在正文里被复述**，却没有任何东西守它。
   *
   * 42 = 2026-10-08 实测（`e2e/cases/` 下 `^t\d.*\.mjs$` 的文件数，与 `40-e2e`
   * 的发现逻辑同一条判据，不另写一份数法）。
   */
  e2eCases: 42,
  /**
   * verify 程序的 `assert!` **总数**（棘轮，只许涨）——见 `gate.d/55-verify.mjs`。
   *
   * 这一格补的是 `verifyPrograms` / `verifyNegatives` 的盲区：那两格只到**文件级**，
   * 把某个程序的断言删到只剩一条，两个数都不变 ⇒ 全绿。而 `03 §5.1` 纪律 1 自己写着
   * 「零断言的程序不可能失败」——这条纪律此前没有任何机器判据。
   *
   * ⚠️ **计数口径**：`assert!` / `assert_eq!` / `assert_ne!` 的**文本出现次数**，
   * 含 `#[cfg(feature = "should_not_compile")]` 模块内那些**正常档不执行**的断言。
   * 刻意保守：把它们算进来让基线略高于实际执行量，而「删断言必减计数」这个方向
   * 不受口径影响——棘轮要抓的是消失，不是精确。
   *
   * 165 = 2026-10-08 实测，14 个程序逐文件数清（逐文件明细由 `55-verify` 打进日志，
   * 便于按 `_shared.mjs` 里那条经验归因：「溯源方法有盲区，不等于来源不存在」）。
   */
  verifyAsserts: 165,
  /**
   * 非测试 Rust 里的 **panic 面处数**（棘轮，**只许减**）——见 `scripts/panic-audit.mjs`。
   *
   * 与上面所有「只许涨」的棘轮方向**相反**：这里的每一处 `unwrap` / `expect` /
   * `panic!` 都是一条健壮性债，清掉一处是改进。而「只许减」的另一面是「不许增」——
   * 新增一处会被 PN-001 拦住（必须带一行非空理由的登记），所以这个数只降不升。
   *
   * 为什么不能反过来做成「只许涨」：那等于给健壮性面发一张「可以随便加」的许可证。
   *
   * 64 = 2026-10-08 实测，48 个登记项（按 `文件::函数` 归并）。口径与守卫一致：
   * 排除 `*.test.rs` / `tests.rs` / 内联 `#[cfg(test)] mod` / **单独带 `#[cfg(test)]`
   * 的项**，排除注释里的形态，按**出现次数**计（同一行两处算两处）。
   */
  // 64 → 67（2026-10-08，批 C4）：**+3** = `FrameQueue` 自身的 `Mutex::lock().unwrap()`
  //   （`send` / `close` / `pop`）。`len` / `soft_overflow` 是判据用的观察口，标了
  //   `#[cfg(test)]` ⇒ 生产构建里不存在，clippy 也不判死代码。
  //   这不是新增的债面而是**新增的守卫**：
  //   C7 上线当天就把这 5 处抓了出来（未登记 ⇒ PN-001 红），登记后才放行——
  //   同一批里 `Drop for UiBridge` 补上了「桥 drop 必须关队列」，否则发射任务每轮永久挂起。
  panicSites: 67,
}

/**
 * 棘轮的**回填容差**：实测高出基线、差值在此格数以内只提示，超出即判红。
 *
 * 为什么要有上限：「高于基线」不是无害的——基线不跟着涨，删掉同样多的测试仍然全绿，
 * 棘轮就被削掉了同样的格数。而提示是可以被无限忽略的（日志天天有、门禁天天绿），
 * 所以容差的作用是给「正常的一次提交里先加测试再回填」留出余量，**红才是回填的判据**。
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
 * `docs/reference/GATE_CODES.md` 由它们生成（`gen-gate-codes.mjs`）。
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
        const v = ratchetVerdict({
          actual: total,
          baseline: ciBaseline,
          name: ciBaselineName ?? baselineName,
          kind: 'CI 全量',
        })
        if (v.warn) {
          console.log(
            yellow(
              `      ⚠ ${v.note}：见 scripts/gate.d/_shared.mjs 的 BASELINE.${ciBaselineName ?? baselineName}`,
            ),
          )
        } else if (v.ok) {
          console.log(`      ${total} passed（CI 全量，基线 ${ciBaseline}）`)
        }
        return { ...v, logFile: r.logFile }
      }
      const passed = grabInt(r.output, /test result: ok\. (\d+) passed/)
      if (passed === null) return { ok: true, note: '未能解析通过数' }
      const v = ratchetVerdict({ actual: passed, baseline, name: baselineName })
      if (v.warn) {
        console.log(yellow(`      ⚠ ${v.note}：见 scripts/gate.d/_shared.mjs 的 BASELINE.${baselineName}`))
      } else if (v.ok) {
        console.log(`      ${passed} passed（基线 ${baseline}）`)
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
