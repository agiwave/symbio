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
  // 2026-09-27：智能体详情重设计（方案 A，后端驱动）——`agent/host/vdfs.rs` 的
  // `agent_dir_info` 由「能力拼成一个字符串」改为**结构化下发**（`capabilities`
  // 计数对象 + `capability_kinds` 清单 + `capability_count`），新增
  // `capability_counts()`；`agent/host/detail.rs` 把「已装能力」段提到元数据之前
  // 并按新字段重写；`vdfs.test.rs` 加夹具与 3 条用例。基线 3667 → 3736（+69）。
  // 前端零改动：`DetailForm.staticDisplay` 本就支持数组与对象（数组 `join('、')`、
  // 对象逐项 `k v`），故结构化值直接可渲染。
  // 2026-09-30：`cargo fmt` 后收窄 4 行（无代码删除，纯格式回折），按审计提示收紧。
  "symbio/src/plugins/agent": { maxLines: 3732, exts: [".rs"] },
  "symbio/src/plugins/composite": { maxLines: 1330, exts: [".rs"] },
  "symbio/src/plugins/event_bus": { maxLines: 164, exts: [".rs"] },
  "symbio/src/plugins/gateway": { maxLines: 1190, exts: [".rs"] },
  "symbio/src/plugins/home": { maxLines: 928, exts: [".rs"] },
  "symbio/src/plugins/hook": { maxLines: 467, exts: [".rs"] },
  "symbio/src/plugins/local": { maxLines: 3460, exts: [".rs"] },
  "symbio/src/plugins/mcp": { maxLines: 2897, exts: [".rs"] },
  "symbio/src/plugins/model": { maxLines: 6212, exts: [".rs"] },
  "symbio/src/plugins/plugin_manager": { maxLines: 666, exts: [".rs"] },
  // 2026-09-29（B2）：v2 桥接「投影表」落地——`session` 增 `projections.rs`
  // （三个既有纯函数 `sliding_window` / `fade_aged_content_nodes` /
  // `find_compress_split_point` 的**薄适配器** + 登记点；既有函数本体零改动，
  // `find_compress_split_point` 由 `fn` 提为 `pub(crate)` 供投影直调）。
  // 基线 16790 → 16993（+203）。测试另计（`projections.test.rs`）。
  // 2026-09-29（B3）：v2 桥接「Actor 行」落地——`session` 增 `actors.rs`
  // （把现行 `chat_loop` 隐含的那个 Actor **显式声明为一行** `session.reasoner`
  //  + 层判定 + 运行期以真实会话 id 解析；`chat_loop.rs` 入口取该行，取值与原实现
  //  逐字节相同）。基线 16993 → 17144（+151）。测试另计（`actors.test.rs`）。
  // 2026-09-29（B4）：v2 桥接「注册检索者（S06）」落地——`session` 增
  // `projections.rs` 的第四个投影 `memory.recall`（从事实里挑出 `memory.*` 格子
  // 与当前窗口，折成"可召回集合"；平凡值 = 无记忆事实时只看窗口）。既有三个投影
  // 本体零改动。基线 17144 → 17236（+92）。测试另计（`projections.test.rs`）。
  // 2026-09-30：`memory.recall` 投影落地后经 `cargo fmt` 收窄 3 行（无代码删除，
  // 纯格式回折）。棘轮只进不退 ⇒ 按审计提示收紧。基线 17236 → 17233（-3）。
  "symbio/src/plugins/session": { maxLines: 17233, exts: [".rs"] },
  "symbio/src/plugins/skill": { maxLines: 1470, exts: [".rs"] },
  "symbio/src/plugins/telegram": { maxLines: 887, exts: [".rs"] },
  "symbio/src/plugins/vdfs": { maxLines: 3012, exts: [".rs"] },
  "symbio/src/plugins/web": { maxLines: 1028, exts: [".rs"] },
  "symbio/src/plugins/work": { maxLines: 627, exts: [".rs"] },

  // ── 内核与驱动层（symbio/src/*） ──
  // 2026-09-29：v2 桥接 B1「事实信封」落地——新增 `fact` 域（`FactKind` 七实体×动词
  // 枚举 + `Fact` 只读信封 + `FactPrincipal` + `FactSource` 异步源 trait + `FactError`，
  // 实现 ~413 行，测试另计），`CapabilityVisitor` 增 3 个事实源登记方法
  // （`register_fact_source` / `list_fact_sources` / `get_fact_source`）。
  // 基线 8143 → 8610（+467）。该域为 v2 投影表的前置只读视图，不触碰既有存储层。
  // 2026-09-29（B2）：v2 桥接「投影表」落地——新增 `projection` 域
  // （`registry.rs`：`Projection` 私有构造 + `ProjectionInput` + 类型擦除
  // `ProjectionFn` / `ProjectionSubmit` + `inventory` 登记表 + `projection_list` /
  // `projection_run` / `projection_has` + `submit_projection!` 宏；`view.rs`：
  // `View` 平凡值可区分；`ProjectionError`。实现 ~347 行，测试另计）。
  // 基线 8610 → 8959（+349）。
  // 2026-09-29（B3）：v2 桥接「Actor 行」落地——新增 `actor` 域
  // （`spec.rs`：`ActorSpec` 四字段 + `ActorPattern` / `ActorScope`；
  //  `registry.rs`：进程级登记表 + `ActorSource` 登记方 trait（`layer` 由装配方赋予
  //  ⇒ 越层登记不进去，断言 A5）+ `actor_register` / `actor_get` / `actor_list` /
  //  `actor_clear`。实现 ~360 行，测试另计）。**表为空 ⇒ 内置默认行**（现行行为），
  // 故本域是扩展点而非运行时必需品。基线 8959 → 9409（+450）。
  // 2026-09-29（B4）：
  //  ① 新增可选插件工厂 id 常量 `PLUGIN_ID_RETRIEVAL`（`ids.rs` 一行 +
  //     `plugin/mod.rs` 重导出一行）。检索者插件本体住 `plugins/retrieval`，不进 core。
  //  ② **`FactKind` 的序列化收敛为单一词形**（`kind.rs`）：原本 `derive` +
  //     `snake_case` 产出 `memory_encoded`，而 `wire()` 产出 `memory.encoded`
  //     ——同一事实两种线上写法，单测测不出（两侧都用 `wire()` 比较），
  //     由 e2e 跨进程真实载荷抓出。改为手写 `Serialize`/`Deserialize`
  //     委托 `wire()`（+ 反序列化在 `ALL` 内反查，认不出即报错不兜底），
  //     并在 `tests.rs` 加两条钉死该契约。实现 +34 行，测试另计。
  //  ③ 修 4 处 intra-doc 断链（`cargo doc -D rustdoc::broken_intra_doc_links` 拦下，
  //     属 B1/B3 遗留）：`crate::Fact` 一类链接漏了 `symbio_core::` 段，另有两处
  //     指向不存在的方法 / 常量。注释改写 +4 行。
  // 基线 9409 → 9449（+2 +34 +4）。
  "symbio/src/symbio_core": { maxLines: 9449, exts: [".rs"] },
  "symbio/src/providers": { maxLines: 2732, exts: [".rs"] },

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
  "tauri/src": { maxLines: 21137, exts: [".ts", ".vue"] },
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
