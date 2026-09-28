#!/usr/bin/env node
/**
 * 全仓实现行数棘轮审计（line-budget-audit）
 *
 * 目标：防代码膨胀。对全仓各主要模块/插件的**生产实现行数**设定天花板（基线）：
 * - 超出基线：ERROR 报错，阻断 gate 提交；若增长是有意的，必须显式修改基线并注明日期与理由；
 * - 低于基线：WARN 提示下调收紧棘轮，防止历史债务回弹；
 * - 测试代码（*.test.rs / *.spec.ts / tests.rs / #[cfg(test)] 内联模块）不计入天花板，鼓励写测试。
 *
 * 统计口径与 `scripts/line-count.mjs`（即 `docs/CURRENT.md` §5.1）完全一致。
 *
 * 用法：
 *   node scripts/line-budget-audit.mjs                  # 全仓审计
 *   node scripts/line-budget-audit.mjs --strict         # 告警视同错误（CI 可选）
 *   node scripts/line-budget-audit.mjs ROOT=<dir>       # 指定根目录（测试 fixture 用）
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { red, yellow, green, dim } from "./color.mjs";
import { scopeRow } from "./line-count.mjs";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const defaultRoot = path.resolve(scriptDir, "..");
const rootArg =
  process.argv.find((a) => a.startsWith("ROOT=")) ??
  process.argv.find((a) => a.startsWith("--root="));
const repoRoot = rootArg
  ? path.resolve(rootArg.includes("=") ? rootArg.split("=")[1] : "")
  : defaultRoot;
const baselineArg = process.argv.find((a) => a.startsWith("--baseline-file="));
const STRICT = process.argv.includes("--strict");

/**
 * 生产实现行数天花板基线（按模块细化）
 *
 * 口径：各目录下非测试生产代码的换行数，剥除内联 #[cfg(test)] 模块。
 * 调整基线需显式注明日期与增长理由。
 */
