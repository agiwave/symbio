import './_selfrun.mjs';
// T72 web_search 上游基址可配：Tavily 与 DuckDuckGo 回退两条路都打本地靶子。
//
// ## 这一条补的是哪一格
//
// RE-005「Web 外取」行的 `web_search` 格：三个上游地址（Tavily / Serper /
// DuckDuckGo HTML）曾**全部写死在代码里** ⇒ 只能打真外网、CI 无网 ⇒ 零覆盖。
// 给 `WebConfig` 加了 `search_api_base`（空 = 官方端点，
// 对现有部署零行为变化；非空 = 替换基址、路径按供应商契约保留），本用例把它
// 指向 `MockHttp`，两条实际可达的上游路各钉一条判据。
//
// ## 为什么只打 Tavily + DDG 回退，不打 Serper
//
// Tavily 与 Serper 共用 `POST /search` 路径、只差鉴权头，mock 端点对两者无差别；
// 分支选择逻辑（有 Tavily key ⇒ 不走 Serper / DDG）与基址替换是**两件事**，后者
// 已有单测（`upstream_base_*`）。本用例取「有 key 走 Tavily」与「无 key 落 DDG」
// 两条**生产上真实相邻**的路径；Serper 需要独立 key 才可达，与 Tavily 无交互，
// 单测覆盖即够。
//
// ## 为什么走模型工具调用，不直连插件路由
//
// 工具只有一条执行面（`D-440` / `D-466`）：`local` / `web` 的 `route` 按工具名一律
// `NotFound`，执行只经 `port.tool` 契约。所以本用例和 t65 一样用 `MockLlm` 编排两轮
// 工具调用，再从**工具结果回灌的文本**里读 `provider` / `results`——那是用户结果在
// 模型侧的唯一投影。
//
// ## 第二轮怎么切 provider：改配置文档，不重启进程
//
// `vdfs/write` 打到 `<根>/web/PLUGIN.yml` 会走 `PluginConfigMount` 的 apply：
// 校验 → **进内存槽** → 落盘（`plugin/dir.rs` 的 Write 臂）。`WebSearchTool` 持有
// 的正是这个槽的 `RwLock` ⇒ 把 `tavily_api_key` 清空后，同一进程内下一次搜索
// 就该落到 DDG 回退——这同时钉住「配置文档是活的」这一机制事实。
import {
  MockLlm,
  MockHttp,
  makeHomedir,
  dialogFaceOff,
  cleanupHomedir,
  startLongLivedCli,
  subscribeSessionRealtime,
  nextPort,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  msgRoleOf,
} from '../helpers.mjs';

const TAVILY_KEY = 'T71TAVILYKEY';
const SID = 'e2e-t72';

/** gateway 入站配置（与 t65 / t16 / t27 同一约定）。 */
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

