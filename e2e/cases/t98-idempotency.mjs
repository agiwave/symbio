import './_selfrun.mjs';
// T98 幂等台账是一条**插件能力**：一个工具（`idempotent_record`）落下领域事实，一张视图
// （`idempotency.ledger`）把它们按键折成台账；摘掉插件后工具面消失而系统照常运行。
//
// ## 本用例钉的是「新增能力只改自己的能力包」
//
// 名册 #37 幂等与重试（`docs/003-design/026-能力拆分与影响类.md` §2.3）的**幂等半边**
// 单独成包落地；重试不在本片（重放身份与续写幕的口径属后续片）。判据必须**同时**看四段：
//
// | 段 | 唯一能看见它的地方 |
// |---|---|
// | 工具面 | 首次请求的 `tools` 里有 `idempotent_record` |
// | 事实落格 | 同一键第二次调用在执行前被拒，`capability_views['idempotency.ledger']` 仍只有一格 |
// | 台账的形状 | 同一格里 `keys` 按键去重、首写为准 |
// | 普通工具不受影响 | 装与不装本插件，`notes_add` 两次不同正文都落两格（`notes.recent.count == 2`） |
//
// ## 为什么跨两轮调同一个键
//
// 同轮同键由写口按事件 id 去重；**执行前拒绝**只有跨轮才看得见——第一轮的
// `idempotency.recorded` 已在会话事实里，第二轮 `before` 读到它才拒。只判同轮会漏掉
// 这条判据的一半。
//
// ## 为什么经 `session/stats` 读视图
//
// 视图是 owner 对自己事实的纯折叠，读它必须经**生产读出口**（`session/stats` 遍历
// 清单生成的公开表，D-221）——只有这条路能证明「装配真的把 owner 的 `ViewDecl`
// 冻结进了同一张表」。直接调用本插件的求值函数是同义反复。
//
// ## 停用侧断言什么、不断言什么
//
// 运行期停用拿掉的是**插件实例与它注册的工具**；事实声明、视图与行为是**链接期**
// 贡献（D-217 / D-218），它们不会因为翻一个装配位就从二进制里消失。因此这里断言
// 「工具面消失 + 普通工具照常 + 会话照常跑完」，不把链接期贡献的存在与否伪装成
// 运行期读数——与 T88 / T94 同一口径。
import { join } from 'node:path';
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';
import { toolNamesOf, envelopeOf, isSilentAuxRequest } from '../llm-contract.mjs';

const SID = 'e2e-t98';
const SID_OFF = 'e2e-t98-off';
const GW = nextPort();

/** 同一个键两次：第一次落格，第二次在执行前被拒。 */
const KEY = 'build-42';
const VALUE_FIRST = 'compiled';
const VALUE_SECOND = 'again';

/** 普通工具（便签）的两条不同正文——装与不装都该落两格。 */
const NOTES = ['alpha', 'beta'];

const ASK_FIRST = '把 build-42 记成 compiled';
const ASK_DUP = '再记一次 build-42';
const ASK_NOTES = '记两条便签';
const ASK_OFF_NOTES = '离线记两条便签';

function noteCalls(prefix) {
  return NOTES.map((text, i) => ({
    id: `${prefix}${i + 1}`,
    name: 'notes_add',
    arguments: { text },
  }));
}

function scenarios() {
  return [
    {
      id: 'idem-first',
      match: '把 build-42',
      times: 1,
      toolCalls: [{ id: 'call_i1', name: 'idempotent_record', arguments: { key: KEY, value: VALUE_FIRST } }],
    },
    { id: 'idem-first-final', match: '把 build-42', afterTool: true, times: 1, content: '已记下。' },
    {
      id: 'idem-dup',
      match: '再记一次',
      times: 1,
      toolCalls: [{ id: 'call_i2', name: 'idempotent_record', arguments: { key: KEY, value: VALUE_SECOND } }],
    },
    { id: 'idem-dup-final', match: '再记一次', afterTool: true, times: 1, content: '这个键已经记过。' },
    { id: 'notes-on', match: '记两条便签', times: 1, toolCalls: noteCalls('call_n') },
    { id: 'notes-on-final', match: '记两条便签', afterTool: true, times: 1, content: '好了。' },
    // 停用侧：幂等工具不存在，只有普通便签
    { id: 'notes-off', match: '离线记两条便签', times: 1, toolCalls: noteCalls('call_o') },
    { id: 'notes-off-final', match: '离线记两条便签', afterTool: true, times: 1, content: '好了。' },
  ];
}

function homedirFor(port) {
  return makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(port) }],
    pluginConfigs: {
      // 网关拉起才有 `session/stats` 这条生产路由可打（视图读数走它）
      gateway: {
        inbound_enabled: true,
        inbound_protocol: 'http',
        inbound_bind: '127.0.0.1',
        inbound_port: GW,
        inbound_token: '',
        inbound_readonly: false,
      },
      session: { ...DIALOG_FACE_OFF },
    },
  });
}

/** 取第一条主线请求的工具名清单（辅助请求恒不带工具面，`DIALOG_FACE_OFF` 下也没有） */
function firstMainTools(reqs) {
  const main = reqs.find((q) => !isSilentAuxRequest(q) && envelopeOf(q).hasTools);
  return main ? toolNamesOf(main) : null;
}