export const LINE_BUDGET_BASELINES = {
  // ── 插件层（symbio/src/plugins/*） ──
  // 2026-09-27：gateway / local / telegram / web 改实现 `PluginConfigMount`（四臂 dispatch
  // 收口到 symbio_core），四处共减 ~137 行；core 因新增该机制 +128 行。
  // 2026-09-27：vdfs 的七个同形工具改由 `tools/spec.rs` 一张表驱动（唯一 Capability
  // 实现），删掉七个逐字复制的工具文件，减 ~75 行（机制行数不变）。
  // 2026-09-27：session 基线由 16802 校正为 16811。原值与实测差 ~9 行：基线是在
  // 更早的 `cargo fmt` 之前记录的，fmt 在 `session` 树内累积了净增。本次用精确实测
  // 而非推算重设（`context/window.rs` 三处硬编码 `24` 提为 `ENTRY_NAME_TOKEN_CAP`，
  // 该文件净 -4 行：16815（未改）→ 16811（改后））。
  // 2026-09-27：`clear_messages` 整体下线（与「删除会话」功能重叠），session 基线
  // 收紧 16811 → 16790（净 -21）。
  // 2026-09-28：+30 行（16790 → **16820**）。会话详情页的「进入下一级」改为由后端
  // 声明（`plugins/session/options/mod.rs::session_detail_actions`，与会话选项同一份
  // `schema`），行数全在那一处函数与它的说明里。
  // 2026-09-28（同日第二笔）：+90 行（16820 → **16910**）。压缩节点的**结构化交代**：
  // `context/pipeline.rs` 新增纯函数 `compression_stats`（触发来源 / 上限 /
  // 前后水位 / 丢了几条）与它的口径说明——注释占一半，那几条口径（内容水位不含
  // 请求级开销、`after_tokens` 只在成功时写入）是防止下一个人"顺手补齐"的关键；
  // `chat_loop/state.rs` 新增 `merge_meta`（字段补丁不覆盖已有键）。
  // 2026-09-28（同日第三笔）：+103 行（16910 → **17013**）。压缩摘要的**增量改道**：
  // `context/pipeline.rs` 的 `send_compression_request` 由「出口恒静默」改为
  // 「有发射器 ⇒ `ExecEventSink::filtered` 白名单改道到压缩节点，无发射器 ⇒ 静默」
  // （含 `compression_delta_gate` 的逐帧白名单与它的口径说明），`chat_loop/state.rs`
  // 新增 `CompressionEmitter::transcript_writer` 与 `TranscriptWriterBridge`
  // （只做「锁转写 → apply」的最小 writer）。骨架帧照静、摘要增量照流——注释占
  // 增量的过半，说明的是「为什么不能逐窗口二选一」。
  // 2026-09-27：智能体详情重设计（方案 A，后端驱动）——`agent/host/vdfs.rs` 的
  // `agent_dir_info` 由「能力拼成一个字符串」改为**结构化下发**（`capabilities`
  // 计数对象 + `capability_kinds` 清单 + `capability_count`），新增
  // `capability_counts()`；`agent/host/detail.rs` 把「已装能力」段提到元数据之前
  // 并按新字段重写；`vdfs.test.rs` 加夹具与 3 条用例。基线 3667 → 3736（+69）。
  // 前端零改动：`DetailForm.staticDisplay` 本就支持数组与对象（数组 `join('、')`、
  // 对象逐项 `k v`），故结构化值直接可渲染。
  // 2026-09-28：**收紧 -613 行**（3736 → **3123**）。`agent` 切分：智能体自身功能
  // （记忆 / 配置 / 指令）整体迁出——记忆归 `memory` 插件（ADR-040 之后它还带工作区腿），
  // `agent` 只管 agent 目录库、子树装配与 `agent_run` 委托；`config.rs` / `instruction.rs` /
  // `memory.rs` 及其测试删除，子树装配清单改用共享 `ASSEMBLY_SUB_AGENT_PLUGINS`。
  "symbio/src/plugins/agent": { maxLines: 3123, exts: [".rs"] },
  "symbio/src/plugins/composite": { maxLines: 1330, exts: [".rs"] },
  "symbio/src/plugins/event_bus": { maxLines: 164, exts: [".rs"] },
  "symbio/src/plugins/gateway": { maxLines: 1190, exts: [".rs"] },
  "symbio/src/plugins/home": { maxLines: 928, exts: [".rs"] },
  "symbio/src/plugins/hook": { maxLines: 467, exts: [".rs"] },
  "symbio/src/plugins/local": { maxLines: 3460, exts: [".rs"] },
  // 2026-09-28：新增 **992 行**（首个基线，不含测试）。`memory` 插件 = 智能体记忆 +
  // 工作区记忆两个作用域（原 `work` 插件并入，见 ADR-040）：共享实现 + 挂载名覆盖 +
  // 五字段配置 + 双腿注入 + VDFS 挂载点。
  "symbio/src/plugins/memory": { maxLines: 992, exts: [".rs"] },
  "symbio/src/plugins/mcp": { maxLines: 2897, exts: [".rs"] },
  "symbio/src/plugins/model": { maxLines: 6212, exts: [".rs"] },
  "symbio/src/plugins/plugin_manager": { maxLines: 666, exts: [".rs"] },
  // 2026-09-28：+1 行（17013 → **17014**）。`MemoryNodeSpec` 新增 `name` 覆盖字段
  // （ADR-040 挂载名 ≠ 物理名），session 侧两处构造点各补 `name: None`。
  "symbio/src/plugins/session": { maxLines: 17014, exts: [".rs"] },
  // 2026-09-28：新增 **427 行**（首个基线，不含测试）。`setting` 插件：当前智能体
  // 自身的信息设置（档案 + 偏好），分形（系统树 + 每棵子树各一份）。
  "symbio/src/plugins/setting": { maxLines: 427, exts: [".rs"] },
  "symbio/src/plugins/skill": { maxLines: 1470, exts: [".rs"] },
  "symbio/src/plugins/telegram": { maxLines: 887, exts: [".rs"] },
  // 2026-09-28：+18 行（3012 → **3030**）。列表**检索入口**的服务端回答：
  // `host.rs` 新增 `LIST_SEARCH_MIN_ITEMS`（阀值统一住服务端，各前端不各判一份）
  // 与 `list_at` 里给目录节点补 `search` 的三行。
  "symbio/src/plugins/vdfs": { maxLines: 3030, exts: [".rs"] },
  "symbio/src/plugins/web": { maxLines: 1028, exts: [".rs"] },

  // ── 内核与驱动层（symbio/src/*） ──
  // 2026-09-28：+89 行（8143 → **8232**）。三件事：
  // ① `exec/mod.rs` 新增 [`ExecEventSink::Filtered`] 出口（白名单可改写 / 可吞帧的
  //    过滤桥，压缩摘要增量改道的类型基础）与 `filtered()` 构造器；
  // ② `schemas/detail.rs` 的 `DetailCondition` 三个约束位改 `skip_serializing_if`，
  //    并写明「缺席不得序列化成 `null`」的理由（缺席 `null` ⇒ 整条恒假、界面静默
  //    少一个动作——一次真回归换来判据）；
  // ③ `vdfs/node.rs` 的 `VdfsNode::search` 声明位（三级语义 + 为什么在节点上而
  //    不在列表响应上）。
  // 2026-09-28：+6 行（8232 → **8238**）。`PLUGIN_ID_WORK` 删除、`PLUGIN_ID_MEMORY`
  // 注释与 `symbio_core/mod.rs` 插件域注释更新（work 并入 memory，ADR-040）。
  "symbio/src/symbio_core": { maxLines: 8238, exts: [".rs"] },
  // 2026-09-28：+13 行（2732 → **2745**）。`providers/memory` 的 `MemoryNodeSpec`
  // 新增 `name` 覆盖字段（挂载名 ≠ 物理名）与 `MemoryFile::node` 解析更新，测试同步。
  "symbio/src/providers": { maxLines: 2745, exts: [".rs"] },

  // ── 宿主与工具层 ──
  "cli/src": { maxLines: 1573, exts: [".rs"] },
  "tauri/src-tauri/src": { maxLines: 449, exts: [".rs"] },
  // 2026-09-27：前端 UI/UX 优化 +662 行（20269 → 20931）。
  // 新增三个生产文件（骨架屏两件 + 导航记忆 store）229 行；其余为既有件的机制扩展：
  // `VdfsActions.vue`（进入下一级动作右置与视觉分隔）、`VdfsWorkbench.vue`（列表筛选 UI）、
  // `useVdfs.ts`（筛选投影）、`vdfs-form.ts`（`mergeDetailActions` 排序规则）、
  // `NodeShell.vue`（节点头键盘可达 + aria）。测试代码不计入本口径
  // （`*.spec.ts` 与内联测试模块另行统计，本轮 +19 个用例）。
  // 2026-09-27（同日第二笔）：+134 行（20931 → 21065）。三件事：
  // ① `VdfsWorkbench.vue` 自动开新会话草稿（三条边界 + 注释是本笔的主要行数）；
  // ② `router/index.ts` 冷启动落点改为「无记忆 ⇒ 会话目录」，异步解析必须走
  //    `beforeEach`（vue-router 的 redirect 不接受 Promise），故多一层守卫与注释；
  // ③ `Workbench.vue` 详情区毛玻璃（`@supports` 回退）+ `tokens.css` 玻璃令牌两态。
  // 另 `VdfsWorkbench.vue` 的「新建」按钮由透明 icon-btn 改为填充主题色。
  // 2026-09-27（同日第三笔）：+3 行（21065 → 21068）。图标查找收敛为唯一实现
  // `registry/vdfsIcons.ts::iconForNode`，并**删除**两个中间层
  // （`getVdfsIcon` / `getVdfsIconFor` / `vdfsTypes.ts::dirIconOf`）——净增几乎为零，
  // 剩下的 +3 是新增的「名单级」兜底步骤（`<根>` 挂载点 `kind` 恒为 `dir`，
  // 没有这一步它们会整排退成同一张默认图）+ 该步骤的注释。详见
  // `docs/design/frontend-ui-ux-plan.md` §9。
  // 2026-09-27（同日第四笔）：+103 行（21068 → 21171），两件事：
  //   ① **冷启动守卫在打包环境下不执行**（`router/index.ts`）：判据由
  //      `to.path !== '/'` 改为路由名（打包后 webview 的 `pathname` 不是 `/`，
  //      旧判据每次提前 return），并加兜底路由 `:unknown(.*)*` → `/`（无它时
  //      那种地址渲染成空白页）；`coldStart.spec` 加 3 条把「首段不是 `/`」
  //      搬进单测。⚠️ 这两处**不随「回上次地址」下线而撤销**——它们修的是
  //      「守卫会不会执行」与「陌生地址会不会白屏」，是落点本身的前提。
  //   ② **列表为空时收起中栏**（`common/Workbench.vue` + `vdfs/VdfsWorkbench.vue`）：
  //      容器加 `hasDetail` / `hasDraft` / `canCreate` 三个 prop 与 `showList`
  //      判据，控件侧新增 `hasDetail` / `isDraftSelected` 两个 computed 往上传。
  //      注释已按「够用即可」压缩过一轮——别为了压数字把判据的理由删掉，那几条
  //      正是这份代码里最容易被人"优化"回去的部分。
  // 2026-09-28：收紧 21171 → **21097**（净 −74）。「回上次地址」整条下线：
  //      删 `stores/nav.ts`（-73，与其单测同批删除）、`MainLayout.vue` 的
  //      `nav.remember` watch 与随之不再使用的 `useRoute`（-7）、
  //      `router/index.ts` 的记忆分支（-9），另 `coldStartPath` 与守卫的注释
  //      改写为「为什么不再回上次地址」（+15）——**这 15 行不能省**：它是本机制
  //      唯一留下的「为什么被删掉」的记录，省了下一轮就会有人把它加回来。
  // 2026-09-28（同日第二笔）：收紧 21097 → **21096**（净 −1）。收起中栏的上游
  //      「进入可新建目录 ⇒ 自动备一张草稿」改成**类型无关**（去掉
  //      `creatableType.ext === 'session'` 这个写死的判断）：代码 −1 行、注释
  //      原地改写为「为什么不得出现类型名」。注释**没有变长**——那句「为什么」
  //      已经写在本文件的 spec 与 `gate.d/_shared.mjs` 基线里，此处不再复述。
  // 2026-09-28（同日第三笔）：+41 行（21096 → 21137）。修「切侧边栏 ⇒ 详情页不跟着
  //      变」，加了**两条与类型无关的机制**，注释占其中大半（它们是这类"写了却看不
  //      出为什么"的代码里唯一能防回退的部分）：
  //      ① `schemas/vdfs.ts::isVdfsUnder`（+17）：「选中项是否还属于当前目录」的
  //         归属判据——换目录时按它立刻作废旧选中项（判「还在不在列表里」要等列表
  //         回来，且有更早分页时判不了，那正是旧选中项能一直挂着的漏洞）。
  //      ② `useVdfs.ts` 的换目录清理 watch（+21）。
  //      ③ 控件侧自动开草稿的监听源由 `cwd` 改为 `cwdNode`（+3 注释）。
  // 2026-09-28（同日第四笔）：+730 行（21137 → **21867**）。前端 UI/UX 第三批
  //      「抵达与导航」，主体是两个新生产文件：首启引导（`components/common/Onboarding.vue`
  //      + `stores/onboarding.ts`）与会话级 UI 状态（`stores/sessionUiState.ts`）；
  //      其余是既有件的机制扩展（`composables/useVdfs.ts` 的挂载层左栏、
  //      `components/vdfs/VdfsWorkbench.vue` 的空态即引导、
  //      `components/vdfs/VdfsSessionDetail.vue` 改为只做动作投影、
  //      `schemas/vdfs-form.ts` 的 `projectDetailActions`）。
  //      测试代码不计入本口径。
  // 2026-09-28（同日第五笔）：+283 行（21867 → **22150**）。会话流与上下文压缩的
  //      可见性。主体是新渲染器 `components/message/MemoryNode.vue`（压缩后的历史
  //      记忆，此前被当成用户消息渲染成气泡）；其余是既有件的机制扩展：
  //      `registry/messageTypes.ts`（`facets.compacted` + 两个 meta 读取口 +
  //      触发来源词表 + token 展示写法）、`registry/messageRenderers.ts`（登记
  //      `memory`）、`components/message/NodeShell.vue`（系统记忆不给悬停操作）、
  //      `components/message/CompressionNode.vue`（按字段渲染事实行）、
  //      `components/chat/ChatInputArea.vue`（发送键 / 停止键收敛为一个语义）。
  //      测试代码不计入本口径。
  // 2026-09-28（同日第六笔）：+54 行（22150 → **22204**）。**检索入口的使用方接线**
  //      与约束位的消费侧口径：`schemas/vdfs.ts` 的 `search?: boolean` 声明 +
  //      `composables/useVdfs.ts` 的 `searchable` computed 与入口收起时清筛选词的
  //      watch、`schemas/vdfs-form.ts` 的 `evalDetailCondition` 把 `null` 与缺席
  //      同义（`!= null` 判据）+ `DetailCondition` 三个约束位的类型放宽、
  //      `VdfsWorkbench.vue` 的筛选框按服务端声明显隐。注释占大半：阀值不得由各
  //      前端自判、`null` 不得当约束——都是「不写就会被下一轮优化掉」的判据。
  //      测试代码不计入本口径。
  // 2026-09-28：+7 行（22204 → **22211**）。`vdfsIcons.ts` 注册 `memory` 挂载点图标
  // （work 从未登记过图标，其删除不产生抵扣；合并见 ADR-040）。
  "tauri/src": { maxLines: 22211, exts: [".ts", ".vue"] },
};

