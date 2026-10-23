import './_selfrun.mjs';
// T71 web_search 上游基址可配：Tavily 与 DuckDuckGo 回退两条路都打本地靶子。
//
// ## 这一条补的是哪一格
//
// RE-005「Web 外取」行的 `web_search` 格：三个上游地址（Tavily / Serper /
// DuckDuckGo HTML）曾**全部写死在代码里** ⇒ 只能打真外网、CI 无网 ⇒ 零覆盖
// （ ⑤）。 给 `WebConfig` 加了 `search_api_base`（空 = 官方端点，
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
// ## 第二轮怎么切 provider：改配置文档，不重启进程
//
// `vdfs/write` 打到 `<根>/web/PLUGIN.yml` 会走 `PluginConfigMount` 的 apply：
// 校验 → **进内存槽** → 落盘（`plugin/dir.rs` 的 Write 臂）。`WebSearchTool` 持有
// 的正是这个槽的 `RwLock` ⇒ 把 `tavily_api_key` 清空后，同一进程内下一次搜索
// 就该落到 DDG 回退——这同时钉住「配置文档是活的」这一机制事实。
import {
  MockHttp,
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
  nextPort,
  waitFor,
  assert,
  assertEq,
  defineCase,
} from '../helpers.mjs';

const TAVILY_KEY = 'T71TAVILYKEY';

export default defineCase(
  'T72 web_search：上游基址可配 ⇒ Tavily 与 DDG 回退都打本地靶子（含配置热切 provider）',
  async () => {
    const mock = await new MockHttp({ marker: 'T71SEARCH', slowMs: 0 }).start();
    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      pluginConfigs: {
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        web: { web_timeout: 15, tavily_api_key: TAVILY_KEY, search_api_base: mock.base },
      },
    });
    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 't72',
      gatewayPort: GATEWAY_PORT,
    });
    try {
      await cli.waitGatewayReady(20_000, 't72');
      const searchHits = async (path) =>
        (await mock.requests()).filter((r) => r.path === path);

      // ── ① Tavily 路径：基址被替换，载荷（query / api_key）真的到了靶子 ──
      // 路径 = `<挂载点>/web_search`：composite 只按 `名字/剩余` 分发，裸工具名
      // 走不到 web 插件（t65 那边是经 LLM 能力链路路由的，与此处的直连不同）。
      const r1 = await cli.invoke('web/web_search', { query: 't71 搜索词' });
      assertEq(r1.status, 200, `web_search 应 200（实得 ${r1.status}）`);
      assertEq(r1.body?.data?.provider, 'tavily', '有 Tavily key ⇒ 应走 tavily');
      assertEq(r1.body?.data?.results?.length, 2, '靶子回两条 ⇒ 透传两条');
      assert(
        String(r1.body?.data?.results?.[0]?.title).includes('t71 搜索词'),
        `结果应来自靶子（标题带请求词，实际: ${JSON.stringify(r1.body?.data?.results)}）`,
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

      // ── ② 配置热切：vdfs/write 清空 tavily key ⇒ 同进程内下一轮落 DDG 回退 ──
      //
      // 写法是**读原文件 → 只改目标行 → 写回**，不是整文件重写：`PLUGIN.yml` 既有
      // 配置键也有装配期投影的身份键（`plugin_provider` / `plugin_name`…，t17 ①），
      // 整文件重写会把身份键洗掉 ⇒ manifest 校验 400。改一行 = 前端表单保存的同款语义。
      const rootResp = await cli.invoke('vdfs/root', {});
      const root = rootResp.body?.data?.path;
      const cfgAddr = `${root.replace(/\/+$/, '')}/web/PLUGIN.yml`;
      const cur = (await cli.invoke('vdfs/read', { path: cfgAddr })).body?.data?.text ?? '';
      assert(cur.includes('tavily_api_key'), `原 PLUGIN.yml 应含 tavily_api_key（实际: ${cur.slice(0, 300)}）`);
      // `vdfs/read` 对 `ext=form` 回 **pretty JSON**（YAML 只是落盘形态，见
      // `PluginConfigFile::read`）⇒ 按改一行 = 前端表单保存的同款语义，只动
      // `tavily_api_key` 一个键（JSON.parse 后改值再序列化，身份键原样保留）。
      const cfgObj = JSON.parse(cur);
      cfgObj.tavily_api_key = '';
      const yml = JSON.stringify(cfgObj, null, 2);
      const w = await cli.invoke('vdfs/write', { path: cfgAddr, text: yml });
      assertEq(w.status, 200, `写 PLUGIN.yml 应 200（实得 ${JSON.stringify(w)}）`);

      const r2 = await cli.invoke('web/web_search', { query: 't71 回退词' });
      assertEq(r2.status, 200, `第二轮应 200（实得 ${r2.status}）`);
      assertEq(r2.body?.data?.provider, 'duckduckgo', 'key 已清空 ⇒ 应落 DDG 回退');
      assert(
        String(r2.body?.data?.results?.[0]?.title).includes('t71 回退词'),
        `DDG 结果应来自靶子（实际: ${JSON.stringify(r2.body?.data?.results)}）`,
      );

      const ddgReqs = await searchHits('/html/');
      assertEq(ddgReqs.length, 1, '靶子应恰收到 1 次 GET /html/');
      // q 是否真到了靶子由上一条结果标题判（mock 把 q 回显进 `result__a` 文本）。
    } finally {
      cli.stop();
      mock.stop();
      cleanupHomedir(hd);
      // 收尾等子进程退干净（Windows 上带活子进程退出会撞 libuv 断言）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-http 子进程退出',
      });
    }
  },
);
