#!/usr/bin/env node
/**
 * schema-audit.mjs —— schemas 定义使用情况审计
 *
 * 目标：
 *   1. 找出「未被使用」的 schema 定义（后端 symbio_core/schemas、前端 tauri/src/schemas）
 *   2. 找出「仅被一个模块使用」的 schema 模块（下放候选）
 *
 * 性质：**决策支持报告，不是判定**。它只输出候选清单，退出码恒为 0（除非自身崩溃）——
 *   因为「未被使用」的判定有天然盲区，必须人工 grep 复核后才可动手：
 *     · 后端：仅统计 `schemas::` 路径引用与标识符出现；`pub use` 再导出的顶层名
 *       （`crate::symbio_core::ChatMessage`）靠标识符出现兜底。
 *     · 前端：模块级引用识别 `from '...'`（含 `export * from`）与相对路径，
 *       故 schemas 内部的再导出链（`vdfs.ts` → `./vdfs-form`）不再误报死文件。
 *   本脚本已接入 `gate.mjs`（docs 阶段）：**崩溃会让门禁失败**，报告内容不判失败。
 *
 * 用法：node scripts/schema-audit.mjs
 * 输出：控制台报告。零外部依赖，纯 Node 实现。
 */
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative, sep, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// `--root=` 只为回归测试开的口子：测试在临时目录造一棵最小仓库，好让本脚本的判据
// 能被「注入真实形状并断言报告正确」地验证。报告型脚本坏了没人发现（它不判失败），
// 但它**会误导人去改本来没坏的东西**——实测事故：`schemas/hook` 被报成「仅 1 个外部
// 消费文件」的下放候选，而它实际有 3 个消费文件（hook 插件自己走 `schemas::{HookEvent}`
// 顶层再导出名，路径里没有子模块名，按「最长模块前缀」匹配得到 null ⇒ 消费方整个丢失）。
const rootArg = process.argv.find((a) => a.startsWith('--root='));
const ROOT = rootArg
  ? resolve(rootArg.slice(7))
  : join(dirname(fileURLToPath(import.meta.url)), '..');

// ---------- 工具 ----------
function walk(dir, exts, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) {
      if (name === 'target' || name === 'node_modules' || name === '.workbuddy') continue;
      walk(p, exts, out);
    } else if (exts.some((e) => name.endsWith(e))) {
      out.push(p);
    }
  }
  return out;
}
const rel = (p) => relative(ROOT, p).split(sep).join('/');

// ---------- 后端 ----------
const backendFiles = walk(join(ROOT, 'symbio/src'), ['.rs']);
const schemasRoot = join(ROOT, 'symbio/src/symbio_core/schemas');
const backendSchemaFiles = backendFiles.filter((f) => f.startsWith(schemasRoot));

// 模块路径：schemas/<a>/b.rs -> a::b ; schemas/<a>/mod.rs -> a ; schemas/common.rs -> common
// 统一用 '::' 连接，与 import 路径（schemas::a::b）一致
function moduleKeyOf(file) {
  const r = relative(schemasRoot, file).split(sep).join('::').replace(/\.rs$/, '');
  if (r === 'mod') return ''; // schemas/mod.rs
  return r.endsWith('::mod') ? r.slice(0, -'::mod'.length) : r;
}
const display = (key) => key.split('::').join('/');

// 每个文件里的 pub 项定义
const itemsByFile = new Map(); // file -> [{name, kind}]
for (const f of backendSchemaFiles) {
  const src = readFileSync(f, 'utf8');
  const names = [];
  const re = /\bpub\s+(struct|enum|type|const|fn|trait|static)\s+([A-Za-z0-9_]+)/g;
  let m;
  while ((m = re.exec(src))) names.push({ name: m[2], kind: m[1] });
  itemsByFile.set(f, names);
}
const allModKeys = new Set(backendSchemaFiles.map(moduleKeyOf).filter(Boolean));

