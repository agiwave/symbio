import './_selfrun.mjs';
// T13 会话选项（schema 化）——「定义 / 值 / 落库」三条通路的真实边界验证。
//
// 会话选项在 2026-09-23 之前走一套**独立于其它资源**的节点协议
// （`options/list` + `OptionNode`），已整体下线。现在它就是**会话配置表单的字段**：
//
//   定义   <根>/session/<id> → node.schema           （已落盘会话）
//          <根>/session      → new_type.schema       （新建草稿）
//   当前值 <根>/session/<id> → node 上的 metadata     （键 = 定义里的字段 key）
//   落库   vdfs/write(<根>/session/<id>, { metadata: … })
//
// ⚠️ 「值在节点上」的**层级**容易写错：后端 `VdfsNode.attributes` 是
// `#[serde(flatten)]` 的，线上（以及前端 `services/session.ts`）看到的是
// **摊平**后的节点——`metadata` / `message_count` / `meta_tags` 都是节点的顶层键，
// 没有 `attributes` 这一层。本用例断言的是**线上形状**，故直接读 `node.metadata`。
//
// 本用例**完全在 gateway HTTP 边界上**跑完这条链（不经过任何前端代码），
// 因此它验证的是机制本身，不是某个渲染器的行为。
//
// | 断言 | 意图 |
// |---|---|
// | 草稿定义挂在 `new_type.schema` | 「新建会话」与「打开会话」进入的是同一份定义 |
// | 清单里每一项的 `schema` 与草稿**逐字节相同** | 一处真相、两处投递 |
// | 字段 key 含会话自有的四项、且各有 `widget` | 定义真的被收集到（不是空壳） |
// | `create: true` 时 metadata 随创建写入 | 草稿态的选择不丢 |
// | 覆盖写入是**浅合并**（改一项不抹掉其它项） | 改一项不该抹掉另一项 |
// | `stat` **不带** `schema`、但带 `metadata` | 定义只挂两处载体；`stat` 是热路径，不跑全项目收集 |
// | 重启进程后值仍在 | 「落库」真的落到了盘上 |
//
// ⚠️ 最后一条（`stat` 不带 schema）锁的是一个**刻意的设计选择**（见
// `docs/archive/session-options-unification.md` §7 / §9 S2 实施注记 3）：定义要经一次
// 全项目广播（含 agent 目录扫描），挂到 `stat` 上等于给每次变更通知加一次目录 I/O。
// 若将来确实要改，请同步改设计文档并评估那个代价——不要只是把这条断言删掉。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
  assert,
  assertEq,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

/** 会话插件**自有**的四项（其余两项由 agent / model 插件贡献，见 §9.3） */
const OWN_FIELD_KEYS = ['workdir', 'risk_level', 'mode', 'heartbeat'];

/** 草稿态选中的两项：一项随 `create: true` 写入，一项留给后面的覆盖写入验证浅合并 */
const DRAFT = { risk_level: 'low' };

