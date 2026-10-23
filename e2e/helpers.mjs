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
export const MOCK_HTTP = join(HERE, 'mock-http.mjs');

// ---------- 端口分配 ----------
/**
 * 端口游标。基址可由 `E2E_PORT_BASE` 覆盖——门禁**并发**跑用例时
 * （`58-e2e.mjs`）按用例序号注入 `18080 + i*100`，使各用例子进程落在互不
 * 重叠的号段上（每段 100 个，够一个用例的 gateway + mock LLM + mock MCP 用）。
 *
 * 为什么不用「按 pid 派生」：并发跑的进程数（40+）与可用槽位同量级，`pid % N`
 * 的碰撞概率按生日问题算很高，而端口一撞用例就红——**确定性分段**（由调用方
 * 给序号）没有这个问题。
 *
 * 不设 `E2E_PORT_BASE` 时退回 18080：`node e2e/cases/tNN.mjs` 单独跑、或
 * `run-tests.mjs` 串行跑，都只有一个进程在用这段，安全。
 */
let portCursor = Number(process.env.E2E_PORT_BASE) || 18080;
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
  constructor(scenarios, { port = nextPort(), name = 'mock', rateLimit = null } = {}) {
    this.port = port;
    this.name = name;
    this.base = `http://127.0.0.1:${port}`;
    this.scenarioPath = join(mkdtempSync(join(tmpdir(), 'symbio-e2e-llm-')), 'scenarios.json');
    // `rateLimit` 是 plan 级 429 限流语义（账号级配额，见 mock-llm.mjs 文件头），
    // 黑盒：只关心「真 provider 会限流」，不关心被测系统怎么改。
    this.plan = { scenarios: scenarios ?? [], fallback: null, ...(rateLimit ? { rateLimit } : {}) };
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

  async requests() {
    const { body } = await fetchJson(`${this.base}/_requests`);
    return body?.requests ?? [];
  }

  async reset() {
    await fetchJson(`${this.base}/_reset`, { method: 'POST' });
  }

  /**
   * 运行中换场景表（`POST /_plan`）——用例用它表达**服务恢复了**：坏的那段时间
   * 用静态 `status` 注入，恢复时刻换成正常应答场景。
   *
   * 为什么不用 `failTimes` 的名额来定"什么时候好"：名额按**请求体指纹**消耗，而
   * 相邻两次自动压缩很可能压的是同一段前缀（切分点只在 User 边界落刀，边界每
   * 若干轮才前移一次）⇒ 两次压缩共用一个指纹、只烧一个名额。于是"第 N 次逻辑
   * 请求起正常"这条编排在跨轮重发时**对不上次数**（T18 的熔断/重试判据曾因此飘）。
   */
  async setPlan(scenarios, fallback = null) {
    const { status, body } = await fetchJson(`${this.base}/_plan`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ scenarios, fallback }),
    });
    assert(status === 200, `mock-llm /_plan 应受理（${status}：${JSON.stringify(body)?.slice(0, 200)}）`);
    this.plan = { scenarios, fallback };
    return body;
  }

  stop() {
    this.proc?.kill();
  }
}

// ---------- Mock HTTP 端点（web 外取工具的本地靶子） ----------
/**
 * 起一个本地 HTTP 服务当 `web_fetch` / `http_request` 的靶子。
 *
 * ## 为什么需要它（而不是打真外网）
 *
 * 真外网在 CI 上不存在，且内容会变、对方会限流——那三类失败都**不在被测系统里**，
 * 却会被记成产品缺陷。本地靶子把「网络」换成确定性读数：状态码、`content-type`、
 * 正文，以及 `requests()` 里的**请求留痕**（回答「请求真的发出去了没有」）。
 *
 * ## `marker` 每次随机
 *
 * 与 `t64` 同一条思路：标记只在靶子上，模型侧无从得知 ⇒ 「结果里出现它」就是
 * 「真的取到了」。用例断言里的标记就是这里生成的那一个（构造时可显式指定）。
 *
 * @param port   监听端口（默认从 `nextPort()` 取，与 mock-llm 同一套分段）
 * @param slowMs `/slow` 的延迟毫秒数（配 `web_timeout` 逼超时）
 * @param marker 正文里的随机标记；不传则本进程生成
 */