// 符号名 -> 定义它的模块（`schemas::X` 这种**顶层再导出名**的归属）
//
// 为什么需要：消费方可以写 `use ...::schemas::{HookEvent}`（走 `schemas/mod.rs` 的
// `pub use hook::{HookEvent, HookOutput}`），而不是 `schemas::hook::HookEvent`。
// 前者的路径里**根本没有子模块名**，只按「最长模块前缀」匹配会得到 `null` ⇒
// 该消费方整个丢失。实测后果：`schemas/hook` 有 3 个消费模块（hook 自己 3 个文件 +
// session），报告却写「仅 1 个外部消费文件」，把它列成**下放候选**——
// 报告说假话，读的人会顺着去改本来没坏的东西。
//
// 只在该名字**唯一**归属一个模块时启用；同名多处定义（有歧义）时保守放弃。
const nameToModule = new Map(); // name -> moduleKey（仅唯一时）
{
  const seen = new Map(); // name -> Set(moduleKey)
  for (const [f, names] of itemsByFile) {
    const key = moduleKeyOf(f);
    if (!key) continue;
    for (const { name } of names) {
      if (!seen.has(name)) seen.set(name, new Set());
      seen.get(name).add(key);
    }
  }
  for (const [name, keys] of seen) if (keys.size === 1) nameToModule.set(name, [...keys][0]);
}

/**
 * 把一条 `schemas::…` 引用路径归到某个模块。
 * @param {string[]} parts 路径分段（已剥掉 `schemas::` 前缀与 `as` 别名）
 * @returns {string|null} 模块 key
 */
function moduleOfRef(parts) {
  for (let i = parts.length; i >= 1; i--) {
    const cand = parts.slice(0, i).join('::');
    if (allModKeys.has(cand)) return cand;
  }
  // 顶层再导出名（`schemas::HookEvent`）：按符号名的唯一定义归属
  return nameToModule.get(parts[0]) ?? null;
}

// 全库扫描：schemas:: 路径引用 + 标识符出现
// consumerFile -> Set(schemaModuleKey)
const consumersByModule = new Map(); // moduleKey -> Map(file -> count)
const identFiles = new Map(); // identifier -> Set(file)（排除定义文件与 schemas/mod.rs）

// 展开 use 语句花括号组：a::{b, c::d} -> [a::b, a::c::d]
function splitTop(s) {
  const parts = [];
  let depth = 0, cur = '';
  for (const ch of s) {
    if (ch === '{') depth++;
    if (ch === '}') depth--;
    if (ch === ',' && depth === 0) { parts.push(cur); cur = ''; } else cur += ch;
  }
  if (cur.trim()) parts.push(cur);
  return parts.map((p) => p.trim()).filter(Boolean);
}
function expandUse(path) {
  const i = path.indexOf('{');
  if (i === -1) return [path.trim()];
  const close = path.lastIndexOf('}');
  const prefix = path.slice(0, i);
  const body = path.slice(i + 1, close);
  const rest = path.slice(close + 1); // 如 "::X"
  const out = [];
  for (const p of splitTop(body)) out.push(...expandUse(prefix + p + rest));
  return out;
}

for (const f of backendFiles) {
  const src = readFileSync(f, 'utf8');
  const isSchemasMod = rel(f) === 'symbio/src/symbio_core/schemas/mod.rs';

  // 1) use 语句（含花括号组展开）
  if (!isSchemasMod) {
    const useRe = /(?:pub\s+)?use\s+([^;]+);/g;
    let um;
    while ((um = useRe.exec(src))) {
      for (const full of expandUse(um[1])) {
        const idx = full.indexOf('schemas::');
        if (idx === -1) continue;
        const rest = full.slice(idx + 'schemas::'.length).split(' as ')[0].trim();
        if (!rest) continue;
        const best = moduleOfRef(rest.split('::'));
        if (best) {
          if (!consumersByModule.has(best)) consumersByModule.set(best, new Map());
          const map = consumersByModule.get(best);
          map.set(f, (map.get(f) ?? 0) + 1);
        }
      }
    }

    // 2) 内联路径引用（非 use 语句）
    const re = /schemas::([A-Za-z0-9_]+(?:::[A-Za-z0-9_]+)*)/g;
    let m;
    while ((m = re.exec(src))) {
      const best = moduleOfRef(m[1].split('::'));
      if (best) {
        if (!consumersByModule.has(best)) consumersByModule.set(best, new Map());
        const map = consumersByModule.get(best);
        map.set(f, (map.get(f) ?? 0) + 1);
      }
    }
  }

  // 2) 标识符出现（排除定义文件自身与 schemas/mod.rs 的 re-export 行）
  if (!isSchemasMod) {
    for (const [defFile, names] of itemsByFile) {
      if (defFile === f) continue;
      for (const { name } of names) {
        const re = new RegExp(`\\b${name}\\b`);
        if (re.test(src)) {
          if (!identFiles.has(name)) identFiles.set(name, new Set());
          identFiles.get(name).add(f);
        }
      }
    }
  }
}

