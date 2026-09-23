/**
 * 回读的**理由**（闭集）——三个回读动词（`listVdfs` / `statVdfs` / `readVdfs`）的
 * 必填首参，随请求进 `metadata.origin`，由 Tauri 侧的 `Start routing` 留痕打出。
 *
 * ## 为什么要有它
 *
 * `vdfs/list` / `stat` / `read` 的调用方有好几类，路由留痕里却只有一个路由名。
 * 于是「一轮会话里出现多次 `vdfs/stat`，正常吗」这类问题只能从时间戳反推，
 * 而反推最容易错的地方正是**同一个动词的不同理由**——「缺基线补读」是设计内
 * 必要的，「无载荷资源信号分辨删除」也是，但「每帧都回读」就是缺陷。三者
 * 在日志里长得一模一样。
 *
 * 把理由作为**必填形参**（而不是注释、不是约定）后：漏填是编译错误；一旦发出，
 * `Start routing` 那一行直接给出答案。理由因此成为**事实**，不是推断。
 *
 * ## 为什么单独成一个模块
 *
 * 它是**词表**，不是服务：不依赖任何东西，也不做任何事。实现方
 * （`services/vdfs.ts` 的三个动词）与消费方（各 store / composable）都从这一处
 * 取值——于是测试替身不必把这份闭集**再抄一遍**（抄一份就是第二份真相，
 * 词表一改，替身照旧绿）。
 *
 * ## 取值怎么定
 *
 * 按「**什么触发**」分，不按「谁调用」分——同一个模块在不同触发下理由不同
 * （`sessionTranscriptSync` 就同时是 `MISSING_BASELINE` 与 `IDENTITY_UNKNOWN`），
 * 而「谁」已由代码位置给出。新增一个调用方时，先问它属于哪一类；都不属于才加取值。
 */
export const READBACK_REASON = {
  /** 引导：解析根锚点 / 会话挂载点 / 转写段。应用启动期一次性，之后走缓存 */
  BOOTSTRAP: 'bootstrap',
  /** 实时面**缺基线**：状态帧到达，本端没有该节点（首帧丢失或订阅晚于节点出现） */
  MISSING_BASELINE: 'missing-baseline',
  /** 实时面**身份未知**：窄增量帧到达，本端没有该节点（只有 delta，没有身份） */
  IDENTITY_UNKNOWN: 'identity-unknown',
  /** **资源信号**：无载荷的会话节点变更（自动命名 / metadata / 删除），回读只分辨删除 */
  RESOURCE_SIGNAL: 'resource-signal',
  /** **清单刷新**：会话列表整表重拉（打开界面 / 资源变更后的防抖收敛） */
  LIST_REFRESH: 'list-refresh',
  /** **用户动作**：VDFS 浏览器的导航 / 分页 / 选中 / 手动刷新 */
  VDFS_BROWSER: 'vdfs-browser',
  /** **整份转写重读**：打开会话时取历史（与实时面无关） */
  TRANSCRIPT_LOAD: 'transcript-load',
} as const

export type ReadbackReason = (typeof READBACK_REASON)[keyof typeof READBACK_REASON]
