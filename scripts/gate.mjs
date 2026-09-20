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
 *   2. frontend  vue-tsc --noEmit / vitest run --coverage / vite build（在 `tauri/`）
 *   3. docs      grep-audit / mechanism-audit / plugin-entry-audit / style-audit
 *                / doc-link-audit / test-layout-audit / dead-code-audit（均判定型）
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
import { red, green, yellow, dim, bold, stripAnsi } from './color.mjs'

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
  // 732 → 765：后端审查 8 项发现落地的回归测试（+33，按文件分布：
  //   web/http_request 6、local/policy 5、web/web_search 4、telegram/schemas 4、
  //   hook/registry 4、model/protocols/mod 3、event_bus/plugin 3、session/store/tests 2、
  //   vdfs_service/dir 1、vdfs/fs 1）。
  //   覆盖：路径穿越的反斜杠分支与条目层分隔符走私（写哨兵目录证明 `remove_dir_all`
  //   逃不出条目根）、shell 判定改为逐段过白名单 / 命令替换被拒 / wrapper 强制审批 /
  //   限流由恒 false 变为真生效、会话并发保存的 tmp 名唯一与「整段读-改-写」串行化、
  //   SSE 前缀容忍、web 的 SSRF 与通配域名标签边界。
  //   该批同时修掉两个真实缺陷（SSRF 私有 IP 前缀判定、裸 `ends_with` 放行
  //   `evil-example.com`），故这 +33 不只是「补测试」，也把缺陷本身钉住了；
  //   另补 web / hook / telegram / event_bus 四个原零测试插件的首批用例。
  // 765 → 785：后端架构评估的改进项落地（+20，按文件分布：
  //   gateway/server 12、home/plugin.test 4、symbio_core/plugin 3、symbio_core/error 1）。
  //   覆盖：此前**零测试**的 gateway 网络面（SHA-1 已知答案与 RFC 6455 握手向量、
  //   Bearer 鉴权不得凭前缀放行、头 64K / 体 8M / WS 帧上限须在**分配之前**拒绝、
  //   掩码帧解码与三种长度编码往返）、home 的 `parse_path` 路由原语与
  //   `work/get_workspace` 读配置缓存、`SimpleRequest::child_of` 的环境**快照**语义
  //   （改子不得回流到父）、锁辅助在毒化后**恢复数据而非二次 panic**。
  //   顺带把 `read_request` 由 `TcpStream` 泛化到 `AsyncRead`——那三道防 DoS 的
  //   闸门此前根本无法被测，只能靠"读代码看起来对"。
  // 765 → 788：VDFS 挂载根名收口（+3，全部在 symbio_core/vdfs/address.rs：
  //   join_addr 拼接规则 1 例、AddrRootDecl 静态声明可达与归一化 1 例、
  //   descend_addr 转发跳改写规则 1 例——顶层落到声明根、嵌套原地续接）。
  //   该批同时把挂载根改名 `.vdfs` → `.vdfsv2` 证明系统与根名无关；
  //   新机制「当前父地址」＝转发即改写上下文（VDFS_PARENT_ADDR）+ 协议级
  //   绝对地址经 absolute_addr 拼接，原全局登记槽整体删除。
  // 788 → 793：嵌套装配的断点修复与验证（+5，全部在 plugins/composite/）。
  //   背景：bundle 子树的 provider 经 SubAgentVisitor 以**多段名**（`agent/<id>/<name>`）
  //   注册进系统容器，但此链路从未被测试——实测发现两处断点并修复：
  //   1) `CompositeVdfs::resolve` 原按首段全等匹配，多段名永远不可寻址 → 改**最长
  //      前缀**命中（`agent/b1/skill` 先于 `agent`），空地址守卫语义保留；
  //   2) `Composite::route` / `Composite::traverse` 原无条件从声明根起算父地址，
  //      嵌套容器收集期/路由期会拼出与实际挂载不符的地址 → 改 `descend_addr`
  //      从 ctx 已携带的父地址续接。
  //   新测试：vdfs.rs 2 例（多段名最长前缀命中 + 父地址=完整挂载点；根清单原样
  //   呈现多段名）、composite.rs 3 例（route / traverse 转发链逐级续接、顶层落到
  //   声明根——探针插件断言，根名无关）。
  // 793 → 794：子智能体挂载点穿越九操作一致——read / write 改为经子 composite
  //   视图（此前 list / stat / delete 穿了、read / write 落到裸 agent 目录），
  //   新增数据落点回归（列 / 统计 / 读 / 写 / 删 / 建 / 移 + 跨挂载点拒移）。
  rustTests: 794,
  // 31 → 47：前端半边的棘轮**长期停摆**（详见下方 vitestTests 的说明）。
  //   与覆盖率阈值不同，**文件数 / 用例数与平台无关**：全仓 `*.spec.ts` 里零
  //   `skipIf` / `runIf` / `process.platform` 分支，两处 `it.each` 遍历的也都是
  //   静态常量数组（`ALL_RENDERERS` / `MESSAGE_TYPES`）⇒ 注册数由源码唯一决定。
  //   所以这一项可以照实测值钉死，不必等 CI。
  vitestFiles: 47,
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
  // 321 → 319（-2，用例数**下降**故须说明）：
  //   · **删 -12**：`utils/message.ts` 是零引用的死代码（「消息内容 → 文本」的第 5 份
  //     实现），连同它的 12 条用例一并删除——那 12 条是在给一段没人调用的实现作证。
  //   · **增 +14**：`schemas/chat_message.spec.ts` ×7（`messageTextOf` 三种内容形状，
  //     含「`ContentPart[]` 曾被读成空串」的回归）、`stuckFailurePlanOf` ×3、
  //     会话节点订阅的启停与幂等 ×2、提示音未注入来源时的兜底 ×1、
  //     `messageRendererKey` 防漏登记 ×1。
  // 319 → 360（文件 24 → 28）：前端机制化收敛（P1–P7）——
  //   · P1 三栏拍平：NavRail / VdfsShell 并入 Workbench（删 2 个组件、去死插槽）；
  //   · P2 公共详情外壳 `DetailShell.vue` + 渲染器统一契约 `rendererContract.ts`
  //     （此前头注释里的散文契约变成一份接口）；
  //   · P3 机制动作单点：`useVdfs.mechanismActions` 一处计算，`mergeDetailActions`
  //     一处装配（同 id 去重 + divider）——那条规则此前在三处各写一遍；
  //   · P4 注册表工厂化：`registry/factory.ts` 的 `createRendererRegistry` 成为
  //     「标识 → 组件 + 兜底」的唯一实现，VDFS 域与消息域各声明一次；
  //   · P5 死代码：`utils/time.ts::formatTime`（零引用但被测试养着）、
  //     `services/vdfs.ts::mkdirVdfs` / `treeVdfs`（零引用）一并删除；
  //   · P6/P7 交互收口与时间归一：`window.prompt/confirm` → 机制内联重命名 +
  //     `ConfirmDialog`；相对/绝对时间各自一处（含秒/毫秒两口径）。
  //   新增 spec：`schemas/vdfs-form.spec.ts` ×6（mergeDetailActions）、
  //   `registry/factory.spec.ts` ×6（注册表三动作 + 两域隔离）、
  //   `components/vdfs/VdfsDetailActions.spec.ts` ×8（三个渲染器的动作装配护栏）、
  //   `schemas/vdfs.spec.ts` +5（`isVdfsDraft` / `isVdfsSystemAddr`）；
  //   `VdfsFormDetail.spec.ts` 由「自算去重」改为「传参直通」基线（3 → 3，净零）。
  // 360 → 400（文件 28 → 31）：二次复核识别出的 6 项缺口逐项落地。
  //   · **竞态守卫单点**：新增 `composables/useGenerationGuard.ts`（约 25 行）+
  //     `useGenerationGuard.spec.ts` ×7，替掉 `useVdfs` 的 `detailToken` /
  //     `appendGen+appendGenPath+appliedAppends` 与 `ChatMainPanel.loadSequence`
  //     三份手写实现（「取代次 → 响应回来比对 → 过期即丢」此前写了三遍）。
  //   · **`DetailForm` 纯逻辑下沉**：`schemas/vdfs-form.spec.ts` ×13 —— 预设联动
  //     （`detailPresetOf` / `…FieldOptions` / `…FieldSuggestions` / `…PresetPatch`）
  //     从 907 行的组件里搬到 schemas，规则因此可直接单测；6 → 19。
  //   · **VdfsWorkbench 内联 prompt 机制化**：新增
  //     `components/vdfs/__tests__/VdfsWorkbench.spec.ts` ×9 —— 两个布尔
  //     （`creatingTyped`/`renaming`）+ 三个载荷 ref 收成一个判别式 `promptKind`，
  //     外壳改用 `DetailShell`；用例钉住「三个瞬态互斥」与「类型清单不显示 `ext`」。
  //   · **会话懒创建握手收敛**：新增 `components/chat/__tests__/ChatComposer.spec.ts`
  //     ×8 —— 输入区（`ChatInputArea + ChatOptionBar`）此前在 `Session.vue` 与
  //     `ModelChatPanel.vue` 各装配一遍，收成一个 `ChatComposer`；用例钉住两件都在、
  //     `sessionId` 草稿态必须是 `undefined`（透传成空串会丢掉草稿选择）、
  //     `resetHeight`/`getDraftMetadata` 只经一个 ref 委派。
  //     `sessions.spec.ts` ×3 —— `createSessionWithFirstMessage` 把「建会话 + 排队
  //     首条消息」合成原子操作，「队列项 id === 新建出的 id」这条不变式由 store
  //     保证；邮箱本体不再对外暴露（唯一入口是该原子方法）。
  //   · 余下两项是**改脚本/纯减法**，不增用例：`style-audit.mjs` 新增 §1.5
  //     「registry 词表」规则（认识动态绑定的**来源**，消掉 7 条 scoped 死类误报，
  //     而非加白名单）；删 `createSessionId` / `SESSION_LIST_LIMIT` / `VDFS_TREE` /
  //     `VDFS_MKDIR` / `VdfsTreeResponse` / `setVdfsSessionScheme` 及 CSS 通用
  //     别名层（10 个 `--color-*` 改名到语义令牌，60 处引用）。
  //   注：`--color-chip-*` / `--color-error-*` / `--color-banner-*` 等是**聊天域
  //   令牌**（直接给字面值，与语义令牌并列成组），不属被删的别名层，勿连带删除。
  // 400 → 661（文件 31 → 47）：**前端半边的棘轮长期停摆**。上面 rustTests 一路
  //   精细维护到 794，而这一项自「二次复核」定到 400 后再没动过——实测已是 661，
  //   也就是说**删掉 261 个用例也不会红**（占现有用例的四成）。这与覆盖率阈值
  //   42.27% → 60.93% 是同一个病：**棘轮一旦没人拧，就成了摆设**。
  //
  //   为什么这次可以直接上调（而覆盖率阈值仍留待决策）：**用例数与平台无关**。
  //   全仓 `*.spec.ts` 零 `skipIf` / `runIf` / `process.platform` 分支；仅有的两处
  //   `it.each`（`messageRenderers.spec.ts`）遍历的是静态常量数组
  //   （`ALL_RENDERERS` / `MESSAGE_TYPES`）⇒ 用例数由源码唯一决定，Linux runner
  //   与本地必然同数。覆盖率则是**分支命中率**，路径处理等平台分支会真的不同，
  //   故那一项仍须先在 CI 取实测值（见 tauri/vitest.config.ts 的注释）。
  //
  //   另补上了缺失的「超过基线 ⇒ 提示更新」——Rust 半边一直有，前端半边没有，
  //   于是它涨了 261 个用例都没人被告知（**没有提示的棘轮等于没有棘轮**）。
  vitestTests: 661,
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
// 配色与 `stripAnsi` 都来自 `color.mjs`（唯一实现）。

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

