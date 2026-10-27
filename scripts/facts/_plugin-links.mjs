/**
 * 事实 · **插件链接行**的共享材料（DE-025 D-211 / `GEN-002`）
 *
 * ## 它替掉了什么
 *
 * 「有哪些插件」这件事在装配 crate 里被手写了三遍：`crates/symbio/Cargo.toml` 的
 * 依赖行、`crates/symbio/src/lib.rs` 的 `pub use` 行、以及 `PLUGIN.yml` 的清单本身。
 * 三者必须逐字对齐，而**没有任何东西会因此变红**——漏一行 `pub use` 的后果是
 * `inventory` 里那批注册项被链接器丢掉：插件「装上了」，只是它的工具、行为、
 * 上下文 provider 全部静默不生效。
 *
 * 现在事实源是**目录**（`crates/symbio-plugin-<id>/`，与 `_plugin-yml.mjs` 的
 * `pluginIds` 同源）：加一个插件 = 建一个目录 + 带 `PLUGIN.yml`，两处链接行由
 * 生成器补，手改会被 `gen-facts --check` 判红。
 *
 * ## 为什么是「标记块」而不是整份文件
 *
 * 这两份文件都不是生成物：`Cargo.toml` 还有 package / lints / dev-deps，
 * `lib.rs` 还有门面出口与模块声明。整份渲染等于把「谁写这两份文件」换成生成器，
 * 每加一条普通依赖都要先改模板。标记块把生成器的权限收在**它真正拥有**的那一段：
 * 块外的字节原样保留，块内的行由目录投影。
 *
 * 标记缺失即抛错（不生成）：一份「找不到块所以什么都没改」的生成器会在漂移那天
 * 报绿——那正是本仓反复买到教训的一侧。
 */

import { pluginIds, PLUGIN_CRATE_PREFIX } from "./_plugin-yml.mjs";

/** `event_bus` → `symbio-plugin-event_bus`（crate 名就是目录名，D-191）。 */
export function pluginCrateName(id) {
  return `${PLUGIN_CRATE_PREFIX}${id}`;
}

/** 装配 crate 的**依赖行**（`Cargo.toml`）与**链接行**（`lib.rs`）的同一份来源。 */
export function pluginLinks(root) {
  return pluginIds(root).map((id) => ({
    id,
    crateName: pluginCrateName(id),
    cargoLine: `${pluginCrateName(id)} = { path = "../${pluginCrateName(id)}" }`,
    libLine: `pub use symbio_plugin_${id};`,
  }));
}

/**
 * 把 `text` 里 `begin` / `end` 两行之间的内容替换成 `lines`。
 *
 * 契约三条：标记各出现**一次**、begin 在 end 之前、块内不嵌套。任何一条不成立都
 * 抛错——标记被删掉一半的后果是「生成器以为它管着这段，其实一个字都没写」。
 */
export function replaceBlock(text, { begin, end, lines, where }) {
  const src = text.split("\n");
  const begins = src.flatMap((l, i) => (l.trim() === begin ? [i] : []));
  const ends = src.flatMap((l, i) => (l.trim() === end ? [i] : []));
  if (begins.length !== 1 || ends.length !== 1) {
    throw new Error(
      `${where}：生成标记必须各出现一次（读到 begin×${begins.length} / end×${ends.length}）` +
        `——标记缺失时生成器会「什么都没改」却报绿`,
    );
  }
  const [b] = begins;
  const [e] = ends;
  if (b > e) throw new Error(`${where}：end 标记在 begin 之前`);
  return [...src.slice(0, b + 1), ...lines, ...src.slice(e)].join("\n");
}
