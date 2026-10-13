# 任务登记索引（自动生成）

> ⚠️ **本文件由 `node scripts/registry-audit.mjs --write` 从 `docs/task/*.md` 生成，请勿手改。**
> 状态的唯一登记处是各任务文件本身，本页是投影。字段规范见 [SP-001](../spec/SP-001.md)，原则与流程见 [PL-011 · 登记与汇总机制](../plan/PL-011-登记与汇总机制.md)。

**计数**：☐ todo 1 · 🟡 doing 0 · ☑ done 68 · ❌ wont 0（共 69）

## 待处理（按依赖顺序）
| 编号 | 任务 | 优先 | 轨 | 依赖 |
|---|---|---|---|---|
| `T-071-core形状变宽的必需性论证` | core 形状棘轮亮红（fn 73>72 / defaults 25>24）：逐格补「必需」论证 ADR 或回退 | now | quality | — |

## 进行中
（无）

## 已完成
| 编号 | 任务 | 轨 | 证据 |
|---|---|---|---|
| `T-061` | 登记机制支持条目分解：parent 字段（父子机制） | meta | registry-lib.mjs RG-024（parent 形状 / 存在 / 同 kind / 自指 / 成环）+ RG-025（子生效⇒父生效、父退场⇒子退场，档位表按 KINDS 取）；INDEX 分解树小节（条件渲染，无 parent 数据零变化）；board --children / --parent；SP-001 字段表 + PL-011 §1.2 + docs/README 两字段口径同批落地（RG-011 对账）；验证：registry-audit 249 条目零违规 · registry-audit.test.mjs 41/41 · gen-doc-refs.test.mjs 14/14 · doc-link-audit 0 失效；子任务 T-062 / T-063 / T-064 全部 done（RG-025 父退场⇒子退场的正向用例） |
| `T-066` | megabatch 守卫：单响应工具调用数上限 + 中止原因如实标注（`6cf9520d` 会话死循环） | quality | 1. 单响应上限32（stream.rs ToolCallDelta 分支，`MAX_TOOL_CALLS_PER_RESPONSE`，超限即 Err）2. abort-reason 如实标注（tool_executor.rs 收口循环，`aborted` vs `not_executed`）3. 测试 stream.test.rs×2 + aborted_batch_terminates_every_tool_call；`cargo test --lib` 1541 通过 / clippy 0 警告 |
| `T-067-mockLLM体系化机制化` | mockLLM 体系化 / 机制化完善：真实 LLM 服务与真实网络环境的异常形态注入 | quality | e2e/mock-llm.mjs（新增注入字段 latencyMs / chunkJitterMs / stallAfter+stallMs / dropAfter / malformed+malformedAfter，file 头写清注入语义；plan 级 rateLimit: {limit, windowMs} 滑动窗口 429 限流（retry-after + x-ratelimit-*，被拒不占配额）；/_requests 增记响应 status）；e2e/helpers.mjs MockLlm 第三选项 rateLimit 透传；四条黑盒用例各钉一类：t49-stream-drop（断流轮可见失败 + 恢复轮照常）、t50-ratelimit-429（双探针占配额、限流后冷却自动恢复）、t51-malformed-stream（三种脏帧不打破对话）、t52-network-timing（延迟/抖动/停顿下完整回复）；验证：e2e 门禁 49/49（含既有 45 条不返红， 2026-10-13）；批次提交 `34d11a52` |
| `T-068-e2e用例体系化升级` | e2e 用例体系化建设与升级：正常 + 异常场景全覆盖，按真实用户使用过程取形 | quality | 覆盖矩阵落 [RE-005-e2e覆盖矩阵](../ref/RE-005-e2e覆盖矩阵.md)（十条能力域 × 三形态，52 条用例入格；缺口四处逐格记判定与理由：4xx/不可达已清偿 t53、MCP 断连待 ST-009 接线批次、WS 断链明确不做（红点角标 + 自动重同步已承担可见性）、压缩中插话待评估）；第一批缺口清偿 t53-llm-4xx-unreachable（401 不可重试错误态 + 起点不可达两类此前与 5xx 重试同通道的异语义失败，各自可红 + 恢复轮）；验证：e2e 全量绿（含 t53）· registry-audit / gen-doc-refs 零违规；用例形纪律（实时面等待 + 历史等事实出现再断言）在 t50/t53 已体现 |
| `T-069-前端会话机制分析与优化` | 前端 v2 会话机制分析与优化：多线程会话模型与「一个会话线程一个显示区域」 | frontend | 分析与裁决全量落于 [ADR-063](../adr/ADR-063.md)（"2.1 现有体系是否支持一个会话内多条实际会话线程 / 2.2 多实线对话该放哪 / 2.3 呈现裁决"三节）；现状认定以用户可见形状为证：ModelChatPanel 的 dialog_panel_split 两列（同一批节点两种读法）、t24 形状锚、t34 / t30 / ADR-026 的线与主体事实；实施另立 [T-070-会话呈现合流单窗口](T-070-会话呈现合流单窗口.md)；验证：registry-audit 零违规 + doc-link 全绿；黑盒纪律全程遵守（分析只引用用户可见出口与公开文档事实，未把后端内部实现写成判据） |
| `T-070-会话呈现合流单窗口` | 前端呈现合流：退役「对话 / 工作」分栏，一个会话线程一个显示区域 | frontend | ModelChatPanel 单列化（分栏模板分支 / 工作列 / 双滚动与 split 样式全撤，恒用 messageTree）；appearance store 无 dialogPanelSplit（持久化里残留的旧键被忽略，appearance.spec 新增对退役键的负向用例）；设置页移除分栏项；前端镜像 schemas/conversation_line.ts 与其 spec 撤除（无消费方即撤，按镜像纪律不写假指针），后端 conversation_view.rs 的 Corresponding Frontend 指针同步撤除；数据层零改动（Rust conversation_view 不动）；验证：tauri vitest 54 文件 838 测试全绿、vue-tsc 0 错误、t24 e2e 不返红、protocol-mirror-audit / test-layout-audit 零违规 |
| `T-011` | 五轴的外部效标：每条轴挂一个可复现的读数，而不是形容词 | plan | PL-002 新增 §3.1.1 外部效标表（五轴逐条挂读数口，纪律三条：读数没出来不得改可测量口吻 / 设定值必标 / 给不出就写"本轴目前只能人工判"）；三条轴新落生产读数口——时域跨度 = span 投影（会话首末事件 ts 跨度 + 跨会话 P50 中位数）、主动性 = conation 的 adoption_rate（已升格 / 已表达，未升格算分母）、自主程度 = autonomy 投影（闭环且全程未 task.held / 已闭环，progress 恢复不洗掉干预事实）；自我模型精度沿用 calibration 列既有口径；抽象层级写明只能人工判（跨域迁移要离线评测，不在生产读数口内）。三族各配 core 判据（含 as-of、曾 held 不算自主、未闭环不进分母、空输入有据的零）+ stats 层「删事件 ⇒ 读数跟着动」的手术刀用例；core-export-audit C-003 单消费方 58/58 未增（新导出收窄为 pub(crate)）；cargo test --lib 1526 全绿、clippy 零警告 |
| `T-012` | 能力名册 ↔ 代码锚点：名册的每一条能力都要指到实现它的地方 | plan | PL-002 §6.1 名册新增「锚点」列（54 行，取值 `文件` / `文件::符号` 反引号形态或 `—` 表示尚未落地），已填 30 行真实锚点；判据落到 doc-symbol-audit 的 D-011（复用 D-005 的同一份 Rust 语料，不新建第二份符号解析）：文件须在仓内存在、`::符号` 查语料存在、列须是反引号形态、名册读不出即红；锚定率进日志「名册锚点 30/54 已锚定」，使「名册在长、锚定率不动」看得见；回归 7 个用例双向钉住（不存在符号 / 不存在文件 / 非反引号形态 ⇒ 红；两种有效形态、`—` 不判红、名册不在范围不适用、名册读不出 ⇒ 红）——doc-symbol-audit.test.mjs 35/35；capability_frame.rs 编译运行全绿（名册多一列未破坏 facts 解析） |
| `T-015` | 出厂 prompt 在线上是两条 system 消息，部分 provider 直接 400 拒收——full 档在那类 provider 上第一轮就失败 | S | 口径取「前缀并入基础 system」（形状只该有一个产地，不让每家协议各并一次）：openai_chat::prepare_request 新增 merge_leading_system，吃掉数组最前的连续 system 段、按「基础 → 前缀」空行分段拼成一条（空行分段是本仓既有惯例，NativeMessage::to_api_value 拼 prompt 用的就是它）；边界——只合纯文本段，遇 Parts（多模态）停下不再吃（拼错比多一条 system 更糟），吃掉 0 条时行为与改前逐字相同；判据三层接住：① 5 个单元测试（合并顺序 / 0 条不变 / 空基础不留前导空行 / Parts 停下 / 端到端断言请求体里 system 恰好一条且前缀文本未丢）；② 形状基线 PROMPT_SHAPE.md 随形状重生成（req0 消息序列 system → system → user 变 system → user、消息数 3→2），改回两条即红；③ cargo test --lib 1531 全绿 |
| `T-065` | 修复 e2e t30 既有失败：父会话线程里出现了子会话的用户正文 | quality | t30 回绿；全量 e2e 45/45（2026-10-13） |
| `T-013` | Rust 覆盖率阈值：与 C7 同族的已知盲区，排在会话真源归一之后 | plan | 处置取第二条——理由的 owner 是 [ADR-060](../adr/ADR-060.md)（不做的四条：覆盖率答「有没有被执行」不答「有没有被断言」、只调用不断言的测试照样顶满而它守不住任何东西，与「出口可核对」相悖；「测试被删就红」已有 RATCHETS 用例数 / 断言数 floor 棘轮落点，覆盖率是冗余第二道；成本每次提交都付而收益是低频观察量，且要引入 cargo-llvm-cov + llvm-tools 外部工具链；总量数字会稀释 panic-audit / 形状基线 / 外部效标这些专用判据）。ADR 同时写明**缺口不闭合**（无法机械发现从未被执行的分支）与三条**重开条件**（出现只有覆盖率能发现的缺陷实例 / 工具链成本降到可进门禁 / 仓的风险面变了），重开唯一入口是新 ADR 取代本条。`gate.d/35-baseline.mjs` 的 RATCHETS 未加覆盖率落点，scripts 未引入覆盖率依赖 |
| `T-014` | 分布式 Store 的 seq 单调与「无溯源不声明」如何共存（G4） | plan | 判断先行——多副本部署不在计划内（G4 / R5 的触发信号「出现多副本部署需求」从未发生），故 [ADR-061](../adr/ADR-061.md) 判 **`store = distributed` 从值域退役**（留一格「将来也许能用」比删掉更脏：它是语义上必然违反 I1 的取值，而 `in_domain` 会判它合法，值域自己在撒谎）。ADR 同时留下**重新引入必须答全的四问**（全局序来源 / 同 event_id 冲突裁决 / 溯源边跨副本指向 / 脑裂与 I3），答不全不得回值域。**判据落点**：PL-001 §8 值域表已去掉该格 ⇒ 生成的 facts::DOMAINS 里没有它 ⇒ `scenario_ladder.rs` 断言「store 值域不含 distributed」且「in_domain("store","distributed") 为假」——谁加回值域表，facts 里就会有它，断言立刻红（编译运行通过）。连带收敛 PL-003 §7.3 G4 与 PL-004 §5 R5 的措辞 |
| `T-001` | 登记机制落地：单文档 + 守卫 + 汇总 + 总纲 | meta | 提交 8252c4a（机制全量落地：4 脚本 + 23 项回归测试 + docs/plan/14 总纲 + 门禁接线，docs+facts 58/58） |
| `T-003` | 接管 docs/README 纪律地图（每条纪律成为 spec 条目） | doc | 提交 36a9eb3（21 条 spec 条目 + 地图收敛指路；docs+facts 门禁 58/58） |
| `T-004` | 散落决策清点与登记形态定案（决策类元素的接管边界） | meta | 清点结论：唯一散落处 = `docs/arch/AR-011-third-party-plugin-spec.md §10`（6 条待确认决策）⇒ 处置为「原表自持、不另登 task」；形态定案写回 14 §6；验证：registry-audit 零违规 |
| `T-005` | 接管 13 §2.1 豁免登记表（豁免 → spec + clears 字段） | meta | c167e00（clears + RG-016 机制扩展与 5 条豁免迁移；验证：77 条零违规 · 回归 24/24 · gate 58/58） |
| `T-006` | 接管 PL-004 §3.1 工具台账（先分清守卫投影行与人维护登记行） | meta | T-058（22 步状态）+ SP-015…3（三张守卫表的投影口径）；04 §3.1 已收敛为指路索引；验证：registry-audit 零违规 · gate docs 阶段全绿 |
| `T-008` | ADR 接管 registry（新开 adr kind：索引与正文一并迁入，删除 DECISIONS.md 与 decisions/*.md） | meta | adr kind（KINDS.adr + renderAdrIndex + RG-017/RG-018/RG-019）；docs/adr/ADR-001…054 共 55 条（44 条并入正文、11 条索引-only）；DECISIONS.md 与五个分册已删除；验证：registry-audit 零违规 · registry-audit.test.mjs 绿 · doc-link 全绿 · doc-symbol 全绿 · gate --only=docs,facts 绿 |
| `T-058` | 22 步实施状态闭环（PL-004 §3.1 A 表——每步实际接入 + 死码归零 + 门禁绿 + 出厂档位压过） | meta | 04 §3.1 A 表 22 步状态列全 ✅（接入判据逐行在表内「缺口 → 接入判据」列）；聚合快照见 `git log --grep=批` 的清偿批次提交；守卫 `scripts/dead-code-audit.mjs` R-002 终局基线 8 |
| `T-002` | 接管 13 §1 问题登记（状态表格迁成任务单文档） | meta | 提交 82d7f9f（44 条目 + §1 收敛 + 9 处锚点引用改写；docs+facts 门禁 58/58） |
| `T-010` | registry 迁入 docs——条目库与文档目录合一 | meta | ADR-056（决策与四条被否决的方案）；`git mv registry docs` + 链接重算 675 处 / 反引号路径 133 处（304 篇 md）；装载与豁免口径改动落在 registry-audit / registry-board / registry-lib（RG-010 / RG-013 消息）、doc-link-audit（扫描根去 registry、新增 BODY_EXEMPT_DIRS）、doc-symbol-audit、core-shape-audit（CS-002 执照路径）、dead-code-audit（SP-015 规则源）、gen-current-facts；验证：registry-audit 143 条目零违规 · doc-link-audit 941 条链接 0 失效 · gate --only=docs,facts 60/60 · doc-link 回归测试 62/62（含「条目写历史不判 / 条目失效链接照判」两条新增双向用例） |
| `T-016` | 对话面理由词表是两份抄本，靠一条跨插件测试防漂移 | C | `ec62308` |
| `T-017` | 两个巨型文件混域（`tool_executor.rs` / `actors/mod.rs`） | C | `7930c5a` |
| `T-018` | 无界队列一处：流式 UI 帧通道不设界 | C | `git log --grep=C4` |
| `T-019` | 前端两个热点文件混域（`sessions.ts` / `DetailForm.vue`） | C | `fde74c6` |
| `T-020` | 健壮性面无任何判据：非测试 Rust 的 `unwrap` / `expect` / `panic!` 逐处无登记 | C | `git log --grep=C7` |
| `T-021` | 文档里的数字既没有守卫、也没有 owner（D3 / D4 的出口写着「由新增计数守卫守」而那个守卫不存在） | D | `git log --grep=D0` |
| `T-022` | 已全批落地的施工文档仍是活跃文档 | D | `git log --grep=D1` |
| `T-023` | 同一事实在多处分册复述（机制矩阵 / C-D 断言 / OUTPUT.md / PL-002 §2.3） | D | `git log --grep=D2` |
| `T-024` | 口径不一致，其中一条论证本身写错（共 7 处，不止原登记的 1 处） | D | `git log --grep=D3` |
| `T-025` | 计数与代码脱节（「N 条」对不上号） | D | `git log --grep=D4` |
| `T-026` | 归档与变更史卫生（`WINDOWS-RESTART*.md`、生成模板的变更史措辞） | D | `git log --grep=D5` |
| `T-027` | verify 程序的数据全部手填 ⇒ 验的是「计划自洽」，不是「计划 == 代码」（直接违反 README §4 纪律 2「断言的输入不得包含结论」） | M | `git log --grep=M1` |
| `T-028` | 路由常量的名↔值与值是否真实存在两条都没有判据，而 `plugin/route.rs` 的模块文档声称有守卫 | M | `cb6ad44` |
| `T-029` | 门禁自身的结构约定靠约定：重生成阶段（`autoWork`）必须最后跑，无任何东西判这一点；守卫命名空间未闭合（一份既无回归测试又不在任何名单里的 `.mjs` 是完全隐形的） | M | `git log --grep=M11` |
| `T-030` | 棘轮覆盖面只到一处：`35-baseline.mjs` 只守 `_shared.mjs` 的 `BASELINE` 一个对象，同类 floor / ceiling 三条在守卫之外 | M | `git log --grep=M12` |
| `T-031` | 棘轮计量单位只到文件级：verify 只数程序数、e2e 侧完全无数，删到只剩一条 `assert!` 两个数都不变 | M | `git log --grep=M13` |
| `T-032` | `BASELINE` 是源码常量，任一格可被 PR 静默下调；高于基线只打黄字不红 | M | `a86f78f` |
| `T-033` | 守卫判据码无中央登记处，防撞号纯靠约定 | M | `f7c9bdc` |
| `T-034` | 前后端契约手写两份：除 3 条别名外其余 `ROUTE_*` 无前端对账 | M | `176e01e` |
| `T-035` | 文档指认的符号名与代码实名不逐字 | M | `99cc54e` |
| `T-036` | 代码注释以批次口径叙述实施排期 | M | `8ba096f` |
| `T-037` | 测试文件存在 ≠ 会被门禁跑：两份名单无人对账 | M | `65669ce` |
| `T-038` | 「13 阶全程 0 值域扩展」这句话没有可执行判据 | M | `59f4525` |
| `T-039` | 值域判据套到 16 条外部需求上撞出标注与 §8 不符 | M | `4a8513d` |
| `T-040` | 规划缺产品层：无目标用户、无用户任务，出口判据是「git diff 没改 F1–F6」而非任何用户可见指标 | P | `git log --grep=P1` |
| `T-041` | SLO 数字基于 mock 毫秒，唯一扫描口是 `#[ignore]` 测试 ⇒ 从未在 CI 出数 | P | `git log --grep=P2` |
| `T-042` | 反射 / 快路在出厂档位下结构上产生不了样本；照出的八个静默缺口已全部修完 | P | `git log --grep=P3` |
| `T-043` | 反射档样本恒为零：两个出厂 `false` 的开关缺一不可 | P | 13 §2「P3b 未开工的理由」（只登记、不动默认值的定案）+ `config.rs` 的 `default_skill_compile_enabled` / `default_skill_fast_path` 两条成文理由 |
| `T-044` | 读数口「读得出」但「看不见」：`session/stats` 八列在生产前端与 CLI 零调用方 | P | `git log --grep=P4` |
| `T-045` | 召回率无评测集（`vec` 恒空、检索判据只吃 tag），但对应阶段状态记为已完成 | P | `git log --grep=P5` |
| `T-046` | 没有一份 prompt 形状基线，无法回答「结构化对模型是变好还是变坏」——没有它，任何「更有效」的判断都是凭感觉拍板 | S | 判据住 `scripts/prompt-shape-audit.mjs`（PS-001…003）+ 门内执行方 `e2e/cases/t46-prompt-shape-baseline.mjs`；基线本体 `docs/ref/PROMPT_SHAPE.md`；口径见 `docs/adr/ADR-057.md`。回归测试 `node --test scripts/prompt-shape-audit.test.mjs` → 20/20；反向验证（手改基线里一个工具名）⇒ t46 红 |
| `T-048` | 「结构化对模型是变好还是变坏」仍无读数：门内判据只到形状与有界性，模型侧对照需要一次独立的可授权实验 | S | 执行方 `e2e/ab/`（`capture-corpus.mjs` 从出厂真实链路捕获请求体 → `flatten.mjs` 纯函数拍平 → `tasks.mjs` 22 道题面与真值口径 → `run.mjs` 两臂交替回放并出报告）；读数 `docs/ref/PROMPT_AB.md`（2026-10-12T05:48Z，MiMo-V2.6-Flash，22 题 × 两臂 × 2 次 = 88 请求，未收束 1、出错 0，重复之间 43 组可比对里 2 组翻过；同实验更早一遍 05:29Z 判据逐项同分或差 1 题）；花费 provider 自报 usage.cost 合计 $0.0033 + $0.0045（两遍，经用户授权）；撞出来的独立缺口另立 [T-015](T-015.md) |
| `T-047` | `full` 没被真实会话负载压过：e2e 全绿只是必要条件，不是「跑过真实负载」 | S | 压测执行方 `e2e/cases/t47-real-session-soak.mjs`（21 轮 = 20 正文 + 1 个 45 步工具长轮，跑过两次压缩）；口径决策 `docs/adr/ADR-058.md`；压出的两个静默分叉修在 `symbio/src/symbio_core/projection/transcript.rs` 与 `prompt_fidelity.rs`，各带双向单点判据（`the_user_turn_a_snapshot_carries_is_not_emitted_twice` / `..._claims_only_once`）。全量 e2e 44/44 绿、`cargo test` 0、CS-001 三格未变宽 |
| `T-049` | 会话消息的事实源是 `SessionStore`，WAL 轮末入格但读侧不以它为准 ⇒「Store 是唯一原语」在生产路径上不成立 | S | `git log --grep=S0` |
| `T-050` | 读路径不并读两源 ⇒ 真源切换的偏差无声 | S | `git log --grep=S1` |
| `T-051` | v1 档与 `V2Mode` 旋钮仍在，v2 无法独立运行 | S | `git log --grep=S2.5` |
| `T-052` | 思考与半截产出不入格 ⇒ 中断后历史里那半句话消失，上下文悬空 | S | `git log --grep=S2.6` |
| `T-056` | 会话消息的事实源仍是 `SessionStore`（`messages.json` 整份覆盖），WAL 只是读侧兜底 | S | `1bfce16`（主体切源）+ `941cf88`（S2.7-b/c）+ `git log --grep=S2.7-a`（S2.7-a 退役对账） |
| `T-053` | 主读路径没切：缓存不在才重建，写侧仍整份落正文；S1 双源对账与 S2 旧读法尚未退役（切源后单源、无对账对象） | S | `git log --grep=S2.7-a` |
| `T-054` | 写入期裁剪不落事实 ⇒ 切源后被裁节点复活（ADR-054 后果节：裁剪入格 or 读侧按同一窗口裁，无第三种） | S | `941cf88` |
| `T-055` | 写者令牌会静默丢事实：编辑恰逢一轮运行中拿 `NotTheWriter` ⇒ 该 `message.edited` / `removed` 被丢弃（切源后即「编辑偶发不生效」且无人报错） | S | `941cf88` |
| `T-057` | Q3 进展读不走 WAL | S | `git log --grep=S2` |
| `T-059` | core 根出口豁免账本 18 条，大半条目在补一个口径的洞（「core 内另有消费方，审计只数跨出 core 的那一个」） | W | `27922b8` |
| `T-060` | 其余四条豁免通道（doc-link / dead-code / panic / color）的清除任务挂在不存在的批次上 | W | `7eabd54` |
| `T-062` | parent 机制实现：schema + RG-024/025 + 分解树渲染 + board 查询 | meta | registry-lib.mjs（COMMON_OPTIONAL 加 parent；KINDS 加 retiredStates 退场档，与 requiresDepsDone 生效档成对；auditEntries 增 RG-024 结构段 / RG-025 档位单调段；topoOrder 纳入 parent 边；treeSection + withTree 条件渲染；parentDepth 导出）；registry-board.mjs（--children 子树递归 / --parent 父链 / 组内按深度缩进 / notFound 前缀提示）；验证：registry-audit.test.mjs 41/41（新增 3 个 parent 用例） |
| `T-063` | parent 规范与总纲更新：SP-001 / PL-011 / docs/README | meta | docs/spec/SP-001.md（字段表 parent 行 + 「分解：parent」小节：与 deps 正交表 / 同 kind / 编号不体现层级 / 档位单调 / 呈现=投影；编号规范段与判据段连带）；docs/plan/PL-011-登记与汇总机制.md（§1.2 新节 + §0 判据范围 RG-025 + §3 父子档位单调 + §5 命令表 + §7 明确不做「不把层级编进编号」）；docs/README.md（目标分层节两字段口径 + 编号不体现分解层级）；验证：RG-011 字段对账通过 |
| `T-064` | parent 回归测试与数据示范：注入用例 + GO/T 两棵分解树 | meta | registry-audit.test.mjs 新增 3 个 test（RG-024 五形态各红一条 + 同 kind 反向；RG-025 两规则 + 「父 doing 子 todo / 子 wont 父 doing / 父子同档」三合法组合 + 文档类同款；渲染缩进到孙层 / 同层编号序 / 无数据零变化；board --children 递归孙辈 / --parent 深度链 / 三选一互斥 / 编号不存在有出口）41/41 全绿；dogfood 数据：GO-001 前端去业务化 → GO-002 三栏机制 / GO-003 表单机制 / GO-004 统一资源管理，T-061 → T-062/063/064；实测：goal/INDEX 与 task/INDEX 分解树小节缩进呈现、board --children=GO-001-前端去业务化 三子、--parent=T-064 父链深度 1；gen-doc-refs 展开编号引用 8 处 |

## 不做
（无）

## 分解树

分解关系只写在 `parent` 字段（子条目指向父条目、同 kind；编号平铺不体现层级——层级是投影不是身份）。
同状态的父子在上面的分组表里相邻；本小节是跨状态的分解全景。

- [`T-001`](T-001.md) 登记机制落地：单文档 + 守卫 + 汇总 + 总纲（done）
- [`T-002`](T-002.md) 接管 13 §1 问题登记（状态表格迁成任务单文档）（done）
- [`T-003`](T-003.md) 接管 docs/README 纪律地图（每条纪律成为 spec 条目）（done）
- [`T-004`](T-004.md) 散落决策清点与登记形态定案（决策类元素的接管边界）（done）
- [`T-005`](T-005.md) 接管 13 §2.1 豁免登记表（豁免 → spec + clears 字段）（done）
- [`T-006`](T-006.md) 接管 PL-004 §3.1 工具台账（先分清守卫投影行与人维护登记行）（done）
- [`T-008`](T-008.md) ADR 接管 registry（新开 adr kind：索引与正文一并迁入，删除 DECISIONS.md 与 decisions/*.md）（done）
- [`T-010`](T-010.md) registry 迁入 docs——条目库与文档目录合一（done）
- [`T-011`](T-011.md) 五轴的外部效标：每条轴挂一个可复现的读数，而不是形容词（done）
- [`T-012`](T-012.md) 能力名册 ↔ 代码锚点：名册的每一条能力都要指到实现它的地方（done）
- [`T-013`](T-013.md) Rust 覆盖率阈值：与 C7 同族的已知盲区，排在会话真源归一之后（done）
- [`T-014`](T-014.md) 分布式 Store 的 seq 单调与「无溯源不声明」如何共存（G4）（done）
- [`T-015`](T-015.md) 出厂 prompt 在线上是两条 system 消息，部分 provider 直接 400 拒收——full 档在那类 provider 上第一轮就失败（done）
- [`T-016`](T-016.md) 对话面理由词表是两份抄本，靠一条跨插件测试防漂移（done）
- [`T-017`](T-017.md) 两个巨型文件混域（`tool_executor.rs` / `actors/mod.rs`）（done）
- [`T-018`](T-018.md) 无界队列一处：流式 UI 帧通道不设界（done）
- [`T-019`](T-019.md) 前端两个热点文件混域（`sessions.ts` / `DetailForm.vue`）（done）
- [`T-020`](T-020.md) 健壮性面无任何判据：非测试 Rust 的 `unwrap` / `expect` / `panic!` 逐处无登记（done）
- [`T-021`](T-021.md) 文档里的数字既没有守卫、也没有 owner（D3 / D4 的出口写着「由新增计数守卫守」而那个守卫不存在）（done）
- [`T-022`](T-022.md) 已全批落地的施工文档仍是活跃文档（done）
- [`T-023`](T-023.md) 同一事实在多处分册复述（机制矩阵 / C-D 断言 / OUTPUT.md / PL-002 §2.3）（done）
- [`T-024`](T-024.md) 口径不一致，其中一条论证本身写错（共 7 处，不止原登记的 1 处）（done）
- [`T-025`](T-025.md) 计数与代码脱节（「N 条」对不上号）（done）
- [`T-026`](T-026.md) 归档与变更史卫生（`WINDOWS-RESTART*.md`、生成模板的变更史措辞）（done）
- [`T-027`](T-027.md) verify 程序的数据全部手填 ⇒ 验的是「计划自洽」，不是「计划 == 代码」（直接违反 README §4 纪律 2「断言的输入不得包含结论」）（done）
- [`T-028`](T-028.md) 路由常量的名↔值与值是否真实存在两条都没有判据，而 `plugin/route.rs` 的模块文档声称有守卫（done）
- [`T-029`](T-029.md) 门禁自身的结构约定靠约定：重生成阶段（`autoWork`）必须最后跑，无任何东西判这一点；守卫命名空间未闭合（一份既无回归测试又不在任何名单里的 `.mjs` 是完全隐形的）（done）
- [`T-030`](T-030.md) 棘轮覆盖面只到一处：`35-baseline.mjs` 只守 `_shared.mjs` 的 `BASELINE` 一个对象，同类 floor / ceiling 三条在守卫之外（done）
- [`T-031`](T-031.md) 棘轮计量单位只到文件级：verify 只数程序数、e2e 侧完全无数，删到只剩一条 `assert!` 两个数都不变（done）
- [`T-032`](T-032.md) `BASELINE` 是源码常量，任一格可被 PR 静默下调；高于基线只打黄字不红（done）
- [`T-033`](T-033.md) 守卫判据码无中央登记处，防撞号纯靠约定（done）
- [`T-034`](T-034.md) 前后端契约手写两份：除 3 条别名外其余 `ROUTE_*` 无前端对账（done）
- [`T-035`](T-035.md) 文档指认的符号名与代码实名不逐字（done）
- [`T-036`](T-036.md) 代码注释以批次口径叙述实施排期（done）
- [`T-037`](T-037.md) 测试文件存在 ≠ 会被门禁跑：两份名单无人对账（done）
- [`T-038`](T-038.md) 「13 阶全程 0 值域扩展」这句话没有可执行判据（done）
- [`T-039`](T-039.md) 值域判据套到 16 条外部需求上撞出标注与 §8 不符（done）
- [`T-040`](T-040.md) 规划缺产品层：无目标用户、无用户任务，出口判据是「git diff 没改 F1–F6」而非任何用户可见指标（done）
- [`T-041`](T-041.md) SLO 数字基于 mock 毫秒，唯一扫描口是 `#[ignore]` 测试 ⇒ 从未在 CI 出数（done）
- [`T-042`](T-042.md) 反射 / 快路在出厂档位下结构上产生不了样本；照出的八个静默缺口已全部修完（done）
- [`T-043`](T-043.md) 反射档样本恒为零：两个出厂 `false` 的开关缺一不可（done）
- [`T-044`](T-044.md) 读数口「读得出」但「看不见」：`session/stats` 八列在生产前端与 CLI 零调用方（done）
- [`T-045`](T-045.md) 召回率无评测集（`vec` 恒空、检索判据只吃 tag），但对应阶段状态记为已完成（done）
- [`T-046`](T-046.md) 没有一份 prompt 形状基线，无法回答「结构化对模型是变好还是变坏」——没有它，任何「更有效」的判断都是凭感觉拍板（done）
- [`T-047`](T-047.md) `full` 没被真实会话负载压过：e2e 全绿只是必要条件，不是「跑过真实负载」（done）
- [`T-048`](T-048.md) 「结构化对模型是变好还是变坏」仍无读数：门内判据只到形状与有界性，模型侧对照需要一次独立的可授权实验（done）
- [`T-049`](T-049.md) 会话消息的事实源是 `SessionStore`，WAL 轮末入格但读侧不以它为准 ⇒「Store 是唯一原语」在生产路径上不成立（done）
- [`T-050`](T-050.md) 读路径不并读两源 ⇒ 真源切换的偏差无声（done）
- [`T-051`](T-051.md) v1 档与 `V2Mode` 旋钮仍在，v2 无法独立运行（done）
- [`T-052`](T-052.md) 思考与半截产出不入格 ⇒ 中断后历史里那半句话消失，上下文悬空（done）
- [`T-053`](T-053.md) 主读路径没切：缓存不在才重建，写侧仍整份落正文；S1 双源对账与 S2 旧读法尚未退役（切源后单源、无对账对象）（done）
- [`T-054`](T-054.md) 写入期裁剪不落事实 ⇒ 切源后被裁节点复活（ADR-054 后果节：裁剪入格 or 读侧按同一窗口裁，无第三种）（done）
- [`T-055`](T-055.md) 写者令牌会静默丢事实：编辑恰逢一轮运行中拿 `NotTheWriter` ⇒ 该 `message.edited` / `removed` 被丢弃（切源后即「编辑偶发不生效」且无人报错）（done）
- [`T-056`](T-056.md) 会话消息的事实源仍是 `SessionStore`（`messages.json` 整份覆盖），WAL 只是读侧兜底（done）
- [`T-057`](T-057.md) Q3 进展读不走 WAL（done）
- [`T-058`](T-058.md) 22 步实施状态闭环（PL-004 §3.1 A 表——每步实际接入 + 死码归零 + 门禁绿 + 出厂档位压过）（done）
- [`T-059`](T-059.md) core 根出口豁免账本 18 条，大半条目在补一个口径的洞（「core 内另有消费方，审计只数跨出 core 的那一个」）（done）
- [`T-060`](T-060.md) 其余四条豁免通道（doc-link / dead-code / panic / color）的清除任务挂在不存在的批次上（done）
- [`T-061`](T-061.md) 登记机制支持条目分解：parent 字段（父子机制）（done） · 子项 done 3
  - [`T-062`](T-062.md) parent 机制实现：schema + RG-024/025 + 分解树渲染 + board 查询（done）
  - [`T-063`](T-063.md) parent 规范与总纲更新：SP-001 / PL-011 / docs/README（done）
  - [`T-064`](T-064.md) parent 回归测试与数据示范：注入用例 + GO/T 两棵分解树（done）
- [`T-065`](T-065.md) 修复 e2e t30 既有失败：父会话线程里出现了子会话的用户正文（done）
- [`T-066`](T-066.md) megabatch 守卫：单响应工具调用数上限 + 中止原因如实标注（`6cf9520d` 会话死循环）（done）
- [`T-067-mockLLM体系化机制化`](T-067-mockLLM体系化机制化.md) mockLLM 体系化 / 机制化完善：真实 LLM 服务与真实网络环境的异常形态注入（done）
- [`T-068-e2e用例体系化升级`](T-068-e2e用例体系化升级.md) e2e 用例体系化建设与升级：正常 + 异常场景全覆盖，按真实用户使用过程取形（done）
- [`T-069-前端会话机制分析与优化`](T-069-前端会话机制分析与优化.md) 前端 v2 会话机制分析与优化：多线程会话模型与「一个会话线程一个显示区域」（done）
- [`T-070-会话呈现合流单窗口`](T-070-会话呈现合流单窗口.md) 前端呈现合流：退役「对话 / 工作」分栏，一个会话线程一个显示区域（done）
- [`T-071-core形状变宽的必需性论证`](T-071-core形状变宽的必需性论证.md) core 形状棘轮亮红（fn 73>72 / defaults 25>24）：逐格补「必需」论证 ADR 或回退（todo）
