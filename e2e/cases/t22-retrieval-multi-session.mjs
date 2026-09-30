import './_selfrun.mjs';
// T22 检索者·多会话召回（B4 / S06 补强）：两个会话各带记忆 → 派生事实合并 →
// `memory.recall` 的窗口必须**按主体（principal）各算各的**。
//
// ## 这条用例锁什么
//
// 检索者喂给投影的事实是**全部会话合并**的：seq 高位按会话字典序编码
// （`retrieval::derive::SESSION_STRIDE = 2^40`，位次从 1 起）。若窗口全局取
// "尾部 8 轮用户事实"，只会罩住位次最大的会话——其余会话的近期轮次全部落窗，
// S06 的"跨会话记住"名存实亡。修复后每个主体从**自己的**用户事实序列取尾部
// 8 轮起点；本用例用两个真实落盘的会话（各 10 轮）把它钉死：
//
// | 性质 | 判据 | 为什么单测证明不了 |
// |---|---|---|
// | **多主体窗口独立** | `recall.value.windows` 恰两行，各行下界 = 自己的位次×2^40 + 本地第 3 轮用户 seq | 投影单测的 seq 是手造的；这里要证明**真实磁盘布局**（真实 messages.json + 追加轮次）派生出的主体身份与高位编码正确 |
// | **跨会话候选并存** | 两个会话的窗口起点都在 `candidate_seqs` 里，各自的窗口外早期轮次都不在 | 同上——且必须跨进程（CLI 真跑派生 → 真跑投影） |
// | **双跑逐字节相同（A4）** | 同一份磁盘连调两次，`recall` 逐字节相同 | 需真实存储 + 真实插件实例 |
//
// ## 为什么追加轮次而不是跑 20 轮真实对话
//
// 窗口 = 8，判定需要**每个会话 > 8 轮**。真实轮次由每个会话的第 1 轮对话提供
// （证明 messages.json 是真跑出来的，不是纯手工造）；其余 9 轮以**真实消息为
// 原型克隆**（id / seq 换新，其余字段照抄）追加进同一文件——派生只读
// `role` / `type` / `seq`（公开约定），克隆不引入第二个 schema。若 schema
// 漂移，派生会读不到或读错，断言会响，不会静默。
import { writeFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  readMessagesJson,
  startLongLivedCli,
  assert,
  assertEq,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

// 与 `retrieval::derive::SESSION_STRIDE` 同值：会话位次 → 全局 seq 的高位步长。
const STRIDE = 2 ** 40;
const SESSION_A = 'e2e-t22-a'; // 字典序在前 ⇒ 位次 1
const SESSION_B = 'e2e-t22-b'; // 字典序在后 ⇒ 位次 2

function gatewayConfig(port) {
  return {
    inbound_enabled: true,
    inbound_protocol: 'http',
    inbound_bind: '127.0.0.1',
    inbound_port: port,
    inbound_token: '',
    inbound_readonly: false,
  };
}

function dataOf(resp, what) {
  assertEq(resp.status, 200, `${what} 应受理（${resp.status}）`);
  const d = resp.body?.data;
  assert(d != null, `${what} 应返回 Data 载荷（${JSON.stringify(resp.body)?.slice(0, 300)}）`);
  return d;
}

/** 追加 9 轮（user + assistant）到会话的 messages.json：以真实消息为原型克隆。 */
function padSessionTurns(homedir, sid, extraTurns = 9) {
  const path = join(homedir, 'session', sid, 'messages.json');
  const msgs = readMessagesJson(homedir, sid);
  assert(Array.isArray(msgs) && msgs.length >= 2, `${sid} 应已有真实落盘消息`);
  const userProto = msgs.find((m) => m.role === 'user');
  const asstProto = msgs.find((m) => m.role === 'assistant');
  assert(userProto != null && asstProto != null, `${sid} 首轮应含 user/assistant 两条真实消息`);
  let nextSeq = Math.max(...msgs.map((m) => m.seq ?? 0)) + 1;
  for (let i = 0; i < extraTurns; i++) {
    msgs.push({ ...userProto, id: `${sid}-pad-u${i}`, seq: nextSeq++ });
    msgs.push({ ...asstProto, id: `${sid}-pad-a${i}`, seq: nextSeq++ });
  }
  writeFileSync(path, JSON.stringify({ messages: msgs }, null, 2));
  // 磁盘即真相：断言用的本地用户 seq 序列从文件回读
  const userSeqs = readMessagesJson(homedir, sid)
    .filter((m) => m.role === 'user')
    .map((m) => m.seq)
    .sort((x, y) => x - y);
  assertEq(userSeqs.length, extraTurns + 1, `${sid} 应有 ${extraTurns + 1} 轮用户消息`);
  return userSeqs;
}

/** 写会话记忆（2 条非空行 ⇒ 1 条 memory.encoded + 2 条 memory.recalled）。 */
function writeMemory(homedir, sid) {
  const dir = join(homedir, 'session', sid);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, 'MEMORY.md'), `# ${sid} 记忆\n\n钉住的结论：跨会话召回\n约束：窗口按主体分组\n`);
}

/** 窗口下界（本地 seq）：尾部 8 轮用户事实的起点——投影规则的独立验算。 */
function windowFromLocal(userSeqs) {
  const W = 8;
  return userSeqs.length > W ? userSeqs[userSeqs.length - W] : 0;
}

