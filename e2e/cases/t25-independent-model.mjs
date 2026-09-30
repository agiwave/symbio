import './_selfrun.mjs';
// T25 对话面调优（S6）：两个插件各自选模型 / 提示词可覆盖 / 缺省与写错都落回会话选定值。
//
// ## 本用例钉的是什么
//
// S6 给两个插件各加了一个 provider id 与一段可覆盖的提示词。**这两件事的判据都在
// 「请求真的发到哪去了」**——插件内部怎么取、怎么降级，从外部看不见；能看见的只有
// mock LLM 收到的那份请求体（`body.model` / `body.temperature` / `body.messages[0]`）。
//
// | 线 | 配置 | 期望 |
// |---|---|---|
// | A 各自独立 | `classify.model=classify-prov`、`compose.model=compose-prov`、会话选 `session-prov` | 分类请求带 `cheap-classifier`、答话生成带 `good-writer`、worker 请求带 `session-model` |
// | A 提示词 | `classify.system_prompt=<本用例那段>` | 两次分类请求的 system 段**逐字等于**它 |
// | B 平凡值 | 两个插件的 `PLUGIN.yml` 里**不写**这两个键 | 全部请求带 `session-model`；system 段里**没有**本用例那段 |
// | C 写错 id | `classify.model=nope-not-exist` | 全部请求带 `session-model`，**而判决照常发生**（用户仍拿到答话） |
//
// ## 为什么温度也在断言里
//
// 「独立温度」没有、也不该有独立的配置字段——温度是 **provider 条目自己的参数**
// （`<根>/model/<id>/provider.json`），换一个条目就是换温度。三个条目给三个不同的
// 温度值，于是"换条目"这件事在请求体里是可观测的：`classify-prov` 是 0.0、
// `compose-prov` 是 0.7、`session-prov` 是 0.1。若哪天有人真给插件加了个 temperature
// 字段，这三条会红——那时该先回答"为什么同一个 provider 要被两处改温度"。
//
// ## 为什么 B 与 C 都要另起 homedir 与进程
//
// 插件的配置在**装配期**读进实例（`Plugin::build` 只跑一次），运行期改
// `PLUGIN.yml` 不生效。因此三条线只能各起一个进程；端口来自各自 homedir 的插件配置
// （`nextPort()`），互不干扰。
//
// ## 为什么三条线用**三句不同的**输入
//
// mock 的场景表按「最后一条 user 消息」的子串匹配，而**分类与生成请求的最后一句话
// 是同一句**（两者读的是同一条对话线）。A 线靠 `once` 把分类那条消耗掉，让生成落到
// 下一条；B / C 若复用 A 的输入，三个进程就会在同一批场景上互相抢匹配。三句不同的
// 输入把三条线彻底隔开——判据是"各线的请求数"，串味会让它直接数错。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
  readMessagesJson,
  nextPort,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  assertTranscriptInvariants,
} from '../helpers.mjs';

/** 三个 provider 条目：**只有 `model` 与 `temperature` 不同**，都指向同一个 mock */
const PROVIDERS = {
  session: { id: 'session-prov', model: 'session-model', temperature: 0.1 },
  classify: { id: 'classify-prov', model: 'cheap-classifier', temperature: 0.0 },
  compose: { id: 'compose-prov', model: 'good-writer', temperature: 0.7 },
};

/** 本用例给 `classify` 写的那段提示词（**逐字**比对，证明覆盖真的生效） */
const TRIAGE_PROMPT = '只输出一个词：direct / clarify / refuse / work。';

/** 会话 id：一条线一个，断言互不污染 */
const SIDS = {
  independent: 'e2e-t25-independent',
  defaults: 'e2e-t25-defaults',
  typo: 'e2e-t25-typo',
};

/** 答话正文（由本用例的 mock 场景给出） */
const A_ANSWER = '我们刚才在聊 README 的事。';
const B_ANSWER = 'B 线这句是生成的。';
const C_ANSWER = 'C 线这句是生成的。';

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

/** 会话配置：判决与措辞都开，汇报**显式关掉**——本用例数请求，不该被汇报调优影响 */
function sessionConfig() {
  return { classify_enabled: true, compose_enabled: true, progress_enabled: false };
}

/** 一个请求的 system 段（`execute_turn` 的第一条消息；缺席给空串） */
function sysOf(r) {
  const m = (r.body?.messages ?? []).find((x) => x.role === 'system');
  return typeof m?.content === 'string' ? m.content : '';
}

/** 带工具的那些请求 = worker 的请求（分类 / 生成都是无工具的内部请求） */
function workerRequests(requests) {
  return requests.filter((r) => (r.body?.tools ?? []).length > 0);
}

