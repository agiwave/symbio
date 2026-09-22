// e2e 基座：临时 homedir 夹具、Mock LLM/MCP 进程编排、CLI 运行、会话落盘读取。
//
// 设计：
// - CLI 与后端是**进程内直连**（cli/src/client.rs），单进程即可跑完整链路
//   （路由 → session 插件 → model 插件 → HTTP → mock LLM；工具 → mcp 插件 → stdio → mock MCP）。
// - mock 与 CLI 全部走真实边界：HTTP（SSE）、stdio（JSON-RPC）、磁盘（homedir）。
// - 每个用例独立的临时 homedir + 独立 mock 实例，互不串扰。

import { spawn, spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, mkdirSync, readFileSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
export const E2E_ROOT = resolve(HERE, '..');
// ⚠️ 不是 `cli/target/`：`cli/.cargo/config.toml` 把 `target-dir` 指到
// `../symbio/target`（刻意共享 symbio 已预热的依赖缓存），所以 `cli/target/`
// **永远不存在**——按它找会让全部用例以「退出码 -1、stderr 为空」失败，
// 且看不出是路径问题。与 `scripts/gate.d/_shared.mjs::cliBinaryPath` 同源。
export const CLI_EXE = process.env.E2E_CLI_EXE || join(
  E2E_ROOT, 'symbio', 'target', 'release',
  `symbio-cli${process.platform === 'win32' ? '.exe' : ''}`,
);
export const MOCK_LLM = join(HERE, 'mock-llm.mjs');
export const MOCK_MCP = join(HERE, 'mock-mcp.mjs');

// ---------- 端口分配（避让：18080 起，逐用例递增） ----------
let portCursor = 18080;
export function nextPort() {
  return ++portCursor;
}

// ---------- 进程等待辅助 ----------
function waitFor(fn, { timeoutMs = 15_000, what = 'condition' } = {}) {
  const deadline = Date.now() + timeoutMs;
  return new Promise((res, rej) => {
    const tick = async () => {
      try {
        if (await fn()) return res();
      } catch {
        /* 重试 */
      }
      if (Date.now() > deadline) return rej(new Error(`等待 ${what} 超时（${timeoutMs}ms）`));
      setTimeout(tick, 100);
    };
    void tick();
  });
}
export { waitFor };

async function fetchJson(url, opts = {}) {
  const r = await fetch(url, opts);
  return { status: r.status, body: await r.json().catch(() => null) };
}

// ---------- Mock LLM 进程 ----------
export class MockLlm {
  constructor(scenarios, { port = nextPort(), name = 'mock' } = {}) {
    this.port = port;
    this.name = name;
    this.base = `http://127.0.0.1:${port}`;
    this.scenarioPath = join(mkdtempSync(join(tmpdir(), 'symbio-e2e-llm-')), 'scenarios.json');
    this.plan = { scenarios: scenarios ?? [], fallback: null };
    writeFileSync(this.scenarioPath, JSON.stringify(this.plan));
    this.proc = null;
  }

  async start() {
    this.proc = spawn(process.execPath, [MOCK_LLM, '--port', String(this.port), '--scenarios', this.scenarioPath], {
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    this.proc.stderr.on('data', (d) => process.env.E2E_DEBUG && process.stderr.write(`[mock-llm:${this.name}] ${d}`));
    await waitFor(async () => {
      const r = await fetch(`${this.base}/_health`);
      return r.ok;
    }, { what: `mock-llm(${this.name}) 启动` });
    return this;
  }

  /** 更换场景脚本（用例运行中重新编排）。 */
  setScenarios(scenarios, fallback = null) {
    this.plan = { scenarios, fallback };
    writeFileSync(this.scenarioPath, JSON.stringify(this.plan));
  }

  async requests() {
    const { body } = await fetchJson(`${this.base}/_requests`);
    return body?.requests ?? [];
  }

  async reset() {
    await fetchJson(`${this.base}/_reset`, { method: 'POST' });
  }

  stop() {
    this.proc?.kill();
  }
}

// ---------- 临时 homedir 夹具 ----------
export function makeHomedir({ providers, mcpServers, sessionConfig, pluginConfigs } = {}) {
  const homedir = mkdtempSync(join(tmpdir(), 'symbio-e2e-home-'));
  const workdir = join(homedir, 'work');
  mkdirSync(workdir, { recursive: true });

  // model provider：`<homedir>/model/<id>/provider.json`
  for (const p of providers ?? []) {
    const dir = join(homedir, 'model', p.id);
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, 'provider.json'), JSON.stringify(p.config));
  }

  // mcp server：`<homedir>/mcp/<name>/server.json`
  for (const s of mcpServers ?? []) {
    const dir = join(homedir, 'mcp', s.name);
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, 'server.json'), JSON.stringify(s.config));
  }

  // 插件配置：`<homedir>/<插件名>/PLUGIN.yml`（插件根 = 系统根本身，一层目录 = 一个插件）。
  // 身份键 `plugin_provider` 是装配方（composite）的加载判据，由这里自动补上；
  // 它是保留键，插件读配置时会被剥离，不影响业务键。
  for (const [plugin, config] of Object.entries(pluginConfigs ?? {})) {
    const dir = join(homedir, plugin);
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, 'PLUGIN.yml'), yamlStringify({ plugin_provider: plugin, ...config }));
  }

  return { homedir, workdir };
}

