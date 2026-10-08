#!/usr/bin/env node
/**
 * gen-routes-ts —— 控制面路由常量生成器：后端 `route()` 臂 → `tauri/src/constants/routes.gen.ts`
 *
 * ## 它解决的是「同一个契约手写两份」
 *
 * 前端要调后端的一个动作，就得知道那条地址。此前这件事有两份抄本：后端插件里的
 * `match` 臂（**注册处**），和前端手写的常量登记表。
 * 抄本漂移不会有任何信号——它表现为运行期一次「找不到插件」，而报出来的错是后端的
 * `NotFound`，看不出前端写了个过期的词。
 *
 * 现在前端那一侧**是生成的**：`gen → routes.gen.ts` 是纯函数，所以「前后端地址不一致」
 * 这件事在结构上不再可表达。要新增一条控制面路由，只有一条路——在后端加一条臂，
 * 门禁自动重跑本生成器（见 `scripts/gate.d/60-facts.mjs`）。
 *
 * ## 提取实现不在这里
 *
 * 「什么算一条路由」由 `scripts/route-facts.mjs` 单独拥有，`gen-current-facts.mjs`
 * 的 §1 路由列读的是同一份实现。两份生成物各写一遍正则，就会各有一个「臂」的定义。
 *
 * ## 命名
 *
 * `ROUTE_` + 地址大写（`/` → `_`），与后端 `symbio_core/plugin/route.rs` 的常量命名
 * 规则同构，因此**同名同值**：一条被 core 登记过的地址，两端检索同一个标识符就能命中。
 *
 * 用法：
 *   node scripts/gen-routes-ts.mjs            # 生成 tauri/src/constants/routes.gen.ts
 *   node scripts/gen-routes-ts.mjs --check    # 与现有文件比对，漂移则非零退出（CI 用）
 */

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { controlPlaneRoutes, routeConstName } from "./route-facts.mjs";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(SCRIPT_DIR, "..");
const OUT_REL = "tauri/src/constants/routes.gen.ts";
const OUT = path.join(ROOT, ...OUT_REL.split("/"));

const HEADER = `/**
 * 控制面路由常量 —— **由后端生成，请勿手改**。
 *
 * ⚠️ 本文件由 \`scripts/gen-routes-ts.mjs\` 从后端 \`route()\` 的 \`match\` 臂生成，
 * 门禁会自动重新生成（\`scripts/gate.d/60-facts.mjs\`）。在这里手改一行，下一次门禁
 * 就会把它覆盖掉——真正要改的地方是后端那条臂。
 *
 * 地址的**注册处**就是插件里的 \`match\` 臂（提取实现：\`scripts/route-facts.mjs\`），
 * 所以这份清单没有第二份真相，也不可能与后端不一致。
 *
 * ## 这里不包含什么
 *
 * - \`vdfs/*\` 的操作词：它们按 \`VDFS_OPS\` 动态校验，契约与响应类型同处
 *   \`schemas/vdfs.ts\`（定义权由 \`mechanism-audit\` 的 M-007 钉住）。
 * - 动态分发的地址（\`local/<工具短名>\`、容器挂载名）：静态提取不到，见
 *   \`docs/CURRENT.md\` §1 标「（动态）」的那些行。
 *
 * 命名与后端 \`symbio_core/plugin/route.rs\` 同构：\`ROUTE_\` + 地址大写（\`/\` → \`_\`），
 * 同名同值，跨栈检索只需一个标识符。
 */
`;

/** 按首段（插件目录名）分组——常量表是用来**查**的，分组即索引 */
function groupByPrefix(routes) {
  const groups = new Map();
  for (const r of routes) {
    const prefix = r.split("/")[0];
    if (!groups.has(prefix)) groups.set(prefix, []);
    groups.get(prefix).push(r);
  }
  return [...groups.entries()];
}

/** 纯函数：路由清单 → 文件内容。导出给回归测试（测试不应触发写盘） */
export function render(routes) {
  const L = [HEADER];
  for (const [prefix, group] of groupByPrefix(routes)) {
    L.push(`// ==================== ${prefix} ====================`);
    L.push("");
    for (const r of group) L.push(`export const ${routeConstName(r)} = '${r}'`);
    L.push("");
  }
  return `${L.join("\n").replace(/\n+$/, "")}\n`;
}

const MAIN =
  process.argv[1] && path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url));

if (MAIN) {
  const routes = controlPlaneRoutes(ROOT);
  // 空清单不是「没有路由」，而是插件目录读不到 / 提取口径变了——静默生成一份空表
  // 会让前端以为「后端没有控制面路由」。
  if (routes.length === 0) {
    console.error("✗ 未提取到任何静态路由——插件目录读不到，还是提取口径变了？");
    process.exit(1);
  }

  const out = render(routes);
  if (process.argv.includes("--check")) {
    const current = existsSync(OUT) ? readFileSync(OUT, "utf8") : "";
    if (current !== out) {
      console.error(
        `✗ ${OUT_REL} 与后端路由不一致 —— 跑 node scripts/gen-routes-ts.mjs 重新生成`,
      );
      process.exit(1);
    }
    console.log(`✅ ${OUT_REL} 与代码一致（${routes.length} 条）`);
    process.exit(0);
  }

  writeFileSync(OUT, out, "utf8");
  console.log(`✅ ${OUT_REL}：${routes.length} 条路由`);
}