export default defineCase(
  'T72 web_search：上游基址可配 ⇒ Tavily 与 DDG 回退都打本地靶子（含配置热切 provider）',
  async () => {
    const mock = await new MockHttp({ marker: 'T71SEARCH', slowMs: 0 }).start();

    // 两轮各一次工具调用；每个工具结果轮都带 `once`（夹具陷阱 14：不带就是无限循环）。
    const llm = await new MockLlm([
      {
        id: 'search-tavily',
        match: '第一轮查一下',
        once: true,
        toolCalls: [{ id: 'c1', name: 'web_search', arguments: { query: 't71 搜索词' } }],
      },
      { id: 'final-1', afterTool: true, once: true, content: '第一轮搜索完成。' },
      {
        id: 'search-ddg',
        match: '第二轮查一下',
        once: true,
        toolCalls: [{ id: 'c2', name: 'web_search', arguments: { query: 't71 回退词' } }],
      },
      { id: 'final-2', afterTool: true, once: true, content: '第二轮搜索完成。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: gatewayConfig(GATEWAY_PORT),
        ...dialogFaceOff(),
        web: { web_timeout: 15, tavily_api_key: TAVILY_KEY, search_api_base: mock.base },
      },
    });
    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'unused-t72',
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });

    let rt = null;
    try {
      await cli.waitGatewayReady(20_000, 't72');
      rt = await subscribeSessionRealtime(cli, { sessionId: SID, gatewayPort: GATEWAY_PORT });

      const rootResp = await cli.invoke('vdfs/root', {});
      const root = rootResp.body?.data?.path;
      assert(typeof root === 'string' && root.length > 0, 'vdfs/root 应返回根地址');
      const sessionAddr = `${root.replace(/\/+$/, '')}/session/${SID}`;
      const inboxAddr = `${sessionAddr}/inbox`;
      const created = await cli.invoke('vdfs/write', {
        path: sessionAddr,
        create: true,
        text: JSON.stringify({ metadata: { workdir: hd.workdir, mode: 'auto' } }),
      });
      assertEq(created.status, 200, `创建会话（${JSON.stringify(created.body)?.slice(0, 300)}）`);

      const searchHits = async (path) =>
        (await mock.requests()).filter((r) => r.path === path);

      // ── ① Tavily 路径：基址被替换，载荷（query / api_key）真的到了靶子 ──
      await runTurn(cli, rt, inboxAddr, '第一轮查一下');

      // ── ② 配置热切：vdfs/write 清空 tavily key ⇒ 同进程内下一轮落 DDG 回退 ──
      //
      // 写法是**读原文件 → 只改目标行 → 写回**，不是整文件重写：`PLUGIN.yml` 既有
      // 配置键也有装配期投影的身份键（`plugin_provider` / `plugin_name`…，t17 ①），
      // 整文件重写会把身份键洗掉 ⇒ manifest 校验 400。改一行 = 前端表单保存的同款语义。
      const cfgAddr = `${root.replace(/\/+$/, '')}/web/PLUGIN.yml`;
      const cur = (await cli.invoke('vdfs/read', { path: cfgAddr })).body?.data?.text ?? '';
      assert(cur.includes('tavily_api_key'), `原 PLUGIN.yml 应含 tavily_api_key（实际: ${cur.slice(0, 300)}）`);
      // `vdfs/read` 对 `ext=form` 回 **pretty JSON**（YAML 只是落盘形态，见
      // `PluginConfigFile::read`）⇒ 按改一行 = 前端表单保存的同款语义，只动
      // `tavily_api_key` 一个键（JSON.parse 后改值再序列化，身份键原样保留）。
      const cfgObj = JSON.parse(cur);
      cfgObj.tavily_api_key = '';
      const w = await cli.invoke('vdfs/write', {
        path: cfgAddr,
        text: JSON.stringify(cfgObj, null, 2),
      });
      assertEq(w.status, 200, `写 PLUGIN.yml 应 200（实得 ${JSON.stringify(w)}）`);

      await runTurn(cli, rt, inboxAddr, '第二轮查一下');

      // ── 证据①：四次请求 = 两轮用户消息 + 两次工具结果回灌 ──
      const reqs = await llm.requests();
      assertEq(reqs.length, 4, `应四次请求（2 用户轮 + 2 工具结果轮），实际 ${reqs.length}`);
      const wireNames = (reqs[0].body.tools ?? []).map((t) => t.function?.name);
      assert(
        wireNames.includes('web_search'),
        `web_search 应注册进模型可见工具面（实际: ${wireNames.join(',')}）`,
      );

      // ── 证据②：第一轮走 Tavily，结果来自靶子 ──
      const tavilyText = toolResultText(reqs[1]);
      assert(tavilyText.includes('tavily'), `有 Tavily key ⇒ 应走 tavily（实际: ${tavilyText.slice(0, 400)}）`);
      assert(
        tavilyText.includes('t71 搜索词'),
        `Tavily 结果应来自靶子（实际: ${tavilyText.slice(0, 400)}）`,
      );

      const searchReqs = await searchHits('/search');
      assertEq(searchReqs.length, 1, '靶子应恰收到 1 次 POST /search');
      assert(
        searchReqs[0].body.includes('t71 搜索词'),
        `载荷应含 query（实际: ${searchReqs[0].body.slice(0, 200)}）`,
      );
      assert(
        searchReqs[0].body.includes(TAVILY_KEY),
        '载荷应含 api_key（Tavily 契约：key 在 body 里）',
      );

      // ── 证据③：key 清空后第二轮落 DDG 回退 ──
      const ddgText = toolResultText(reqs[3]);
      assert(ddgText.includes('duckduckgo'), `key 已清空 ⇒ 应落 DDG 回退（实际: ${ddgText.slice(0, 400)}）`);
      assert(
        ddgText.includes('t71 回退词'),
        `DDG 结果应来自靶子（实际: ${ddgText.slice(0, 400)}）`,
      );

      const ddgReqs = await searchHits('/html/');
      assertEq(ddgReqs.length, 1, '靶子应恰收到 1 次 GET /html/');
      // q 是否真到了靶子由上一条结果标题判（mock 把 q 回显进 `result__a` 文本）。
    } finally {
      rt?.close();
      cli.stop();
      llm.stop();
      mock.stop();
      cleanupHomedir(hd);
      // 收尾等子进程退干净（Windows 上带活子进程退出会撞 libuv 断言）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm / mock-http 子进程退出',
      });
    }
  },
);

/** 往收件箱发一句，等这一轮真正跑完（水位线不可省：首帧空闲态会让条件在开跑前成立）。 */
async function runTurn(cli, rt, inboxAddr, text) {
  const mark = rt.sessionChanges().length;
  const w = await cli.invoke('vdfs/write', { path: inboxAddr, text });
  assertEq(w.status, 200, `写收件箱「${text}」：${JSON.stringify(w.body)?.slice(0, 300)}`);
  await waitFor(
    () => {
      const sc = rt.sessionChanges().slice(mark);
      return (
        sc.length > 0 &&
        sc.some((c) => c.data?.status === 'working') &&
        sc[sc.length - 1]?.data?.status !== 'working'
      );
    },
    { what: `回合「${text}」跑完（实时面离开 working）`, timeoutMs: 90_000 },
  );
}

/**
 * 取「本轮工具结果回灌」的那条 `role=tool` 消息正文。
 *
 * ⚠️ **取最后一条，不是第一条**（夹具陷阱：历史里带着前面每一轮的工具结果）。
 */
function toolResultText(req) {
  const msgs = req?.body?.messages ?? [];
  const tool = [...msgs].reverse().find((m) => msgRoleOf(m) === 'tool');
  if (tool) return typeof tool.content === 'string' ? tool.content : JSON.stringify(tool.content);
  return '';
}
