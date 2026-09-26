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
import { cliBinaryPath, ensureCliBinary } from '../scripts/cli-binary.mjs';
// gateway WS 的客户端：复用前端依赖（Tauri 侧已装 `ws`，不为测试再引一份）
import WebSocket from '../tauri/node_modules/ws/index.js';

const HERE = dirname(fileURLToPath(import.meta.url));
export const E2E_ROOT = resolve(HERE, '..');

// ---------- 被测二进制：总是最新的那一份 ----------
//
// 路径解析与**新鲜度判定**都在 `scripts/cli-binary.mjs`（与门禁同一个真相）。
//
// 这里曾经是「两个候选路径取第一个存在的」，于是本机那份过期 exe 被一直用下去：
// e2e 断言失败的方式与眼前的源码**直接矛盾**（源码里明明有的字段，运行时是
// undefined），排查方向被带偏到源码上。现在判据是**内容指纹**——产物是否对应当前
// 源码；对不上就重建，绝不静默使用。
//
// `E2E_CLI_EXE` 仍然优先：它是「我要用这一份」的显式声明，故意绕过新鲜度判定
// （外部二进制无法用本仓源码指纹衡量）。用它会失去这层保护，属知情选择。
let resolvedExe = process.env.E2E_CLI_EXE || null;
let ensured = false;

/** 当前被测二进制路径；首次调用时确保它对应当前源码（必要时重建）。 */
export function cliExe() {
  if (resolvedExe) return resolvedExe;
  if (!ensured) {
    // 只在真正要起进程时才付这个成本；指纹一致时这里不启动 cargo。
    ensureCliBinary(E2E_ROOT, { log: (m) => process.stderr.write(`[cli-binary] ${m}\n`) });
    ensured = true;
  }
  resolvedExe = cliBinaryPath(E2E_ROOT);
  return resolvedExe;
}

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
 *
 * `env` 是**追加**给 CLI 的环境变量（如 `SYMBIO_ROUTE_LOG=1` 打开路由留痕），
 * 不是替换——被测系统仍然继承本进程环境。
 */
