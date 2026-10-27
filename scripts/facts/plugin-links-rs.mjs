/**
 * 事实 · `crates/symbio/src/lib.rs` 的**插件链接行**（`pub use <crate>;`）
 *
 * ## 为什么它必须生成，而不是「顺手加一行」
 *
 * `pub use <crate>;` 在这里同时干两件事：链接 + 门面出口。**漏一行的后果不是编译
 * 失败**——插件 crate 仍在依赖图里，只是链接器看不到它的 `inventory` 注册项，
 * 于是那个插件的工具 / 行为 / 上下文 provider 全部**静默不生效**，而构建全绿。
 * 这是「加插件要记得改一处中心文件」里最贵的一处：忘掉它不会红，只会少功能。
 *
 * 事实源与依赖行同源（目录），两块由同一份共享材料渲染，见 `_plugin-links.mjs`。
 */

import fs from "node:fs";
import path from "node:path";
import { pluginLinks, replaceBlock } from "./_plugin-links.mjs";

const OUT_REL = "crates/symbio/src/lib.rs";

const BEGIN = "// >>> plugin-links:begin";
const END = "// <<< plugin-links:end";

const NOTE =
  "// 以下链接行由 `scripts/facts/plugin-links-rs.mjs` 从 `crates/symbio-plugin-<id>/` 目录投影，" +
  "请勿手改；漏一行的后果是那个插件的 inventory 注册项被链接器丢掉（不报错，只少功能）。";

/** 纯函数：目录清单 + 现有文件 → 新文件内容。导出给回归测试（测试不写盘）。 */
export function renderInto(text, links) {
  return replaceBlock(text, {
    begin: BEGIN,
    end: END,
    where: OUT_REL,
    lines: [NOTE, ...links.map((l) => l.libLine)],
  });
}

export default {
  id: "plugin-links-rs",
  title: "插件链接行（`crates/symbio-plugin-<id>/` 目录投影，漏一行不报错只少功能）",
  out: OUT_REL,
  stamp: false,
  render: ({ root }) => {
    const abs = path.join(root, ...OUT_REL.split("/"));
    const text = fs.readFileSync(abs, "utf8");
    return renderInto(text, pluginLinks(root));
  },
};
