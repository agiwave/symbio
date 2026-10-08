/**
 * route-facts — 「后端有哪些控制面路由」的**唯一提取实现**
 *
 * ## 为什么提取本身要有一份 owner
 *
 * 路由的注册处就是各插件 `async fn route()` 体内的 `match` 臂：容器按**插件目录名**
 * 建实例表、剥掉首段再把余下部分塞回 `PATH` 转发（地址规则见
 * `symbio_core/plugin/route.rs` 的模块文档）。所以「臂 + 目录名」就是那条地址的
 * 完整真相，**没有别的地方登记它**。
 *
 * 两份生成物都要读它：`docs/CURRENT.md` §1 的路由列，和前端常量
 * `tauri/src/constants/routes.gen.ts`。各写一遍正则 = 两份「什么算一条路由」的判据，
 * 它们会在某个不引人注目的下午分叉（比如一份开始把 `|` 复合臂漏掉）。
 * 本模块因此从 `gen-current-facts.mjs` 里提出这段解析，两边共用。
 *
 * ## 判据形态（为什么只认 `match` 臂）
 *
 * 只取 `match` 臂左侧的字符串字面量（`"a/b" | "c" => …`），不整段抓字符串——后者会把
 * `get("approved")` 这类参数名当成路由（实测踩坑）。
 *
 * ## 动态分发不在本模块的产出里
 *
 * `vdfs/<操作>` 按 `VDFS_OPS` 校验后分发、`local/<工具短名>` 按已注册工具名分发、
 * 容器按挂载名分发——三者都不是静态臂，因此**不会**出现在这里。它们的清单各有 owner
 * （`plugins/vdfs/protocol.rs`、运行期 `traverse`），前端镜像它们走的是另一条链
 * （VDFS 契约在 `tauri/src/schemas/vdfs.ts`，由 mechanism-audit 的 M-007 钉住定义权）。
 */

import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { stripComments, stripTestModules } from "./rust-scan.mjs";

/**
 * 「未接线」标记：模块级 `#![allow(dead_code)]`。
 *
 * 整体抑制 dead_code 等于自述「这块代码还没有接线」，它里面的 `CapabilityMeta`
 * 与 `match` 臂都是**未接线的定义**——不排除就会让生成物报出一条模型看不到的工具、
 * 或一条**没人能到达的路由**。
 *
 * 判据取**模块级属性**而不是猜注释文案——可用 grep 复核。当前全仓**无实例**，
 * 保留此判据是为了将来再出现未接线模块时自动生效。
 */
export const UNWIRED_MARKER = "#![allow(dead_code)]";

/** 递归收集插件源码（排除测试文件与 docs 目录：测试里的 meta 不是生产事实） */
export function collectRs(dir) {
  const out = [];
  for (const ent of readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, ent.name);
    if (ent.isDirectory()) {
      if (ent.name === "docs") continue;
      out.push(...collectRs(p));
    } else if (
      ent.name.endsWith(".rs") &&
      !ent.name.endsWith(".test.rs") &&
      ent.name !== "tests.rs"
    ) {
      out.push(p);
    }
  }
  return out;
}

/** 该插件目录里**参与生产事实**的源码，已去注释、去内联测试模块 */
export function pluginSources(dir) {
  return collectRs(dir)
    .map((f) => readFileSync(f, "utf8"))
    .filter((raw) => !raw.includes(UNWIRED_MARKER))
    .map((raw) => stripTestModules(stripComments(raw)));
}

/**
 * 从 `async fn route(…)` 体内提取路由臂。
 *
 * 臂是**相对路径**：容器（composite）先剥掉首段，插件只分发剩下的部分，
 * 故渲染时要补回前缀。**`home` 例外**——它是根容器，收到的 `PATH` 未经剥离，
 * 臂本身就是完整地址。
 *
 * ## 前缀取**目录名**，不取 `PluginMeta::new` 的首参
 *
 * 容器按**目录名**建实例表并在 `route` 里按它分发（`composite.rs`「目录名 = 实例名」），
 * 所以目录名才是真正的路由前缀。`PluginMeta` 首参曾长期被当作前缀用，而它**不参与路由**
 * ——ADR-032 之后它只是**出厂 id**：身份取自 `PLUGIN.yml`，`composite/vdfs.rs` 只读它的
 * 「挂载点呈现」那部分（`order` / `hidden` / `root_access`）——`hook` 插件写成 `"hooks"`
 * 就由此产出了 `hooks/fire` 这类**不存在的路由**，并被下游文档照抄。改用目录名后，
 * 「生成器说出的路由」与「容器真正认的路由」同源；`plugin-entry-audit.mjs` 的 E-001
 * 另外把「`PluginMeta` 首参 == 目录名」钉住，使两者不会再分叉。
 */
export function extractRouteArms(t, pluginName) {
  const out = new Set();
  const ri = t.indexOf("async fn route(");
  if (ri < 0) return [];
  const rest = t.slice(ri);
  const nextFn = rest.indexOf("async fn ", 20);
  const chunk = rest.slice(0, nextFn < 0 ? Math.min(6000, rest.length) : nextFn);
  for (const line of chunk.split("\n")) {
    const eq = line.indexOf("=>");
    if (eq < 0) continue;
    const left = line.slice(0, eq);
    for (const mm of left.matchAll(/"([a-z][a-z0-9_/-]*)"/g)) {
      if (mm[1] === "_") continue;
      const arm = mm[1];
      const full =
        pluginName && pluginName !== "home" && !arm.startsWith(`${pluginName}/`)
          ? `${pluginName}/${arm}`
          : arm;
      out.add(full);
    }
  }
  return [...out].sort();
}

/** 插件根目录（`symbio/src/plugins`）下的一层目录名 = 路由前缀 = 实例名 */
export function pluginDirs(root) {
  const base = path.join(root, "symbio", "src", "plugins");
  return readdirSync(base, { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => e.name)
    .sort();
}

/** 单个插件的静态路由（完整地址，已按目录名补前缀） */
export function pluginRouteArms(root, dirName) {
  const routes = new Set();
  for (const t of pluginSources(path.join(root, "symbio", "src", "plugins", dirName))) {
    for (const arm of extractRouteArms(t, dirName)) routes.add(arm);
  }
  return [...routes].sort();
}

/** 全仓静态可提取的控制面路由（跨插件去重、排序） */
export function controlPlaneRoutes(root) {
  const all = new Set();
  for (const dir of pluginDirs(root)) {
    for (const r of pluginRouteArms(root, dir)) all.add(r);
  }
  return [...all].sort();
}

/**
 * 地址 → 常量名：`ROUTE_` + 地址大写（`/` → `_`）。
 *
 * 与 `plugin/route.rs` 的命名规则**逐字同构**（「标识符去掉 `ROUTE_` 前缀后必须
 * 等于值的大写形式」），于是后端手写常量与前端生成常量在**名字**上也对得上——
 * 读代码的人不需要在两栈之间做一次翻译。
 */
export function routeConstName(routePath) {
  return `ROUTE_${routePath.replace(/\//g, "_").toUpperCase()}`;
}
