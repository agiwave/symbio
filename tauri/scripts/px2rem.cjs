/**
 * px → rem / spacing-token 转换脚本（B-3，一次性工具，用后即删）
 *
 * 规则（team-lead 指定）：
 *  1. 仅处理 .vue 文件的 <style> 块；<template>/<script> 不动。
 *  2. box-shadow 声明整行保留 px（阴影偏移/模糊不参与等比缩放，含跨行声明）。
 *  3. 1px（含 -1px）保留 —— 物理 hairline，缩放会导致 2px 粗线。
 *  4. font-size：11/12/14/16/18/20/24px → var(--font-size-*) 就近匹配；其余 → rem。
 *  5. spacing 属性（padding、margin、gap、top/left/right/bottom、inset）：
 *     正整数 2/4/8/12/16/24/32px → var(--space-*)；其余 → rem。
 *  6. 其余属性：Npx → (N/16)rem（负值直接负 rem），0px 保持不变。
 *  7. 不改动行内注释里的 1px 语义说明。
 */
const fs = require('fs');
const path = require('path');

const ROOTS = [
  path.join(__dirname, '..', 'src', 'views'),
  path.join(__dirname, '..', 'src', 'components'),
];

const SPACE_TOKENS = {
  2: 'var(--space-05)',
  4: 'var(--space-1)',
  8: 'var(--space-2)',
  12: 'var(--space-3)',
  16: 'var(--space-4)',
  24: 'var(--space-5)',
  32: 'var(--space-6)',
};
const FONT_TOKENS = {
  11: 'var(--font-size-xs)',
  12: 'var(--font-size-sm)',
  14: 'var(--font-size-base)',
  16: 'var(--font-size-md)',
  18: 'var(--font-size-lg)',
  20: 'var(--font-size-xl)',
  24: 'var(--font-size-2xl)',
};
const SPACING_PROPS = /^(padding|padding-top|padding-right|padding-bottom|padding-left|margin|margin-top|margin-right|margin-bottom|margin-left|gap|row-gap|column-gap|top|right|bottom|left|inset|inset-inline|inset-block)(-(top|right|bottom|left))?$/;

function toRem(numStr) {
  const n = parseFloat(numStr);
  const r = Math.round((n / 16) * 10000) / 10000;
  return `${r}rem`;
}

function walk(dir, out) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p, out);
    else if (e.isFile() && e.name.endsWith('.vue')) out.push(p);
  }
  return out;
}

/** 处理一段 style 文本，返回 { text, pxLeft } */
function convertStyle(text) {
  const lines = text.split('\n');
  let inShadow = false; // 跨行 box-shadow 声明中
  const out = lines.map((line) => {
    // 进入/延续 box-shadow 声明：整行保留
    if (/box-shadow\s*:/.test(line)) {
      inShadow = !/;\s*(\/\/.*)?$/.test(line.replace(/\/\*.*?\*\//g, ''));
      return line;
    }
    if (inShadow) {
      if (/;/.test(line)) inShadow = false;
      return line; // 阴影续行保留 px
    }
    if (!/\dpx/.test(line)) return line;

    // 行首属性名
    const propM = line.match(/^\s*(-?[A-Za-z-]+)\s*:/);
    const prop = propM ? propM[1] : '';
    const isFont = prop === 'font-size';
    const isSpacing = SPACING_PROPS.test(prop);

    return line.replace(/(-?\d*\.?\d+)px/g, (m, numStr) => {
      const n = parseFloat(numStr);
      if (n === 0) return m;           // 0px 保持
      if (Math.abs(n) === 1) return m; // hairline 保留
      if (isFont && !numStr.startsWith('-') && FONT_TOKENS[n]) return FONT_TOKENS[n];
      if (isSpacing && !numStr.startsWith('-') && SPACE_TOKENS[n]) return SPACE_TOKENS[n];
      if (numStr.startsWith('-')) return `-${toRem(numStr.slice(1))}`;
      return toRem(numStr);
    });
  });
  const text2 = out.join('\n');
  const pxLeft = (text2.match(/\d+px/g) || []).length;
  return { text: text2, pxLeft };
}

const files = ROOTS.flatMap((r) => walk(r, []));
const report = [];
for (const f of files) {
  const src = fs.readFileSync(f, 'utf8');
  if (!/<style[^>]*>/.test(src)) continue;
  // 逐个 style 块处理
  const parts = [];
  let last = 0;
  const rel = path.relative(path.join(__dirname, '..'), f);
  let before = 0;
  const styleRe = /(<style[^>]*>)([\s\S]*?)(<\/style>)/g;
  let m;
  let after = 0;
  let converted = src;
  while ((m = styleRe.exec(src)) !== null) {
    before += (m[2].match(/\d+(\.\d+)?px/g) || []).length;
  }
  converted = src.replace(styleRe, (whole, open, body, close) => {
    const { text } = convertStyle(body);
    return open + text + close;
  });
  after = (converted.match(/\d+(\.\d+)?px/g) || []).length;
  if (converted !== src) {
    fs.writeFileSync(f, converted, 'utf8');
    report.push(`${rel}\tstyle块px: ${before} -> 全文件剩余px: ${after}`);
  }
}
fs.writeFileSync(path.join(__dirname, 'report.txt'), report.join('\n'), 'utf8');
console.log('done');
