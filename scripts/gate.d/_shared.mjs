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
  //        +3  `plugins/session/options.test.rs`——产物换成 `DetailField` 后的锚点
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
  rustTests: 958,
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
  vitestFiles: 49,
  vitestTests: 712,
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
 * 自定义任务而非声明式命令：同一次运行既判退出码又解析通过数，
 * 避免为了拿输出再跑一遍测试。
 */
export function cargoTestRatchet(ctx, { label, cwd, args, baseline, baselineName }) {
  return {
    label,
    run: async () => {
      const r = await ctx.run({ label, cmd: 'cargo', args, cwd })
      if (!r.ok) {
        const note = r.timedOut ? '超时终止' : `exit=${r.code}${r.signal ? `, ${r.signal}` : ''}`
        return { ok: false, note }
      }
      if (ctx.ci) {
        // CI 跑全量（含集成测试）：每个测试目标各打一行 ⇒ 求和；数字仅作信息展示
        const total = sumInt(r.output, /test result: ok\. (\d+) passed/)
        if (total !== null) console.log(`      ${total} passed（--workspace 全量；只信退出码）`)
        return { ok: true }
      }
      const passed = grabInt(r.output, /test result: ok\. (\d+) passed/)
      if (passed === null) return { ok: true, note: '未能解析通过数' }
      if (passed < baseline) {
        return { ok: false, note: `通过数 ${passed} < 基线 ${baseline}（有测试被删或失败）` }
      }
      if (passed > baseline) {
        console.log(
          yellow(
            `      ⚠ 通过数 ${passed} > 基线 ${baseline}：请更新 scripts/gate.d/_shared.mjs 的 BASELINE.${baselineName}`,
          ),
        )
        return { ok: true, note: `通过数 ${passed}（基线待更新）` }
      }
      console.log(`      ${passed} passed（基线 ${baseline}）`)
      return { ok: true }
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
