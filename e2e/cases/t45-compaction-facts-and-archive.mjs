import './_selfrun.mjs';
// T45 压缩**入格 + 段归档**（S2.7-b2 判据，[ADR-053] 决策 6）：
// 真实压缩触发 ⇒ 快照落 `session.consolidated`（`message × progressed`）⇒
// 被取代的前缀移出 live 段、进归档段 ⇒ 删掉 `messages.json` 后重建 =
// **快照 + 其后事实**（压缩前的轮次**不复活**）。
//
// ## 为什么压缩必须入格
//
// 压缩此前只重写 `messages.json`（`replace_messages`）——**没进事实源**。于是
// 「主读路径整体切到重建」时，被压掉的那一段会**复活**（WAL 里从没记过它没了），
// 而模型看到的 prompt 却是压缩后的短历史 ⇒ 两条真源各说各话。
//
// ## 本用例钉的四件事
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 压缩进了事实源 | live WAL 里有 `session.consolidated` 事件 |
// | 快照**取代**了前缀 | live WAL 里**没有**压缩前的用户消息；首条就是快照 |
// | 被取代的前缀没丢 | 归档段目录里有段文件（ADR-053 b2 行的「零回退损失」） |
// | 切源后历史是压缩后那份 | 删缓存后读会话叶子：快照在、压缩前的轮次不复活 |
//
// ## 为什么读 live WAL 而不是 `history()`
//
// 本用例证的是「live 段就是全史」（快照取代前缀）——那正是 b2 选的语义。
// 归档段是审计 / 回退的依据，**不参与常规读**（见 `WalStore::archive_before`）。
import { join } from 'node:path';
import { rmSync, existsSync, readdirSync } from 'node:fs';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  waitFor,
  readFileSyncSafe,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t45';

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

/** `messages.json`（与 `plugins/session/store/mod.rs::MESSAGES_FILE` 同一常量口径） */
const MESSAGES_JSON = 'messages.json';

/** 归档段子目录（与 `symbio_core/store/wal.rs::WAL_ARCHIVE_SUBDIR` 同一常量口径） */
const WAL_ARCHIVE_SUBDIR = join('transcripts', 'wal-archive');

/** 推高水位的大段回复（CJK ≈ 1 token/字，见 t8 的实测口径） */
const REPLY = `填充：${'上下文填充内容用于推高历史水位。'.repeat(600)}`;

/** 压缩请求的结构化快照应答（带一个可检索的标记，证摘要真的活到了重建后） */
const SNAPSHOT_MARKER = 'T45快照标记';
const SNAPSHOT_XML = [
  '<state_snapshot>',
  '  <overall_goal>验证压缩入格与段归档端到端可用。</overall_goal>',
  '  <key_knowledge>',
  `    - ${SNAPSHOT_MARKER}：压缩后的历史记忆替身。`,
  '  </key_knowledge>',
  '  <completed_items>',
  '    - 压缩前的轮次已被摘要取代。',
  '  </completed_items>',
  '  <in_progress_items>',
  '    - 等待压缩后的下一轮对话验证记忆延续。',
  '  </in_progress_items>',
  '  <open_questions></open_questions>',
  '</state_snapshot>',
].join('\n');

/** 压缩前的那一问（压缩后**必须**从 live 段消失——它是被取代的前缀）。 */
const Q_FIRST = '第一轮问题';

/** 读一份 WAL 文件的行（逐行 JSON）。 */
function walLines(path) {
  return readFileSyncSafe(path)
    .split('\n')
    .filter(Boolean)
    .map((l) => JSON.parse(l));
}

/** 取消息的正文文本（与 helpers 的 `textOf` 同口径，就地复算以保持用例自洽）。 */
function contentText(m) {
  const c = m.content;
  if (c == null) return '';
  if (typeof c === 'string') return c;
  if (Array.isArray(c)) return c.map((p) => p.text ?? '').join('');
  return JSON.stringify(c);
}

