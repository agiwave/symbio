import './_selfrun.mjs';
// T24 委派者（R1-a 地基）：判定该不该开 worker · worker 进展快照 · 停用即退化（J2）
//
// ## 这条用例锁什么
//
// 主会话目标形态是**不持有工具**（`docs/plan/06` §10.2）。那它缺三样外部事实，
// `delegate` 插件给的就是这三样的唯一真源——本用例锁的是「三样都真的可取」：
//
// | 问 | 出口 | 本用例判据 | 为什么单测证明不了 |
// |---|---|---|---|
// | 这轮该不该开 worker？ | `delegate/decide` | 前缀 / 词边界 / 关键词三态各自命中或不命中 | 配置是**装配期**从 PLUGIN.yml 读的，进程内造不出 |
// | 「进展如何」怎么答？ | `delegate/progress` | 真实落盘布局被扫出、按 `parent` 归属过滤 | 读的是**另一会话**的磁盘布局，要真实会话先存在 |
// | 停用会怎样？ | 两条路由 | 路由不可达而**会话照常** | 停用是装配方行为 |
//
// ## 为什么不真开一个 worker
//
// 真正跑 `agent_run` 会把「委派判定对不对」和「子智能体编排对不对」缠在一起——
// 本用例要锁的只是**判定与快照这两件事实**。因此 worker 会话按**公开落盘布局**
// 造（`<根>/session/<父>/sessions/<子>/`，即 `session` 存储的嵌套路由约定），
// 布局对不对由 `delegate` 单测 + 这条 e2e 共同守住。
import { mkdirSync, writeFileSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  readMessagesJson,
  startLongLivedCli,
  waitFor,
  assert,
  assertEq,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

const MAIN = 'e2e-t24-main';
const OTHER = 'e2e-t24-other';
const WORKER = 'e2e-t24-worker-1';

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
  assertEq(resp.status, 200, `${what} 应受理（${resp.status}: ${JSON.stringify(resp.body)?.slice(0, 200)}）`);
  const d = resp.body?.data;
  assert(d != null, `${what} 应返回 Data 载荷（${JSON.stringify(resp.body)?.slice(0, 300)}）`);
  return d;
}

/** 按公开布局造一个 worker 会话目录（`session.json` = 清单投影，`messages.json` = 转写） */
function writeWorkerSession(homedir, parent, child, title, messages) {
  const dir = join(homedir, 'session', parent, 'sessions', child);
  mkdirSync(dir, { recursive: true });
  writeFileSync(
    join(dir, 'session.json'),
    JSON.stringify({
      id: child,
      title,
      created_at: 1,
      updated_at: 2,
      metadata: { parent_session_id: parent, agent_id: 'e2e-worker' },
      message_count: messages.length,
      summary: null,
      meta_tags: [],
    }),
    'utf8',
  );
  writeFileSync(join(dir, 'messages.json'), JSON.stringify({ messages }), 'utf8');
}

/** 等一轮对话收敛（turn 节点进终态） */
async function waitSettled(homedir, sid, what) {
  await waitFor(
    async () => {
      const m = readMessagesJson(homedir, sid) ?? [];
      return m.some((x) => x.type === 'turn' && ['completed', 'failed'].includes(x.status));
    },
    { what, timeoutMs: 30_000 },
  );
  return readMessagesJson(homedir, sid);
}