/** 简单 YAML 序列化：只支持扁平键值 / 嵌套对象两层 / 布尔数字字符串。 */
function yamlStringify(obj, indent = 0) {
  const pad = '  '.repeat(indent);
  return (
    Object.entries(obj)
      .map(([k, v]) => {
        if (v == null) return `${pad}${k}: null`;
        if (typeof v === 'object' && !Array.isArray(v)) {
          return `${pad}${k}:\n${yamlStringify(v, indent + 1)}`;
        }
        if (typeof v === 'string') {
          // 含特殊字符的字符串加引号，避免 YAML 语法歧义
          return /^[A-Za-z0-9_.:/-]+$/.test(v) ? `${pad}${k}: ${v}` : `${pad}${k}: "${v.replace(/"/g, '\\"')}"`;
        }
        return `${pad}${k}: ${v}`;
      })
      .join('\n') + '\n'
  );
}

/** 在已建好的 homedir 上追加一个 MCP server 条目（供引用 homedir 路径的 args 用）。 */
export function addMcpServer(hd, name, config) {
  const dir = join(hd.homedir, 'mcp', name);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, 'server.json'), JSON.stringify(config));
}

export function cleanupHomedir(hd) {
  try {
    rmSync(hd.homedir, { recursive: true, force: true });
  } catch {
    /* Windows 句柄延迟，留给系统临时目录清理 */
  }
}

// ---------- CLI 运行器 ----------
/**
 * 跑一次非交互 CLI。
 * 返回 { code, stdout, stderr }；stdout = 模型正文，stderr = 进度/工具/错误。
 */
export function runCli({ homedir, workdir, message, provider = null, session = null, mode = 'auto', timeoutMs = 120_000, stdinText = null }) {
  // 二进制缺失要**当场说清楚**：否则表现为 `code = -1` + 空 stderr，
  // 与"CLI 崩了"无法区分，得翻源码才知道是路径写错了。
  if (!existsSync(CLI_EXE)) {
    throw new Error(
      `CLI 二进制不存在：${CLI_EXE}\n` +
        `先构建：node cli/scripts/build-cli.mjs（release 用 cargo build --release）；\n` +
        `或经 E2E_CLI_EXE 指向既有二进制。`,
    )
  }
  const argv = [CLI_EXE, '--homedir', homedir, '--workdir', workdir, '--mode', mode];
  if (message != null) argv.push('-m', message);
  if (provider) argv.push('--provider', provider);
  if (session) argv.push('--session', session);
  if (stdinText != null) argv.push('--repl'); // stdin 喂多轮时强制 REPL

  const t0 = Date.now();
  const r = spawnSync(CLI_EXE, argv.slice(1), {
    encoding: 'utf8',
    timeout: timeoutMs,
    input: stdinText ?? '',
    maxBuffer: 64 * 1024 * 1024,
    env: { ...process.env, E2E_STDIO: '1' },
  });
  return {
    code: r.status ?? -1,
    stdout: r.stdout ?? '',
    stderr: r.stderr ?? '',
    ms: Date.now() - t0,
  };
}

