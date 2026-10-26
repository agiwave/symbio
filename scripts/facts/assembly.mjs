/**
 * 事实 · `crates/symbio-deploy/src/lib.rs` —— 编译期装配清单
 *
 * ## 它替掉了什么
 *
 * `assembly.rs` 里手写的三样东西——`PLUGIN_ID_*`、`ASSEMBLY_SUB_AGENT_PLUGINS`、
 * `ASSEMBLY_UNDISABLABLE_PLUGINS`——其实都只是各插件目录 `PLUGIN.yml` 的投影。
 * 手写时加一个插件要动装配方（甚至忘了动，运行时才「找不到插件」）。现在：
 *
 * - `PLUGIN_ID_*` ← `plugin_id`（必须等于目录名，E-001 同判据）
 * - `ASSEMBLY_SUB_AGENT_PLUGINS` ← `plugin_assembly: sub_agent`，按 `plugin_order`
 * - `ASSEMBLY_UNDISABLABLE_PLUGINS` ← `plugin_undisablable: true`
 *
 * **加插件 = 建目录 + 写 `PLUGIN.yml` + 声明**；`assembly.rs` 与 `symbio_core` 零改动
 * （DE-023 D-181 / C5）。命中手改时，门禁 `gen-facts --check` 判红。
 *
 * ## 命名规则
 *
 * `PLUGIN_ID_` + 目录名大写；`plugin_manager` 这类带 `plugin_` 前缀的目录按既有
 * 标识符 `PLUGIN_ID_MANAGER` 收口（去掉重复的 `PLUGIN_`，否则读成 `PLUGIN_ID_PLUGIN_MANAGER`）。
 *
 * ## 为什么未用常量也留着
 *
 * 插件 id 是**部署身份**：目录名、运行期 `PLUGIN.yml` 的 `plugin_provider`、前端路由
 * 前缀都取同一个字面量。某个 id 暂时没有代码引用不代表它可以删——删掉会让下一次
 * 「按名字查找插件」重新手写字符串。文件头按 R-003 承认这一点，口径写在生成器里。
 */

import path from "node:path";
import { fileURLToPath } from "node:url";
import { pluginIds, pluginManifestRel, readPluginManifest } from "./_plugin-yml.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const OUT_REL = "crates/symbio-deploy/src/lib.rs";

const WAIVER =
  "// dead-code-allow R-003: 插件 id / 装配清单是部署契约面，未用常量仍须存在（owner: scripts/facts/assembly.mjs 生成，每个插件目录的 PLUGIN.yml 是唯一声明处）";

/** `plugin_manager` → `PLUGIN_ID_MANAGER`；其余目录 → `PLUGIN_ID_<大写>` */
function constName(id) {
  return `PLUGIN_ID_${id.toUpperCase().replace(/^PLUGIN_/, "")}`;
}

function readPlugins(root) {
  const plugins = [];
  for (const id of pluginIds(root)) {
    const rel = pluginManifestRel(id);
    const m = readPluginManifest(root, id);
    if (m.plugin_id !== id) {
      throw new Error(`${rel}：plugin_id=${JSON.stringify(m.plugin_id)} 与目录名 ${JSON.stringify(id)} 不一致`);
    }
    const assembly = m.plugin_assembly;
    if (assembly !== "sub_agent" && assembly !== "none") {
      throw new Error(`${rel}：plugin_assembly 必须是 sub_agent / none，读到 ${JSON.stringify(assembly)}`);
    }
    const order = assembly === "sub_agent" ? Number(m.plugin_order) : 0;
    if (assembly === "sub_agent" && !Number.isInteger(order)) {
      throw new Error(`${rel}：plugin_assembly: sub_agent 必须给整数 plugin_order`);
    }
    const undisablable = m.plugin_undisablable === "true";
    if (m.plugin_undisablable && m.plugin_undisablable !== "true" && m.plugin_undisablable !== "false") {
      throw new Error(`${rel}：plugin_undisablable 只能是 true / false`);
    }
    if (undisablable && assembly !== "sub_agent") {
      throw new Error(`${rel}：不在子树清单里的插件不存在「停用」动作，plugin_undisablable 没有意义`);
    }
    plugins.push({
      id,
      comment: m.plugin_comment ?? "",
      constName: constName(id),
      assembly,
      order,
      undisablable,
    });
  }
  return plugins;
}

function renderAssembly(root) {
  const plugins = readPlugins(root);

  const dupConst = new Map();
  const dupOrder = new Map();
  for (const p of plugins) {
    if (dupConst.has(p.constName)) {
      throw new Error(`常量名 ${p.constName} 同时来自 ${dupConst.get(p.constName)} 与 ${p.id}`);
    }
    dupConst.set(p.constName, p.id);
    if (p.assembly === "sub_agent") {
      if (dupOrder.has(p.order)) {
        throw new Error(`plugin_order ${p.order} 同时来自 ${dupOrder.get(p.order)} 与 ${p.id}`);
      }
      dupOrder.set(p.order, p.id);
    }
  }

  const mounted = plugins.filter((p) => p.assembly === "sub_agent").sort((a, b) => a.order - b.order);
  const undisablable = mounted.filter((p) => p.undisablable);

  const lines = [];
  lines.push("// 编译期装配清单 —— **由 `scripts/facts/assembly.mjs` 生成，请勿手改**。");
  lines.push("//");
  lines.push("// 事实源：`crates/symbio-plugin-<id>/PLUGIN.yml`。加插件 = 建目录 + 写声明 + 代码，");
  lines.push("// 本文件与 `assembly.rs` 都不需要人改；漂移由 `gen-facts --check` 判红。");
  lines.push(WAIVER);
  lines.push("");
  lines.push("// ============ 插件工厂 id（部署身份） ============");
  lines.push("");
  for (const p of [...plugins].sort((a, b) => a.id.localeCompare(b.id))) {
    if (p.comment) lines.push(`/// ${p.comment}`);
    lines.push(`pub const ${p.constName}: &str = "${p.id}";`);
    lines.push("");
  }
  lines.push("/// 子 Agent 子树默认挂载的插件清单（按 `plugin_order` 排序）。");
  // 生成物必须与 `render` 逐字相等（`gen-facts --check`）；`cargo fmt --all` 会给
  // 数组里的行尾注释补对齐，那会把生成物改成「手改过」的样子。跳过格式化，
  // 让两侧对同一份文本达成一致。
  lines.push("#[rustfmt::skip]");
  lines.push("pub const ASSEMBLY_SUB_AGENT_PLUGINS: &[&str] = &[");
  for (const p of mounted) {
    lines.push(`    "${p.id}", // ${p.comment}`);
  }
  lines.push("];");
  lines.push("");
  lines.push("/// **不可停用的插件**——界面自身的底座（停用其中一个，少的是整个界面）。");
  lines.push("#[rustfmt::skip]");
  lines.push("pub const ASSEMBLY_UNDISABLABLE_PLUGINS: &[&str] = &[");
  for (const p of undisablable) {
    lines.push(`    "${p.id}", // ${p.comment}`);
  }
  lines.push("];");
  lines.push("");
  return lines.join("\n");
}

export default {
  id: "assembly",
  title: "编译期装配清单（插件 id / 子 Agent 清单 / 不可停用集）",
  out: OUT_REL,
  stamp: false,
  render: ({ root }) => renderAssembly(root),
};
