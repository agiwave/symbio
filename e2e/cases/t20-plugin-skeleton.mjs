import './_selfrun.mjs';
// T20 对话面插件边界：装配 + 两条路由可达 + 契约往返 + 卸载平凡值。
//
// ## 本用例钉的是什么
//
// 对话面拆成两个插件的**判据是"没有它系统也能正确运行"**，而这句话的可检验形式
// 有三样——它们都与"插件内部怎么实现"无关，只与**边界**有关：
//
// 1. 它有自己的目录与身份（装配期补出来，不是手写的）；
// 2. 它的路由**经真实边界可达**（外部客户端 → 容器 → 插件），契约能往返；
// 3. 不挂载它，系统**照旧正确运行**（"没有也可以正确运行"）。
//
// 边界成立之后，"对话面在会话里怎么被调用、产出什么"由 `t21` / `t22` 验——那是
// **行为**，本用例只验**边界**。两者分开，是因为行为会随批次演化（S2 判决、S3 措辞、
// S4 汇报），而边界从 S1 起就不该再动。
//
// ## 四条断言线
//
// | 线 | 验什么 |
// |---|---|
// | A 装配 | 空 homedir 下两个插件的目录 / `plugin_provider` / 出厂身份被补出来（同 T17 形态） |
// | B 路由 | 经 gateway HTTP 边界可达两条路由；契约往返正确（规则命中 / 兜底判决 / 三条措辞产线 / 未知子命令 NotFound） |
// | C 会话内边界 | 跑一轮真实工具回路：工具真落盘，且对话面节点**只由 `reply` 产出、只挂根级** |
// | D 卸载 | 停用两个插件 ⇒ 路由消失（`NotFound`），而工具回路照旧跑通 |
//
// ## B 线为什么只断言"不需要模型"的路径
//
// 网关这条 ctx 里**没有能力访问器**（`CAPABILITY_VISITOR` 是装配期由容器挂上的），
// 因此任何要调模型的产线在这里都取不到模型服务。这不是缺陷，恰好让断言**确定**：
//
// - `triage/decide`：规则命中 ⇒ 判决出来了（连模型都没有 ⇒ 这条路上没碰模型）；
//   规则未命中 ⇒ 兜底 `Escalate`（失败方向是"照旧进工具循环"，绝不是 `Answered`）；
// - `reply/compose`：模板产线照常出文本；**生成产线取不到模型 ⇒ 落变体兜底**；
//   `Report` 本批无产线 ⇒ 空串（平凡值）。
//
// ## 为什么经 gateway 而不是 CLI 命令
//
// 路由是**跨插件调用**的地址，CLI 没有"直呼一条路由"的命令；而 gateway 的
// `POST /api/v1/invoke` 正是「外部客户端 → 容器 `route`」的真实边界（与前端同构）。
// 用一个测试专用的调用钩子只能证明"我写的那条路通"——那是同义反复。
//
// ## 为什么只起一个 CLI 进程
//
// 网关的监听端口来自 homedir 里的插件配置：**两个进程读同一个 homedir 会抢同一个端口**，
// 后绑的那个静默失败，而 `waitGatewayReady` 会被先绑上的那个答成"就绪"，请求于是被
// 路由到错误的进程（T19 踩过这个坑）。因此装配、路由、工具回路、停用后重启全部
// 走同一个长驻进程 / 同一个会话 id。
import { join } from 'node:path';
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
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
  PROVIDER_ID,
  assertTranscriptInvariants,
} from '../helpers.mjs';

const SID = 'e2e-t20';
/** 停用后那一轮用另一个会话：两条线各自从零开始，断言不互相污染 */
const SID_OFF = 'e2e-t20-off';

/** 本批（S1）两个插件的出厂身份（`PluginMeta::new` 的第二参 → 投影成 `plugin_title`） */
const PLUGINS = [
  { name: 'triage', title: '意图判决' },
  { name: 'reply', title: '对话措辞' },
];

/** 模板产线的两句期望文本（与 `reply/templates.rs` 的对应行一致，逐字比对） */
const TEMPLATE_UNCLASSIFIED = '我先看一下。';
/** `from_context` 在模板表里**没有行** ⇒ 生成不了时落的是**变体兜底**（`FALLBACK_ANSWERED`） */
const FALLBACK_ANSWERED = '好的。';

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

/** 从 `PLUGIN.yml` 正文里取一个顶层标量（剥掉可选的 YAML 引号） */
function yamlValue(text, key) {
  const m = text.match(new RegExp(`^${key}:\\s*"?([^"\\n]*?)"?\\s*$`, 'm'));
  return m ? m[1] : null;
}

