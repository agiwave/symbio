import './_selfrun.mjs';
// T16 实时面的顺序锚点必须与存储一致（多轮）。
//
// ## 钉的是哪条不变量
//
// `ChatMessage.seq` 是**唯一权威顺序锚点**（ADR-025：顺序是节点属性，不是投递属性），
// 而它**只由存储在写入时分配**。因此有一条硬约束（见
// `symbio/src/plugins/session/docs/vdfs-session-messages.md` §3.4）：
//
// > 每一条被落库的消息都必须发一次变更（带存储分配的 `seq`）。
//
// 漏一条，那条消息在前端就**永远只有在途号**（`INFLIGHT_SEQ_BASE = 1 << 50`）。
// 前端按 `seq ?? timestamp` 排序，于是同一棵树里并存两套号：
//
// - 用户消息带**存储号**（「第几条消息」量级，小）；
// - 漏掉的助手消息带**在途号**（≈1.1e15，大）。
//
// 下一条用户消息一落库（存储号仍很小），它就会**排到上一轮助手消息之前**——
// 界面显示成 `user-user-assistant-assistant`，而重开会话（整份回读）又是对的。
// 这正是本用例的判据：**实时面见过的每个节点，其顺序锚点必须等于存储里的那个**。
//
// ## 为什么必须两轮
//
// 单轮里用户消息只有一条、且在最前，错序**看不出来**（在途号虽然大，但前面没有
// 第二条小号消息跟它比）。错位需要「第二条存储号消息」来暴露——两轮是能复现它的
// 最小形状。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
  subscribeSessionRealtime,
  waitFor,
  assert,
  assertEq,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
  readMessagesJson,
  assertTranscriptInvariants,
} from '../helpers.mjs';

const SID = 'e2e-t16';
const TERMINAL = new Set(['completed', 'failed', 'aborted', 'waiting_user_action']);

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

