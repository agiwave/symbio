#!/usr/bin/env node
// Mock MCP server（stdio JSON-RPC，MCP 规范握手：initialize → initialized 通知 → tools/list → tools/call）。
//
// 用法（由 symbio mcp 插件按 `<homedir>/mcp/<name>/server.json` spawn）：
//   node e2e/mock-mcp.mjs --script ./mock-actions.json --record ./records/mcp.ndjson
//
// 行为：
// - `initialize` 回规范形状（camelCase protocolVersion —— 对端 types.rs 解析依赖它），
//   serverInfo 含 name/version；脚本可注入 sleepMs（握手延迟）、failInitialize（回 error）。
// - `tools/list` 返回脚本定义的工具清单（默认 2 个：echo / add）。
// - `tools/call` 按脚本编排：固定结果 / 睡眠 / 直接断连（进程退出）/ 回 JSON-RPC 错误。
// - 调用记录追加写到 --record <file>（NDJSON），测试断言用。
//
// 脚本形状：
// {
//   "sleepMs": 0,
//   "failInitialize": false,
//   "tools": [{ "name": "echo", "description": "...", "inputSchema": {...} }],
//   "onCall": {
//     "echo": { "content": "echo-result", "sleepMs": 300 },
//     "add":  { "error": "boom" },
//     "slow": { "disconnect": true }
//   }
// }

import { readFileSync, appendFileSync, mkdirSync } from 'node:fs';
import { dirname } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

// ---------- 参数 ----------
const args = process.argv.slice(2);
function argOf(name, dflt) {
  const i = args.indexOf(`--${name}`);
  return i >= 0 && args[i + 1] ? args[i + 1] : dflt;
}
const SCRIPT_PATH = argOf('script', null);
const RECORD_PATH = argOf('record', null);

let script = { tools: null, onCall: {} };
if (SCRIPT_PATH) {
  try {
    script = JSON.parse(readFileSync(SCRIPT_PATH, 'utf8'));
  } catch (e) {
    // 脚本缺失/损坏不致命：退回默认行为，原因写 stderr（不会污染 JSON-RPC 通道）
    console.error(`[mock-mcp] 加载脚本失败（使用默认行为）: ${e.message}`);
  }
}

const tools = script.tools ?? [
  {
    name: 'echo',
    description: '回显输入文本（mock）',
    inputSchema: { type: 'object', properties: { text: { type: 'string' } }, required: ['text'] },
  },
  {
    name: 'add',
    description: '两个数相加（mock）',
    inputSchema: { type: 'object', properties: { a: { type: 'number' }, b: { type: 'number' } }, required: ['a', 'b'] },
  },
];

function record(entry) {
  if (!RECORD_PATH) return;
  try {
    mkdirSync(dirname(RECORD_PATH), { recursive: true });
    appendFileSync(RECORD_PATH, JSON.stringify({ at: Date.now(), ...entry }) + '\n');
  } catch {
    /* 记录失败不影响服务 */
  }
}

function send(msg) {
  process.stdout.write(JSON.stringify(msg) + '\n');
}

// ---------- 请求处理 ----------
async function handleLine(line) {
  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    return; // 非 JSON 行忽略
  }

  // JSON-RPC：无 id → 通知（不回包）
  if (msg.id === undefined || msg.id === null) {
    record({ kind: 'notification', method: msg.method });
    return;
  }

  const { id, method, params } = msg;

  if (method === 'initialize') {
    if (script.sleepMs) await sleep(script.sleepMs);
    if (script.failInitialize) {
      send({ jsonrpc: '2.0', id, error: { code: -32603, message: 'mock 注入的 initialize 失败' } });
      return;
    }
    send({
      jsonrpc: '2.0',
      id,
      result: {
        protocolVersion: '2025-03-26',
        capabilities: { tools: {} },
        serverInfo: { name: 'mock-mcp', version: '0.1.0' },
      },
    });
    return;
  }

  if (method === 'tools/list') {
    send({ jsonrpc: '2.0', id, result: { tools } });
    return;
  }

  if (method === 'tools/call') {
    const name = params?.name;
    const toolArgs = params?.arguments ?? {};
    record({ kind: 'call', name, args: toolArgs });
    const plan = script.onCall?.[name] ?? {};
    if (plan.sleepMs) await sleep(plan.sleepMs);
    if (plan.disconnect) {
      process.exit(42); // 模拟 server 崩溃，逼出客户端失败路径
      return;
    }
    if (plan.error) {
      send({ jsonrpc: '2.0', id, error: { code: -32000, message: plan.error } });
      return;
    }
    const text = plan.content ?? JSON.stringify(toolArgs);
    send({ jsonrpc: '2.0', id, result: { content: [{ type: 'text', text }], isError: false } });
    return;
  }

  send({ jsonrpc: '2.0', id, error: { code: -32601, message: `mock-mcp: unknown method ${method}` } });
}

// ---------- 主循环：按行读 stdin ----------
let buffer = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', (chunk) => {
  buffer += chunk;
  let idx;
  while ((idx = buffer.indexOf('\n')) >= 0) {
    const line = buffer.slice(0, idx);
    buffer = buffer.slice(idx + 1);
    if (line.trim()) void handleLine(line);
  }
});
process.stdin.on('end', () => process.exit(0));
process.on('SIGHUP', () => process.exit(0));
