#!/usr/bin/env node
/**
 * style-audit.mjs —— 前端样式定义使用情况静态审计（编译检查的一部分）
 *
 * 目标（双向闭环）：
 *   A. 被使用的样式定义必须存在 —— 「使用未定义」= ERROR
 *   B. 已存在的样式定义必须被使用 —— 「定义未使用」= WARNING（--strict 时也算失败）
 *   C. 已存在的组件必须被使用 —— import 依赖图不可达的 .vue = WARNING
 *
 * 覆盖范围：
 *   - 全局 CSS：tauri/src 下的 styles/ 与 assets/ 目录
 *   - Vue 组件：<style> / <style scoped> 块；scoped 样式视为组件私有，
 *     非 scoped 的 <style> 视为全局定义
 *   - 类名使用来源：静态 class="a b"、:class="{ key: ... }" / :class="['a']"
 *     / :class="'a'" 字面量、classList.add/remove/toggle/toggle 字面量、
 *     <script> 中的 kebab-case 字符串字面量（覆盖 computed 返回动态 class 的场景，
 *     属于过近似的"已使用"判定 —— 宁可漏报 unused，不误报 used）
 *   - **registry 词表**：组件 import 的 `src/registry/**` 模块里的字面量 / 对象键 /
 *     字符串联合成员。registry 的职责正是「数据 → 呈现」的纯映射（修饰类 / 状态
 *     色调 / 图标 / 标题），组件把它的返回值直接绑上 `:class`，因此那些词
 *     **不在组件源码里出现**——不认这条来源就会把 `.node-head.tool` 这类
 *     真实在用的 scoped 修饰类误报成「定义未使用」。
 *   - 设计令牌（--custom-property）：定义 vs var() 使用，双向检查
 *
 * 「无引用但属正常」只在本文件内显式登记（每条附原因），共四处：
 * ALLOW_UNUSED_CLASS_PREFIXES / ALLOW_UNUSED_EXACT（第三方库钩子）、
 * ALLOW_UNUSED_SCOPED（后端协议词表驱动的修饰类）、ALLOW_RUNTIME_PROPS
 * （运行时由 JS 写入）、ALLOW_UNUSED_PROPS（设计系统成员）。不要为消警告绕过登记。
 *
 * 用法：
 *   node scripts/style-audit.mjs                 # 审计 tauri/src
 *   node scripts/style-audit.mjs --strict        # WARNING 也算失败
 *   node scripts/style-audit.mjs --root=<dir>    # 换仓库根（回归测试用）
 *
 * 退出码：0 = 通过；1 = 存在 ERROR（或 --strict 下存在 WARNING）
 *
 * 约定：平台无关（Node.js ESM、零外部依赖、spawnSync 数组传参）。
 */

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { paint } from './color.mjs';

const defaultRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
// `--root=<仓库根>` 供回归测试用：在临时目录里铺夹具再跑，才能证明它会红
// （与 `mechanism-audit` 同一约定）。
const rootArg = process.argv.find((a) => a.startsWith('--root='));
const ROOT = rootArg ? path.resolve(rootArg.slice(7)) : defaultRoot;
const SRC = path.join(ROOT, 'tauri', 'src');

// ── 白名单：允许"定义了但没被静态引用"的类名前缀（多为第三方库钩子）──
const ALLOW_UNUSED_CLASS_PREFIXES = [
  'language-', // prism 语言标记（动态拼接）
  'token', // prism 语法节点
  'milkdown', // milkdown 编辑器内部结构
  'katex', // katex 公式渲染
  'mermaid', // mermaid 图表渲染
  'prose', // 排版体系钩子
  'cm-', // CodeMirror 内部结构
  'eq-', // 公式相关钩子
];

const ALLOW_UNUSED_EXACT = new Set([
  'ProseMirror', // milkdown 运行时创建的编辑器根元素类名
  // 其他保留给外部注入 DOM 的类名（在此逐个登记，附注原因）
]);

const isAllowedUnused = (name) =>
  ALLOW_UNUSED_EXACT.has(name) ||
  ALLOW_UNUSED_CLASS_PREFIXES.some((p) => name.startsWith(p));

