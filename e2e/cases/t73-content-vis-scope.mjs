import './_selfrun.mjs';
// T73 内容侧可见域（[T-094](../../docs/009-task/094-个人认知可见域产地批.md "T-094-个人认知可见域产地批") ②）：
// 矩阵内主体读得到**他人**的共享内容，矩阵外主体读不到；且判据随**内容**走——
// 同一个读方（`agent:main`）在「无共享内容」的会话上照旧空切片。
//
// ## 本用例钉的是什么
//
// T-130 §3.1 的 grants 行 `person:*` ▸ shared 判为**不加行**（归属裁决：人的读权 =
// owner 相等），「共享」落成**内容侧**可见域产地。生产链三格，本用例钉后两格的端到端：
//
// | 格 | 单测钉在哪 | 只有 e2e 能看见什么 |
// |---|---|---|
// | 产地（`memory.encoded` 载荷 `vis_scope`） | `v2_memory.test.rs` | 真实轮次真的把 `vis_scope = shared` 写进了 WAL |
// | 读侧闸（`stats` 按内容可见域判） | `stats.test.rs` | 出口（gateway → 容器 → `session/stats`）上三种读方的收放 |
// | 归属（owner 恒放行） | `stats.test.rs` | 属主读数与不声明 principal 逐字一致 |
//
// ## 为什么必须有两个会话
//
// 只有「共享会话」一半的话，闸退化成「矩阵成员恒放行」也能全绿——那正是 T-094 之前
// 恒传 `VisScope::default()`（thread_private）的镜像错误。负面对照会话 B 把这件事钉住：
// 它**有过轮次**（WAL 在、属主读得到）但**没有共享内容**（发言臂收束被授权拒绝 ⇒
// 派生事实不落格，`v2_exec` 的 `closure_denied` 早退）⇒ `agent:main` 照旧空切片。
// 同一读方、两个会话、两种答案——差异只来自内容的可见域，不来自读方的身份。
//
// ## 会话 B 怎么造
//
// `--agent reflex` 把轮次主体绑到 `agent:reflex`——授权表里登记的窄行（只持
// `judge.intent`，无 `reply.*`）。运行器落收束格前判 `closure_granted`（发言臂），
// 未持有 ⇒ 拒绝入格：无 final 格 ⇒ 本轮派生事实（承诺 / 任务 / 记忆）全不写。
// 用户的答案照旧经 `produced` 回给调用方——收束格是**记录**，不是呈现。
// （同形先例：t71 对等主体的汇总轮，发言闸拒 ⇒ 那句话不进任何格子。）
import {
  MockLlm,
  makeHomedir,
  makeAgentDir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  readFileSyncSafe,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';
import { join } from 'node:path';

/** 会话 A：正常一轮（无 `--agent` ⇒ 主体 `agent:main`）⇒ 用户发言编成 shared 记忆。 */
const SHARED_SID = 'e2e-t73-shared';
/** 会话 B：绑 `reflex`（登记窄行，无发言权）⇒ 有轮次无记忆。 */
const NOSHARED_SID = 'e2e-t73-noshared';

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径）。 */
const V2_WAL = 'v2-events.wal';

/** 读一份 WAL 文件的行（逐行 JSON）。 */
function walLines(path) {
  return readFileSyncSafe(path)
    .split('\n')
    .filter(Boolean)
    .map((l) => JSON.parse(l));
}

export default defineCase(
  'T73 内容侧可见域：矩阵成员读得到他人共享内容，判据随内容走（无共享内容照旧空切片）',
  async () => {
    const llm = await new MockLlm([
      { id: 'shared-answer', match: '共享认知提问', content: '这是共享会话的回答。' },
      { id: 'nos-answer', match: '无收束权提问', content: '这是无记忆会话的回答。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        // 每轮收束把事实写进事实源（actor = 会话主体）。
        session: { ...DIALOG_FACE_OFF },
      },
    });
    // reflex 智能体目录：会话绑定 `reflex` 时能力收集要求它存在（与 t30 / t71 同一硬约束）。
    // 目录只是准入；能力集以授权表的登记行为权威（`agent:reflex` = 只持 `judge.intent`）。
    makeAgentDir(hd.homedir, {
      id: 'reflex',
      name: '抢占判定者',
      providerId: PROVIDER_ID,
      providerPort: llm.port,
    });

    const base = { homedir: hd.homedir, workdir: hd.workdir, provider: PROVIDER_ID };
    const sharedWal = join(hd.homedir, 'session', SHARED_SID, V2_WAL);
    const nosharedWal = join(hd.homedir, 'session', NOSHARED_SID, V2_WAL);

    try {
      // ── ① 会话 A：正常一轮 ⇒ 步 11 把用户发言编成 `vis_scope = shared` 的记忆 ──
      const a = runCli({ ...base, session: SHARED_SID, message: '共享认知提问' });
      assertEq(a.code, 0, `会话 A 一轮退出码（stderr: ${a.stderr.slice(0, 400)}）`);

      // ── ② 会话 B：绑 reflex 跑一轮 ⇒ 发言臂收束被拒 ⇒ 无记忆，但开口格在 ──────
      // CLI 仍以 0 收尾：答案照旧回给调用方（收束格是记录不是呈现，见文件头）。
      const b = runCli({
        ...base,
        session: NOSHARED_SID,
        agent: 'reflex',
        message: '无收束权提问',
      });
      assertEq(b.code, 0, `会话 B 一轮退出码（stderr: ${b.stderr.slice(0, 400)}）`);

      // ── ③ 事实源直读（T-094 ① 的生产出口）：产地真的落进了 WAL ─────────────
      const sharedEvents = walLines(sharedWal);
      const sharedMemory = sharedEvents.find((e) => e.kind === 'memory.encoded');
      assert(
        sharedMemory,
        `会话 A 应有一条 memory.encoded（实际 kinds: ${JSON.stringify(sharedEvents.map((e) => e.kind))}）`,
      );
      assertEq(sharedMemory.payload.tag, '经验', '步 11 编码的是经验记忆（用户发言）');
      assertEq(
        sharedMemory.payload.vis_scope,
        'shared',
        '经验记忆的内容侧可见域 = shared（人的认知可供他人借用）',
      );

      const nosharedKinds = walLines(nosharedWal).map((e) => e.kind);
      assert(
        nosharedKinds.includes('user.message'),
        `会话 B 应有过轮次（开口格在，实际 kinds: ${JSON.stringify(nosharedKinds)}）`,
      );
      assert(
        !nosharedKinds.some((k) => k.startsWith('memory.')),
        `发言闸拒 ⇒ 记忆族一格都不落（实际 kinds: ${JSON.stringify(nosharedKinds)}）`,
      );

      // ── ④ 从生产路由读出口（gateway HTTP 边界，与前端同构）──────────────────
      const api = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SHARED_SID,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await api.waitGatewayReady();

        /** 以某个读方身份读某个会话的 stats（session_id 走 metadata，与 CLI 绑定的会话无关）。 */
        const readAs = (principal, sid) =>
          api.invoke('session/stats', { principal }, { session_id: sid });

        // ── 会话 A：同一条事实源、三种读法，差异只来自闸 ─────────────────────
        const ownerA = (await readAs('user', SHARED_SID)).body.data;
        assertEq(ownerA.session_id, SHARED_SID, '读的是会话 A');
        assert(ownerA.has_wal, '会话 A 事实源应在');
        assert(
          ownerA.checkpoint.event_count > 0,
          `属主看得到会话 A 的事实（实际: ${ownerA.checkpoint.event_count}）`,
        );

        // 矩阵内主体（`agent:main`）⇒ **读得到**他人的共享内容——T-094 ② 的正判据。
        // 读数与属主逐字相同：闸是整列判定，放行即放行整个快照。
        const mainA = (await readAs('agent:main', SHARED_SID)).body.data;
        assertEq(mainA.has_wal, true, 'agent:main: 事实源存在，只是今天给你看');
        assertEq(
          mainA.checkpoint.event_count,
          ownerA.checkpoint.event_count,
          'agent:main: 矩阵成员读得到他人共享内容 ⇒ 与属主读到同样多的事实',
        );
        assertEq(mainA.cost.total_ms, ownerA.cost.total_ms, 'agent:main: 成本列逐字相同');
        assertEq(mainA.tiers, ownerA.tiers, 'agent:main: 档位行逐字相同');

        // 矩阵外的未知主体 ⇒ 空切片（fail-closed 不因这次接线放宽）。
        // `has_wal` 仍为真 ⇒ 「有源但不给你看」与「没有源」可分辨。
        const ghostA = (await readAs('agent:ghost', SHARED_SID)).body.data;
        assertEq(ghostA.has_wal, true, 'agent:ghost: 事实源存在，只是不给你看');
        assertEq(ghostA.checkpoint.event_count, 0, 'agent:ghost: 断点列为空（越界读取 0）');
        assertEq(ghostA.cost.total_ms, 0, 'agent:ghost: 成本列为 0');
        assertEq(ghostA.tiers.length, 0, 'agent:ghost: 四列为空 ⇒ 无档位行');

        // ── 会话 B：同一读方、无共享内容 ⇒ 照旧空切片（判据随内容走的反判据）───
        const ownerB = (await readAs('user', NOSHARED_SID)).body.data;
        assertEq(ownerB.session_id, NOSHARED_SID, '读的是会话 B');
        assert(ownerB.has_wal, '会话 B 事实源应在（有过轮次）');
        assert(
          ownerB.checkpoint.event_count > 0,
          `属主看得到会话 B 的轮次事实（实际: ${ownerB.checkpoint.event_count}）——下面的拒绝不是「没东西可读」`,
        );

        const mainB = (await readAs('agent:main', NOSHARED_SID)).body.data;
        assertEq(mainB.has_wal, true, 'agent:main: 会话 B 事实源存在');
        assertEq(
          mainB.checkpoint.event_count,
          0,
          'agent:main: 会话 B 无共享内容 ⇒ 空切片（闸按内容侧可见域判，不是按读方恒放行）',
        );
        assertEq(mainB.tiers.length, 0, 'agent:main: 会话 B 无可见事实 ⇒ 无档位行');

        const ghostB = (await readAs('agent:ghost', NOSHARED_SID)).body.data;
        assertEq(ghostB.checkpoint.event_count, 0, 'agent:ghost: 会话 B 同样空切片');
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净：Windows 上带着活子进程 `process.exit()` 会撞
      // libuv 的 `uv_async_send` 断言，表现是**断言全过、退出码却是 0xC0000409**
      // ——判据过了却判失败。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
