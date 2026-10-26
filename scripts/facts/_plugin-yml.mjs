/**
 * `PLUGIN.yml` 的**唯一**读取点。
 *
 * ## 为什么它是一份共享材料
 *
 * 两份事实生成器读同一批清单：`assembly.mjs`（插件 id / 子树 / 不可停用集）与
 * `routes-rs.mjs`（跨插件地址常量）。各写一遍「清单长什么样」，分叉不会报错——
 * 它先表现为某一份生成器读不到新字段，然后表现为生成物静默少一条。
 *
 * ## schema 只有一行一个 `key: value`
 *
 * 本仓不引 YAML 解析器：清单是**线性声明**，不是嵌套文档。`#` 注释与空行跳过，
 * 重复键直接抛错（后写的悄悄覆盖前一条，正是「声明了但没生效」的形态）。
 * 列表值用逗号分隔（`plugin_routes: root, watch`），不是缩进列表——缩进会把
 * 这份 schema 变成需要解析器的那一类。
 */

import fs from "node:fs";
import path from "node:path";

/** 插件 crate 前缀；`symbio-plugin-api` 是宿主门面，不算插件。 */
export const PLUGIN_CRATE_PREFIX = "symbio-plugin-";
export const PLUGIN_API_DIR = "symbio-plugin-api";

/**
 * 插件目录名（= 路由前缀 = 实例名），按字典序。
 *
 * 空清单是**范围读空了**，不是「没有插件」——抛错让生成器停在这里，
 * 而不是产出一份空清单（`gen-facts` 的退出码 2 同一条判据）。
 */
export function pluginIds(root) {
  const dir = path.join(root, "crates");
  const ids = fs
    .readdirSync(dir, { withFileTypes: true })
    .filter(
      (e) =>
        e.isDirectory() &&
        e.name.startsWith(PLUGIN_CRATE_PREFIX) &&
        e.name !== PLUGIN_API_DIR,
    )
    .map((e) => e.name.slice(PLUGIN_CRATE_PREFIX.length))
    .sort();
  if (ids.length === 0) throw new Error(`${path.join(root, "crates")} 里一个插件目录都没有——范围读空了`);
  return ids;
}

/** 单个插件清单的仓根相对路径（报错定位用）。 */
export function pluginManifestRel(id) {
  return `crates/${PLUGIN_CRATE_PREFIX}${id}/PLUGIN.yml`;
}

/** `key: value` 扁平 schema；任何读不懂的行、重复键都抛错。 */
export function parseManifest(text, where) {
  const out = {};
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const m = line.match(/^([a-z_]+):\s*(.*)$/);
    if (!m) throw new Error(`${where}：读不懂这一行 ${JSON.stringify(raw)}`);
    const [, key, value] = m;
    if (key in out) throw new Error(`${where}：\`${key}\` 出现了两次`);
    out[key] = value.trim();
  }
  return out;
}

/** 读一个插件的清单；缺失即抛错（新插件必须带声明）。 */
export function readPluginManifest(root, id) {
  const rel = pluginManifestRel(id);
  const abs = path.join(root, ...rel.split("/"));
  if (!fs.existsSync(abs)) {
    throw new Error(`缺少 ${rel}——新插件必须带 PLUGIN.yml（加插件 = 目录 + 声明 + 代码）`);
  }
  return parseManifest(fs.readFileSync(abs, "utf8"), rel);
}
