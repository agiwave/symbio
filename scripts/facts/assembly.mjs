/**
 * 事实 · `crates/symbio-platform/src/deploy/mod.rs` —— 编译期装配清单
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
 * - `ASSEMBLY_CORE_PLUGINS` ← `sub_agent` 且**非** `plugin_optional`（默认装配，含可删项）
 * - `ASSEMBLY_OPTIONAL_PLUGINS` ← `sub_agent` 且 `plugin_optional: true`（插件市场候选，）
 * - `ASSEMBLY_REQUIRED_PLUGINS` ← 本文件写死的 [`REQUIRED_IDS`]（系统装配策略，不可删除的最小底座）
 *
 * 三档语义（）：**必装**（`REQUIRED_IDS`，不可删）/ **默认内置可删**（core 里非
 * 必装的那批，开箱即装、目录自动补出、但用户可删）/ **市场按需**（optional，不默认装配）。
 * 「默认装配」与「不可删除」是两件事：前者看是否在 core（决定 `ensure_required` 补目录），
 * 后者只看必装集（决定 `uninstall` 拒绝与 `PluginEntry.required`）。
 *
 * **为什么必装集写死在系统、不由插件声明**：「我是不是系统的骨干」是装配策略的判断，
 * 不是插件的自我描述。让插件写 `plugin_required: true` 自称必装，等于把「减少必装」
 * 的裁决权交给每个插件——任何人加一个插件都能顺手标成不可删。故与容器写死
 * `SYSTEM_LEVEL_PROVIDERS`（home / composite 不作为子插件）同构：必装 id 收口在装配方。
 * 代价是加/减必装要动本文件——这正是「谁能改必装」应有的门槛。
 *
 * **加插件 = 建目录 + 写 `PLUGIN.yml` + 声明**；`assembly.rs` 与共享层零改动
 * （DE-023  / C5）。命中手改时，门禁 `gen-facts --check` 判红。
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
// @gate test

import path from "node:path";
import { fileURLToPath } from "node:url";
import { pluginIds, pluginManifestRel, readPluginManifest } from "./_plugin-yml.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const OUT_REL = "crates/symbio-platform/src/deploy/mod.rs";

const WAIVER =
  "// dead-code-allow R-003: 插件 id / 装配清单是部署契约面，未用常量仍须存在（owner: scripts/facts/assembly.mjs 生成，每个插件目录的 PLUGIN.yml 是唯一声明处）";

/**
 * 系统必装集 —— **装配策略写死在此，不来自任何插件的 `PLUGIN.yml`**（）。
 *
 * 判据：缺了它一次正常会话就跑不起来，或少了它整个界面无法再操作。逐个理由见注释；
 * 其余插件一律默认可删（core 里非本集）或市场可选（optional）。增删必装要改这里并经
 * 下方校验：必装 id 必须是 sub_agent 且非 optional，否则生成器判红。
 */
const REQUIRED_IDS = [
  "plugin_manager", // 插件管理入口：删了就没有启停/删除任何插件的界面
  "session", // 会话：没有它不存在一次对话
  "loop", // 唯一的驱动提供者：缺它宿主按 NoDebt 收尾，会话不会再推进一轮
  "model", // 模型服务：没有它不产生任何回复
  "vdfs", // 资源树底座：界面所有资源页经它取数（同时不可停用）
  "ui_prefs", // 界面偏好底座：外观/关于的后端权威（同时不可停用）
];

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
    const optional = m.plugin_optional === "true";
    if (m.plugin_optional && m.plugin_optional !== "true" && m.plugin_optional !== "false") {
      throw new Error(`${rel}：plugin_optional 只能是 true / false`);
    }
    if (optional && assembly !== "sub_agent") {
      throw new Error(`${rel}：不在子树清单里的插件不参与默认装配，plugin_optional 没有意义`);
    }
    if (optional && undisablable) {
      throw new Error(`${rel}：可选插件不默认加载，不能同时是界面底座（plugin_undisablable）`);
    }
    // 必装不由插件声明（）：若某个 PLUGIN.yml 残留 plugin_required，判红并要求删除——
    // 否则「谁都能把自己标成必装」的后门还在，写死的策略就形同虚设。
    if ("plugin_required" in m) {
      throw new Error(`${rel}：plugin_required 已收口到装配策略（scripts/facts/assembly.mjs 的 REQUIRED_IDS），插件不得自行声明必装`);
    }
    plugins.push({
      id,
      comment: m.plugin_comment ?? "",
      constName: constName(id),
      assembly,
      order,
      undisablable,
      optional,
    });
  }
  return plugins;
}

function renderAssembly(root, requiredIds = REQUIRED_IDS) {
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
  const core = mounted.filter((p) => !p.optional);
  const optional = mounted.filter((p) => p.optional);

  // 必装集来自系统写死的 REQUIRED_IDS，按子树装配序投影（）。
  // 每条校验挡住「系统点了一个不存在 / 可选 / 非子树的插件为必装」——
  // 写死的策略一旦和插件事实脱节，宁可生成失败，也不产出一张骗人的必装表。
  const byId = new Map(mounted.map((p) => [p.id, p]));
  const required = [];
  for (const id of requiredIds) {
    const p = byId.get(id);
    if (!p) {
      throw new Error(`REQUIRED_IDS 里的「${id}」不在 sub_agent 装配清单里（目录缺失或 plugin_assembly 不是 sub_agent）`);
    }
    if (p.optional) {
      throw new Error(`REQUIRED_IDS 里的「${id}」声明了 plugin_optional，必装插件不能同时是市场可选`);
    }
    required.push(p);
  }

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
    lines.push(
      "// dead-code-allow R-001: 插件 id 是部署身份，未被引用不等于可删（同文件头 R-003）",
    );
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
  lines.push("/// **核心插件**——默认装配且不可删除。这是注入 `REQUIRED_PLUGINS` 的唯一清单（）。");
  lines.push("#[rustfmt::skip]");
  lines.push("pub const ASSEMBLY_CORE_PLUGINS: &[&str] = &[");
  for (const p of core) {
    lines.push(`    "${p.id}", // ${p.comment}`);
  }
  lines.push("];");
  lines.push("");
  lines.push("/// **可选插件**——不默认装配，出现在插件市场里，安装后与核心插件同权（）。");
  lines.push("#[rustfmt::skip]");
  lines.push("pub const ASSEMBLY_OPTIONAL_PLUGINS: &[&str] = &[");
  for (const p of optional) {
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
  lines.push("/// **必装插件**——不可删除的最小底座（`plugin_required: true`）。");
  lines.push("/// 与核心集是两件事：核心集决定首次启动补哪些目录，本集只决定「能不能删」。");
  lines.push("/// 删除拒绝（`uninstall`）与前端只读投影（`PluginEntry.required`）都查这一份。");
  lines.push("#[rustfmt::skip]");
  lines.push("pub const ASSEMBLY_REQUIRED_PLUGINS: &[&str] = &[");
  for (const p of required) {
    lines.push(`    "${p.id}", // ${p.comment}`);
  }
  lines.push("];");
  lines.push("");
  return lines.join("\n");
}

export default {
  id: "assembly",
  title: "编译期装配清单（插件 id / 子 Agent 清单 / 核心集 / 可选集 / 不可停用集）",
  out: OUT_REL,
  stamp: false,
  // `requiredIds` 仅供回归测试注入小仓库的必装集；真仓走写死的 REQUIRED_IDS。
  render: ({ root, requiredIds }) => renderAssembly(root, requiredIds),
};