// ---------- 长驻 CLI（REPL）与 gateway HTTP 边界 ----------
const CLI_BUILTIN_GATEWAY_PORT = 9231; // gateway 默认端口（gateway/config.rs Default）

/**
 * 启动一个长驻 REPL CLI 进程。返回句柄：`invoke()`（gateway HTTP 边界）、
 * `send()`（喂一行消息进 REPL）、`waitGatewayReady()`、`stop()`。
 *
 * 为什么是 REPL 而不是 one-shot：one-shot（`-m`）一轮结束进程即退出，
 * 进程内插件树连同 gateway 一起消失——「发送」与「中止」两个入口要同时活着，
 * 只有常驻进程能做到（与 Tauri 前端同构）。
 */
export function startLongLivedCli({ homedir, workdir, session = 'e2e-live', mode = 'auto', provider = null, gatewayPort = CLI_BUILTIN_GATEWAY_PORT }) {
  const argv = [
    '--homedir', homedir, '--workdir', workdir,
    '--mode', mode, '--session', session, '--repl', '--quiet',
  ];
  if (provider) argv.push('--provider', provider);
  const child = spawn(CLI_EXE, argv, {
    stdio: ['pipe', 'pipe', 'pipe'],
    env: { ...process.env, E2E_STDIO: '1' },
  });
  const out = { code: '', stdout: '', stderr: '' };
  child.stdout.on('data', (d) => { out.stdout += d; });
  child.stderr.on('data', (d) => { out.stderr += d; });

  const api = {
    proc: child,
    out,
    session,
    gatewayPort,
    /** REPL 发一轮对话（异步写 stdin，不等待回复——等待用 waitFor 轮询存储/LLM） */
    send(message) {
      child.stdin.write(`${message}\n`);
    },
    /** 等网关入站服务就绪（真实边界探活：GET /api/v1/health） */
    async waitGatewayReady(timeoutMs = 20_000) {
      try {
        await waitFor(
          async () => {
            try {
              const r = await fetch(`http://127.0.0.1:${gatewayPort}/api/v1/health`);
              return r.ok;
            } catch {
              return false;
            }
          },
          { what: 'gateway /api/v1/health', timeoutMs },
        );
      } catch (e) {
        // 超时必带诊断：网关绑定失败/插件装配失败的线索都在 stderr
        throw new Error(`${e.message}\n--- CLI stderr 尾部 ---\n${out.stderr.slice(-1200) || '（空）'}`);
      }
    },
    /** gateway HTTP 边界调用：`{metadata:{path,...}, payload}` —— 与 route_v2 同线格式 */
    async invoke(path, payload, metadata = {}) {
      const r = await fetch(`http://127.0.0.1:${gatewayPort}/api/v1/invoke`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ metadata: { path, ...metadata }, payload }),
      });
      const body = await r.json().catch(() => null);
      return { status: r.status, body };
    },
    stop() {
      try { child.stdin.end(); } catch { /* 已退出 */ }
      try { child.kill(); } catch { /* 已退出 */ }
    },
  };
  child.on('exit', (code) => { out.code = code; });
  return api;
}

// ---------- 会话落盘读取 ----------
export function sessionDir(homedir, sid) {
  return join(homedir, 'session', sid);
}