export default defineCase(
  'T45 压缩入格 + 段归档：快照落 message × progressed，被取代的前缀进归档段，删缓存后重建 = 快照 + 其后事实',
  async () => {
    const llm = await new MockLlm([
      // 压缩请求：末条 user 是压缩指令（compression.rs 唯一措辞），返回结构化快照
      { id: 'snapshot', match: 'Distill the conversation above', content: SNAPSHOT_XML },
      // 普通轮：回复超大文本，把历史水位顶过 70% 阈值
      { id: 'big', match: '轮问题', content: REPLY },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [
        { id: PROVIDER_ID, config: providerConfig(llm.port, { max_context_tokens: 16000 }) },
      ],
      pluginConfigs: {
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        // context_messages: 0 = 上下文窗口不按轮次截断（压缩判定的原料是完整历史）
        session: { ...DIALOG_FACE_OFF, auto_compress: true, context_messages: 0 },
      },
    });

    const base = { homedir: hd.homedir, workdir: hd.workdir, provider: PROVIDER_ID };
    const dir = join(hd.homedir, 'session', SID);
    const messagesPath = join(dir, MESSAGES_JSON);
    const walPath = join(dir, V2_WAL);
    const archiveDir = join(dir, WAL_ARCHIVE_SUBDIR);

    try {
      // ── 跑四轮（水位越过 70% ⇒ 中途自动压缩）──────────────────────────────
      const r = runCli({
        ...base,
        session: SID,
        stdinText: `${Q_FIRST}\n第二轮问题\n第三轮问题\n第四轮问题\n`,
        timeoutMs: 120_000,
      });
      assertEq(r.code, 0, `CLI 退出码（stderr 尾部: ${r.stderr.slice(-600)}）`);
      assert(
        r.stdout.includes('填充：'),
        `压缩触发的当前轮应照常完成回复（stdout 尾部: ${r.stdout.slice(-200)}）`,
      );

      // ── ① 落格：live WAL 里有 `session.consolidated`，格子 = message × progressed ──
      assert(existsSync(walPath), `跑过轮次 ⇒ 事实源应在：${walPath}`);
      const live = walLines(walPath);
      const snaps = live.filter((e) => e.kind === 'session.consolidated');
      assert(snaps.length >= 1, `压缩必须落一条事实（现有 kind: ${JSON.stringify([...new Set(live.map((e) => e.kind))])}）`);
      assertEq(snaps[0].entity, 'Message', '快照实体的格子 = message');
      assertEq(snaps[0].verb, 'Progressed', '快照动词的格子 = progressed');
      assertEq(snaps[0].turn, 0, '压缩不属于任何一轮（turn = 0）');
      assert(
        Array.isArray(snaps[0].payload?.messages) && snaps[0].payload.messages.length > 0,
        `快照载荷应带压缩后的消息列表（实际: ${JSON.stringify(snaps[0].payload)?.slice(0, 300)}）`,
      );
      assert(
        typeof snaps[0].payload?.turns === 'number' && snaps[0].payload.turns >= 1,
        `快照载荷应带轮次水位（归档后轮号靠它续上，实际: ${JSON.stringify(snaps[0].payload?.turns)}）`,
      );

      // ── ② 取代：live 段以快照开头，且**不含**压缩前的用户格 ──────────────
      //
      // 判据只看 `user.message`（**轮次**事实），不看原始文本：记忆事实
      // （`memory.consolidated` 载荷里会引用用户原话）是**另一族**事实，它跨压缩
      // 保留恰恰是对的——压缩取代的是**消息历史**，不是记忆。
      assertEq(
        live[0].kind,
        'session.consolidated',
        `live 段应以快照开头（被取代的前缀已移走，实际首条: ${live[0].kind}）`,
      );
      const liveUserFacts = live.filter((e) => e.kind === 'user.message');
      assert(
        !liveUserFacts.some((e) => JSON.stringify(e.payload).includes(Q_FIRST)),
        `压缩前的用户格不得留在 live 段（实际: ${JSON.stringify(
          liveUserFacts.map((e) => e.payload?.text),
        )}）`,
      );

      // ── ③ 零回退损失：被取代的前缀在归档段里，不重不漏 ────────────────────
      assert(existsSync(archiveDir), `归档段目录应存在：${archiveDir}`);
      const segs = readdirSync(archiveDir).filter((n) => n.startsWith(`${V2_WAL}.`));
      assert(segs.length >= 1, `应有归档段（实际: ${JSON.stringify(readdirSync(archiveDir))}）`);
      const archived = segs.flatMap((n) => walLines(join(archiveDir, n)));
      assert(
        archived.some((e) => e.kind === 'user.message' && JSON.stringify(e.payload).includes(Q_FIRST)),
        '压缩前的用户消息应在归档段里（「零回退损失」的依据）',
      );
      // 段与 live 不重叠：归档段的号恒严格小于 live 的首号。
      const archivedMax = Math.max(...archived.map((e) => e.seq ?? 0));
      const liveMin = Math.min(...live.map((e) => e.seq ?? Number.MAX_SAFE_INTEGER));
      assert(
        archivedMax < liveMin,
        `归档段与 live 段的号区间不得重叠（归档 max=${archivedMax}, live min=${liveMin}）`,
      );

      // ── ④ 切源后历史 = 快照 + 其后事实（压缩前的轮次不复活）────────────────
      const cli = startLongLivedCli({ ...base, session: SID, gatewayPort: GATEWAY_PORT });
      try {
        await cli.waitGatewayReady();
        const rootPath = (await cli.invoke('vdfs/root', {})).body?.data?.path;
        assert(typeof rootPath === 'string' && rootPath.length > 0, 'vdfs/root 应返回根地址');
        const sessionAddr = `${rootPath.replace(/\/+$/, '')}/session/${SID}`;

        const readLeaf = async (what) => {
          const resp = await cli.invoke('vdfs/read', { path: sessionAddr });
          assertEq(resp.status, 200, `${what} 应成功（${JSON.stringify(resp.body)?.slice(0, 300)}）`);
          const text = resp.body?.data?.text;
          assert(
            typeof text === 'string' && text.length > 0,
            `${what} 应带回文本正文（${JSON.stringify(resp.body)?.slice(0, 300)}）`,
          );
          return JSON.parse(text);
        };

        rmSync(messagesPath, { force: true });
        assert(!existsSync(messagesPath), '手术刀必须真的删掉缓存文件');

        const rebuilt = await readLeaf('删缓存后读会话叶子');
        const texts = rebuilt.messages.map(contentText).join('\n');
        assert(
          texts.includes('CONTEXT SNAPSHOT'),
          `重建的历史应含压缩快照（实际: ${texts.slice(0, 400)}）`,
        );
        assert(
          texts.includes(SNAPSHOT_MARKER),
          `重建的历史应含快照正文（摘要真的活到了重建后，实际: ${texts.slice(0, 400)}）`,
        );
        assert(
          !texts.includes(Q_FIRST),
          `重建后压缩前的轮次**不得复活**（否则压缩没入格，实际: ${texts.slice(0, 400)}）`,
        );

        // 读路径无写副作用：读完缓存仍不该存在。
        assert(
          !existsSync(messagesPath),
          '读出口不得重建缓存——缓存缺失时读的是 WAL 重建结果',
        );
      } finally {
        cli.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净（与 t37 / t43 / t44 同一约定：Windows 上带活子进程
      // 退出会撞 libuv 断言，表现为「断言全过、退出码 0xC0000409」）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);