export default defineCase(
  'T20 对话面插件边界：装配 + 两条路由可达 + 契约往返 + 卸载平凡值',
  async () => {
    // 一轮真实工具回路：有工具调用 ⇒ 顺带证明"两个插件的存在没有改变工具链"
    const llm = await new MockLlm([
      {
        id: 'call-write',
        match: '写文件',
        toolCalls: [
          { id: 'call_w1', name: 'vdfs_write', arguments: { path: 'notes.md', text: '# T20\n由 mock 写入' } },
        ],
      },
      { id: 'after-write', afterTool: true, content: '文件已写入。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: { gateway: gatewayConfig(GATEWAY_PORT) },
    });

    /** 起一个长驻 CLI 并等网关就绪（本用例反复用两次：初始 / 停用后） */
    const startCli = async () => {
      const c = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      await c.waitGatewayReady();
      return c;
    };

    /**
     * 建会话并把一条消息写进它的收件箱（**唯一输入入口**），再等这一轮真正跑完。
     *
     * 落库判据用**节点数**而不是"有没有回复"：一轮的节点是
     * user + tool_call + tool 结果 + assistant 正文 ≥ 4 条，取到 4 条即本轮已收尾。
     * 阈值写成「基线 + 本轮新增」的形式——写死绝对条数会让"再加一条线"变成"改三处断言"。
     */
    const runTurn = async (cli, sid, text) => {
      const root = (await cli.invoke('vdfs/root', {})).body?.data?.path;
      assert(typeof root === 'string' && root.length > 0, 'vdfs/root 应返回根地址');
      const base = root.replace(/\/+$/, '');
      const sessionAddr = `${base}/session/${sid}`;

      const created = await cli.invoke('vdfs/write', {
        path: sessionAddr,
        create: true,
        text: JSON.stringify({ metadata: { workdir: hd.workdir, mode: 'auto', risk_level: 'medium' } }),
      });
      assertEq(created.status, 200, `创建会话 ${sid}（${JSON.stringify(created.body)?.slice(0, 300)}）`);

      const w = await cli.invoke('vdfs/write', { path: `${sessionAddr}/inbox`, text });
      assertEq(w.status, 200, `写收件箱 ${sid}（${JSON.stringify(w.body)?.slice(0, 300)}）`);

      await waitFor(() => (readMessagesJson(hd.homedir, sid) ?? []).length >= 4, {
        what: `${sid} 一轮工具回路落库（user + tool_call + tool + assistant）`,
        timeoutMs: 40_000,
      });
      return readMessagesJson(hd.homedir, sid);
    };

    let cli = await startCli();
    try {
      // ── A 装配：两个插件的目录与身份是这次跑起来才补出来的（磁盘上此前不存在）──
      for (const p of PLUGINS) {
        const yml = join(hd.homedir, p.name, 'PLUGIN.yml');
        assert(existsSync(yml), `插件 ${p.name} 应被装配出来（缺 ${yml}）`);
        const text = readFileSync(yml, 'utf8');
        assertEq(yamlValue(text, 'plugin_provider'), p.name, `${p.name} 的 plugin_provider`);
        assertEq(yamlValue(text, 'plugin_name'), p.name, `${p.name} 的 plugin_name`);
        // ADR-032：出厂身份在装配期投影进 manifest（此后运行期只读 manifest）
        assertEq(yamlValue(text, 'plugin_title'), p.title, `${p.name} 的出厂标题应投影进 manifest`);
      }

      // ── B 路由：经 gateway 的真实边界可达，契约往返正确 ──
      //
      // 断言刻意选**不需要模型**的路径（网关这条 ctx 里没有能力访问器，
      // 因此要模型的产线必然退化——退化方向本身也是契约的一部分）：
      //   ① 规则命中 ⇒ 判决出来了 —— 连模型都没有，说明这条路上根本没碰模型；
      //   ② 规则未命中 ⇒ 兜底 `Escalate` —— 失败方向是"照旧进工具循环"，
      //      绝不是 `Answered`（那会让用户看到沉默）；
      //   ③ 措辞三条产线：模板照常 / 生成取不到模型 ⇒ **变体兜底**（不是空白）/
      //      `Report` 无产线 ⇒ 空串（平凡值）。
      const decided = await cli.invoke('triage/decide', { session_id: SID, utterance: '你好' });
      assertEq(
        decided.status,
        200,
        `triage/decide 应可达（${JSON.stringify(decided.body)?.slice(0, 300)}）`,
      );
      assertEq(decided.body?.data?.verdict, 'answered', '规则命中 ⇒ Answered');
      assertEq(decided.body?.data?.reason, 'greeting', '问候的理由码');

      const undecided = await cli.invoke('triage/decide', {
        session_id: SID,
        utterance: '我们刚才聊了什么',
      });
      assertEq(undecided.body?.data?.verdict, 'escalate', '判不出来 ⇒ 兜底 Escalate');
      assertEq(undecided.body?.data?.reason, 'unclassified', '兜底理由码');

      /** 调一次 `reply/compose` 并取出文本（载荷形状见 `schemas::dialog::ComposeRequest`） */
      const compose = async (verdict, context = []) => {
        const r = await cli.invoke('reply/compose', { session_id: SID, verdict, context });
        assertEq(
          r.status,
          200,
          `reply/compose 应可达（${JSON.stringify(r.body)?.slice(0, 300)}）`,
        );
        return r.body?.data;
      };

      // ① 模板产线：`Escalate` 的首响**立刻**给出，不等任何东西
      assertEq(
        await compose({ verdict: 'escalate', reason: 'unclassified' }),
        TEMPLATE_UNCLASSIFIED,
        'Escalate 的首响应来自模板表（逐字比对，不是"非空即可"）',
      );
      // ② 生成产线取不到模型 ⇒ 落**变体兜底**（`Answered` 口吻），不是空白、也不是模板表里
      //    别的行——`from_context` 在表里没有行，这正是"降级而不失效"的落点。
      assertEq(
        await compose({ verdict: 'answered', reason: 'from_context' }),
        FALLBACK_ANSWERED,
        '生成不了时应落 Answered 的变体兜底（绝不能是空白）',
      );
      // ③ `Report` 本批没有产线 ⇒ 空串。空串是**平凡值**（"没有对话面文本"），
      //    调用方据此不写节点——它必须与"生成失败"区分开：后者有兜底，前者没有。
      assertEq(
        await compose({ verdict: 'report' }),
        '',
        'Report 本批无产线 ⇒ 空串（平凡值：没有对话面文本）',
      );

      // 路由是**静态分派**：未知子命令必须响亮失败，而不是"什么都接"
      const bogus = await cli.invoke('triage/bogus', {});
      assert(bogus.status >= 400, `triage/bogus 应失败（实得 ${bogus.status}）`);

      // ── C 会话内边界：一轮真实工具回路 + 对话面节点只由 `reply` 产出、只挂根级 ──
      //    本线验的是**边界**，不是行为（行为见 `t21` / `t22`）。三条判据：
      //    ① 工具链照旧（插件装配没有改变工具链）；
      //    ② 对话面文本由 `reply` 产出 —— 它带 `meta.surface`，且 `reason` 来自判决；
      //    ③ 它挂在**根级**（对话线与用户消息互为兄弟），不是某个 Turn 的子节点。
      //
      //    本线的输入被 mock 判成 `unclassified`（mock 对这一句返回的是工具调用、
      //    没有文本 ⇒ 分类解析不出词 ⇒ 兜底 `Escalate`），因此期望恰好一条首响节点。
      const msgs = await runTurn(cli, SID, '帮我写文件');
      assertTranscriptInvariants(msgs, 'T20');
      const tc = msgs.find((m) => m.type === 'tool_call');
      assert(tc, '应有 tool_call 节点');
      assertEq(tc.status, 'completed', 'tool_call 终态应为 completed');
      assert(
        readFileSync(join(hd.workdir, 'notes.md'), 'utf8').includes('# T20'),
        '工具应真实落盘（插件装配没有改变工具链）',
      );
      const surfaced = msgs.filter((m) => m.meta && 'surface' in m.meta);
      assertEq(surfaced.length, 1, `本轮应恰有一条对话面节点（实得 ${surfaced.length} 条）`);
      assertEq(surfaced[0].meta.surface, 'reply', '对话面节点由 reply 产出（谁产出的）');
      assertEq(surfaced[0].meta.reason, 'unclassified', '理由码来自上游判决（原样透传）');
      assertEq(surfaced[0].role, 'assistant', '对话面节点是助手说的话');
      assert(
        surfaced[0].parent_id == null,
        '对话面节点必须挂根级——挂在 Turn 之下会让前端「对话」面板看不到它',
      );
      assertEq(
        surfaced[0].meta.exclude_from_context,
        true,
        'Escalate 的首响必须被标记为不进请求包（界面开场白 ≠ 模型的对话内容）',
      );

      // ── D 卸载平凡值：停用 ⇒ 路由消失，而系统**照旧正确运行** ──
      //    「没有它也能正确运行」的可执行形式不是配置开关，而是**装配期**的不挂载：
      //    停用即"根本不构造"（因此它不启动任何后台行为），路由随之 `NotFound`。
      cli.stop();
      for (const p of PLUGINS) {
        const yml = join(hd.homedir, p.name, 'PLUGIN.yml');
        writeFileSync(yml, `${readFileSync(yml, 'utf8')}plugin_enabled: false\n`);
        assert(existsSync(join(hd.homedir, p.name)), '停用不删目录（那是卸载的语义）');
      }
      await llm.reset();

      cli = await startCli();
      for (const path of ['triage/decide', 'reply/compose']) {
        const r = await cli.invoke(path, { session_id: SID_OFF });
        assert(
          r.status >= 400,
          `停用后 ${path} 应不可达（实得 ${r.status}：${JSON.stringify(r.body)?.slice(0, 200)}）`,
        );
      }

      const offMsgs = await runTurn(cli, SID_OFF, '帮我写文件');
      assertTranscriptInvariants(offMsgs, 'T20-off');
      const offTc = offMsgs.find((m) => m.type === 'tool_call');
      assert(offTc, '停用两个插件后仍应有 tool_call 节点');
      assertEq(offTc.status, 'completed', '停用两个插件后 tool_call 终态仍应为 completed');
    } finally {
      cli.stop();
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