export function readSessionJson(homedir, sid) {
  const p = join(sessionDir(homedir, sid), 'session.json');
  return existsSync(p) ? JSON.parse(readFileSync(p, 'utf8')) : null;
}

export function readMessagesJson(homedir, sid) {
  const p = join(sessionDir(homedir, sid), 'messages.json');
  return existsSync(p) ? JSON.parse(readFileSync(p, 'utf8')).messages : null;
}

// ---------- 文件辅助 ----------
export function readFileSyncSafe(path) {
  try {
    return readFileSync(path, 'utf8');
  } catch {
    return '';
  }
}

// ---------- 断言辅助 ----------
export function assert(cond, msg) {
  if (!cond) throw new Error(`断言失败: ${msg}`);
}

export function assertEq(actual, expected, msg) {
  const a = typeof actual === 'object' ? JSON.stringify(actual) : String(actual);
  const e = typeof expected === 'object' ? JSON.stringify(expected) : String(expected);
  if (a !== e) throw new Error(`断言失败: ${msg}\n  期望: ${e}\n  实际: ${a}`);
}

// ---------- 用例定义 ----------
/**
 * 用例文件默认导出：`export default defineCase('名称', async () => { ... })`。
 *
 * 用例放在 `e2e/cases/<名字>.mjs`（`_` 前缀为共享材料，不当作用例）：
 * `run-tests.mjs` 与门控（scripts/gate.d/60-e2e.mjs）都按目录**发现式**加载，
 * 新增用例文件即自动纳入两者，无需改任何清单。
 *
 * 同时登记到进程级登记表：用例文件被**直接运行**时，`cases/_selfrun.mjs`
 * （每个用例文件首行引入的副作用 import）据它执行本用例并按结果退出。
 */
export function defineCase(name, fn) {
  const c = { name, fn };
  globalThis.__E2E_CASES = globalThis.__E2E_CASES ?? [];
  globalThis.__E2E_CASES.push(c);
  return c;
}

// ---------- 共享用例材料 ----------
export const PROVIDER_ID = 'mock-prov';

/** 标准 mock provider 配置（openai_chat 协议，指向指定端口的 mock-llm）。 */
export function providerConfig(port, overrides = {}) {
  return {
    id: PROVIDER_ID,
    name: 'Mock Provider',
    provider: 'openai',
    api_base: `http://127.0.0.1:${port}/v1`,
    api_key: 'mock-key',
    model: 'mock-model',
    temperature: 0.1,
    max_tokens: 1024,
    api_protocol: 'openai_chat', // mock 只实现了 chat completions 线格式
    enabled: true,
    ...overrides,
  };
}

export function textOf(m) {
  if (m == null) return '';
  if (typeof m.content === 'string') return m.content;
  if (Array.isArray(m.content)) return m.content.map((p) => p.text ?? '').join('');
  return '';
}

/**
 * 转写不变量（所有用例共享的落盘断言）：
 * - `seq` 严格递增、消息 id 唯一；
 * - 每个 tool_call 必有 role=tool 的结果子节点（「有请求必有响应」）。
 */
export function assertTranscriptInvariants(messages, label) {
  assert(Array.isArray(messages), `${label}: messages.json 缺失`);
  let lastSeq = -Infinity;
  const ids = new Set();
  for (const m of messages) {
    if (m.seq != null) {
      assert(m.seq > lastSeq, `${label}: seq 未严格递增（${lastSeq} → ${m.seq} @ ${m.id}）`);
      lastSeq = m.seq;
    }
    assert(!ids.has(m.id), `${label}: 消息 id 重复 ${m.id}`);
    ids.add(m.id);
  }
  for (const m of messages) {
    if ((m.type ?? m.msg_type) === 'tool_call') {
      const result = messages.find((x) => x.parent_id === m.id && x.role === 'tool');
      assert(result, `${label}: tool_call ${m.name ?? m.id} 没有结果子节点（有请求无响应）`);
    }
  }
}
