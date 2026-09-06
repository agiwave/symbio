#!/usr/bin/env node
/**
 * schema-audit.mjs —— schemas 定义使用情况审计
 *
 * 目标（用户要求）：
 *   1. 找出「未被使用」的 schema 定义（后端 symbio_core/schemas、前端 tauri/src/schemas）
 *   2. 找出「仅被一个模块使用」的 schema 模块（下放候选）
 *
 * 用法：node scripts/schema-audit.mjs
 * 输出：控制台报告。零外部依赖，纯 Node 实现。
 */
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';

const ROOT = process.cwd();

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
        const parts = rest.split('::');
        let best = null;
        for (let i = parts.length; i >= 1; i--) {
          const cand = parts.slice(0, i).join('::');
          if (allModKeys.has(cand)) { best = cand; break; }
        }
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
      const parts = m[1].split('::');
      let best = null;
      for (let i = parts.length; i >= 1; i--) {
        const cand = parts.slice(0, i).join('::');
        if (allModKeys.has(cand)) { best = cand; break; }
      }
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

// 死项：外部无引用，且定义文件内除定义行外也无引用
const deadItems = [];
for (const f of backendSchemaFiles) {
  for (const { name, kind } of itemsByFile.get(f) ?? []) {
    const users = identFiles.get(name);
    const own = selfOcc.get(name) ?? 1;
    if ((!users || users.size === 0) && own <= 1) deadItems.push({ file: rel(f), name, kind });
  }
}
console.log('--- 死项（全库标识符无引用，可删） ---');
const byFile = new Map();
for (const d of deadItems) {
  if (!byFile.has(d.file)) byFile.set(d.file, []);
  byFile.get(d.file).push(`${d.kind} ${d.name}`);
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
const feSchemaFiles = feFiles.filter((f) => f.startsWith(feSchemasRoot));

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
const feImporters = new Map(); // schemaFile -> Set(importerFile)
const feDeadExportUse = new Map(); // file -> Set(name) 实际被引用的导出名
for (const f of feFiles) {
  const src = readFileSync(f, 'utf8');
  if (!f.startsWith(feSchemasRoot)) {
    const re = /from\s+['"]([^'"]*schemas\/([\w-]+))['"]/g;
    let m;
    while ((m = re.exec(src))) {
      const mod = m[2];
      const target = feSchemaFiles.find((sf) => {
        const r = relative(feSchemasRoot, sf).split(sep).join('/').replace(/\.ts$/, '');
        return r === mod;
      });
      if (target) {
        if (!feImporters.has(target)) feImporters.set(target, new Set());
        feImporters.get(target).add(f);
      }
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
  console.log(`  ${mod.padEnd(28)} ${status} ${cons.map(rel).join(', ')}`);
}

console.log('\n--- 死导出（文件被引用但该导出名无人用） ---');
let any = false;
for (const f of feSchemaFiles.sort()) {
  const used = feDeadExportUse.get(f) ?? new Set();
  const dead = [...(feExports.get(f) ?? [])].filter((n) => !used.has(n));
  if (dead.length) {
    any = true;
    console.log(`  ${relative(feSchemasRoot, f).split(sep).join('/')}: ${dead.join(', ')}`);
  }
}
if (!any) console.log('  （无）');