export default defineCase(
  'T98 幂等台账：同键跨轮被拒、视图按键折台账、装与不装普通工具行为一致',
  async () => {
    const llm = await new MockLlm(scenarios()).start();
    const hd = homedirFor(llm.port);

    try {
      // ── A 工具面：能力包的工具有没有出现在模型看得到的地方 ──────────────
      const first = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: ASK_FIRST,
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(first.code, 0, `第一轮 CLI 退出码（stderr: ${first.stderr.slice(0, 600)}）`);

      const tools = firstMainTools(await llm.requests());
      assert(tools, '应有一条带工具面的主线请求');
      assert(
        tools.includes('idempotent_record'),
        `工具面应含 idempotent_record（实际: ${JSON.stringify(tools)}）`,
      );
      assert(tools.includes('notes_add'), '普通工具（notes_add）应同时在场');
      assert(first.stdout.includes('已记下。'), `第一轮 stdout 应为收尾正文（实际: ${JSON.stringify(first.stdout)}）`);

      // ── B 跨轮同键：第二次在执行前被拒，台账仍只有一格 ──────────────────
      const dup = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: ASK_DUP,
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(dup.code, 0, `第二轮 CLI 退出码（stderr: ${dup.stderr.slice(0, 600)}）`);
      assert(
        dup.stdout.includes('这个键已经记过。'),
        `第二轮应跑完且模型看到拒因（实际: ${JSON.stringify(dup.stdout)}）`,
      );

      // ── C 普通工具（启用侧）：两条不同正文 ⇒ 两格便签 ────────────────────
      const notesOn = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: ASK_NOTES,
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(notesOn.code, 0, `便签轮 CLI 退出码（stderr: ${notesOn.stderr.slice(0, 600)}）`);

      // ── D 经生产读出口读两格视图 ────────────────────────────────────────
      const api = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GW,
      });
      try {
        await api.waitGatewayReady();
        const inv = await api.invoke('session/stats', {}, { session_id: SID });
        assertEq(inv.status, 200, `session/stats 应成功（实际: ${JSON.stringify(inv.body)}）`);

        const views = inv.body?.data?.capability_views;
        const ledger = views?.['idempotency.ledger'];
        assert(ledger, `能力视图列应有 idempotency.ledger 这一格（实际: ${JSON.stringify(views)}）`);
        assertEq(ledger.status, 'ok', `跨插件读应成功，不是缺席态：${JSON.stringify(ledger)}`);
        assertEq(ledger.owner, 'idempotency', 'owner 来自清单生成物');
        assertEq(ledger.ver, 1, '版本随 owner 的 ViewDecl 走');
        assertEq(
          ledger.value?.count,
          1,
          `同一键两次（一次落格、一次被拒）⇒ 只有一格：${JSON.stringify(ledger.value)}`,
        );
        assertEq(ledger.value?.entries?.[0]?.key, KEY, '键原样透出');
        assertEq(
          ledger.value?.entries?.[0]?.value,
          VALUE_FIRST,
          `首写为准（不是第二次的 ${VALUE_SECOND}）：${JSON.stringify(ledger.value)}`,
        );
        // **折出来的**那一格：`keys` 按键去重
        assertEq(ledger.value?.keys, [KEY], `键应去重且保序：${JSON.stringify(ledger.value)}`);

        const notes = views?.['notes.recent'];
        assert(notes, `能力视图列应有 notes.recent 这一格（实际: ${JSON.stringify(views)}）`);
        assertEq(
          notes.value?.count,
          NOTES.length,
          `启用侧普通工具两条不同正文应落两格：${JSON.stringify(notes.value)}`,
        );
      } finally {
        api.stop();
      }

      // ── E 卸任：工具面消失，普通工具与普通工具读数逐字一致 ───────────────
      //    「可卸载」的可执行形式是**装配期**不构造（`plugin_enabled: false`）。
      const manifest = join(hd.homedir, 'idempotency', 'PLUGIN.yml');
      assert(existsSync(manifest), `幂等插件应已被装配出来（缺 ${manifest}）`);
      const kept = readFileSync(manifest, 'utf8')
        .split('\n')
        .filter((l) => !/^plugin_enabled\s*:/.test(l))
        .join('\n');
      writeFileSync(manifest, `${kept.trimEnd()}\nplugin_enabled: false\n`);
      await llm.reset();

      const off = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: ASK_OFF_NOTES,
        provider: PROVIDER_ID,
        session: SID_OFF,
      });
      assertEq(off.code, 0, `停用后 CLI 应正常退出（stderr: ${off.stderr.slice(0, 600)}）`);

      const offTools = firstMainTools(await llm.requests());
      assert(offTools, '停用后仍应有主线请求（系统照常运行）');
      assert(
        !offTools.includes('idempotent_record'),
        `停用后工具面不该再有 idempotent_record（实际: ${JSON.stringify(offTools)}）`,
      );
      assert(offTools.includes('notes_add'), '停用后普通工具（notes_add）应在场');
      assert(off.stdout.includes('好了。'), `停用后这一轮仍应跑完（实际: ${JSON.stringify(off.stdout)}）`);

      const apiOff = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID_OFF,
        provider: PROVIDER_ID,
        gatewayPort: GW,
      });
      try {
        await apiOff.waitGatewayReady();
        const inv = await apiOff.invoke('session/stats', {}, { session_id: SID_OFF });
        assertEq(inv.status, 200, `停用侧 session/stats 应成功（实际: ${JSON.stringify(inv.body)}）`);
        const notes = inv.body?.data?.capability_views?.['notes.recent'];
        assert(notes, `停用侧仍应有 notes.recent 这一格（实际: ${JSON.stringify(inv.body?.data)}）`);
        assertEq(
          notes.value?.count,
          NOTES.length,
          `停用侧普通工具两条不同正文也应落两格（与启用侧逐字一致）：${JSON.stringify(notes.value)}`,
        );
      } finally {
        apiOff.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