/**
 * 沙箱「批量删除守卫」的提示。
 *
 * vite 构建要清空 `dist/`、vitest 覆盖率要清 `coverage/.tmp` —— 两者都是
 * **几百个文件**的一次性删除。某些沙箱（含本机开发容器）对超过阈值的批量删除
 * 会直接拦下，于是**构建本身没失败**、退出码却是 1。
 *
 * 这里只加提示、**不改判定**：退出码说了算（与 clippy 的沙箱误报同一口径）。
 */
const SANDBOX_DELETE_RE = /safe-delete|SAFE_DELETE/

function maybeSandboxDeleteHint(output) {
  if (!SANDBOX_DELETE_RE.test(output)) return
  console.log(
    yellow('      ⚠ 输出含「批量删除被拦」字样：这是沙箱限制，不是构建/测试失败。'),
  )
  console.log(yellow('        确认方法：在沙箱外跑同一条命令，或先手工清掉 dist/ 与 coverage/。'))
  console.log(
    yellow('        只验覆盖率阈值时可绕过这次删除：`vitest run --coverage --coverage.clean=false`'),
  )
}

/**
 * 是否因沙箱拦截而**没被真正判定**。
 *
 * 判定只看这一个特征串：它是环境抛出的（不是 vite / vitest 自己的错误），
 * 真失败不会带它。命中即记「未判定」而非「失败」——否则每次跑完构建，
 * 下一轮门禁必然红（要清 dist/ 与 coverage/，几百个文件），
 * 一个**必然红**的门禁比没有门禁更糟：人会学会忽略它。
 */