/** gateway 入站配置（与其它用例同款：本机回环、无 token） */
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
  'T13 会话选项（schema 化）：定义随节点下发 + 值随节点回读 + vdfs/write 落库 + 重启后仍在',
  async () => {
    // 本用例不发言，mock-llm 只是让 model 插件有一个可用 Provider（否则
    // `provider_id` 字段退化为「未配置」占位候选，与本用例无关但会干扰阅读）
    const llm = await new MockLlm([{ id: 'unused', match: '（本用例不发言）', content: '—' }]).start();
    // gateway 的**监听端口**来自插件配置，`startLongLivedCli` 的 `gatewayPort` 只是
    // 「去哪个端口探活 / 调谁」——两者必须是同一个数字，故这里算一次、两处共用。
    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: { gateway: gatewayConfig(GATEWAY_PORT) },
    });

    /** 起一个 CLI 并取回它的 gateway 句柄（重启验证要起两次） */
    const bootCli = () =>
      startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: 'unused-t13',
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });

    /** 解信封：`PluginPayloadWire = { type: 'Data', data: <载荷> }` */
    const dataOf = (resp, what) => {
      assertEq(resp.status, 200, `${what} 应受理（${resp.status}）`);
      const d = resp.body?.data;
      assert(d != null, `${what} 应返回 Data 载荷（${JSON.stringify(resp.body)?.slice(0, 300)}）`);
      return d;
    };

    let cli = bootCli();
    try {
      await cli.waitGatewayReady();

      // ① 进入地址空间：根地址是**运行期数据**（根叫什么归 vdfs 插件，使用方不写死）
      const rootPath = dataOf(await cli.invoke('vdfs/root', {}), 'vdfs/root').path;
      assert(typeof rootPath === 'string' && rootPath.length > 0, 'vdfs/root 应返回根地址');
      const mount = `${rootPath.replace(/\/+$/, '')}/session`;

      /** 列会话清单（定义与值都在 `items` 上；新建类型在 `node` 上） */
      const listMount = async (what) => dataOf(await cli.invoke('vdfs/list', { path: mount }), what);

      // ② 草稿定义：`new_type.schema`——新建会话前选项栏就要能完整渲染
      const draftList = await listMount('vdfs/list(<根>/session)');
      const sessionType = draftList.node?.new_type;
      assert(
        sessionType && sessionType.ext === 'session',
        `会话挂载目录应声明可新建「会话」类型（实际 new_type: ${JSON.stringify(draftList.node?.new_type)}）`,
      );
      const draftSchema = sessionType.schema;
      assert(draftSchema, `新建类型应携带选项定义（schema），实际 ${JSON.stringify(sessionType)}`);
      assertEq(draftSchema.binding, 'option', '选项定义应是 option 绑定（值来自节点 metadata）');
      const draftKeys = (draftSchema.sections ?? []).flatMap((s) => s.fields ?? []).map((f) => f.key);
      for (const k of OWN_FIELD_KEYS) {
        assert(draftKeys.includes(k), `草稿定义应含会话自有字段 ${k}（实际 ${JSON.stringify(draftKeys)}）`);
      }
      for (const f of (draftSchema.sections ?? []).flatMap((s) => s.fields ?? [])) {
        assert(
          typeof f.widget === 'string' && f.widget.length > 0,
          `字段 ${f.key} 必须声明 widget（紧凑渲染器按它分派交互）`,
        );
      }

      // ③ 草稿态落库：`create: true` + metadata —— 新建会话前选的值随创建一次写入
      const created = dataOf(
        await cli.invoke('vdfs/write', {
          path: mount,
          create: true,
          text: JSON.stringify({ metadata: DRAFT }),
        }),
        'vdfs/write(create)',
      );
      // 匿名写（打在会话挂载根上）→ 回执只给**名字**（provider 生成的 id）；
      // 地址由调用方拿自己的请求目录 + 这个名字拼
      const sid = created.name;
      assert(typeof sid === 'string' && sid.length > 0, '新建应回执会话名');
      assertEq(created.created, true, '新建应回执 created=true');
      assert(sid && sid !== 'session', `会话 id 应由 provider 生成（实得 ${sid}）`);
      const sessionPath = `${mount}/${sid}`;

      const findItem = (list, what) => {
        const item = (list.items ?? []).find((n) => n.name === sid);
        assert(
          item,
          `${what} 的清单里应有会话 ${sid}（实际 ${JSON.stringify((list.items ?? []).map((n) => n.path))}）`,
        );
        return item;
      };

      // ④ 已落盘会话：**同一份定义**（逐字节）+ 草稿选择已在 metadata 里
      const afterCreate = await listMount('vdfs/list（新建后）');
      const item = findItem(afterCreate, '新建后');
      assert(item.schema, `清单里的会话节点应携带选项定义（实际字段：${Object.keys(item).join(',')}）`);
      assertEq(
        JSON.stringify(item.schema),
        JSON.stringify(draftSchema),
        '清单里的定义必须与草稿定义逐字节相同（一处真相、两处投递）',
      );
      const meta = item.metadata ?? {};
      assertEq(meta.risk_level, 'low', '草稿态选的风险等级应随 create 一次写入 metadata');

      // ⑤ 选项落库：`vdfs/write(<会话地址>, { metadata })` —— 与前端选项栏同一条路径
      dataOf(
        await cli.invoke('vdfs/write', {
          path: sessionPath,
          text: JSON.stringify({ metadata: { mode: 'auto' } }),
        }),
        'vdfs/write(覆盖)',
      );
      const afterWrite = await listMount('vdfs/list（覆盖写入后）');
      const wm = findItem(afterWrite, '覆盖写入后').metadata ?? {};
      assertEq(wm.mode, 'auto', '写入的 mode 应回读得到');
      assertEq(wm.risk_level, 'low', '覆盖写入是**浅合并**：未提供的键必须保持不变');

      // ⑥ `stat` 不带定义（热路径设计），但带 `metadata`（值随节点）
      const stat = dataOf(await cli.invoke('vdfs/stat', { path: sessionPath }), 'vdfs/stat');
      assert(
        stat.schema == null,
        '`stat` 不应携带选项定义（定义只挂 list 与 new_type 两处；stat 是热路径，不跑全项目收集）',
      );
      assertEq(stat.metadata?.mode, 'auto', '`stat` 应带回当前值（值随节点下发）');

      // ⑦ 重启进程：值仍在 —— 「落库」真的落到了盘上，不是进程内状态
      cli.stop();
      cli = bootCli();
      await cli.waitGatewayReady();

      const afterRestart = await listMount('vdfs/list（重启后）');
      const rm = findItem(afterRestart, '重启后').metadata ?? {};
      assertEq(rm.mode, 'auto', '重启后 mode 应仍在（落库已落盘）');
      assertEq(rm.risk_level, 'low', '重启后 risk_level 应仍在');
      assert(
        findItem(afterRestart, '重启后').schema != null,
        '重启后定义仍应随清单下发（定义是后端的，不随进程生命周期）',
      );
    } finally {
      cli.stop();
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
