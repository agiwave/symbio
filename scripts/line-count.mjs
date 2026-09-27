/**
 * 行数与规模统计口径（唯一真源）
 *
 * 供 `scripts/gen-current-facts.mjs` 与 `scripts/line-budget-audit.mjs` 等审计共同引用。
 * 本模块提取自 `gen-current-facts.mjs`，确保事实表与棘轮口径逐字一致，绝无漂移。
 */

import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";

/** 目录遍历时跳过的名字（构建产物 / 依赖 / 版本库） */
export const SCOPE_SKIP = ["target", "vendor", "node_modules", "dist", ".git"];

/**
 * 递归收集指定后缀的文件（测试文件按 `*.test.rs` / `*.spec.ts` / `tests.rs` 单独归类，不混进实现）
 */
export function walkScope(dir, exts, isTest) {
  const out = [];
  let ents;
  try {
    ents = readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const e of ents) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) {
      if (SCOPE_SKIP.includes(e.name)) continue;
      out.push(...walkScope(p, exts, isTest));
      continue;
    }
    const test =
      e.name.endsWith(".test.rs") ||
      e.name.endsWith(".spec.ts") ||
      e.name === "tests.rs";
    if (!exts.some((x) => e.name.endsWith(x)) || test !== isTest) continue;
    out.push(p);
  }
  return out;
}

export function countLines(files) {
  let n = 0;
  for (const f of files) n += readFileSync(f, "utf8").split("\n").length - 1;
  return n;
}

/**
 * 跳过字符串字面量，返回闭引号（或原始串收尾分隔符末位）的下标。
 *
 * 只**向前**扫描：返回值恒 ≥ `start`，调用方据此推进扫描位置；
 * 一旦返回更小的下标（曾用"向前 lastIndexOf 找 r" 的实现），
 * 外层扫描就会原地打转（实测踩坑：生成器死循环）。
 */
export function skipString(txt, start) {
  // 原始字符串判定：开引号紧跟在 `r` 或 `r###` 之后（`r"…"` / `r#"…"#` / `br#"…"#`）
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

/** 从 `{` 起做字符串感知的花括号配对，返回对应 `}` 的下标（失败返回文本末尾） */
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
  return txt.length - 1;
}

/**
 * 收集 `#[cfg(test)]` 测试模块占据的字符区间 `[start, end)`。
 *
 * 扫描逻辑的**唯一真源**：`stripTestModules`（提取事实时剔除测试代码）与
 * `splitRustLines`（统计行数时把内联测试归属到"测试"列）都建立在它之上。
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

/** 剔除上面那些区间，其余文本原样保留（顺序拼接即等价于"挖掉"） */
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
 * `.rs` 文件的实现/测试行数归属。
 *
 * 为什么需要：`#[cfg(test)] mod tests { … }` 是**内联**在实现文件里的，按"整文件"
 * 归类会把测试行算进实现（实测：`symbio/src` 的实现行因此虚高约 7.5k，测试行虚低
 * 同量，会让人误判测试密度）。独立测试文件（`*.test.rs` / `tests.rs`）本来就被
 * `walkScope` 分开了，这里只处理内联模块。
 *
 * 返回仍按**行数**（换行符个数）计，与 `countLines` 同口径。
 */
export function splitRustLines(files) {
  let impl = 0;
  let test = 0;
  for (const f of files) {
    const txt = readFileSync(f, "utf8");
    const total = txt.split("\n").length - 1;
    let inlineTest = 0;
    for (const [s, e] of testModuleSpans(txt)) {
      inlineTest += txt.slice(s, e).split("\n").length - 1;
    }
    impl += total - inlineTest;
    test += inlineTest;
  }
  return { impl, test };
}

/**
 * 一个统计范围：`root` 仓库根目录绝对路径，`dir` 相对仓库根，`exts` 参与统计的后缀。
 *
 * ⚠️ `dir` 会**原样进生成物**（§5.1 表格首列），故调用方必须给**正斜杠字面量**，
 * 不得用 `path.join` —— 后者在 Windows 上产出 `symbio\src`、在 Linux 上产出
 * `symbio/src`，同一份代码在两平台生成出不同内容；CI（Linux）的 `--check`
 * 因「Windows 提交的生成物 vs Linux 重生成」逐字比对而必红。
 * 真实文件访问仍走 `path.join`（正斜杠在 Windows 上同样可解析）。
 */
export function scopeRow(root, dir, exts) {
  const impl = walkScope(path.join(root, dir), exts, false);
  const test = walkScope(path.join(root, dir), exts, true);
  // `.rs` 的内联测试模块按归属从「实现」移入「测试」；其它后缀没有这个概念
  const split = exts.includes(".rs")
    ? splitRustLines(impl)
    : { impl: countLines(impl), test: 0 };
  return {
    dir,
    implFiles: impl.length,
    implLines: split.impl,
    testFiles: test.length,
    testLines: countLines(test) + split.test,
  };
}