export default defineCase(
  'T24 委派者（R1 地基）：前缀/词边界/关键词判定 · worker 快照按归属过滤 · 停用即退化',
  async () => {
    const llm = await new MockLlm([{ id: 'generic', content: '好的。' }]).start();
    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: gatewayConfig(GATEWAY_PORT),
        // 可选插件：目录在 + provider 合格 ⇒ 装配方构造。**不在系统默认清单里**
        // （它给的是事实、不是地基），所以本用例必须显式装——这本身就是「可选」的断言。
        delegate: { enabled: true, keywords: ['重构'] },
      },
    });

    try {
      // ① 先跑一轮真实对话，让主会话目录真的存在于磁盘（快照读的是它）
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '先聊一句',
        provider: PROVIDER_ID,
        session: MAIN,
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);

      // ② 造两个 worker：一个属于 MAIN，一个属于别的会话（串台反例）
      writeWorkerSession(hd.homedir, MAIN, WORKER, '整理 T24 文档', [
        { id: 'u1', role: 'user', type: 'text', content: '把 T24 文档整理一遍', seq: 1, status: 'completed' },
        { id: 'a1', role: 'assistant', type: 'text', content: '正在读文档', seq: 2, status: 'streaming' },
      ]);
      writeWorkerSession(hd.homedir, OTHER, 'e2e-t24-foreign', '别人的活', [
        { id: 'u1', role: 'user', type: 'text', content: '别家的任务', seq: 1, status: 'completed' },
      ]);

      const cli = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: MAIN,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await cli.waitGatewayReady();

        // ③ 判定：显式前缀 ⇒ work
        const d1 = dataOf(await cli.invoke('delegate/decide', { text: '/work 帮我重构这个模块' }), 'decide(前缀)');
        assertEq(d1.dispatch, 'work', '/work 前缀 ⇒ 判定要动手');
        assert(typeof d1.reason === 'string' && d1.reason.length > 0, '判定必须回理由（"为什么没动手"的唯一出口）');

        // ④ 判定：词边界护栏 —— `/worker` 不是 `/work `（这条在跨进程下才算真守住）
        const d2 = dataOf(await cli.invoke('delegate/decide', { text: '/worker 是什么' }), 'decide(词边界)');
        assertEq(d2.dispatch, 'chat', '/worker 不是指令（前缀 /work 带尾空格，不匹配它）');

        // ⑤ 判定：关键词来自 PLUGIN.yml（证明判据是数据，不是硬编码）
        const d3 = dataOf(await cli.invoke('delegate/decide', { text: '帮我把这块重构一下' }), 'decide(关键词)');
        assertEq(d3.dispatch, 'work', '配置里的关键词「重构」应命中');

        // ⑥ 判定：未命中 ⇒ chat（平凡值：默认就是「主会话直接答」）
        const d4 = dataOf(await cli.invoke('delegate/decide', { text: '你好呀' }), 'decide(未命中)');
        assertEq(d4.dispatch, 'chat', '未命中任何判据 ⇒ 主会话直接答');
        assert(typeof d4.reason === 'string' && d4.reason.length > 0, '平凡值也必须说清为什么（不许静默）');

        // ⑦ 进展：全局视图看得到两个 worker，且状态从消息推出
        const g = dataOf(await cli.invoke('delegate/progress', {}), 'progress(全局)');
        assertEq(g.count, 2, `应扫到两个 worker（实得 ${g.count}）`);
        const mine = g.workers.find((w) => w.session_id === WORKER);
        assert(mine, `应含本会话的 worker（实得 ${g.workers.map((w) => w.session_id).join(',')}）`);
        assertEq(mine.state, 'running', '有在途消息 ⇒ 运行中');
        assertEq(mine.rounds, 1, '用户消息条数 = 跑了几个来回');
        assertEq(mine.last_step, '正在读文档', '最新一步取末条助手文本');
        assert(
          g.rendered.includes(WORKER) && g.rendered.includes('运行中'),
          `rendered 应可直接注入提示词（实得: ${g.rendered}）`,
        );

        // ⑧ 进展：按归属过滤 —— 主会话只看得见自己的活
        const scoped = dataOf(await cli.invoke('delegate/progress', { parent: MAIN }), 'progress(按归属)');
        assertEq(scoped.count, 1, `只应报本主会话的 worker（实得 ${scoped.count}）`);
        assertEq(scoped.workers[0].session_id, WORKER, '归属过滤后的那一条');
        assert(!scoped.rendered.includes('别家的活'), '别人的后台任务不该出现在本会话的注入段里');

        // ⑨ 进展：查无此会话 ⇒ 空表 + 空串（是平凡值，不是错误）
        const none = dataOf(await cli.invoke('delegate/progress', { parent: 'no-such-session' }), 'progress(无归属)');
        assertEq(none.count, 0, '查无此会话 ⇒ 0 个 worker');
        assertEq(none.rendered, '', '0 个 worker ⇒ 渲染空串（调用方据此不注入空标题）');

        // ⑩ 双跑逐字节相同（A4）：同一份磁盘两次快照完全一致
        const again = dataOf(await cli.invoke('delegate/progress', {}), 'progress(第二次)');
        assertEq(again.rendered, g.rendered, '同一份磁盘两次快照必须逐字节相同');

        // ⑪ 本插件不参与聊天：判不判开 worker，会话都照常跑
        cli.send('/work 顺手帮我记一笔');
        const msgs = await waitSettled(hd.homedir, MAIN, '带 /work 前缀的一轮对话仍收敛');
        assert(Array.isArray(msgs) && msgs.length > 0, '会话照常落盘（delegate 不改会话行为）');
      } finally {
        cli.stop();
      }

      // ⑫ J2：停用 ⇒ 两条路由不可达，会话与其余系统照常
      const ymlPath = join(hd.homedir, 'delegate', 'PLUGIN.yml');
      writeFileSync(ymlPath, `${readFileSync(ymlPath, 'utf8')}plugin_enabled: false\n`, 'utf8');

      const cli2 = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: OTHER,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await cli2.waitGatewayReady();
        assert((await cli2.invoke('delegate/decide', { text: '/work x' })).status !== 200, '停用后 delegate/decide 应不可达');
        assert((await cli2.invoke('delegate/progress', {})).status !== 200, '停用后 delegate/progress 应不可达');

        // 反证：其余系统照常
        assertEq((await cli2.invoke('vdfs/root', {})).status, 200, '停用 delegate 不应影响 vdfs');
        cli2.send('随便说一句');
        const msgs2 = await waitSettled(hd.homedir, OTHER, '停用 delegate 后会话仍收敛');
        assert(Array.isArray(msgs2) && msgs2.length > 0, '停用 delegate 后会话仍应落盘');
      } finally {
        cli2.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);