// ---------- 后端报告 ----------
console.log('==================== 后端 schemas 审计 ====================\n');

// 同文件自引用计数：定义行之外的出现次数（>0 说明定义文件内部还在用它，非死项）
const selfOcc = new Map(); // name -> count in defining file
for (const [f, names] of itemsByFile) {
  const src = readFileSync(f, 'utf8');
  for (const { name } of names) {
    const count = (src.match(new RegExp(`\\b${name}\\b`, 'g')) ?? []).length;
    selfOcc.set(name, (selfOcc.get(name) ?? 0) + count);
  }
}

/**
 * 声明处（或其上一行）带 `// dead-code-allow R-001: <理由>` ⇒ **已承认保留**。
 *
 * 报告里若不区分，读者会把"刻意保留的跨栈契约半边"当成可以清理的垃圾——而那正是
 * `dead-code-audit` 的承认通道存在的意义。两个守卫必须说同一句话。
 */
function waiverOf(src, name) {
  const lines = src.split('\n');
  const decl = new RegExp(`\\b(const|struct|enum|fn|type)\\s+${name}\\b`);
  for (let i = 0; i < lines.length; i += 1) {
    if (!decl.test(lines[i])) continue;
    for (const j of [i, i - 1]) {
      if (j < 0) continue;
      const m = lines[j].match(/\/\/\s*dead-code-allow\s+R-\d+\s*:\s*(.+?)\s*$/);
      if (m) return m[1];
    }
  }
  return null;
}

// 死项：外部无引用，且定义文件内除定义行外也无引用
const deadItems = [];
for (const f of backendSchemaFiles) {
  const src = readFileSync(f, 'utf8');
  for (const { name, kind } of itemsByFile.get(f) ?? []) {
    const users = identFiles.get(name);
    const own = selfOcc.get(name) ?? 1;
    if ((!users || users.size === 0) && own <= 1) {
      deadItems.push({ file: rel(f), name, kind, waiver: waiverOf(src, name) });
    }
  }
}
console.log('--- 死项（全库标识符无引用） ---');
console.log('    ⚠️ 标注「已承认保留」的**不要删** —— 那是刻意保留的跨栈契约半边；');
console.log('       其余的才是可清理项。（本表是**报告**，判定型规则是 dead-code-audit 的 R-001）');
const byFile = new Map();
for (const d of deadItems) {
  if (!byFile.has(d.file)) byFile.set(d.file, []);
  byFile.get(d.file).push(`${d.kind} ${d.name}${d.waiver ? `（已承认保留：${d.waiver}）` : ''}`);
}
for (const [f, list] of [...byFile].sort()) console.log(`  ${f}\n    ${list.join(', ')}`);
if (deadItems.length === 0) console.log('  （无）');