export function runCli({ homedir, workdir, message, provider = null, session = null, mode = 'auto', timeoutMs = 120_000, stdinText = null, env = null }) {
  // `cliExe()` 会保证拿到的是「对应当前源码」的那一份（指纹不符则先重建）。
  // 二进制仍然缺失要**当场说清楚**：否则表现为 `code = -1` + 空 stderr，
  // 与"CLI 崩了"无法区分，得翻源码才知道是路径写错了。
  const exe = cliExe();
  if (!existsSync(exe)) {
    throw new Error(
      `CLI 二进制不存在：${exe}\n` +
        `构建：node scripts/cli-binary.mjs（等价于 cd cli && cargo build --release）；\n` +
        `或经 E2E_CLI_EXE 指向既有二进制（会绕过新鲜度判定）。`,
    )
  }
  const argv = [exe, '--homedir', homedir, '--workdir', workdir, '--mode', mode];
  if (message != null) argv.push('-m', message);
  if (provider) argv.push('--provider', provider);
  if (session) argv.push('--session', session);
  if (stdinText != null) argv.push('--repl'); // stdin 喂多轮时强制 REPL

  const t0 = Date.now();
  const r = spawnSync(exe, argv.slice(1), {
    encoding: 'utf8',
    timeout: timeoutMs,
    input: stdinText ?? '',
    maxBuffer: 64 * 1024 * 1024,
    env: { ...process.env, E2E_STDIO: '1', ...env },
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
  const child = spawn(cliExe(), argv, {
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

// ---------- 会话实时面（与 Tauri 前端同构） ----------

/** 会话下的**消息集合段名**（后端 `plugins/session` 的 `SEG_MESSAGES`）。 */
export const SEG_MESSAGES = 'message';

/**
 * 在 gateway WS 边界上订阅**会话实时面**——与 Tauri 前端逐字同构的两步：
 *
 * 1. `event_bus/subscribe`（WS 首帧）→ 拿到总线广播口，收 `kind = "vdfs"` 的帧；
 * 2. `vdfs/watch`（HTTP）→ 把 sink 登记进 provider 的变更表（**谁来 publish**）。
 *
 * ## 为什么两步缺一不可（这里踩过，且症状极具误导性）
 *
 * 事件总线只是**广播口**：`EventBus::try_publish(KIND_VDFS, …)` 只投给已注册的
 * 订阅者，而"有没有变更要投"取决于 provider 的 `VdfsChangeSubscriptions` —— sink 由
 * `vdfs/watch` 登记。**只 subscribe 不 watch，一条变更都收不到**：不报错、不断连、
 * 帧数为零，看起来就像"模型没有产出内容"。判据：订阅后跑一轮对话，若一条
 * `vdfs` 帧都没有，就是这里少了一半。
 *
 * 与 CLI（`cli/src/client.rs`）同款：watch 的是**父目录** `<根>/session`，不是
 * `<根>/session/<sid>`——订阅一次覆盖全部会话，切会话不必重连；且会话尚不存在时
 * 父目录一定在（`<根>/session/<sid>` 可能还没被创建）。归属由客户端按作用域过滤。
 *
 * @returns 实时面句柄（`changes` 按**到达顺序**，`close()` 收连接）
 */
export async function subscribeSessionRealtime(cli, { sessionId, gatewayPort }) {
  // ① 根地址锚点（前端 `main.ts` 启动期 `ensureVdfsRoot` 同款引导）
  const rootResp = await cli.invoke('vdfs/root', {});
  const root = rootResp.body?.data?.path;
  assert(
    typeof root === 'string' && root.length > 0,
    `vdfs/root 应返回根地址（${JSON.stringify(rootResp.body)?.slice(0, 200)}）`,
  );
  const rootAddr = root.replace(/\/+$/, '');
  const watchPath = `${rootAddr}/session`;
  /** 本次观测的作用域（订阅的父目录的**子集**） */
  const scope = `${watchPath}/${sessionId}`;

  // ② 总线广播口（WS：首帧即订阅请求，此后这条连接只承载该频道的帧）
  const ws = new WebSocket(`ws://127.0.0.1:${gatewayPort}/api/v1/ws`);
  await new Promise((res, rej) => {
    ws.once('open', res);
    ws.once('error', rej);
  });
  ws.send(JSON.stringify({ metadata: { path: 'event_bus/subscribe' }, payload: {} }));

  /** 作用域内的变更（`{path, data?}`，按到达顺序） */
  const changes = [];
  /** 作用域外的路径（诊断：订阅接错时它是唯一线索） */
  const outOfScope = [];
  ws.on('message', (data) => {
    try {
      const frame = JSON.parse(data.toString());
      // 信封：`PluginFrame::Data(build_envelope(kind, sid, data))`
      const env = frame?.Data?.type === 'bus_event' ? frame.Data.data : null;
      if (!env || env.kind !== 'vdfs') return;
      const change = env.data; // `{ path, data? }`
      if (!change || typeof change.path !== 'string') return;
      if (change.path !== scope && !change.path.startsWith(`${scope}/`)) {
        outOfScope.push(change.path);
        return;
      }
      changes.push(change);
    } catch {
      /* 忽略非 JSON 帧 */
    }
  });

  // ③ 登记 sink（缺这一步 = 零变更，见上面文档）
  const watched = await cli.invoke('vdfs/watch', { path: watchPath });
  assert(
    watched.status === 200,
    `vdfs/watch 应成功（${watched.status}: ${JSON.stringify(watched.body)?.slice(0, 200)}）`,
  );

  /** 会话节点自身的变更（`path` = 作用域根） */
  const sessionChanges = () => changes.filter((c) => c.path === scope);
  /** 消息节点变更 → `{ mid, data, arrival }`；`arrival` = 到达序号（**顺序的唯一来源**） */
  const messages = () =>
    changes
      .map((c, arrival) => ({ c, arrival }))
      .filter(({ c }) => {
        const rel = c.path.slice(scope.length + 1).split('/');
        return rel.length === 2 && rel[0] === SEG_MESSAGES && rel[1].length > 0;
      })
      .map(({ c, arrival }) => ({
        mid: c.path.slice(scope.length + 1).split('/')[1],
        data: c.data ?? null,
        arrival,
      }));
  /** 某节点在流上的全部帧（按到达顺序） */
  const framesOf = (mid) => messages().filter((m) => m.mid === mid);

  return {
    scope,
    watchPath,
    changes,
    outOfScope,
    messages,
    framesOf,
    sessionChanges,
    /** 会话节点最后一次视图（离开 working 的那一帧就是本轮实时面的最后一帧） */
    sessionNode() {
      const all = sessionChanges();
      return all.length ? (all[all.length - 1].data ?? null) : null;
    },
    close() {
      try { ws.close(); } catch { /* 已关闭 */ }
    },
  };
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

/**
 * 子智能体空间里的会话目录：`<homedir>/agent/<agentId>/session/<sid>`。
 *
 * 子 Agent 目录是**一棵与系统树同构的 composite 子树**（根 = `<homedir>/agent/<id>`），
 * 子树里的插件目录因此是 `<根>/<插件名>`——子会话与父会话**不是同一个 store**，
 * 这是「子智能体在自己的空间里跑完整会话」的落盘判据。
 */
export function agentSessionDir(homedir, agentId, sid) {
  return join(homedir, 'agent', agentId, 'session', sid);
}

/** 读子智能体空间里某会话的消息（文件不存在 ⇒ `null`，与 `readMessagesJson` 同形） */
export function readAgentMessagesJson(homedir, agentId, sid) {
  const p = join(agentSessionDir(homedir, agentId, sid), 'messages.json');
  return existsSync(p) ? JSON.parse(readFileSync(p, 'utf8')).messages : null;
}

/**
 * 造一个 v2 子智能体目录：`<homedir>/agent/<id>/{manifest.yaml,AGENTS.md,model/<provider>/provider.json}`。
 *
 * 三件东西各自有主，缺一不可：
 * - `manifest.yaml` 声明 `agent-dir/v2`（不合规即**拒绝接入**，§10）；
 * - `AGENTS.md` 是**这个智能体自己**的指令层（子树里 `agent` 实例的宿主目录
 *   就是 agent 目录，因此它会给本空间的会话注入这一段）；
 * - `model/<id>/provider.json`——子树的 `model` 插件读**自己的目录**，
 *   「子智能体有自己的模型服务」因此不是声明，而是必须落在这个包里的事实。
 */
export function makeAgentDir(homedir, { id, name = id, persona = '', providerId, providerPort }) {
  const dir = join(homedir, 'agent', id);
  mkdirSync(dir, { recursive: true });
  writeFileSync(
    join(dir, 'manifest.yaml'),
    `spec: "agent-dir/v2"\nid: "${id}"\nname: "${name}"\nversion: "1.0.0"\nrequires:\n  spec: "^2"\n`,
  );
  writeFileSync(join(dir, 'AGENTS.md'), persona);
  const pdir = join(dir, 'model', providerId);
  mkdirSync(pdir, { recursive: true });
  writeFileSync(join(pdir, 'provider.json'), JSON.stringify(providerConfig(providerPort)));
  return dir;
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

/**
 * 在途号起点（镜像 `symbio/src/plugins/session/transcript.rs::INFLIGHT_SEQ_BASE`）。
 *
 * 号分两段：**在途号**（转写在首次见到节点时分配，起点 1 << 50）与**权威号**
 * （存储在写入时分配，"第几条消息"量级）。在途号永不落库。
 */
export const INFLIGHT_SEQ_BASE = 1 << 50;

/**
 * 断言一个节点流上的 `seq` 是**节点属性**、不是投递序号。
 *
 * `seq` 只在存储写入时分配（见
 * `symbio/src/plugins/session/docs/vdfs-session-messages.md` §3.4），而节点是先以
 * **在途号**上线的；落库后由一条带存储号的完整帧把它交回权威号——这是**唯一**的
 * 换号时机。因此一条节点流的合法形态只有：
 *
 * ```text
 * 在途号…（若干帧）→ 权威号（其后全部帧）
 * ```
 *
 * 断言拦的是四类错误，它们各自对应一种真实事故：
 * - **逐帧递增**：那是投递序号（S27 已随 `session/stream` 退役），顺序会退化成
 *   到达属性——两条通道 / 乱序合并就要靠补丁纠正；
 * - **权威号之间来回跳**：同一条消息以两种身份排在列表的两个位置；
 * - **换号后又回到在途号**：某条帧把在途号当成了权威值发出去（前端会把它排到
 *   全部历史之后）；
 * - **多个权威号**：存储侧重复分配（`seq` 严格递增那条不变量已破）。
 *
 * ⚠️ **不要退化成「逐帧必须完全相同」**：助手侧节点在落库回包之前持的正是在途号，
 * 逐帧相同只在上线后从未落库时才成立——那恰恰是 bug（前端永远拿不到权威号，
 * 下一条用户消息会排到它前面，显示成 `user-user-assistant-assistant`）。
 * 端到端回归：`e2e/cases/t16-live-order.mjs`。
 */
export function assertSeqAnchorIsNodeAttribute(frames, label) {
  const seqs = frames.map((f) => f.data?.seq).filter((s) => s != null);
  const authority = seqs.filter((s) => s < INFLIGHT_SEQ_BASE);
  assert(
    new Set(authority).size <= 1,
    `${label}：权威 seq 至多一个（位置不变，变的是正文），实得 ${JSON.stringify(seqs)}`,
  );
  if (authority.length === 0) return;
  const first = seqs.indexOf(authority[0]);
  assert(
    seqs.slice(first).every((s) => s === authority[0]),
    `${label}：交回权威号之后不得再变（换号只有一次：在途 → 存储），实得 ${JSON.stringify(seqs)}`,
  );
  assert(
    seqs.slice(0, first).every((s) => s >= INFLIGHT_SEQ_BASE),
    `${label}：权威号之前只应出现在途号，实得 ${JSON.stringify(seqs)}`,
  );
}