export class MockHttp {
  constructor({ port = nextPort(), slowMs = 3000, marker = null } = {}) {
    this.port = port;
    this.base = `http://127.0.0.1:${port}`;
    this.marker = marker ?? `MOCKHTTP_${Math.random().toString(36).slice(2, 10).toUpperCase()}`;
    this.slowMs = slowMs;
    this.proc = null;
  }

  /** 靶子上的绝对 URL（用例里拼 `mock.url('/page')`）。 */
  url(path) {
    return `${this.base}${path}`;
  }

  async start() {
    this.proc = spawn(
      process.execPath,
      [MOCK_HTTP, '--port', String(this.port), '--slow-ms', String(this.slowMs), '--marker', this.marker],
      { stdio: ['ignore', 'pipe', 'pipe'] },
    );
    this.proc.stderr.on('data', (d) => process.env.E2E_DEBUG && process.stderr.write(`[mock-http] ${d}`));
    await waitFor(async () => {
      const r = await fetch(`${this.base}/_health`);
      return r.ok;
    }, { what: 'mock-http 启动' });
    return this;
  }

  /** 靶子收到过的请求（方法 / 路径 / 正文）。判「请求有没有真的发出去」用。 */
  async requests() {
    const { body } = await fetchJson(`${this.base}/_requests`);
    return body?.requests ?? [];
  }

  stop() {
    this.proc?.kill();
  }
}

// ---------- 临时 homedir 夹具 ----------

/**
 * 「对话面整体关闭」的 session 配置片段。
 *
 * ## 为什么需要它
 *
 * 对话面（`classify` 判决 + `compose` 措辞）**出厂默认开启**——这是产品行为：问候 / 致谢
 * 这类输入不该付任何 LLM 往返，而"这一轮该不该干活"也不该由 worker 每轮自己猜。
 * 但它会改变**每一轮**的请求数（多一次静默分类）与落库节点（多一条根级对话面文本）。
 *
 * 因此主题**不是**对话面的用例必须把它钉死：那些用例验的是流式 / 压缩 / MCP / 装配 /
 * 补充整合等**正交**机制，它们的断言多按"精确请求数"与"精确节点形态"写。不钉的话，
 * 一次对话面调优就会让一批无关用例变红——而那种红没有任何诊断价值。
 *
 * ## 为什么三个键都要给
 *
 * `compose_enabled = false` 已经足够：没有措辞就产不出任何对话面文本——判决判出来的
 * 两个变体与中途汇报都要经它。另外两个键是**冗余但有意**：它们让"本用例与对话面
 * 无关"这句话在夹具里**自明**，而不是依赖"当前恰好是谁在把门"。判决那一侧
 * （`classify_enabled`）与汇报那一侧（`progress_enabled`）各自也是一道闸，其中任一道
 * 被绕过、被重排，或阈值默认值被调小，本片段都不必重审。
 *
 * ## 对话面自己的用例怎么写
 *
 * `t21-classify` / `t22-compose` / `t23-progress-report` **显式**写出各自相关键的取值
 * （含 `true`）——它们验的正是这个功能，不能依赖默认值（默认值翻转不该悄悄改变
 * 它们验的是什么）。
 */