export function runAudit({ root = repoRoot, baselines = LINE_BUDGET_BASELINES, strict = STRICT } = {}) {
  let errors = 0;
  let warnings = 0;
  const reports = [];

  for (const [scopeDir, cfg] of Object.entries(baselines)) {
    const fullDir = path.join(root, scopeDir);
    if (!fs.existsSync(fullDir)) {
      errors++;
      reports.push({
        type: "error",
        scope: scopeDir,
        msg: `目录不存在：${scopeDir}（基线定义了不存在的范围）`,
      });
      continue;
    }

    const stat = scopeRow(root, scopeDir, cfg.exts);
    const actual = stat.implLines;
    const max = cfg.maxLines;

    if (actual > max) {
      errors++;
      const overflow = actual - max;
      reports.push({
        type: "error",
        scope: scopeDir,
        actual,
        max,
        overflow,
        msg: `${scopeDir} 实现行数超标：当前 ${actual} 行，超出基线 ${max} 行共 +${overflow} 行！\n` +
             `    若增长是有意的，请在 scripts/line-budget-audit.mjs 中显式调大基线并注明日期与理由。`,
      });
    } else if (actual < max) {
      warnings++;
      const slack = max - actual;
      reports.push({
        type: "warn",
        scope: scopeDir,
        actual,
        max,
        slack,
        msg: `${scopeDir} 实现行数已缩减：当前 ${actual} 行，低于基线 ${max} 行（可收紧 -${slack} 行）。`,
      });
    } else {
      reports.push({
        type: "ok",
        scope: scopeDir,
        actual,
        max,
      });
    }
  }

  return { errors, warnings, reports };
}

// CLI 执行模式
if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url))
) {
  console.log(dim(`--- line-budget-audit: 实现行数棘轮审计 ---`));
  let baselines = LINE_BUDGET_BASELINES;
  if (baselineArg) {
    const p = path.resolve(baselineArg.slice("--baseline-file=".length));
    baselines = JSON.parse(fs.readFileSync(p, "utf8"));
  }
  const { errors, warnings, reports } = runAudit({ baselines });

  for (const r of reports) {
    if (r.type === "error") {
      console.error(red(`  ERROR: ${r.msg}`));
    } else if (r.type === "warn") {
      console.log(yellow(`  WARN : ${r.msg}`));
    }
  }

  const okCount = reports.filter((r) => r.type === "ok").length;
  console.log(
    dim(
      `审计完成：共 ${reports.length} 个范围，${okCount} 项持平，${warnings} 项可收紧，${errors} 项超标`
    )
  );

  if (errors > 0 || (STRICT && warnings > 0)) {
    console.error(red(`\n✗ 行数棘轮审计未通过`));
    process.exit(1);
  } else {
    console.log(green(`\n✓ 行数棘轮审计全部通过`));
  }
}
