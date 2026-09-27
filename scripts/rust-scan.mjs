/**
 * Rust 源码扫描原语（**唯一真源**）
 *
 * ## 为什么有这个模块
 *
 * 门禁脚本里有一类操作被反复重写：**在 Rust 源码里做「字符串感知」的字符扫描**——
 * 去注释、花括号配对、剔除 `#[cfg(test)]` 模块。它们不是随手三行能替代的东西：
 *
 * - 朴素正则（`/\/\/.*$/`、`/\*[\s\S]*?\*\//`）会在**字符串字面量**上翻车——
 *   `"https://api.openai.com/v1"` 里的 `//` 被当成注释起点，整行后半段被吃掉；
 * - 朴素花括号计数会在 `format!("{{}}")` 这类字面量括号上错位，于是后面所有
 *   代码的位置全错；
 * - 而「写错的扫描器」不报错，只**静默漏报事实**——这是最危险的一类 bug
 *   （生成器说某条路由不存在，其实是抽漏了）。
 *
 * 此前这几件东西在至少四份实现里各写一遍（`line-count.mjs` / `plugin-entry-audit.mjs`
 * / `gen-current-facts.mjs` / `core-surface.mjs`），措辞、边界、返回值都不一致。
 * 本模块把它们收成**一份**，谁是权威一目了然；改行为只改这一处。
 *
 * ## 两种去注释，别拿错
 *
 * | 函数 | 保留行结构 | 适用 |
 * |---|---|---|
 * | [`stripComments`] | **否**（注释整段消失） | 提取事实：只关心「有没有这个符号」，不看行号 |
 * | [`blankComments`] | **是**（注释抹成空白，换行保留） | 报错定位：报告里的 `file:line` 要对得上原文件 |
 *
 * 拿错了会怎样：用 [`stripComments`] 做审计判定，则所有行号在注释之后全部错位，
 * 报告里的 `file:line` 指到无关代码上；用 [`blankComments`] 做事实提取本身没错，
 * 只是没必要。
 */

/**
 * 跳过字符串字面量，返回闭引号（或原始串收尾分隔符末位）的下标。
 *
 * 只**向前**扫描：返回值恒 ≥ `start`，调用方据此推进扫描位置；
 * 一旦返回更小的下标（曾用「向前 `lastIndexOf` 找 `r`」的实现），
 * 外层扫描就会原地打转（实测踩坑：生成器死循环）。
 *
 * 认原始字符串：开引号紧跟在 `r` 或 `r###` 之后（`r"…"` / `r#"…"#` / `br#"…"#`）。
 */
export function skipString(txt, start) {
  let hashes = 0;
  let j = start - 1;
  while (j >= 0 && txt[j] === "#") {
    hashes++;
    j--;
  }
  if (j >= 0 && txt[j] === "r") {
    const close = '"' + "#".repeat(hashes);
    const end = txt.indexOf(close, start + 1);
    return end < 0 ? txt.length - 1 : end + close.length - 1;
  }
  let p = start + 1;
  while (p < txt.length) {
    if (txt[p] === "\\") {
      p += 2;
      continue;
    }
    if (txt[p] === '"') return p;
    p++;
  }
  return txt.length - 1;
}

/**
 * 去掉注释（字符串感知）——注释整段消失，**行结构不保留**。
 *
 * 用于「提取事实」：只要不把字符串里的 `//` 当注释、不把注释里的符号当代码即可。
 * 需要行号准确时改用 `blankComments`。
 *
 * 剥行注释与块注释两种；HTML 注释（`.vue` 那种）另有开关，见 `blankComments`
 * 的 `html` 选项。
 */
export function stripComments(txt) {
  let out = "";
  for (let p = 0; p < txt.length; p++) {
    const c = txt[p];
    if (c === '"') {
      const end = skipString(txt, p);
      // 进度保证：解析结果必须至少吃掉当前字符
      if (end > p) {
        out += txt.slice(p, end + 1);
        p = end;
        continue;
      }
    } else if (c === "/" && txt[p + 1] === "/") {
      const end = txt.indexOf("\n", p);
      if (end < 0) break;
      out += "\n";
      p = end;
      continue;
    } else if (c === "/" && txt[p + 1] === "*") {
      const end = txt.indexOf("*/", p + 2);
      if (end < 0) break;
      p = end + 1;
      continue;
    }
    out += c;
  }
  return out;
}

/**
 * 去掉注释并**保留行结构**（注释抹成空白，换行一律留下）——返回按行数组。
 *
 * 用于「判定 + 报错」：报告里的 `file:line` 必须指回原文件，而豁免注释写在原行上，
 * 故判定行与原始行必须一一对应。
 *
 * - 字符串状态跨行重置（换行时 `quote = null`）：Rust 字符串不跨行，
 *   而一次未闭合的引号若不清，会把此后的全部代码判成「在字符串里」而整段漏报。
 * - `html: true` 额外剥 HTML 注释（`.vue` 的组件文档写在顶部 HTML 注释里，
 *   而那正是最常出现路径字面量的地方；不剥它会把满篇文档判成违规，
 *   而一个只会误报的守卫最后一定会被人用豁免注释喂到失效）。
 *
 * @param {string} source
 * @param {{html?: boolean}} [opts]
 * @returns {string[]} 与原始行一一对应的行数组
 */
