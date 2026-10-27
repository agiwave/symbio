/**
 * 事实 · `crates/symbio/Cargo.toml` 的**插件依赖行**
 *
 * 事实源是目录（`crates/symbio-plugin-<id>/`），产物是依赖区里的标记块。
 * 加一个插件 = 建目录 + 带 `PLUGIN.yml`；手改这一段会被 `gen-facts --check` 判红。
 *
 * 块外的字节原样保留（package / lints / 非插件依赖 / dev-deps 都不归本生成器管），
 * 理由与「为什么是标记块」见 `_plugin-links.mjs` 的文件头。
 */

import fs from "node:fs";
import path from "node:path";
import { pluginLinks, replaceBlock } from "./_plugin-links.mjs";

const OUT_REL = "crates/symbio/Cargo.toml";

const BEGIN = "# >>> plugin-links:begin";
const END = "# <<< plugin-links:end";

const NOTE =
  "# 以下依赖行由 `scripts/facts/plugin-links-cargo.mjs` 从 `crates/symbio-plugin-<id>/` 目录投影，" +
  "请勿手改（加插件 = 建目录 + `PLUGIN.yml`）。";

/** 纯函数：目录清单 + 现有文件 → 新文件内容。导出给回归测试（测试不写盘）。 */
export function renderInto(text, links) {
  return replaceBlock(text, {
    begin: BEGIN,
    end: END,
    where: OUT_REL,
    lines: [NOTE, ...links.map((l) => l.cargoLine)],
  });
}

export default {
  id: "plugin-links-cargo",
  title: "装配 crate 的插件依赖行（`crates/symbio-plugin-<id>/` 目录投影）",
  out: OUT_REL,
  stamp: false,
  render: ({ root }) => {
    const abs = path.join(root, ...OUT_REL.split("/"));
    const text = fs.readFileSync(abs, "utf8");
    return renderInto(text, pluginLinks(root));
  },
};