export default defineCase(
  'T22 检索者·多会话召回：窗口按主体分组 · 跨会话候选并存 · 双跑一致',
  async () => {
    const llm = await new MockLlm([{ id: 'generic', content: '好的。' }]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: gatewayConfig(GATEWAY_PORT),
        retrieval: { enabled: true },
      },
    });

    try {
      // ① 每个会话跑一轮真实对话 → messages.json 真实落盘
      for (const sid of [SESSION_A, SESSION_B]) {
        const r = runCli({
          homedir: hd.homedir,
          workdir: hd.workdir,
          message: `我是 ${sid}`,
          provider: PROVIDER_ID,
          session: sid,
        });
        assertEq(r.code, 0, `${sid} CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
      }

      // ② 各补到 10 轮（> 窗口 8），写会话记忆
      const userSeqsA = padSessionTurns(hd.homedir, SESSION_A);
      const userSeqsB = padSessionTurns(hd.homedir, SESSION_B);
      writeMemory(hd.homedir, SESSION_A);
      writeMemory(hd.homedir, SESSION_B);

      // 预期：位次按会话 id 字典序（a→1，b→2），全局 seq = 位次×2^40 + 本地 seq
      const winA = STRIDE + windowFromLocal(userSeqsA);
      const winB = 2 * STRIDE + windowFromLocal(userSeqsB);
      assert(winA < winB, '位次编码应使 B 的窗口下界严格大于 A');

      // ③ 起长驻 CLI（带 gateway）→ 检索者在场
      const cli = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: 'unused-t22',
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await cli.waitGatewayReady();
        const d = dataOf(await cli.invoke('retrieval/list', {}), 'retrieval/list');

        // B1：两个会话的事实都被派生，主体身份正确
        const facts = d.facts;
        assert(Array.isArray(facts), `facts 应是数组（实得 ${typeof facts}）`);
        const principals = [...new Set(facts.map((f) => f.principal))].sort();
        assertEq(
          JSON.stringify(principals),
          JSON.stringify([SESSION_A, SESSION_B]),
          `派生事实的主体应恰为两个会话（实得 ${principals.join(',')}）`,
        );
        for (let i = 1; i < facts.length; i++) {
          assert(facts[i].seq > facts[i - 1].seq, `合并后 seq 应严格递增（${facts[i - 1].seq}→${facts[i].seq}）`);
        }
        for (const sid of [SESSION_A, SESSION_B]) {
          const mem = facts.filter((f) => f.principal === sid && f.kind.startsWith('memory.'));
          assert(
            mem.some((f) => f.kind === 'memory.encoded') && mem.some((f) => f.kind === 'memory.recalled'),
            `${sid} 应有 memory.encoded 与 memory.recalled`,
          );
        }

        // B2：投影非平凡，窗口**按主体分组**
        assertEq(d.degraded, false, 'memory.recall 已登记 ⇒ 不应降级');
        assertEq(d.recall.trivial, false, '两个会话都有 memory.* ⇒ 非平凡值');
        const windows = d.recall.value.windows;
        assert(Array.isArray(windows), `windows 应是数组（实得 ${JSON.stringify(d.recall.value).slice(0, 200)}）`);
        assertEq(windows.length, 2, '两个主体 ⇒ 恰两行窗口');
        assertEq(windows[0].principal, SESSION_A, '窗口按主体字典序（a 在前）');
        assertEq(windows[0].window_from_seq, winA, `A 的窗口下界 = 位次×2^40 + 本地第 3 轮用户 seq`);
        assertEq(windows[1].principal, SESSION_B, '窗口按主体字典序（b 在后）');
        assertEq(windows[1].window_from_seq, winB, 'B 的窗口下界独立计算');
        // 兼容标量 = 各主体下界的最小值
        assertEq(d.recall.value.window_from_seq, winA, '兼容标量 window_from_seq = 各主体下界最小值');

        // 跨会话召回的核心断言：两个会话的窗口内事实**都在候选里**
        const seqs = new Set(d.recall.value.candidate_seqs);
        assert(seqs.has(winA), `A 的窗口起点应在候选中（跨会话召回成立）`);
        assert(seqs.has(winB), `B 的窗口起点应在候选中（全局窗口实现下 A 会整体落窗）`);
        // 窗口仍然过滤：各自首轮（窗口外）不在候选里
        assert(!seqs.has(STRIDE + userSeqsA[0]), 'A 首轮在窗口外，不应在候选中');
        assert(!seqs.has(2 * STRIDE + userSeqsB[0]), 'B 首轮在窗口外，不应在候选中');
        // 每个主体的窗口内非记忆候选 ≥ 8 轮 × 2 条
        for (const sid of [SESSION_A, SESSION_B]) {
          const n = facts.filter(
            (f) => f.principal === sid && seqs.has(f.seq) && !f.kind.startsWith('memory.'),
          ).length;
          assert(n >= 16, `${sid} 窗口内非记忆候选应 ≥16（实得 ${n}）`);
        }

        // A4：双跑逐字节相同
        const d2 = dataOf(await cli.invoke('retrieval/list', {}), 'retrieval/list(第二次)');
        assertEq(
          JSON.stringify(d2.recall),
          JSON.stringify(d.recall),
          '同一份磁盘两次召回必须逐字节相同（A4）',
        );
      } finally {
        cli.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