// ── 协议驱动的修饰类（按文件登记：定义未使用告警的豁免）─────────────────
// 有些修饰类由**后端下发的词表**取值决定，词表不在本仓库的这一侧：
//   DetailBadge.style  ∈ default | disabled | accent   （标题徽标）
//   DetailAction.style ∈ primary | secondary | danger | icon [danger] | divider（动作按钮）
// 组件以 `:class="b.style"` / `:class="[a.style, …]"` 绑定，静态不可解析。
// 逐个登记（文件 → 类名 → 原因）。**只登记确来自协议词表的类名**，
// 不要拿它掩盖普通的死样式（那类应当直接删除）。
//
// ⚠️ **由本仓库 registry 产出的词表不在此登记**——那条来源由下面的
// 「registry 词表」规则自动认（组件 import 的 registry 模块即其修饰类来源），
// 因为它在仓库这一侧、可解析；登记表只留给「词表在后端、前端只是照做」的情形。
const ALLOW_UNUSED_SCOPED = new Map([
  ['tauri/src/components/vdfs/VdfsActions.vue', new Set(['primary', 'danger'])], // DetailAction.style
  ['tauri/src/components/vdfs/DetailForm.vue', new Set(['disabled', 'accent'])], // DetailBadge.style
]);

// ── 工具 ────────────────────────────────────────────────────────
// 配色走 `color.mjs`（唯一实现）。这里 `red` / `yellow` / `green` 是**预渲染的
// 标签串**（'ERROR' / 'WARN' / 'OK' 带色），与本模块的 `paint(code)(s)` 同源。
const red = paint('31')('ERROR');
const yellow = paint('33')('WARN');
const green = paint('32')('OK');
const rel = (p) => path.relative(ROOT, p).split(path.sep).join('/');

function walk(dir, exts, out = []) {
  // 目录不存在是**常态**而非异常：夹具 / 换根的仓库里 `styles/` 或 `assets/`
  // 完全可能没有。原先直接 `readdirSync` ⇒ 以 ENOENT 崩掉，回归测试无从写起。
  if (!fs.existsSync(dir)) return out;
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) {
      if (e.name === 'node_modules' || e.name.startsWith('.')) continue;
      walk(p, exts, out);
    } else if (exts.some((x) => e.name.endsWith(x))) {
      out.push(p);
    }
  }
  return out;
}

const stripCssComments = (css) => css.replace(/\/\*[\s\S]*?\*\//g, '');

/** 去 JS/TS 注释（块注释 + 行注释）。词表采集不得从注释散文里取词：
 *  注释里出现「.tool / .run 这类类名」是解释，不是产出。 */
const stripJsComments = (src) =>
  src.replace(/\/\*[\s\S]*?\*\//g, ' ').replace(/(^|[^:])\/\/[^\n]*/g, '$1');

/** 提取 CSS 文本中的类名定义与自定义属性定义。
 *  返回 classes: Map<name, deep:boolean> —— deep=true 表示该类出现在
 *  :deep(...) 内（编译后可匹配任意后代/插槽内容，属跨组件共享定义）。 */
function extractCssDefs(cssText) {
  const classes = new Map(); // name -> deep
  const props = new Set();
  const text = stripCssComments(cssText);
  // 逐条规则扫描（选择器 { 声明 }），用简单括号配对，避免把声明区的 .5 小数当成类名
  let i = 0;
  while (i < text.length) {
    const open = text.indexOf('{', i);
    if (open === -1) break;
    const selector = text.slice(i, open);
    // 找配对 }
    let depth = 1;
    let j = open + 1;
    while (j < text.length && depth > 0) {
      if (text[j] === '{') depth++;
      else if (text[j] === '}') depth--;
      j++;
    }
    const body = text.slice(open + 1, j - 1);
    // :deep(...) 段内的类 → 跨组件共享；其余 → 组件私有
    const deepSegs = [...selector.matchAll(/:deep\s*\(([^)]*)\)/g)].map((m) => m[1]);
    for (const seg of deepSegs) {
      for (const m of seg.matchAll(/\.([a-zA-Z_][\w-]*)/g)) classes.set(m[1], true);
    }
    const plain = selector.replace(/:deep\s*\([^)]*\)/g, '');
    for (const m of plain.matchAll(/\.([a-zA-Z_][\w-]*)/g)) {
      if (!classes.has(m[1])) classes.set(m[1], false);
    }
    // 声明区的自定义属性定义
    for (const m of body.matchAll(/(--[\w-]+)\s*:/g)) props.add(m[1]);
    i = j;
  }
  return { classes, props };
}