export function blankComments(source, { html = false } = {}) {
  const out = [];
  let line = "";
  let inBlock = false;
  let inHtml = false;
  let quote = null;
  for (let i = 0; i < source.length; i++) {
    const c = source[i];
    const n = source[i + 1];
    if (c === "\n") {
      out.push(line);
      line = "";
      quote = null;
      continue;
    }
    if (inBlock) {
      if (c === "*" && n === "/") {
        inBlock = false;
        i++;
      }
      continue;
    }
    if (inHtml) {
      if (c === "-" && n === "-" && source[i + 2] === ">") {
        inHtml = false;
        i += 2;
      }
      continue;
    }
    if (quote) {
      line += c;
      if (c === "\\") {
        line += n ?? "";
        i++;
      } else if (c === quote) {
        quote = null;
      }
      continue;
    }
    if (c === "/" && n === "*") {
      inBlock = true;
      i++;
      continue;
    }
    if (html && c === "<" && n === "!" && source[i + 2] === "-" && source[i + 3] === "-") {
      inHtml = true;
      i += 3;
      continue;
    }
    if (c === "/" && n === "/") {
      i++;
      while (i + 1 < source.length && source[i + 1] !== "\n") i++;
      continue;
    }
    if (c === "'" || c === '"' || c === "`") quote = c;
    line += c;
  }
  out.push(line);
  return out;
}

/**
 * 从 `{` 起做字符串感知的花括号配对，返回对应 `}` 的下标。
 *
 * **失败返回 `txt.length`**（越界一位）而不是 `txt.length - 1`：调用方普遍用
 * `slice(open, matchBrace(...) + 1)` 取函数体，越界一位恰好让 `slice` 取到末尾；
 * 若返回末字符下标，则每个未闭合的花括号都会**静默砍掉最后一个字符**。
 *
 * 字符串识别与 [`skipString`] 一致（含原始字符串），故 `format!("{{}}")` 里的
 * 字面量括号不会让计数错位。
 */
export function matchBrace(txt, openIdx) {
  let depth = 0;
  for (let p = openIdx; p < txt.length; p++) {
    const c = txt[p];
    if (c === '"') {
      p = skipString(txt, p);
      continue;
    }
    if (c === "{") depth++;
    else if (c === "}") {
      depth--;
      if (depth === 0) return p;
    }
  }
  return txt.length;
}

/**
 * 收集 `#[cfg(test)]` 测试模块占据的字符区间 `[start, end)`。
 *
 * 为什么需要：测试里的 `CapabilityMeta` / `PluginMeta`（如 `PluginMeta::new("fake", …)`）
 * **不是生产事实**，不剔除会污染事实表（实测踩坑：某插件的注册名被抽成 `fake`）。
 *
 * 为什么不能「从第一个 `#[cfg(test)]` 直接截断到文件尾」：仓库里存在
 * **测试模块之后还有生产代码**的文件（如 `model/plugin.rs`：`mod tests` 在中段，
 * `traverse` 里的挂载点注册在其后）——截断会把生产事实一起丢掉
 * （实测踩坑：`model` 的挂载点消失）。故按**花括号配对**精确剔除模块体。
 *
 * 本函数是扫描逻辑的**唯一真源**：[`stripTestModules`]（剔除测试代码）与
 * 调用方的「把内联测试归属到测试列」都建立在它之上。
 */
export function testModuleSpans(txt) {
  const MARKER = "#[cfg(test)]";
  const spans = [];
  let i = 0;
  for (;;) {
    const idx = txt.indexOf(MARKER, i);
    if (idx < 0) return spans;
    const after = txt.slice(idx + MARKER.length);
    const modHead = after.match(/^\s*(?:#\[[^\]]*\]\s*)*mod\s+[A-Za-z0-9_]+\s*\{/);
    // 进度保证：无论匹配是否成功，i 都必须严格前进（否则死循环）
    if (modHead) {
      const open = idx + MARKER.length + modHead[0].length - 1;
      const end = Math.max(matchBrace(txt, open) + 1, open + 1);
      spans.push([idx, end]);
      i = end;
    } else {
      // 不是模块（如 `#[cfg(test)] use …;`）——只吞掉标记本身，保留后续代码
      i = idx + MARKER.length;
    }
  }
}

/** 剔除 [`testModuleSpans`] 那些区间，其余文本原样保留（顺序拼接即等价于「挖掉」） */
export function stripTestModules(txt) {
  const spans = testModuleSpans(txt);
  if (spans.length === 0) return txt;
  let out = "";
  let i = 0;
  for (const [s, e] of spans) {
    out += txt.slice(i, s);
    i = e;
  }
  return out + txt.slice(i);
}

/**
 * 把 `#[cfg(test)] mod … { … }` 的内容**抹成空白但保留换行**。
 *
 * 为什么不直接删掉：删掉会把后面的代码整体上移，**行号全错**——而报告里的
 * `file:line` 是给人去核对的唯一线索。抹成空白后，索引与原始行一一对应，
 * 豁免注释（写在原始行上）也能按同一个下标取到。
 *
 * 与 [`stripTestModules`] 的分工：前者给「要报行号的审计」，后者给「只取事实的生成器」。
 */
export function blankTestModules(txt) {
  const MARKER = "#[cfg(test)]";
  const out = [];
  let i = 0;
  for (;;) {
    const idx = txt.indexOf(MARKER, i);
    if (idx < 0) {
      out.push(txt.slice(i));
      return out.join("");
    }
    out.push(txt.slice(i, idx));
    const after = txt.slice(idx + MARKER.length);
    const modHead = after.match(/^\s*(?:#\[[^\]]*\]\s*)*mod\s+[A-Za-z0-9_]+\s*\{/);
    if (modHead) {
      const open = idx + MARKER.length + modHead[0].length - 1;
      const end = Math.max(matchBrace(txt, open) + 1, open + 1);
      const span = txt.slice(idx, end);
      out.push(span.replace(/[^\n]/g, " "));
      i = end;
    } else {
      i = idx + MARKER.length;
    }
  }
}