export default defineCase('T16 实时面顺序 = 存储顺序（多轮：每条落库消息都必须交回权威 seq）', async () => {
  const llm = await new MockLlm([
    { id: 'r1', match: '第一轮', content: '第一轮回复' },
    { id: 'r2', match: '第二轮', content: '第二轮回复' },
  ]).start();

  const GATEWAY_PORT = nextPort();
  const hd = makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
    pluginConfigs: { gateway: gatewayConfig(GATEWAY_PORT) },
  });
  const cli = startLongLivedCli({
    homedir: hd.homedir,
    workdir: hd.workdir,
    session: 'unused-t16',
    provider: PROVIDER_ID,
    gatewayPort: GATEWAY_PORT,
  });

  let rt = null;
  try {
    await cli.waitGatewayReady();

    // ① 订阅实时面（**发消息之前**，不漏首帧）
    rt = await subscribeSessionRealtime(cli, { sessionId: SID, gatewayPort: GATEWAY_PORT });

    // ② 会话先建出来（收件箱是会话内部的集合，会话不在就没有它可写）
    const rootResp = await cli.invoke('vdfs/root', {});
    const root = rootResp.body?.data?.path;
    assert(typeof root === 'string' && root.length > 0, 'vdfs/root 应返回根地址');
    const sessionAddr = `${root.replace(/\/+$/, '')}/session/${SID}`;
    const inboxAddr = `${sessionAddr}/inbox`;
    const created = await cli.invoke('vdfs/write', {
      path: sessionAddr,
      create: true,
      text: JSON.stringify({
        metadata: { workdir: hd.workdir, mode: 'auto', risk_level: 'medium' },
      }),
    });
    assertEq(created.status, 200, `创建会话（${JSON.stringify(created.body)?.slice(0, 300)}）`);

    /** 发一条消息（写收件箱 = 入队），并等这一轮**真正跑完** */
    const sendAndSettle = async (text, round) => {
      // 本轮的起始水位：下面「离开 working」必须发生在水位**之后**，否则上一轮的
      // 收尾帧会让条件在开跑前就成立（waitFor 形同虚设，本轮没落库就去读存储）。
      const mark = rt.sessionChanges().length;
      const w = await cli.invoke('vdfs/write', { path: inboxAddr, text });
      assertEq(w.status, 200, `写收件箱（第 ${round} 轮）：${JSON.stringify(w.body)?.slice(0, 300)}`);

      // 已进入终态的助手正文节点（**按 id 去重**）。必须按节点而非帧计数：同一条
      // 消息在一轮里会发多帧（首帧全量 / delta / 状态帧 / **落库回包**），按帧计数
      // 会在同一轮里就凑够「第 N 轮」——于是本轮还没跑完就去读存储，读到上一轮的状态。
      const settledTexts = () => {
        const ids = new Set();
        for (const m of rt.messages()) {
          if (
            m.data?.type === 'text' &&
            m.data?.role === 'assistant' &&
            TERMINAL.has(m.data?.status)
          ) {
            ids.add(m.mid);
          }
        }
        return ids;
      };
      await waitFor(() => settledTexts().size >= round, {
        what: `第 ${round} 轮回复终态（按节点计）`,
        timeoutMs: 30_000,
      });
      // 再等会话离开 working：**必须见过 working**，否则订阅后第一帧空闲态就会
      // 让这个条件立刻成立（收尾帧还没到，会漏掉本轮最后几帧）
      await waitFor(
        () => {
          const sc = rt.sessionChanges().slice(mark);
          return (
            sc.length > 0 &&
            sc.some((c) => c.data?.status === 'working') &&
            sc[sc.length - 1]?.data?.status !== 'working'
          );
        },
        { what: `第 ${round} 轮会话离开 working（本轮实时面的最后一帧）`, timeoutMs: 30_000 },
      );
    };

    await sendAndSettle('第一轮问题', 1);
    await sendAndSettle('第二轮问题', 2);

    // ③ 按**前端口径**把实时帧重建成图：同 id 合并，`seq` / `timestamp` 有则覆盖
    //    （镜像 `tauri/src/stores/sessions.ts::applyTranscriptMessages`）
    const live = new Map();
    for (const m of rt.messages()) {
      const d = m.data;
      if (!d) continue;
      if (d.status === 'removed') {
        live.delete(m.mid);
        continue;
      }
      const cur = live.get(m.mid) ?? { id: m.mid };
      if (d.seq != null) cur.seq = d.seq;
      if (d.timestamp != null) cur.timestamp = d.timestamp;
      if (d.role != null) cur.role = d.role;
      if (d.type != null) cur.type = d.type;
      live.set(m.mid, cur);
    }

    // ④ 落盘权威版本
    const stored = readMessagesJson(hd.homedir, SID);
    assertTranscriptInvariants(stored, 'T16');
    assertEq(stored.filter((m) => m.role === 'user').length, 2, '落盘应有两条 user 消息');

    // ⑤ 核心断言：实时面见过的每个节点，顺序锚点必须**等于**存储分配的那个。
    //    不等 ⇒ 那条消息在前端只有本地号 / 在途号，与存储号并存两套序号空间。
    const drift = [];
    for (const s of stored) {
      const l = live.get(s.id);
      if (!l) {
        drift.push(`${s.id}: 实时面从未见过（角色 ${s.role}/${s.type}）`);
        continue;
      }
      if (l.seq !== s.seq) {
        drift.push(
          `${s.id}: 实时 seq=${l.seq} ≠ 存储 seq=${s.seq}（角色 ${s.role}/${s.type}）`,
        );
      }
    }
    assert(
      drift.length === 0,
      `实时面的顺序锚点必须与存储一致（每一条落库消息都要交回权威 seq）：\n  ${drift.join('\n  ')}\n` +
        `实时面时间线:\n${rt.changes
          .map(
            (c, i) =>
              `#${i} ${c.path.slice(rt.scope.length + 1) || '<会话>'} ` +
              `[${c.data?.type ?? '-'}/${c.data?.role ?? '-'}/${c.data?.status ?? '-'}] ` +
              `seq=${c.data?.seq ?? '-'}`,
          )
          .join('\n')}`,
    );

    // ⑥ 顺序本身：按 `seq ?? timestamp` 排序后，落盘那些 id 的相对顺序必须与存储逐项相同
    //    （缺了 ⑤ 的号、或号不可比，这里就是 `user-user-assistant-assistant`）
    const storedIds = new Set(stored.map((m) => m.id));
    const liveOrder = [...live.values()]
      .sort((a, b) => (a.seq ?? a.timestamp ?? 0) - (b.seq ?? b.timestamp ?? 0))
      .map((x) => x.id)
      .filter((id) => storedIds.has(id));
    assertEq(
      liveOrder,
      stored.map((m) => m.id),
      '实时面按 seq 排序的顺序必须与存储一致（错位即 user-user-assistant-assistant）',
    );
  } finally {
    rt?.close();
    cli.stop();
    llm.stop();
    cleanupHomedir(hd);
  }
});