// ── 1. 收集全局 CSS 定义 ───────────────────────────────────────
const globalCssFiles = [
  ...walk(path.join(SRC, 'styles'), ['.css']),
  ...walk(path.join(SRC, 'assets'), ['.css']),
];
const globalClasses = new Map(); // name -> [file]
const globalProps = new Map();
for (const f of globalCssFiles) {
  const { classes, props } = extractCssDefs(fs.readFileSync(f, 'utf8'));
  for (const [n] of classes) {
    if (!globalClasses.has(n)) globalClasses.set(n, []);
    globalClasses.get(n).push(rel(f));
  }
  for (const n of props) {
    if (!globalProps.has(n)) globalProps.set(n, []);
    globalProps.get(n).push(rel(f));
  }
}

// ── 1.5 registry 词表（组件 scoped 修饰类的**产出方**）─────────────
// registry/ 是「数据 → 呈现」的纯映射层：修饰类、状态色调、图标、标题都以
// 字面量或字符串联合写在那里（如 `MessageHeadModifier` / `MessageStatusTone`），
// 组件把返回值直接绑到 `:class` 上——于是那些词**不在组件源码里**，
// 只扫组件必然把真实在用的 scoped 修饰类报成「定义未使用」。
//
// 规则（刻意不全局放宽）：**只认组件显式 import 的 registry 模块**，把其中的
// 字面量 / 对象键 / 字符串联合成员当作该组件的潜在类名来源。它只影响 check B
// （定义未使用），不参与 check A（使用未定义），因此不会掩盖「引用了不存在
// 的类」这类真错。与文件头的既有偏向一致：宁可漏报 unused，不误报 used。
function harvestPresentationTokens(text) {
  const src = stripJsComments(text);
  const out = new Set();
  for (const m of src.matchAll(/'([^'\n]+)'|"([^"\n]+)"/g)) {
    const s = m[1] ?? m[2];
    if (/^[a-zA-Z_][\w-]*$/.test(s)) out.add(s);
  }
  for (const m of src.matchAll(/\b([a-zA-Z_][\w-]*)\s*:/g)) out.add(m[1]);
  return out;
}

const registryFiles = walk(path.join(SRC, 'registry'), ['.ts']);
const registryTokensByFile = new Map(
  registryFiles.map((f) => [path.resolve(f), harvestPresentationTokens(fs.readFileSync(f, 'utf8'))]),
);

/** import 说明符 → 已扫描的 registry 文件（支持扩展名省略 / @ 别名） */
function resolveRegistryModule(spec, fromFile) {
  const base = spec.startsWith('@/')
    ? path.join(SRC, spec.slice(2))
    : spec.startsWith('.')
      ? path.resolve(path.dirname(fromFile), spec)
      : null;
  if (!base) return null;
  for (const cand of [base, `${base}.ts`, path.join(base, 'index.ts')]) {
    const abs = path.resolve(cand);
    if (registryTokensByFile.has(abs)) return abs;
  }
  return null;
}

// ── 2. 收集 Vue 组件定义与使用 ──────────────────────────────────
const vueFiles = walk(SRC, ['.vue']);
const vueModels = []; // { file, scopedClasses, globalClassesFromVue, templateUsages, scriptLiterals, varUsages }

function splitVue(text) {
  const tpl = text.match(/<template>([\s\S]*)<\/template>/);
  const styles = [];
  const styleRe = /<style([^>]*)>([\s\S]*?)<\/style>/g;
  let m;
  while ((m = styleRe.exec(text))) {
    styles.push({ scoped: /\bscoped\b/.test(m[1]), css: m[2] });
  }
  const script = text.replace(/<template>[\s\S]*<\/template>/, '').replace(styleRe, '');
  return { template: tpl ? tpl[1] : '', styles, script };
}

const VALID_CLASS = /^[a-zA-Z_][\w-]*$/;

