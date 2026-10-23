#!/usr/bin/env node
/**
 * 事实 · `tauri/src/constants/routes.gen.ts` —— 控制面路由常量的前端侧投影
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
 * 门禁自动重跑（见 `scripts/gate.d/60-facts.mjs`）。
 *
 * ## 提取实现不在这里
 *
 * 「什么算一条路由」由 `scripts/route-facts.mjs` 单独拥有，`facts/current.mjs`
 * 的 §1 路由列读的是同一份实现。两份生成物各写一遍正则，就会各有一个「臂」的定义。
 *
 * ## 命名
 *
 * `ROUTE_` + 地址大写（`/` → `_`），与后端 `symbio_core/plugin/route.rs` 的常量命名
 * 规则同构，因此**同名同值**：一条被 core 登记过的地址，两端检索同一个标识符就能命中。
 *
 * 由 `scripts/gen-facts.mjs` 遍历 `scripts/facts/` 时调用，不要直接跑。
 */

import { controlPlaneRoutes, routeConstName } from "../route-facts.mjs";
import { fileURLToPath } from "node:url";
import path from "node:path";

// 本文件住 `scripts/facts/`，故仓根要上溯两级（原来在 `scripts/` 下只需一级）。
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const OUT_REL = "tauri/src/constants/routes.gen.ts";

const HEADER = `/**
 * 控制面路由常量 —— **由后端生成，请勿手改**。
 *
 * ⚠️ 本文件由 \`scripts/facts/routes-ts.mjs\` 从后端 \`route()\` 的 \`match\` 臂生成，
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
 *   \`docs/004-ref/006-当前事实表.md\` §1 标「（动态）」的那些行。
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

/**
 * 空清单**不是**「没有路由」，而是插件目录读不到 / 提取口径变了——
 * 静默生成一份空表会让前端以为「后端没有控制面路由」。
 * 这是本事实的 `render` 唯一会抛的错，runner 把它当失败（而不是当「生成成功」）。
 */
function renderFromCode() {
  const routes = controlPlaneRoutes(ROOT);
  if (routes.length === 0) {
    throw new Error("未提取到任何静态路由——插件目录读不到，还是提取口径变了？");
  }
  return render(routes);
}

// ================= 事实描述符（由 `scripts/gen-facts.mjs` 遍历调用） =================
//
// 不带溯源行：产物是 TypeScript 源码，加一行生成时间戳会污染被编译的输出，
// 而且它每次都变 ⇒ autoWork 的内容哈希永远对不上。
export default {
  id: "routes-ts",
  title: "前端路由常量（后端 `route()` 臂的投影）",
  out: OUT_REL,
  stamp: false,
  render: renderFromCode,
};
