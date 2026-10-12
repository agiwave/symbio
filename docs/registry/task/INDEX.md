# 任务登记索引（自动生成）

> ⚠️ **本文件由 `node scripts/registry-audit.mjs --write` 从 `docs/registry/task/*.md` 生成，请勿手改。**
> 状态的唯一登记处是各任务文件本身，本页是投影。字段规范见 [SP-001](../spec/SP-001.md)，原则与流程见 [14 · 登记与汇总机制](../../plan/14-登记与汇总机制.md)。

**计数**：☐ todo 1 · 🟡 doing 0 · ☑ done 52 · ❌ wont 0（共 53）

## 待处理（按依赖顺序）
| 编号 | 任务 | 优先 | 轨 | 依赖 |
|---|---|---|---|---|
| `S-end③` | 「结构化对模型是变好还是变坏」仍无读数：门内判据只到形状与有界性，模型侧对照需要一次独立的可授权实验 | later | S | S-end① |

## 进行中
（无）

## 已完成
| 编号 | 任务 | 轨 | 证据 |
|---|---|---|---|
| `C1` | 对话面理由词表是两份抄本，靠一条跨插件测试防漂移 | C | `ec62308` |
| `C3` | 两个巨型文件混域（`tool_executor.rs` / `actors/mod.rs`） | C | `7930c5a` |
| `C4` | 无界队列一处：流式 UI 帧通道不设界 | C | `git log --grep=C4` |
| `C6` | 前端两个热点文件混域（`sessions.ts` / `DetailForm.vue`） | C | `fde74c6` |
| `C7` | 健壮性面无任何判据：非测试 Rust 的 `unwrap` / `expect` / `panic!` 逐处无登记 | C | `git log --grep=C7` |
| `D0` | 文档里的数字既没有守卫、也没有 owner（D3 / D4 的出口写着「由新增计数守卫守」而那个守卫不存在） | D | `git log --grep=D0` |
| `D1` | 已全批落地的施工文档仍是活跃文档 | D | `git log --grep=D1` |
| `D2` | 同一事实在多处分册复述（机制矩阵 / C-D 断言 / OUTPUT.md / 02 §2.3） | D | `git log --grep=D2` |
| `D3` | 口径不一致，其中一条论证本身写错（共 7 处，不止原登记的 1 处） | D | `git log --grep=D3` |
| `D4` | 计数与代码脱节（「N 条」对不上号） | D | `git log --grep=D4` |
| `D5` | 归档与变更史卫生（`WINDOWS-RESTART*.md`、生成模板的变更史措辞） | D | `git log --grep=D5` |
| `M1` | verify 程序的数据全部手填 ⇒ 验的是「计划自洽」，不是「计划 == 代码」（直接违反 README §4 纪律 2「断言的输入不得包含结论」） | M | `git log --grep=M1` |
| `M10` | 路由常量的名↔值与值是否真实存在两条都没有判据，而 `plugin/route.rs` 的模块文档声称有守卫 | M | `cb6ad44` |
| `M11` | 门禁自身的结构约定靠约定：重生成阶段（`autoWork`）必须最后跑，无任何东西判这一点；守卫命名空间未闭合（一份既无回归测试又不在任何名单里的 `.mjs` 是完全隐形的） | M | `git log --grep=M11` |
| `M12` | 棘轮覆盖面只到一处：`35-baseline.mjs` 只守 `_shared.mjs` 的 `BASELINE` 一个对象，同类 floor / ceiling 三条在守卫之外 | M | `git log --grep=M12` |
| `M13` | 棘轮计量单位只到文件级：verify 只数程序数、e2e 侧完全无数，删到只剩一条 `assert!` 两个数都不变 | M | `git log --grep=M13` |
| `M2` | `BASELINE` 是源码常量，任一格可被 PR 静默下调；高于基线只打黄字不红 | M | `a86f78f` |
| `M3` | 守卫判据码无中央登记处，防撞号纯靠约定 | M | `f7c9bdc` |
| `M4` | 前后端契约手写两份：除 3 条别名外其余 `ROUTE_*` 无前端对账 | M | `176e01e` |
| `M5` | 文档指认的符号名与代码实名不逐字 | M | `99cc54e` |
| `M6` | 代码注释以批次口径叙述实施排期 | M | `8ba096f` |
| `M7` | 测试文件存在 ≠ 会被门禁跑：两份名单无人对账 | M | `65669ce` |
| `M8` | 「13 阶全程 0 值域扩展」这句话没有可执行判据 | M | `59f4525` |
| `M9` | 值域判据套到 16 条外部需求上撞出标注与 §8 不符 | M | `4a8513d` |
| `P1` | 规划缺产品层：无目标用户、无用户任务，出口判据是「git diff 没改 F1–F6」而非任何用户可见指标 | P | `git log --grep=P1` |
| `P2` | SLO 数字基于 mock 毫秒，唯一扫描口是 `#[ignore]` 测试 ⇒ 从未在 CI 出数 | P | `git log --grep=P2` |
| `P3` | 反射 / 快路在出厂档位下结构上产生不了样本；照出的八个静默缺口已全部修完 | P | `git log --grep=P3` |
| `P3b` | 反射档样本恒为零：两个出厂 `false` 的开关缺一不可 | P | 13 §2「P3b 未开工的理由」（只登记、不动默认值的定案）+ `config.rs` 的 `default_skill_compile_enabled` / `default_skill_fast_path` 两条成文理由 |
| `P4` | 读数口「读得出」但「看不见」：`session/stats` 八列在生产前端与 CLI 零调用方 | P | `git log --grep=P4` |
| `P5` | 召回率无评测集（`vec` 恒空、检索判据只吃 tag），但对应阶段状态记为已完成 | P | `git log --grep=P5` |
| `S-end①` | 没有一份 prompt 形状基线，无法回答「结构化对模型是变好还是变坏」——没有它，任何「更有效」的判断都是凭感觉拍板 | S | 判据住 `scripts/prompt-shape-audit.mjs`（PS-001…003）+ 门内执行方 `e2e/cases/t46-prompt-shape-baseline.mjs`；基线本体 `docs/reference/PROMPT_SHAPE.md`；口径见 `docs/registry/adr/ADR-057.md`。回归测试 `node --test scripts/prompt-shape-audit.test.mjs` → 20/20；反向验证（手改基线里一个工具名）⇒ t46 红 |
| `S-end②` | `full` 没被真实会话负载压过：e2e 全绿只是必要条件，不是「跑过真实负载」 | S | 压测执行方 `e2e/cases/t47-real-session-soak.mjs`（21 轮 = 20 正文 + 1 个 45 步工具长轮，跑过两次压缩）；口径决策 `docs/registry/adr/ADR-058.md`；压出的两个静默分叉修在 `symbio/src/symbio_core/projection/transcript.rs` 与 `prompt_fidelity.rs`，各带双向单点判据（`the_user_turn_a_snapshot_carries_is_not_emitted_twice` / `..._claims_only_once`）。全量 e2e 44/44 绿、`cargo test` 0、CS-001 三格未变宽 |
| `S0` | 会话消息的事实源是 `SessionStore`，WAL 轮末入格但读侧不以它为准 ⇒「Store 是唯一原语」在生产路径上不成立 | S | `git log --grep=S0` |
| `S1` | 读路径不并读两源 ⇒ 真源切换的偏差无声 | S | `git log --grep=S1` |
| `S2` | Q3 进展读不走 WAL | S | `git log --grep=S2` |
| `S2.5` | v1 档与 `V2Mode` 旋钮仍在，v2 无法独立运行 | S | `git log --grep=S2.5` |
| `S2.6` | 思考与半截产出不入格 ⇒ 中断后历史里那半句话消失，上下文悬空 | S | `git log --grep=S2.6` |
| `S2.7` | 会话消息的事实源仍是 `SessionStore`（`messages.json` 整份覆盖），WAL 只是读侧兜底 | S | `1bfce16`（主体切源）+ `941cf88`（S2.7-b/c）+ `git log --grep=S2.7-a`（S2.7-a 退役对账） |
| `S2.7-a` | 主读路径没切：缓存不在才重建，写侧仍整份落正文；S1 双源对账与 S2 旧读法尚未退役（切源后单源、无对账对象） | S | `git log --grep=S2.7-a` |
| `S2.7-b` | 写入期裁剪不落事实 ⇒ 切源后被裁节点复活（ADR-054 后果节：裁剪入格 or 读侧按同一窗口裁，无第三种） | S | `941cf88` |
| `S2.7-c` | 写者令牌会静默丢事实：编辑恰逢一轮运行中拿 `NotTheWriter` ⇒ 该 `message.edited` / `removed` 被丢弃（切源后即「编辑偶发不生效」且无人报错） | S | `941cf88` |
| `T-001` | 登记机制落地：单文档 + 守卫 + 汇总 + 总纲 | meta | 提交 8252c4a（机制全量落地：4 脚本 + 23 项回归测试 + docs/plan/14 总纲 + 门禁接线，docs+facts 58/58） |
| `T-003` | 接管 docs/README 纪律地图（每条纪律成为 spec 条目） | doc | 提交 36a9eb3（21 条 spec 条目 + 地图收敛指路；docs+facts 门禁 58/58） |
| `S22` | 22 步实施状态闭环（04 §3.1 A 表——每步实际接入 + 死码归零 + 门禁绿 + 出厂档位压过） | meta | 04 §3.1 A 表 22 步状态列全 ✅（接入判据逐行在表内「缺口 → 接入判据」列）；聚合快照见 `git log --grep=批` 的清偿批次提交；守卫 `scripts/dead-code-audit.mjs` R-002 终局基线 8 |
| `T-004` | 散落决策清点与登记形态定案（决策类元素的接管边界） | meta | 清点结论：唯一散落处 = `docs/design/third-party-plugin-spec.md §10`（6 条待确认决策）⇒ 处置为「原表自持、不另登 task」；形态定案写回 14 §6；验证：registry-audit 零违规 |
| `T-005` | 接管 13 §2.1 豁免登记表（豁免 → spec + clears 字段） | meta | c167e00（clears + RG-016 机制扩展与 5 条豁免迁移；验证：77 条零违规 · 回归 24/24 · gate 58/58） |
| `T-006` | 接管 04 §3.1 工具台账（先分清守卫投影行与人维护登记行） | meta | S22（22 步状态）+ S31-1…3（三张守卫表的投影口径）；04 §3.1 已收敛为指路索引；验证：registry-audit 零违规 · gate docs 阶段全绿 |
| `T-008` | ADR 接管 registry（新开 adr kind：索引与正文一并迁入，删除 DECISIONS.md 与 decisions/*.md） | meta | adr kind（KINDS.adr + renderAdrIndex + RG-017/RG-018/RG-019）；docs/registry/adr/ADR-001…054 共 55 条（44 条并入正文、11 条索引-only）；DECISIONS.md 与五个分册已删除；验证：registry-audit 零违规 · registry-audit.test.mjs 绿 · doc-link 全绿 · doc-symbol 全绿 · gate --only=docs,facts 绿 |
| `T-002` | 接管 13 §1 问题登记（状态表格迁成任务单文档） | meta | 提交 82d7f9f（44 条目 + §1 收敛 + 9 处锚点引用改写；docs+facts 门禁 58/58） |
| `T-010` | registry 迁入 docs/registry——条目库与文档目录合一 | meta | ADR-056（决策与四条被否决的方案）；`git mv registry docs/registry` + 链接重算 675 处 / 反引号路径 133 处（304 篇 md）；装载与豁免口径改动落在 registry-audit / registry-board / registry-lib（RG-010 / RG-013 消息）、doc-link-audit（扫描根去 registry、新增 BODY_EXEMPT_DIRS）、doc-symbol-audit、core-shape-audit（CS-002 执照路径）、dead-code-audit（S31-1 规则源）、gen-current-facts；验证：registry-audit 143 条目零违规 · doc-link-audit 941 条链接 0 失效 · gate --only=docs,facts 60/60 · doc-link 回归测试 62/62（含「条目写历史不判 / 条目失效链接照判」两条新增双向用例） |
| `W1` | core 根出口豁免账本 18 条，大半条目在补一个口径的洞（「core 内另有消费方，审计只数跨出 core 的那一个」） | W | `27922b8` |
| `W2` | 其余四条豁免通道（doc-link / dead-code / panic / color）的清除任务挂在不存在的批次上 | W | `7eabd54` |

## 不做
（无）