export const DIALOG_FACE_OFF = Object.freeze({
  classify_enabled: false,
  compose_enabled: false,
  progress_enabled: false,
});

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
export function runCli({ homedir, workdir, message, provider = null, session = null, agent = null, mode = 'auto', timeoutMs = 120_000, stdinText = null, env = null }) {
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
  // `--agent <ID>`：绑定智能体 ⇒ 会话 `metadata.agent_id` 落这个值 ⇒
  // `ctx[AGENT_ID]` ⇒ 主体名 `agent:<ID>`（`chat_loop::request_principal` 的取值链）。
  if (agent) argv.push('--agent', agent);
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
export function startLongLivedCli({ homedir, workdir, session = 'e2e-live', mode = 'auto', provider = null, gatewayPort = CLI_BUILTIN_GATEWAY_PORT, headers: baseHeaders = {} }) {
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
    /**
     * 等网关入站服务就绪。
     *
     * 判据是**两条**，缺一条都不算就绪：
     * 1. `GET /api/v1/health` 通（网关进程在监听）；
     * 2. `vdfs/root` 能取到根地址（**插件树能回答 VDFS 路由**）。
     *
     * 第 2 条是必须的：用例的就绪后第一个动作几乎都是取根地址，而"进程在监听"与
     * "树能路由"之间有一段窗口——曾出现 `health` 通过、紧接着 `vdfs/root` 无返回的
     * 偶发（`t15`，392ms 即失败）。就绪判据必须抬到用例真正需要的那**一个能力**上，
     * 否则窗口只是被搬到下一行。真要坏了，这里仍会超时并带上 stderr。
     *
     * `label` 只在失败信息里出现。用例里有多个进程时它是**必需**的：没有它，
     * "网关未就绪"无法区分是哪一个进程——而两个进程失败的原因完全不同
     * （首个进程失败 = 装配坏了；后续进程失败 = 端口被占）。
     */
    async waitGatewayReady(timeoutMs = 20_000, label = `:${gatewayPort}`) {
      try {
        await waitFor(
          async () => {
            try {
              const r = await fetch(`http://127.0.0.1:${gatewayPort}/api/v1/health`);
              if (!r.ok) return false;
            } catch {
              return false;
            }
            try {
              const inv = await api.invoke('vdfs/root', {});
              return inv.status === 200 && typeof inv.body?.data?.path === 'string' && inv.body.data.path.length > 0;
            } catch {
              return false;
            }
          },
          { what: `gateway /api/v1/health + vdfs/root（${label}）`, timeoutMs },
        );
      } catch (e) {
        // 超时必带诊断：网关绑定失败/插件装配失败的线索都在 stderr
        throw new Error(`${e.message}\n--- CLI stderr 尾部 ---\n${out.stderr.slice(-1200) || '（空）'}`);
      }
    },
    /**
     * gateway HTTP 边界调用：`{metadata:{path,...}, payload}` —— 与 route_v2 同线格式。
     *
     * `headers` 追加到基础请求头（鉴权、`Idempotency-Key` 等）；合并顺序让调用点
     * 只写自己关心的那一项，不必复述 `content-type`。
     */
    async invoke(path, payload, metadata = {}, headers = {}) {
      const r = await fetch(`http://127.0.0.1:${gatewayPort}/api/v1/invoke`, {
        method: 'POST',
        headers: { 'content-type': 'application/json', ...baseHeaders, ...headers },
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

/**
 * 会话集合在 VDFS 根下的**顶层段名**（后端 `plugins/session` 的 `SESSION_SEG`）。
 *
 * 与 `SEG_MESSAGES` 同一理由自持：地址是跨端契约，写错表现为「读了个不存在的
 * 地址」而不是编译错误，而 e2e 的断言恰恰要靠地址形状判定行为（收件箱写入
 * 落在 `<根>/<SESSION_SEG>/<sid>/inbox`）。
 */
export const SESSION_SEG = 'session';

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
      // 信封：`PluginFrame::Data(event_bus_build_envelope(kind, sid, data))`
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

/**
 * 经 REPL 发一轮消息，并在**会话实时面**上等这一轮真正跑完。
 *
 * 判据是「会话节点见过 working、且最后一次离开 working」：那一帧是本轮实时面的
 * 最后一帧（后端先清在途、再复位运行态，两者走同一条订阅 FIFO）。水位线 `mark`
 * 不可省——否则订阅后首帧的空闲态会让条件在开跑前就成立（t16 踩过）。
 *
 * 这是「边跑边等、等到了再断言」的统一入口：等的是**实时面**（用户如何看到就
 * 如何等），转写 / 读数断言随后走 `cli.invoke`（`vdfs/read` 等生产路由）。
 * REPL 与一次性 CLI 走同一条收件箱地址面（`ask` 写 `<根>/session/<sid>/inbox`，
 * 见 `cli/src/client.rs`），因此会话节点的运行态帧对这条入口一样成立。
 *
 * @param cli `startLongLivedCli` / `startRealtimeSession` 给出的进程句柄
 * @param sub `subscribeSessionRealtime` 给出的实时面句柄
 * @param message 本轮用户发言（写进 REPL stdin）
 */
export async function sendAndSettle(cli, sub, message, { timeoutMs = 60_000 } = {}) {
  const mark = sub.sessionChanges().length;
  cli.send(message);
  await waitFor(
    () => {
      const sc = sub.sessionChanges().slice(mark);
      return (
        sc.length > 0 &&
        sc.some((c) => c.data?.status === 'working') &&
        sc[sc.length - 1]?.data?.status !== 'working'
      );
    },
    { what: '本轮跑完（实时面离开 working）', timeoutMs },
  );
}

// ---------- 会话读出口（生产路由，不落盘读数） ----------
//
// 转写 / 元数据的读数只走两条**用户可见**的出口（与前端同构）：
// - 消息集合：`vdfs/root` → `vdfs/read(<根>/session/<sid>)` → `JSON.parse(text).messages`
//   （前端 `loadMessages` 走的同一条地址）；
// - 其余列：gateway `invoke` 的其它路由（`session/stats` 等）。
//
// 会话的内部存储文件是**实现细节**，用例不读它：形状一变、缓存策略一改，
// 与被测行为无关的断言就会红，而那种红没有诊断价值。

/**
 * 起一个**长驻读回 CLI** 并等它的 gateway 就绪——一次性轮次跑完后读转写 / 读数的入口。
 *
 * 前置：该 homedir 的 gateway 插件配置必须以**同一个 `gatewayPort`** 开启入站
 * （`inbound_enabled: true`，与 T28/T31 同一约定），否则进程里没有监听口。
 */
export async function startReadbackCli({
  homedir,
  workdir,
  session = 'e2e-readback',
  mode = 'auto',
  provider = PROVIDER_ID,
  gatewayPort = nextPort(),
}) {
  const cli = startLongLivedCli({ homedir, workdir, session, mode, provider, gatewayPort });
  try {
    await cli.waitGatewayReady();
  } catch (e) {
    cli.stop();
    throw e;
  }
  return cli;
}

/**
 * 起读回 CLI 并订阅它的**会话实时面**——「边跑边等、等到了再断言」的用例的统一入口。
 *
 * 这是 e2e 读转写的**正道**（用户如何看到就如何读）：等消息 / 读历史都经这条
 * 句柄——`sub.messages()` 是前端实时面所见，`cli.invoke('vdfs/read', …)` 是前端
 * 加载历史的同一出口。会话尚不存在也能订阅（`vdfs/watch` 挂的是父目录
 * `<根>/session`，归属按作用域过滤）；CLI 以 `--session <session>` 启动，会话
 * 由首条消息 / `vdfs/write` 建立。
 *
 * @returns `{ cli, sub }`；收尾 `sub.close()` + `cli.stop()`（子进程退出仍由用例的
 * `waitFor(!ProcessWrap)` 兜底）
 */
export async function startRealtimeSession({
  homedir,
  workdir,
  session,
  mode = 'auto',
  provider = PROVIDER_ID,
  gatewayPort = nextPort(),
}) {
  const cli = await startReadbackCli({ homedir, workdir, session, mode, provider, gatewayPort });
  try {
    const sub = await subscribeSessionRealtime(cli, { sessionId: session, gatewayPort });
    return { cli, sub };
  } catch (e) {
    cli.stop();
    throw e;
  }
}

/**
 * 按**地址**读一个会话叶子的转写（`{ messages: [...] }`）。
 *
 * 叶子尚不存在（或正文为空）⇒ `null`：与「先跑一轮、再读」的用例前提配合，
 * `assertTranscriptInvariants` 拿到 `null` 会当场报缺。传给 `.some()` 之类的调用
 * 会以 TypeError 失败——同样是失败，不会静默变绿。
 */
export async function readTranscriptAt(cli, addr) {
  const resp = await cli.invoke('vdfs/read', { path: addr });
  if (resp.status !== 200) return null;
  const text = resp.body?.data?.text;
  if (typeof text !== 'string' || text.length === 0) return null;
  const leaf = JSON.parse(text);
  return leaf?.messages ?? null;
}

/**
 * 读某个会话的消息集合（生产读出口版的「转写读数」）。
 *
 * 先取根地址（`vdfs/root`，前端 `ensureVdfsRoot` 同款），再按
 * `<根>/session/<sid>` 读叶子——这条地址就是前端加载历史用的那一条。
 */
export async function readSessionMessages(cli, sid) {
  const rootResp = await cli.invoke('vdfs/root', {});
  const root = rootResp.body?.data?.path;
  assert(
    typeof root === 'string' && root.length > 0,
    `vdfs/root 应返回根地址（${JSON.stringify(rootResp.body)?.slice(0, 200)}）`,
  );
  return readTranscriptAt(cli, `${root.replace(/\/+$/, '')}/${SESSION_SEG}/${sid}`);
}

/**
 * 取 VDFS 根地址（`vdfs/root`——**唯一的「不给地址」入口**）。
 *
 * 根叫什么**归 vdfs 插件**，使用方不写死：问一次，之后一律从父地址往下拼。与前端
 * `ensureVdfsRoot`、CLI 的 `fetch_root_addr` 同款。
 */
export async function vdfsRoot(cli) {
  const rootResp = await cli.invoke('vdfs/root', {});
  const root = rootResp.body?.data?.path;
  assert(
    typeof root === 'string' && root.length > 0,
    `vdfs/root 应返回根地址（${JSON.stringify(rootResp.body)?.slice(0, 200)}）`,
  );
  return root.replace(/\/+$/, '');
}

/** 顶层会话的容器地址 `<根>/session/<sid>`。 */
export async function sessionAddr(cli, sid) {
  return `${await vdfsRoot(cli)}/${SESSION_SEG}/${sid}`;
}

/**
 * **先建会话**（`vdfs/write(<根>/session/<sid>, { create: true, metadata })`）。
 *
 * 收件箱是会话**内部的集合**——会话不在，就没有它可写。曾经这条由
 * `session/chat/send` 悄悄代劳（它 `get_or_create`），于是「先建后写」这件事在
 * 调用方看起来可有可无；走地址面后它显式化了：一次写路径一个事实，会话的存在
 * 由**会话节点的那一次写**负责，消息写入不附带建会话的语义（也不该由它带——
 * 那正是 D-066 清掉的「选项随消息走」的同一处乖违）。
 *
 * 与前端 `createSession`、`register_subsession`、CLI `ensure_session` 同一条形状：
 * 带 `create` 的写是 **upsert**（不存在则就地创建，已存在则浅合并 metadata），
 * 所以「新建」与「改元数据」是同一次调用。
 *
 * @param metadata 写进会话 metadata 的字段（`workdir` / `mode` / `risk_level` / `provider_id`…）
 */
export async function createSession(cli, sid, metadata = {}) {
  const addr = await sessionAddr(cli, sid);
  const resp = await cli.invoke(
    'vdfs/write',
    { path: addr, create: true, text: JSON.stringify({ metadata: { ...metadata } }) },
    { session_id: sid, ...metadata },
  );
  assert(
    resp.status === 200,
    `创建会话 ${sid} 应受理（${resp.status}: ${JSON.stringify(resp.body)?.slice(0, 200)}）`,
  );
  return resp;
}

/**
 * 经 HTTP 边界**发一条用户消息**（收件箱地址面——输入面双轨退役后唯一入口）。
 *
 * `session/chat/send` 已移除（[D-066]）：那条路由的 `message` 分支
 * 只是入队的薄包装、回包恒为 `accepted`，而「接收消息」与「跑一轮」本来就在两处。
 * 现在发消息 = 对空间的一次 `vdfs/write(<根>/session/<sid>/inbox, text)`——
 * 与 T15 写**子智能体空间**收件箱同一条地址语义，机制统一、没有顶层专用分支。
 *
 * 正文写**纯文本**：`parse_inbox_message` 的判别式是「不以 `{` 开头」⇒ 整段就是
 * 消息正文，身份与缺省（`role = user` / `type = text` / `status = completed`）由
 * provider 一侧补齐。
 *
 * @param metadata 透传进 ctx 的调用级字段（`session_id` / `workdir`）
 * @returns 原始回执（`{ status, body }`）；`body.data.name` 是生成的条目名
 */
export async function sendInbox(cli, sid, text, metadata = {}) {
  const addr = `${await sessionAddr(cli, sid)}/inbox`;
  const resp = await cli.invoke(
    'vdfs/write',
    { path: addr, text },
    { session_id: sid, ...metadata },
  );
  return resp;
}

/**
 * 经 HTTP 边界**中止一轮**（`action(<会话地址>, "abort")`）。
 *
 * 曾经的 `session/chat/abort` 是它的 address-less 别名，且本就复用同一个
 * `abort_turn` 实现；双轨退役后只留节点动作这一条。
 * 回执的 `ok` 才是判据（http 层恒 200：会话从未跑过同样返回 200 + `ok: false`）。
 */
export async function abortSession(cli, sid, metadata = {}) {
  const addr = await sessionAddr(cli, sid);
  const resp = await cli.invoke(
    'vdfs/action',
    { path: addr, action: 'abort' },
    { session_id: sid, ...metadata },
  );
  return resp;
}

/**
 * 经 HTTP 边界**回填一次恢复**（`action(<sid>/message/<mid>, <动作词>)`）。
 *
 * 曾经的入口是 `session/chat/send` 的 `resume` 字段（输入面双轨退役）。
 * 落点是**目标消息自身**：恢复不产生新用户消息，它就落在当时那条消息上
 * （删除-重建），而入队会让它排到新消息之后、候选早已被改写。
 *
 * 动作词取 `ResumeAction` 的线上词形（snake_case）：`answer` / `approve` /
 * `reject` / `supply` / `retry` / `retry_subtree` / `retry_compaction`。载荷
 * `args` / `reason` / `answer` 按动作词透传，`mode` / `risk_level` 是**调用级**
 * 运行选项（不同来源各带各的）。
 *
 * 回执的 `ok` 才是判据（http 层恒 200：会话忙碌时同样 200 + `ok: false`）。
 *
 * @param targetId 恢复落点的消息 id（提问所在的 tool_call 节点 id 等稳定锚点）
 */
export async function resumeSession(cli, sid, targetId, action, payload = {}, metadata = {}) {
  const addr = `${await sessionAddr(cli, sid)}/${SEG_MESSAGES}/${targetId}`;
  const resp = await cli.invoke(
    'vdfs/action',
    { path: addr, action, payload },
    { session_id: sid, ...metadata },
  );
  return resp;
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
 * v2 事实网格里的**轮次事实**种类——「这一轮说了什么 / 跑出了什么」。
 *
 * 网格里同时住着两类事件：**轮次事实**（用户格 / 收束格 / 兜底格 / 产物格）与
 * **派生副作用**（`memory.encoded` / `memory.recalled`，S5 步 11–13 与 S11 步 22）。
 * 断言轮次事实时先按本表筛一遍：用「总数等于几」去断言会把两类混在一起——
 * 派生素化多写一条就假红、少写一条又假绿（t26 / t27 各踩过一次）。
 *
 * 与 `symbio/src/plugins/loop/turn_exec.test.rs::turn_facts` 同一口径；两边分开
 * 是因为 e2e 跑在**独立进程**里，够不着 Rust 测试的语料。
 */
export const TURN_FACT_KINDS = new Set([
  'user.message',
  'chat.assistant.final',
  'chat.assistant.fallback',
  'artifact.added',
]);

/** 从事实网格里筛出**轮次事实**（按 `TURN_FACT_KINDS`）——见该常量的文档。 */
export function turnFacts(events) {
  return events.filter((e) => TURN_FACT_KINDS.has(e.kind));
}

/**
 * wire 形态词 → 断言用的**旧类型词**折叠（e2e 侧判据，与前端分派轴无关）。
 *
 * ChatNode 的 `type` 只有五值
 * （body/notice/skeleton/interaction/typing），呈现变体住 `meta.style_tag`。
 * e2e 若继续拿 `m.type === 'tool_call'` 这类**旧词**比较，条件永远不成立——
 * 断言不报错、只是不再命中靶子（静默跳过），这比失败更危险。
 * 所有对消息节点 / 实时帧 `type` 的判据一律经本函数折叠回旧词再比。
 *
 * 前端不认这些旧词（前端只按五值分派）；这里保留旧词是因为**用例断言的就是
 * 后端标注的事实**——`style_tag` 是后端随节点下发的注解，读它正当。
 *
 * 旧词与未知值原样透传（存量 fixture 与未来新词都不会被误判成别的）。
 */
export function msgTypeOf(m) {
  const t = m?.type ?? m?.msg_type ?? undefined;
  const tag = m?.meta?.style_tag;
  if (t === 'skeleton') return tag === 'tool' ? 'tool_call' : 'turn';
  if (t === 'body') return tag === 'reasoning' ? 'reasoning' : 'text';
  // 相应批次：思考事实从 `body + tag` 归位到 `notice`（折叠叶），与压缩同格——
  // 两格共用一个 type，变体只在 style_tag 上分。
  if (t === 'notice') return tag === 'reasoning' ? 'reasoning' : 'compression';
  if (t === 'interaction') return 'user_prompt';
  return t;
}

/**
 * wire 角色的折叠读法（镜像前端 `schemas/chat_node.messageRoleOf` 的判据）。
 *
 * `role` 已退出 wire（`skip_serializing_if`）：实时帧与**新落盘**
 * 的消息都不再带这一格。直接拿 `m.role === 'tool'` 比较的断言会静默失去靶子
 * （147f 的同一课）——一律经本函数折叠再比。
 *
 * 判据与后端 `ChatNode::node_role()` / 前端 `messageRoleOf` 同一条：
 * 残留 `role` 格在先，`principal.kind` 兜底（person→user、agent→assistant、
 * tool→tool、system→system），两格皆无按助手（与前端默认一致）。
 */
const PRINCIPAL_KIND_TO_ROLE = Object.freeze({
  person: 'user',
  agent: 'assistant',
  tool: 'tool',
  system: 'system',
});
export function msgRoleOf(m) {
  if (m?.role != null) return m.role;
  const kind =
    typeof m?.principal === 'object' && m?.principal !== null ? m.principal.kind : undefined;
  return PRINCIPAL_KIND_TO_ROLE[kind] ?? 'assistant';
}

/**
 * 转写不变量（所有用例共享的落盘断言）：
 * - `seq` 严格递增、消息 id 唯一；
 * - 每个 tool_call 必有 role=tool 的结果子节点（「有请求必有响应」）。
 */
export function assertTranscriptInvariants(messages, label) {
  assert(Array.isArray(messages), `${label}: 会话转写（消息集合）读不到`);
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
    if (msgTypeOf(m) === 'tool_call') {
      const result = messages.find((x) => x.parent_id === m.id && msgRoleOf(x) === 'tool');
      assert(result, `${label}: tool_call ${(m.meta?.tool_name ?? m.id)} 没有结果子节点（有请求无响应）`);
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
 * `symbio/src/plugins/session/docs/109-vdfs-session-messages.md` §3.4），而节点是先以
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