/**
 * 从 :class 绑定表达式提取静态可解析的类名（写入 usages）与动态前缀（写入 prefixes）。
 * 捕获规则（刻意保守，宁可漏报不可误报）：
 *   - 整体是字符串字面量：:class="'flat'"
 *   - 数组字面量内的全部字符串：['a', cond && 'b']；数组里同样会有对象字面量与
 *     模板字面量前缀（`[`tag-${x}`, { open: f }]`），故数组分支额外收这两类。
 *     ⚠️ 数组分支**不套用下面的裸标识符简写规则** —— 数组里的裸标识符是变量
 *     （`[statusClass, …]`），当成类名会凭空造出「使用未定义」的 ERROR。
 *   - 对象键（引号/非引号）：{ 'is-expanded': x } / { error: cond }
 *   - 三元分支值：cond ? 'a' : 'b'
 *   - 动态前缀：'risk-' + x、`tag-${x}` → 前缀使用
 * 不捕获：函数调用实参（includes('Other')）等任意表达式中的字符串。
 */
function extractBindingClasses(expr, usages, prefixes) {
  const add = (n) => {
    if (n && VALID_CLASS.test(n)) usages.add(n);
  };
  // 对象键（引号 / 非引号）。注意：模板字面量的插值 `${...}` 不是对象字面量，
  // 必须先剔除——否则 `` `role-${roleKey}` `` 会被"简写对象键"规则误判成
  // 使用了类名 `roleKey`（`{roleKey}` 与 `${roleKey}` 文本上只差一个 `$`）。
  const addKeys = (text) => {
    const noInterp = text.replace(/\$\{[^{}]*\}/g, ' ');
    for (const s of noInterp.matchAll(/([{,]\s*)'([^']+)'\s*:|([{,]\s*)"([^"]+)"\s*:/g)) {
      const key = s[2] ?? s[4];
      if (key) key.split(/\s+/).forEach(add);
    }
    for (const s of noInterp.matchAll(/([{,]\s*)([a-zA-Z_][\w-]*)\s*:/g)) add(s[2]);
  };
  // 动态拼接前缀：'risk-' + x / `tag-${x}`
  const addPrefixes = (text) => {
    for (const s of text.matchAll(/'([a-zA-Z][\w-]*-)'|`([a-zA-Z][\w-]*-)(?:\$\{|)/g)) {
      const p = s[1] ?? s[2];
      if (p) prefixes.add(p);
    }
  };
  // 三元分支值
  const addTernary = (text) => {
    for (const s of text.matchAll(/[?:]\s*'([^']*)'|\?\s*"([^"]*)"/g)) {
      const lit = s[1] ?? s[2];
      if (lit) lit.split(/\s+/).forEach(add);
    }
  };

  const whole = expr.trim().match(/^'([^']+)'$|^"([^"]+)"$/);
  if (whole) {
    (whole[1] ?? whole[2]).split(/\s+/).forEach(add);
    return;
  }
  if (expr.trim().startsWith('[')) {
    for (const s of expr.matchAll(/'([^']*)'|"([^"]*)"/g)) {
      const lit = s[1] ?? s[2];
      if (lit) lit.split(/\s+/).forEach(add);
    }
    addKeys(expr);
    addPrefixes(expr);
    addTernary(expr);
    return;
  }
  addKeys(expr);
  // 简写对象键 { expanded }：只走非数组路径（数组里的裸标识符是变量，见函数头注）。
  // 仍须剔除 `${...}` 插值，理由同 addKeys。
  const noInterp = expr.replace(/\$\{[^{}]*\}/g, ' ');
  for (const s of noInterp.matchAll(/([{,]\s*)([a-zA-Z_][\w-]*)\s*(?=[},])/g)) add(s[2]);
  addTernary(expr);
  addPrefixes(expr);
}

/** 从模板文本提取静态可解析的 class 使用 */
function extractTemplateClassUsages(tpl) {
  const used = new Set();
  const prefixes = new Set();
  const add = (n) => {
    if (n && VALID_CLASS.test(n)) used.add(n);
  };
  // 静态 class="a b"（含单引号形式）。
  // 负向后行断言排除 :class / v-bind:class 绑定值被误当成静态类名。
  for (const m of tpl.matchAll(/(?<![:\w.$-])class\s*=\s*"([^"]*)"/g)) {
    m[1].split(/\s+/).forEach(add);
  }
  for (const m of tpl.matchAll(/(?<![:\w.$-])class\s*=\s*'([^']*)'/g)) {
    m[1].split(/\s+/).forEach(add);
  }
  for (const m of tpl.matchAll(/(?::|v-bind:)class\s*=\s*"([^"]*)"/g)) {
    extractBindingClasses(m[1], used, prefixes);
  }
  for (const m of tpl.matchAll(/(?::|v-bind:)class\s*=\s*'([^']*)'/g)) {
    extractBindingClasses(m[1], used, prefixes);
  }
  // 组件经 prop 传类名（如 `<BaseModal mask-class="confirm-overlay"
  // panel-class="confirm-dialog">`）：类名不写在 `class` 属性里，静态扫描原本
  // 看不见，于是这些 scoped 样式会被误判成"未使用"。`*-class="a b"` 形式的
  // 属性值同样计入**静态**使用——它们确实会落到元素上（与 Transition 的框架
  // 生成类不同，不需要 soft 的宽松待遇）。
  for (const m of tpl.matchAll(/(?<![:\w.$-])[\w-]+-class\s*=\s*"([^"]*)"/g)) {
    m[1].split(/\s+/).forEach(add);
  }
  for (const m of tpl.matchAll(/(?<![:\w.$-])[\w-]+-class\s*=\s*'([^']*)'/g)) {
    m[1].split(/\s+/).forEach(add);
  }
  // <Transition name="x"> / <TransitionGroup name="x"> 会自动应用
  // x-enter-active / x-leave-active / x-enter-from / x-enter-to / x-leave-to 系列类。
  // 这些类由 Vue 框架生成，**允许不写样式**（常见只写 4 个、省略 enter-to），
  // 因此记入"软使用"：计入已用（check B 不误报 unused），但不参与 check A 缺失判定。
  const soft = new Set();
  const addSoft = (n) => {
    if (n && VALID_CLASS.test(n)) soft.add(n);
  };
  for (const m of tpl.matchAll(/<[Tt]ransition(?:Group)?\s+[^>]*\bname\s*=\s*"([^"]+)"/g)) {
    ['enter-active', 'leave-active', 'enter-from', 'enter-to', 'leave-to'].forEach((s) =>
      addSoft(`${m[1]}-${s}`),
    );
  }
  return { used, prefixes, soft: [...soft] };
}

function extractVarUsages(...texts) {
  const used = new Set();
  for (const t of texts) {
    for (const m of t.matchAll(/var\(\s*(--[\w-]+)/g)) used.add(m[1]);
  }
  return used;
}

for (const f of vueFiles) {
  const text = fs.readFileSync(f, 'utf8');
  const { template, styles, script } = splitVue(text);
  const scopedClasses = new Map(); // name -> 是否为 :deep 共享定义（同文件内）
  const globalFromVue = new Set(); // 组件内非 scoped <style> 提供的全局类
  const vueProps = new Set(); // 组件内定义的自定义属性（scoped 与否均可继承）
  for (const s of styles) {
    const { classes, props } = extractCssDefs(s.css);
    for (const [n, deep] of classes) {
      if (s.scoped) scopedClasses.set(n, deep);
      else globalFromVue.add(n);
    }
    for (const n of props) vueProps.add(n);
  }
  // script 中的动态类名痕迹：引号字符串字面量 + 对象键（return 'warn' /
  // { thinking: cond } 等 —— 计入"已使用"判定，避免动态绑定误报 unused）
  const scriptTokens = new Set();
  for (const m of script.matchAll(/'([^'\n]+)'|"([^"\n]+)"/g)) {
    const s = m[1] ?? m[2];
    if (/^[a-zA-Z_][\w-]*$/.test(s)) scriptTokens.add(s);
  }
  for (const m of script.matchAll(/\b([a-zA-Z_][\w-]*)\s*:/g)) scriptTokens.add(m[1]);
  // 组件 import 的 registry 模块 = 其 scoped 修饰类的产出方（见 §1.5）
  const registryTokens = new Set();
  for (const m of script.matchAll(/(?:from\s+|import\s*\(\s*|import\s+)['"]([^'"]+)['"]/g)) {
    const mod = resolveRegistryModule(m[1], f);
    if (mod) for (const t of registryTokensByFile.get(mod)) registryTokens.add(t);
  }
  vueModels.push({
    file: rel(f),
    scopedClasses: [...scopedClasses.keys()],
    scopedDeep: new Set([...scopedClasses.entries()].filter(([, d]) => d).map(([n]) => n)),
    globalFromVue: [...globalFromVue.keys()],
    vueProps: [...vueProps],
    scriptTokens: [...scriptTokens],
    registryTokens: [...registryTokens],
    scriptPrefixes: [...script.matchAll(/`([a-zA-Z][\w-]*-)(?:\$\{|'|"|\s)/g)].map((m) => m[1]),
    tpl: extractTemplateClassUsages(template),
    scriptLiterals: [...script.matchAll(/'([^'\n]+)'|"([^"\n]+)"/g)]
      .map((m) => m[1] ?? m[2])
      .filter((s) => /^[a-zA-Z][\w-]*$/.test(s) && s.includes('-')), // kebab-case 候选
    varUsages: extractVarUsages(template, script, ...styles.map((s) => s.css)),
  });
}
// 展开模板使用结果
for (const v of vueModels) {
  v.templateUsages = v.tpl.used;
  v.prefixUsages = v.tpl.prefixes;
  v.softUsages = new Set(v.tpl.soft);
}

// ── 3. 汇总全局可用定义（全局 CSS + 各组件非 scoped 样式 + classList）──
const allGlobalClasses = new Set(globalClasses.keys());
const tsFiles = walk(SRC, ['.ts']);
let tsLiterals = new Set();
let tsPrefixes = new Set();
for (const f of tsFiles) {
  const t = fs.readFileSync(f, 'utf8');
  for (const m of t.matchAll(/'([^'\n]+)'|"([^"\n]+)"/g)) {
    const s = m[1] ?? m[2];
    if (/^[a-zA-Z][\w-]*$/.test(s) && s.includes('-')) tsLiterals.add(s);
  }
  // 反引号动态类前缀：`status-${...}` → "status-"
  for (const m of t.matchAll(/`([a-zA-Z][\w-]*-)(?:\$\{|'|"|\s)/g)) tsPrefixes.add(m[1]);
}
for (const v of vueModels) for (const n of v.globalFromVue) allGlobalClasses.add(n);

// ── 4. 检查 A：使用未定义 ──────────────────────────────────────
const errors = [];
const warnings = [];
const allDefinedClasses = new Set(allGlobalClasses);
for (const v of vueModels) for (const n of v.scopedClasses) allDefinedClasses.add(n);
// :deep(...) 定义池：编译后可匹配任意后代/插槽内容，跨组件可用
const deepShared = new Set();
for (const v of vueModels) for (const n of v.scopedDeep) deepShared.add(n);

for (const v of vueModels) {
  const own = new Set([...v.scopedClasses, ...v.globalFromVue]);
  for (const n of v.templateUsages) {
    if (own.has(n) || allGlobalClasses.has(n) || deepShared.has(n)) continue;
    // 跨组件 plain scoped 定义：scoped 属性对插槽/子组件内容不生效，
    // 多半是坏味道 —— 降级为警告，提示确认。
    if (vueModels.some((o) => o.scopedClasses.includes(n) && !o.scopedDeep.has(n))) {
      warnings.push(
        `${yellow}: 类名 ".${n}" 在 ${v.file} 中使用，仅定义于其他组件的 scoped 样式（确认是否有意跨组件依赖）`,
      );
      continue;
    }
    errors.push(
      `${red}: 类名 ".${n}" 在 ${v.file} 模板中使用，但未在任何全局 CSS / 该组件样式中定义`,
    );
  }
  // 动态前缀：至少要能命中一个已定义类
  for (const p of v.prefixUsages) {
    if ([...allDefinedClasses].some((d) => d.startsWith(p))) continue;
    errors.push(
      `${red}: 动态类前缀 "${p}*" 在 ${v.file} 中使用，但没有任何已定义类名以它开头`,
    );
  }
}

// ── 5. 检查 B：定义未使用（WARNING）───────────────────────────
const usedSomewhere = new Set();
for (const v of vueModels) {
  for (const n of v.templateUsages) usedSomewhere.add(n);
  for (const n of v.softUsages) usedSomewhere.add(n); // transition 自动类计入已用
  for (const n of v.scriptLiterals) usedSomewhere.add(n);
}
for (const s of tsLiterals) usedSomewhere.add(s);
const allPrefixes = [];
for (const v of vueModels) {
  for (const p of v.prefixUsages) allPrefixes.push(p);
  for (const p of v.scriptPrefixes ?? []) allPrefixes.push(p); // 组件内 `x-${…}` 前缀
}
for (const p of tsPrefixes) allPrefixes.push(p); // 全局 .ts 中的 `x-${…}` 前缀
const usedByPrefix = (name) => allPrefixes.some((p) => name.startsWith(p));

// 5a. 全局类
for (const [name, files] of globalClasses) {
  if (usedSomewhere.has(name) || usedByPrefix(name) || isAllowedUnused(name)) continue;
  warnings.push(
    `${yellow}: 全局类 ".${name}"（定义于 ${files.join(', ')}）未被任何模板/脚本引用`,
  );
}
// 5b. 组件 scoped 类（仅在本组件的模板/script/字面量中找引用；
//     :deep 定义额外接受任意组件的使用 —— 它们本就为后代/插槽内容而写）
const usedByAnyTemplate = new Set(usedSomewhere);
for (const v of vueModels) {
  const localUsed = new Set([
    ...v.templateUsages,
    ...v.softUsages,
    ...v.scriptLiterals,
    ...v.scriptTokens, // return 'warn' / { thinking: cond } 等动态痕迹
    ...v.registryTokens, // 本组件 import 的 registry 词表（见 §1.5）
  ]);
  const localPrefixes = [...v.prefixUsages, ...(v.scriptPrefixes ?? [])];
  const perFileAllowed = ALLOW_UNUSED_SCOPED.get(v.file);
  for (const n of v.scopedClasses) {
    if (
      localUsed.has(n) ||
      localPrefixes.some((p) => n.startsWith(p)) ||
      (v.scopedDeep.has(n) && usedByAnyTemplate.has(n)) ||
      perFileAllowed?.has(n) ||
      isAllowedUnused(n)
    ) {
      continue;
    }
    warnings.push(`${yellow}: scoped 类 ".${n}"（${v.file}）未在本组件中使用`);
  }
}

// ── 6. 令牌（--custom-property）双向检查 ───────────────────────
// 自定义属性沿 DOM 继承：无论定义在全局 CSS 还是组件 scoped 样式，
// var() 引用都视为有定义。
const allPropsDefined = new Map(globalProps); // name -> files
for (const v of vueModels) {
  for (const n of v.vueProps) {
    if (!allPropsDefined.has(n)) allPropsDefined.set(n, []);
    allPropsDefined.get(n).push(`${v.file}（组件内）`);
  }
}
// 运行时由 JS 写入 document 元素的令牌（无法静态定义，登记豁免 + 原因）
const ALLOW_RUNTIME_PROPS = new Set([
  '--font-scale', // appearance store 按用户字体档位写入根元素（见 styles/base.css 头注）
]);
// 已定义、但当前无 var() 引用的令牌（登记豁免 + 原因）。
// 令牌层是**完整语义层**：浅/深各一份，按语义组与刻度成组定义（前端 UI 设计
// 约定即「§4.1 各组令牌齐备」）。成员是否被消费取决于当前组件，「未被引用」
// 不等于死代码。此处只登记确认为设计系统成员的项，其余仍按警告处理。
const ALLOW_UNUSED_PROPS = new Set([
  '--accent-subtle-border', // accent 组第 4 个成员（描边），与 --danger-border 对称
  '--surface-fade', // 折叠区渐隐遮罩终点，浅/深各一份（供折叠面板消费）
  '--color-banner-bg', // banner 组（bg / border / fg）成员，成组齐全
  '--color-banner-fg', // 同上：banner 组的 fg 成员（组内只有 border 被 ChatMainPanel 消费）
  '--radius-xs', // 圆角刻度 xs→xl 的一档（唯一消费者 .path-pill 为死样式，已删）
  '--font-size-xl', // 字号刻度 xs→xl 的一档（20px 页面标题）
  '--font-weight-regular', // 字重刻度 regular / medium / semibold 的一档
  '--text-inverse', // 反色表面前景，与 --text-on-accent 并列（浅/深各一份）；唯一消费者 .danger-btn 为死样式，已删
]);
const propUsed = new Set();
for (const v of vueModels) for (const n of v.varUsages) propUsed.add(n);
for (const f of globalCssFiles) {
  for (const m of fs
    .readFileSync(f, 'utf8')
    .matchAll(/var\(\s*(--[\w-]+)/g)) propUsed.add(m[1]);
}
for (const n of propUsed) {
  if (allPropsDefined.has(n) || ALLOW_RUNTIME_PROPS.has(n)) continue;
  errors.push(`${red}: 设计令牌 "${n}" 被使用，但未在任何 CSS 中定义（若为运行时注入请登记 ALLOW_RUNTIME_PROPS）`);
}
for (const [name, files] of allPropsDefined) {
  if (propUsed.has(name) || ALLOW_UNUSED_PROPS.has(name)) continue;
  warnings.push(`${yellow}: 设计令牌 "${name}"（定义于 ${files.join(', ')}）未被 var() 引用`);
}

// ── 6.5 检查 C：未使用 Vue 组件（依赖图可达性）─────────────────
// 从入口 main.ts 出发沿 import 边做 BFS，任何 .vue 文件若不可达即为死组件。
// 依赖：项目无 unplugin-vue-components / 全局注册（已核实），import 全部显式。
{
  const vueAbs = new Set(vueFiles.map((p) => path.resolve(p)));
  const tsAbs = new Set(tsFiles.map((p) => path.resolve(p)));
  const scannedAbs = new Set([...vueAbs, ...tsAbs]);
  const importGraph = new Map(); // abs file -> [abs imported file]（含 .ts 中转节点）
  // 把 import 说明符解析为已扫描文件（支持扩展名省略：./router → router/index.ts）
  const resolveSpec = (spec, fromFile) => {
    const base = spec.startsWith('.')
      ? path.resolve(path.dirname(fromFile), spec)
      : path.resolve(SRC, spec.replace(/^@\//, ''));
    const candidates = [base];
    if (!path.extname(base)) {
      candidates.push(`${base}.ts`, path.join(base, 'index.ts'), `${base}.vue`);
    }
    return candidates.find((p) => scannedAbs.has(p));
  };
  for (const f of [...vueFiles, ...tsFiles]) {
    const t = fs.readFileSync(f, 'utf8');
    // 三类 import 都要算边：named/default（`from 'x'`）、动态（`import('x')`）、
    // **副作用裸导入**（`import 'x'` —— 注册表类模块的挂载方式，如
    // `VdfsWorkbench.vue` 的 `import '@/registry/vdfsRenderers'`）。
    // 漏掉裸导入会让整条注册链在图上断开，把被注册的组件全误报成死组件。
    const specs = [
      ...[...t.matchAll(/(?:from\s+|import\s*\(\s*|import\s+)['"]([^'"]+)['"]/g)].map((m) => m[1]),
    ];
    const deps = [];
    for (const spec of specs) {
      const p = resolveSpec(spec, f);
      if (p) deps.push(p); // .ts 中转节点必须保留，否则链路在此断裂
    }
    importGraph.set(path.resolve(f), deps);
  }
  const entry = path.join(SRC, 'main.ts');
  const reachable = new Set(fs.existsSync(entry) ? [path.resolve(entry)] : []);
  const queue = [...reachable];
  while (queue.length) {
    for (const dep of importGraph.get(queue.pop()) ?? []) {
      if (!reachable.has(dep)) {
        reachable.add(dep);
        queue.push(dep);
      }
    }
  }
  for (const p of vueFiles) {
    if (!reachable.has(path.resolve(p))) {
      warnings.push(`${yellow}: 组件 ${rel(p)} 未被任何入口/组件 import（疑似废弃组件）`);
    }
  }
}

// ── 7. 输出 ────────────────────────────────────────────────────
const relSrc = rel(SRC);
console.log(`\n== 样式审计（${relSrc} 下 ${vueFiles.length} 个 .vue / ${globalCssFiles.length} 个全局 css）==`);
console.log(`   类定义：全局 ${globalClasses.size} 个 / 令牌定义 ${allPropsDefined.size} 个`);
console.log(`   使用快照：模板类引用 ${usedSomewhere.size} 个 / var() 引用 ${propUsed.size} 个\n`);

for (const e of errors) console.log(`  ${e}`);
for (const w of warnings) console.log(`  ${w}`);

console.log(
  `\n${green}结果:${paint('0')('')} ${errors.length} 个错误, ${warnings.length} 个警告`,
);
process.exit(errors.length > 0 || (process.argv.includes('--strict') && warnings.length > 0) ? 1 : 0);