function blockedBySandboxDelete(output) {
  return SANDBOX_DELETE_RE.test(output)
}

function slug(s) {
  return s.replace(/[^a-zA-Z0-9]+/g, '-').replace(/^-|-$/g, '').toLowerCase() || 'step'
}

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

  // 覆盖率与测试同一次运行（阈值写在 `vitest.config.ts`，不达阈值 vitest 自己
  // 以非零码退出）——单独再跑一遍测试没有意义，只是多花一倍时间。
  const vitest = await run({
    label: 'vitest run --coverage',
    cmd: process.execPath,
    args: [path.join(frontendDir, 'node_modules', 'vitest', 'vitest.mjs'), 'run', '--coverage'],
    cwd: frontendDir,
    timeoutMs: VITEST_TIMEOUT_MS,
  })
  const files = grabInt(vitest.output, /Test Files\s+(\d+) passed/)
  const tests = grabInt(vitest.output, /Tests\s+(\d+) passed/)
  const enough =
    files !== null && tests !== null && files >= BASELINE.vitestFiles && tests >= BASELINE.vitestTests

  if (!vitest.ok) {
    // 区分两类红：覆盖率不达阈值会在输出里留 `ERROR: Coverage for ...`，
    // 与「用例失败」不是一回事，提示要指对地方（阈值在 vitest.config.ts）。
    const coverageRed = /Coverage for .* does not meet/.test(stripAnsi(vitest.output))
    if (!coverageRed) maybeSandboxDeleteHint(vitest.output)
    // 沙箱拦截导致的「未判定」：不记失败，但写明（见 blockedBySandboxDelete 的说明）。
    // **不 return**：本阶段后面还有 vite build / eslint，沙箱只影响覆盖率这一步；
    // 早退会让那两步连跑都不跑（汇总里根本不出现），把「没检查」伪装成「通过」——
    // 本文件顶部那条「一个必然红 / 静默跳过的门禁比没有门禁更糟」说的就是这种。
    if (!coverageRed && blockedBySandboxDelete(vitest.output)) {
      record('frontend', 'vitest run --coverage', true, '沙箱拦截批量删除 ⇒ 本步未判定')
    } else {
      const note = vitest.timedOut
        ? '超时终止'
        : coverageRed
          ? '覆盖率低于阈值（见 tauri/vitest.config.ts 的 coverage.thresholds）'
          : `exit=${vitest.code}, signal=${vitest.signal}`
      record('frontend', 'vitest run --coverage', false, note)
    }
  } else if (!enough) {
    record('frontend', 'vitest run --coverage', false, `文件/用例数未达基线或无法解析：${files}/${tests}（基线 ${BASELINE.vitestFiles}/${BASELINE.vitestTests}）`)
  } else {
    // 与 Rust 半边同款提示：**超过基线要说一声**，否则棘轮会像 400 → 661 那样
    // 悄悄落后（那边一直有这个提示，所以 rustTests 从没掉队；这边没有，于是掉了）。
    const grew = files > BASELINE.vitestFiles || tests > BASELINE.vitestTests
    console.log(
      grew
        ? yellow(`      ⚠ ${files} 文件 / ${tests} 用例 > 基线 ${BASELINE.vitestFiles}/${BASELINE.vitestTests}：请更新 scripts/gate.mjs 的 BASELINE.vitestFiles / vitestTests`)
        : dim(`      ${files} 文件 / ${tests} 用例（基线 ${BASELINE.vitestFiles}/${BASELINE.vitestTests}）`)
    )
    record(
      'frontend',
      'vitest run --coverage',
      true,
      grew ? `文件/用例数 ${files}/${tests}（基线待更新）` : ''
    )
  }

  // 构建：类型检查过了不代表**打包得过**（打包器自己的错误——循环依赖、动态导入
  // 写错、chunk 配置指向已删模块——只在 build 时才暴露）。此前它只在上线的
  // release 流程里跑，等于「构建坏了要等打 tag 才知道」。
  // 直接调本地 vite 二进制（与脚本其余部分一致：不依赖 npx / npm run 的解析）。
  const build = await run({
    label: 'vite build',
    cmd: process.execPath,
    args: [path.join(frontendDir, 'node_modules', 'vite', 'bin', 'vite.js'), 'build'],
    cwd: frontendDir,
    echo: 'filtered',
  })
  if (!build.ok) console.log(yellow('      ↳ 构建失败：本地复现用 `npm run build`（在 tauri/ 下）'))
  if (!build.ok) maybeSandboxDeleteHint(build.output)
  // 同样是「只跳过本步」：后面还有 eslint，早退会把它一起吞掉
  if (!build.ok && blockedBySandboxDelete(build.output)) {
    record('frontend', 'vite build', true, '沙箱拦截批量删除 ⇒ 本步未判定')
  } else {
    record('frontend', 'vite build', build.ok)
  }

  // 分层 lint：`no-restricted-imports` 把 M-003 / M-004 / M-005 与
  // 「service 不认识 store」变成**结构化**判定（正则守卫看不见重导出与动态导入）。
  const lint = await run({
    label: 'eslint（分层约束）',
    cmd: process.execPath,
    args: [path.join(frontendDir, 'node_modules', 'eslint', 'bin', 'eslint.js'), '.'],
    cwd: frontendDir,
  })
  if (!lint.ok) console.log(yellow('      ↳ 本地复现用 `npm run lint`（在 tauri/ 下）'))
  record('frontend', 'eslint（分层约束）', lint.ok)
}