export default defineCase(
  'T25 对话面调优：两个插件各自选模型 / 提示词可覆盖 / 缺省与写错都落回会话选定值',
  async () => {
    const llm = await new MockLlm([
      // ── A 线：先分类（once 消耗掉）再生成，两者读的是同一条对话线 ──
      { id: 'a-cls', match: '我们刚才聊了什么', content: 'direct', once: true },
      { id: 'a-gen', match: '我们刚才聊了什么', content: A_ANSWER },
      // A 线第二句：分类判「要干活」⇒ Escalate ⇒ 首响走模板（零往返）⇒ 工具循环
      { id: 'a-cls-work', match: '读一下 README', content: 'work', once: true },
      {
        id: 'a-read',
        match: '读一下 README',
        toolCalls: [{ id: 'call_r1', name: 'vdfs_read', arguments: { path: 'README.md' } }],
      },
      { id: 'a-after-read', afterTool: true, content: 'README 已经读完了。' },
      // ── B / C 线：各自的输入，避免与 A 抢场景 ──
      { id: 'b-cls', match: 'B 线聊过什么', content: 'direct', once: true },
      { id: 'b-gen', match: 'B 线聊过什么', content: B_ANSWER },
      { id: 'c-cls', match: 'C 线聊过什么', content: 'direct', once: true },
      { id: 'c-gen', match: 'C 线聊过什么', content: C_ANSWER },
    ]).start();

    /** 三个条目都指向同一个 mock，只有 `id` / `model` / `temperature` 不同 */
    const providers = Object.values(PROVIDERS).map((p) => ({
      id: p.id,
      config: providerConfig(llm.port, {
        id: p.id,
        name: p.id,
        model: p.model,
        temperature: p.temperature,
      }),
    }));

    /** 起一条线：自己的 homedir / 端口 / 两个插件的配置 */
    const startLine = async (sid, classifyCfg, composeCfg, label) => {
      const port = nextPort();
      const hd = makeHomedir({
        providers,
        pluginConfigs: {
          gateway: gatewayConfig(port),
          session: sessionConfig(),
          ...(classifyCfg ? { classify: classifyCfg } : {}),
          ...(composeCfg ? { compose: composeCfg } : {}),
        },
      });
      const cli = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: sid,
        provider: PROVIDERS.session.id,
        gatewayPort: port,
      });
      await cli.waitGatewayReady(20_000, label);
      return { cli, hd, sid };
    };

    /** 建会话 + 投一条消息（**唯一输入入口**：写收件箱） */
    const send = async ({ cli, hd, sid }, text) => {
      const root = (await cli.invoke('vdfs/root', {})).body?.data?.path;
      assert(typeof root === 'string' && root.length > 0, 'vdfs/root 应返回根地址');
      const sessionAddr = `${root.replace(/\/+$/, '')}/session/${sid}`;

      const created = await cli.invoke('vdfs/write', {
        path: sessionAddr,
        create: true,
        text: JSON.stringify({
          metadata: { workdir: hd.workdir, mode: 'auto', risk_level: 'medium' },
        }),
      });
      assertEq(created.status, 200, `创建会话 ${sid}（${JSON.stringify(created.body)?.slice(0, 300)}）`);

      const w = await cli.invoke('vdfs/write', { path: `${sessionAddr}/inbox`, text });
      assertEq(w.status, 200, `写收件箱 ${sid}（${JSON.stringify(w.body)?.slice(0, 300)}）`);
    };

    /** 等该会话出现一条指定理由码的对话面节点 */
    const waitReplyNode = (homedir, sid, reason) =>
      waitFor(
        () =>
          (readMessagesJson(homedir, sid) ?? []).some(
            (m) => m.meta?.surface === 'reply' && m.meta?.reason === reason,
          ),
        { what: `${sid} 出现对话面节点（reason=${reason}）`, timeoutMs: 40_000 },
      );

    /** 从 `base` 起的新请求（每条线开跑前取基线，避免线间串味） */
    const since = async (base) => (await llm.requests()).slice(base);

    /** 该批请求里所有请求体字段的快照，失败时能一眼看出"发到哪去了" */
    const modelsOf = (reqs) => reqs.map((r) => r.body?.model);

    const lines = [];
    try {
      // ── A 各自独立：分类用 `classify-prov`、答话用 `compose-prov`、worker 用会话选定的 ──
      const a = await startLine(
        SIDS.independent,
        { model: PROVIDERS.classify.id, system_prompt: TRIAGE_PROMPT },
        { model: PROVIDERS.compose.id },
        'A 线进程（两个插件各配一个模型）',
      );
      lines.push(a);

      let base = (await llm.requests()).length;
      await send(a, '我们刚才聊了什么');
      await waitReplyNode(a.hd.homedir, SIDS.independent, 'from_context');

      let reqs = await since(base);
      // 分类：system 段**逐字等于**配置里那段 ⇒ 提示词覆盖生效，于是它就是分类请求
      const classified = reqs.filter((r) => sysOf(r) === TRIAGE_PROMPT);
      assertEq(
        classified.length,
        1,
        `第一句应恰有一次分类请求（system 段 = 配置那段），实得 ${classified.length}`,
      );
      const generated = reqs.filter(
        (r) => (r.body?.tools ?? []).length === 0 && sysOf(r) !== TRIAGE_PROMPT,
      );
      assertEq(generated.length, 1, `第一句应恰有一次答话生成请求，实得 ${generated.length}`);

      assertEq(
        classified[0].body.model,
        PROVIDERS.classify.model,
        '分类请求必须用 `classify` 自己配的模型（便宜快的那个）',
      );
      assertEq(
        classified[0].body.temperature,
        PROVIDERS.classify.temperature,
        '温度随 provider 条目走（classify-prov 是 0.0）',
      );
      assertEq(
        generated[0].body.model,
        PROVIDERS.compose.model,
        '答话必须用 `compose` 自己配的模型（措辞更好的那个）',
      );
      assertEq(
        generated[0].body.temperature,
        PROVIDERS.compose.temperature,
        '温度随 provider 条目走（compose-prov 是 0.7）',
      );

      // 第二句：`Escalate` ⇒ 首响走模板（零往返）⇒ 工具循环的请求仍归会话
      base = (await llm.requests()).length;
      await send(a, '读一下 README');
      await waitFor(
        () => (readMessagesJson(a.hd.homedir, SIDS.independent) ?? []).some((m) => m.role === 'tool'),
        { what: 'A 线工具结果落库', timeoutMs: 40_000 },
      );

      reqs = await since(base);
      const worker = workerRequests(reqs);
      assert(worker.length >= 1, `A 线第二句应至少发一次 worker 请求（实得 ${worker.length}）`);
      for (const r of worker) {
        assertEq(
          r.body.model,
          PROVIDERS.session.model,
          '会话自己的请求仍用**会话选定**的模型——插件的选择不该改会话',
        );
        assertEq(r.body.temperature, PROVIDERS.session.temperature, '温度随条目走（session-prov 是 0.1）');
      }
      // 第二句的分类请求同样用 classify 的模型（换一句话不改变"谁用哪个模型"）
      const cls2 = reqs.filter((r) => sysOf(r) === TRIAGE_PROMPT);
      assertEq(cls2.length, 1, '第二句也应恰有一次分类请求');
      assertEq(cls2[0].body.model, PROVIDERS.classify.model, '第二句的分类仍用 classify 的模型');

      const aMsgs = readMessagesJson(a.hd.homedir, SIDS.independent);
      assertTranscriptInvariants(aMsgs, 'T25-A');
      assert(
        aMsgs.some((m) => m.meta?.surface === 'reply' && m.meta?.reason === 'from_context'),
        'A 线第一句应拿到生成产线的答话',
      );

      // ── B 平凡值：两个插件的 PLUGIN.yml 里**不写**这两个键 ⇒ 全部落回会话选定值 ──
      const b = await startLine(SIDS.defaults, null, null, 'B 线进程（两个插件都用出厂配置）');
      lines.push(b);

      base = (await llm.requests()).length;
      await send(b, 'B 线聊过什么');
      await waitReplyNode(b.hd.homedir, SIDS.defaults, 'from_context');

      reqs = await since(base);
      assert(reqs.length >= 2, `B 线应有分类与生成各一次（实得 ${reqs.length}）`);
      assertEq(
        new Set(modelsOf(reqs)).size,
        1,
        `B 线全部请求都该用会话选定的模型，实得 ${JSON.stringify(modelsOf(reqs))}`,
      );
      assertEq(modelsOf(reqs)[0], PROVIDERS.session.model, 'B 线的唯一模型应是会话选定的那个');
      for (const r of reqs) {
        assertEq(r.body.temperature, PROVIDERS.session.temperature, 'B 线的温度也应随会话条目');
        assert(
          sysOf(r) !== TRIAGE_PROMPT,
          'B 线没配提示词 ⇒ 不该出现本用例那段（内置那份生效）',
        );
        assert(sysOf(r).trim().length > 0, 'system 段不得为空（空指令段是一条没有信号的失效）');
      }

      // ── C 写错 id：降级而不失效 —— 落回会话选定值，而判决照常发生 ──
      const c = await startLine(
        SIDS.typo,
        { model: 'nope-not-exist' },
        null,
        'C 线进程（classify 的 model 写错了）',
      );
      lines.push(c);

      base = (await llm.requests()).length;
      await send(c, 'C 线聊过什么');
      await waitReplyNode(c.hd.homedir, SIDS.typo, 'from_context');

      reqs = await since(base);
      assert(reqs.length >= 2, `C 线应有分类与生成各一次（实得 ${reqs.length}）`);
      assertEq(
        new Set(modelsOf(reqs)).size,
        1,
        `写错的 id 应落回会话选定的模型，实得 ${JSON.stringify(modelsOf(reqs))}`,
      );
      assertEq(
        modelsOf(reqs)[0],
        PROVIDERS.session.model,
        '取不到配置的 provider ⇒ 落回会话选定值（降级而不失效）',
      );

      const cMsgs = readMessagesJson(c.hd.homedir, SIDS.typo);
      assertTranscriptInvariants(cMsgs, 'T25-C');
      assert(
        cMsgs.some((m) => m.meta?.surface === 'reply' && m.meta?.reason === 'from_context'),
        '写错模型 id **不该**让判决消失——用户必须照旧拿到答话（这是降级与失效的分界）',
      );
    } finally {
      for (const l of lines) {
        l.cli.stop();
        cleanupHomedir(l.hd);
      }
      llm.stop();
    }
  },
);