// 顶层模块（目录 = 下放单元）外部消费方
const topConsumers = new Map(); // top -> Set(consumerFile)
for (const f of backendSchemaFiles) {
  const key = moduleKeyOf(f);
  if (!key) continue;
  const top = key.split('::')[0];
  if (!topConsumers.has(top)) topConsumers.set(top, new Set());
  const cons = consumersByModule.get(key);
  if (cons) for (const cf of cons.keys()) if (!cf.startsWith(schemasRoot)) topConsumers.get(top).add(cf);
}
// schemas/mod.rs 的 pub use re-export 也算作「经由顶层模块出口」的消费途径：
// 消费方 import 的是顶层名（crate::symbio_core::ChatMessage），归属到被 re-export 的模块
const modRs = readFileSync(join(schemasRoot, 'mod.rs'), 'utf8');
const reExportRe = /pub\s+use\s+([A-Za-z0-9_]+(?:::[A-Za-z0-9_]+)*)\s*\{/g;
let rm;
while ((rm = reExportRe.exec(modRs))) {
  const parts = rm[1].split('::');
  if (parts.length >= 1 && topConsumers.has(parts[0])) {
    // re-export 本身不算外部消费方，但其存在说明该模块被「出口」——
    // 真正消费方在 identFiles 里已按标识符计入。此处仅保证模块不因 0 引用被误判为死。
  }
}

const shortMod = (p) => {
  const m = p.match(/src[\\/](plugins|symbio_core)[\\/]([^/\\.]+)/);
  return m ? `${m[1] === 'plugins' ? '' : 'core:'}${m[2]}` : p;
};

console.log('\n--- 顶层模块外部消费方（下放/删除决策单元） ---');
const moveCandidates = [];
const deleteCandidates = [];
for (const [top, cons] of [...topConsumers].sort((a, b) => a[0].localeCompare(b[0]))) {
  const uniq = [...new Set(cons)];
  const label = [...new Set(uniq.map(shortMod))].join(', ') || '（无外部消费方）';
  console.log(`  ${top.padEnd(10)} [${uniq.length}] -> ${label}`);
  if (uniq.length === 1) moveCandidates.push({ top, consumer: uniq[0] });
  if (uniq.length === 0) deleteCandidates.push(top);
}

console.log('\n--- 下放候选（整个顶层模块仅 1 个外部消费文件） ---');
for (const { top, consumer } of moveCandidates) console.log(`  schemas/${top}  =>  ${rel(consumer)}`);
if (moveCandidates.length === 0) console.log('  （无）');

console.log('\n--- 删除候选（整个顶层模块无外部消费方；需人工确认无 re-export 出口） ---');
for (const t of deleteCandidates) console.log(`  schemas/${t}`);
if (deleteCandidates.length === 0) console.log('  （无）');

// 各顶层模块文件清单（便于评估迁移工作量）
console.log('\n--- 顶层模块文件清单 ---');
for (const [top, cons] of [...topConsumers].sort((a, b) => a[0].localeCompare(b[0]))) {
  const files = backendSchemaFiles
    .filter((f) => { const k = moduleKeyOf(f); return k && k.split('::')[0] === top; })
    .map((f) => relative(schemasRoot, f).split(sep).join('/'));
  console.log(`  ${top} (${files.length}): ${files.join(', ')}`);
}

// ---------- 前端 ----------
const feFiles = walk(join(ROOT, 'tauri/src'), ['.ts', '.vue']);
const feSchemasRoot = join(ROOT, 'tauri/src/schemas');
// 测试文件本身不是「schema 定义」，不参与被审计集合；但**仍作为消费方**计入
// （测试引用某个 schema 说明它被使用）。
const isTestFile = (p) => /[\\/]__tests__[\\/]/.test(p) || /\.(spec|test)\.[cm]?[jt]sx?$/.test(p);
const feSchemaFiles = feFiles.filter((f) => f.startsWith(feSchemasRoot) && !isTestFile(f));

const feExports = new Map(); // file -> Set(name)
for (const f of feSchemaFiles) {
  const src = readFileSync(f, 'utf8');
  const names = new Set();
  const re = /export\s+(?:type|interface|const|function|enum|class)\s+([A-Za-z0-9_]+)/g;
  let m;
  while ((m = re.exec(src))) names.add(m[1]);
  feExports.set(f, names);
}

// import 来源分析
// 说明：**不能只看 `import`**——`export * from './x'` / `export { a } from './x'` 同样是引用，
// 且 schemas 目录内部的相对导入（vdfs.ts → ./vdfs-form）也必须计入，
// 否则「统一出口再导出一份」的模块会被误报成死文件。
const stripTs = (p) => p.replace(/\.ts$/, '');
const feSchemaByModule = new Map(); // 'vdfs-form' -> 绝对路径
for (const sf of feSchemaFiles) {
  feSchemaByModule.set(relative(feSchemasRoot, sf).split(sep).join('/').replace(/\.ts$/, ''), sf);
}

const feImporters = new Map(); // schemaFile -> Set(importerFile)
const feDeadExportUse = new Map(); // file -> Set(name) 实际被引用的导出名
for (const f of feFiles) {
  const src = readFileSync(f, 'utf8');
  const re = /from\s+['"]([^'"]+)['"]/g;
  let m;
  while ((m = re.exec(src))) {
    const spec = m[1];
    let target = null;
    if (spec.startsWith('.')) {
      const base = stripTs(join(dirname(f), spec));
      target = feSchemaFiles.find((sf) => stripTs(sf) === base || stripTs(sf) === join(base, 'index')) ?? null;
    } else {
      const i = spec.indexOf('schemas/');
      if (i !== -1) target = feSchemaByModule.get(spec.slice(i + 'schemas/'.length).replace(/\.ts$/, '')) ?? null;
    }
    if (target && target !== f) {
      if (!feImporters.has(target)) feImporters.set(target, new Set());
      feImporters.get(target).add(f);
    }
  }
  // 标识符出现（供死导出判断，排除定义文件）
  for (const [defFile, names] of feExports) {
    if (defFile === f) continue;
    for (const n of names) {
      if (new RegExp(`\\b${n}\\b`).test(src)) {
        if (!feDeadExportUse.has(defFile)) feDeadExportUse.set(defFile, new Set());
        feDeadExportUse.get(defFile).add(n);
      }
    }
  }
}

console.log('\n==================== 前端 schemas 审计 ====================\n');
console.log('--- 文件级：消费方 ---');
for (const f of feSchemaFiles.sort()) {
  const cons = [...(feImporters.get(f) ?? [])].map(rel);
  const mod = relative(feSchemasRoot, f).split(sep).join('/');
  const status = cons.length === 0 ? '【死文件：无引用】' : cons.length === 1 ? '【单消费方】' : `【${cons.length} 个消费方】`;
  console.log(`  ${mod.padEnd(28)} ${status} ${cons.join(', ')}`);
}

console.log('\n--- 死导出（全库无人用，含定义文件自身） ---');
console.log('    ⚠️ 标注「已承认保留」的**不要删** —— 那是刻意保留的跨栈契约半边；');
console.log('       确需保留则在声明行（或紧邻上一行）写 `// dead-code-allow R-001: 理由`。');
/**
 * 同文件自引用：定义文件自己还在用它 ⇒ **不是**死导出。
 *
 * 后端那一半早就有这条判据（见上文 `selfOcc`），前端这一半原先缺它——于是
 * `OUTCOME_COMPLETED`（定义在 `schemas/vdfs.ts`，也在同一文件里被比较）这类
 * **只在本文件内使用**的导出会被列进「死导出」。那不是保守，是**报告在说假话**：
 * 它会把一个健康的词表常量报成可清理的垃圾，读报告的人（包括本次自查的我）
 * 会顺着去"修"本来没坏的东西。
 *
 * 两个守卫必须说同一句话——`dead-code-audit` 的承认通道注释里就是这么写的，
 * 同一条判据在两个脚本里也不该只在一半生效。
 *
 * 计数含定义行本身，故「只出现一次」= 定义处，无人使用；`> 1` = 文件内部在用。
 */
const feSelfUse = new Map(); // `${file}::${name}` -> 定义文件内出现次数（含定义行）
for (const f of feSchemaFiles) {
  const src = readFileSync(f, 'utf8');
  for (const n of feExports.get(f) ?? []) {
    feSelfUse.set(`${f}::${n}`, (src.match(new RegExp(`\\b${n}\\b`, 'g')) ?? []).length);
  }
}

/**
 * 承认通道（与后端 `waiverOf` 同一条约定、同一句理由）：声明处或紧邻其上一行写
 * `// dead-code-allow R-001: <理由>` ⇒ 已承认保留（理由不可为空）。
 *
 * 为什么前端这一半也需要它：本表列的是「**确实**全库无人用」的导出，其中一部分是
 * 刻意保留的**跨栈契约半边**（如 `home_reload.Request` —— 与后端
 * `symbio_core/schemas/home_reload.rs` 的请求半边对应）。报告若不区分，读者会把它
 * 当成可清理项删掉，而删掉它并不会让任何测试变红——只会让契约少一半。
 */
function feWaiverOf(f, name) {
  const lines = readFileSync(f, 'utf8').split('\n');
  const decl = new RegExp(
    `export\\s+(?:type|interface|const|function|enum|class)\\s+${name}\\b`,
  );
  for (let i = 0; i < lines.length; i += 1) {
    if (!decl.test(lines[i])) continue;
    for (const j of [i, i - 1]) {
      if (j < 0) continue;
      const m = lines[j].match(/\/\/\s*dead-code-allow\s+R-\d+\s*:\s*(.+?)\s*$/);
      if (m) return m[1];
    }
  }
  return null;
}

let any = false;
for (const f of feSchemaFiles.sort()) {
  const used = feDeadExportUse.get(f) ?? new Set();
  const dead = [...(feExports.get(f) ?? [])]
    .filter((n) => !used.has(n) && (feSelfUse.get(`${f}::${n}`) ?? 1) <= 1)
    .map((n) => {
      const w = feWaiverOf(f, n);
      return w ? `${n}（已承认保留：${w}）` : n;
    });
  if (dead.length) {
    any = true;
    console.log(`  ${relative(feSchemasRoot, f).split(sep).join('/')}: ${dead.join(', ')}`);
  }
}
if (!any) console.log('  （无）');