async function stageDocs() {
  stageHeader('docs', '静态审计')
  // 判定型守卫的**回归测试**必须先跑：一个只会亮绿灯的守卫等于没有守卫，
  // 而它腐烂的方式恰恰是「规则写错了所以永远不命中」——只有注入真实违规
  // 并断言脚本变红，才能把「通过」和「没在工作」区分开。
  for (const name of [
    'grep-audit',
    'mechanism-audit',
    'plugin-entry-audit',
    'protocol-mirror-audit',
    // dead-code-audit 的 R-001（Rust 声明级）已从「降级提示」提升为**判定型**，
    // 故按同一条教条：先把它的回归测试跑起来（注入真实违规断言变红），
    // 再把脚本本身当门禁——否则「规则写错所以永远不命中」没人会发现。
    'dead-code-audit',
    // `color` 不是审计脚本而是**共享库**（`scripts/color.mjs`），但它带一道守卫：
    // 「`scripts/` 下除它自己外不得手写 ANSI 转义」。这道守卫值得跑——配色样板曾
    // 在 6 个脚本里逐字复制，其中 `protocol-mirror-audit` 那份把 `\x1b` 写丢，
    // 而它的回归测试都设 `NO_COLOR=1`，正好绕过坏掉的分支 ⇒ 谁也没发现。
    // 故它在下面那个"审计脚本"循环里**没有**对应项，只跑回归测试。
    'color',
  ]) {
    const t = await run({
      label: `${name} 回归测试`,
      cmd: process.execPath,
      args: ['--test', path.join(scriptDir, `${name}.test.mjs`)],
      cwd: repoRoot,
    })
    record('docs', `${name} 回归测试`, t.ok)
  }
  // 判定型：有发现即以非零退出码失败。
  // `plugin-entry-audit` 也是判定型，但只有 E-001 ~ E-004（ERROR）会失败；
  // E-005 / E-006 是 WARNING（退出码仍为 0），因为运行期拼路径数不出来，
  // 「定义了但没人用」只能报给人看，不能自动判死。
  for (const name of [
    'grep-audit',
    'mechanism-audit',
    'plugin-entry-audit',
    'style-audit',
    'doc-link-audit',
    'test-layout-audit',
    'dead-code-audit',
    'protocol-mirror-audit',
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
