# VDFS 前端页面规范与迁移路线

状态：现行规划（页面规范 + 迁移路线）
范围：Symbio 前端的「三栏资源页」及其从既有 `entities/*` 协议向 `vdfs/*` 的收敛
关联：
`docs/design/vdfs.md`（**机制规范，权威**：接口、访问位、协议操作、分层）、
`symbio/src/symbio_core/vdfs_provider.rs`（纯接口）、
`symbio/src/plugins/vdfs/protocol.rs`（线路信封）、
`tauri/src/schemas/vdfs.ts`（前端数据契约）、
`tauri/src/composables/useVdfs.ts`（页面逻辑）、
`tauri/src/views/VdfsView.vue`（页面装配）

> 本文件只写**前端页面规范**与**迁移路线**。机制（trait、访问位、协议操作）
> 一律以 `vdfs.md` 为准，本文件不重复、不覆盖。凡与本文件冲突，以 `vdfs.md`
> 的机制条款为准。

---

## 1. 目标

**一个协议、一个页面、一处注册**——把「项目里所有的资源与数据」收敛为同一棵
虚拟树上的节点，让**前端与大语言模型用同一组操作、同一种寻址方式**访问，
从而让整个前端收敛为「通用资源状态查看 / 管理工具」：新增一类资源只需后端
新增一个 `VdfsProvider`，前端零页面开发。

最终目标：**逐步替代前后端现有的几乎全部既有协议（`entities/*` 等），全面
拥抱 VDFS**。迁移期两者并存，新能力一律接入 VDFS，逐模块迁移后下线旧协议。

不变量（承 `vdfs.md` §1，前端侧重申）：

1. **路径是唯一地址**：节点地址 = `.vdfs/<挂载点>/<相对路径>`（前端口径，
   见 §3）。
2. **能力只看访问位**：前端**不得**按 `kind` 判定能力，只能依据 `access`（`r/w/l/t`）。
3. **ext 决定详情**：节点 `ext` 是前端选择详情渲染器的**唯一键**。
4. **前端零资源知识**：前端只持有 `ext → 渲染器`、`挂载名 → 图标` 这类纯 UI
   映射；**不得**硬编码资源类型清单、标签、路径模板、能力开关。
5. **实时靠订阅，禁止轮询**：列表刷新一律经事件总线 `vdfs` 频道 + `vdfs/watch`。

---

## 2. 现状评估

### 2.1 已实现（后端，已暂存）

| 层 | 落点 | 状态 |
|---|---|---|
| 纯接口 | `symbio_core/vdfs_provider.rs` | `VdfsProvider` trait + 域类型 + `VdfsMountTable` |
| 宿主桥 | `symbio_core/vdfs/host.rs` | 上下文注入 + 错误翻译 |
| 线路信封 | `plugins/vdfs/protocol.rs` | 13 个 `vdfs/*` 操作 + 使用方形状 |
| 访问层 | `plugins/vdfs/host.rs` | 取根 + 翻译操作 + 树遍历 + 事件投递 |
| 拓扑 | `plugins/composite/vdfs.rs` | 逐子插件收集挂载 → 组合为根 |
| LLM 工具 | `plugins/vdfs/tools/*` | 11 个 `vdfs_*` 工具 + `ToolVdfs` 地址翻译 |
| 范例 provider | `plugins/setting`、`plugins/local` | 设置分区、本地文件树 |

### 2.2 已实现（前端，已落地）

| 文件 | 职责 | 评估 |
|---|---|---|
| `schemas/vdfs.ts` | 数据契约 + 路径代数 | **保留**（扩展） |
| `services/vdfs.ts` | 路径 → 请求的机械翻译 | **保留**（扩展） |
| `composables/useVdfs.ts` | 页面逻辑（挂载点 / 目录 / 选中 / 详情 / 实时） | **保留**（扩展） |
| `registry/vdfsTypes.ts` | `ext → 渲染器标识`（零组件导入） | **保留** |
| `registry/vdfsRenderers.ts` | `标识 → 组件`（唯一装配点） | **保留** |
| `components/vdfs/*.vue` | form / text / session / readonly 四个详情渲染器 | **保留** |
| `views/VdfsView.vue` | 三栏装配（**嵌入主布局** / 独立整页两种形态） | **保留**（扩展） |
| `router/index.ts` | `/vdfs/:mount?`（S3 起嵌入 `MainLayout` 子路由） | **保留** |

结论：前端既有改动**方向正确、结构合理**，与目标一致的部分**整体保留**，
G1–G3 三处差距已按 §3–§6 补齐（见 §7.1 的 S1）。

### 2.3 与目标的差距

| # | 差距 | 现状 | 目标 | 状态 |
|---|---|---|---|---|
| G1 | **地址口径** | 前端以 `/` 为根 | 以 `.vdfs` 为根，与 LLM 地址模型统一（§3） | **已补齐** |
| G2 | **可接受的新建类型** | 后端无此概念；前端只有「新建目录」 | 节点 / 挂载点声明可新建类型；前端据此出添加按钮与类型选择器（§5） | **已补齐** |
| G3 | **导航元数据** | 左栏仅图标 + 标题 | 导航项携带名字 / 标题 / 图标 / 描述 / 可新建类型（§4.1） | **已补齐** |

### 2.4 取舍

- **保留继续**：§2.2 全部文件（含路径代数、渲染器分层、实时订阅、校验错误消费）。
- **放弃**：无（既有前端改动未发现与目标冲突者）。
- **待补**：无（G1–G3 已在 S1 落地）。

---

## 3. 地址模型：`.vdfs`

### 3.1 前端口径

前端以 **`.vdfs` 为虚拟根**，地址形如：

```
.vdfs                          # 首页数据源：挂载点清单（资源类别）
.vdfs/session                  # 会话挂载点的列表（下一级）
.vdfs/session/<id>             # 会话详情
.vdfs/setting/appearance       # 设置分区
```

这与大语言模型侧 `ToolVdfs` 的**虚拟地址前缀**（`VIRTUAL_PREFIX = ".vdfs/"`）
**同源**：前端与 LLM 因此使用**同一种寻址方式**（`vdfs.md` 不变量 5 的落地）。

### 3.2 与线路协议的关系（关键）

**线路协议不变**：`vdfs/*` 的 `path` 仍是**规范化全路径** `/…`（`vdfs.md` §3.1、
不变量 3）。`.vdfs` 是**前端 / LLM 的地址口径**，二者在**服务层一处**完成翻译：

```
前端地址  .vdfs/session/abc   ──toWirePath──▶   /session/abc   （线路 path）
线路返回  /session/abc        ──toVdfsPath──▶   .vdfs/session/abc（前端节点 path）
```

- 翻译点**唯一**：`services/vdfs.ts`（请求出站前 `toWirePath`；响应入站后
  `toVdfsPath` 递归改写 `path` / `root` / `to` 等字段）。
- 页面逻辑、渲染器、路径代数**一律只认 `.vdfs` 口径**，不认识 `/` 口径。
- 后端机制**零改动**——`.vdfs` 不进机制、不进 provider、不进协议。

### 3.3 路径代数

`schemas/vdfs.ts` 的路径工具（`vdfsJoin` / `vdfsParent` / `vdfsBase` /
`vdfsMountOf`）是**全部路径运算的唯一来源**，统一以 `.vdfs` 为根。
挂载名 = `.vdfs/` 之后的首段。

---

## 4. 三栏页面规范

沿用三栏结构（**左栏导航 / 中栏列表·树 / 右栏详情**），数据源与协议按本节调整。

### 4.1 左栏：导航（挂载点清单）

- **数据源**：`.vdfs` 的目录内容（= `vdfs/providers`，或等价地 `vdfs/list { path: ".vdfs" }`）。
- **每一项携带**：`name`（挂载名）、`title`（展示标题）、`icon`（纯 UI 映射）、
  `description`（语义说明）、`new_types`（可接受的新建类型，§5）。
- **渲染**：图标 + 标题为主，`description` 作 tooltip；选中态 = 当前挂载点。
- **点击**：进入 `.vdfs/<挂载名>`（中栏显示其列表）。
- 挂载点集合、顺序、标签**全部由后端下发**，前端零硬编码。
- **装配形态（S3 起）**：本栏由**应用外壳**承担（`MainLayout` 的 `NavRail`），VDFS 页
  作为工作区内容**嵌入**其中——全 App 因此只有一台三栏工作台，页面切换不替换外壳。
  `VdfsView` 保留**独立整页**形态（自渲染本栏 + 返回键），供将来以独立窗口 / 面板复用。
  过渡期说明：应用外壳的导航项**暂仍来自 `entities/providers` 注册表**
  （`useNavRailItems`），因为 `model` / `agent` / `skill` / `mcp` 尚未有 VDFS provider（§7 S4）；
  此时若切到 `.vdfs` 根会丢失这些导航项。待 S4 补齐后，左栏一并切到 `.vdfs` 根，S5 下线 `entities/*`。

### 4.2 中栏：列表 / 树

- **数据源**：当前目录地址 `.vdfs/<…>` 的目录内容（`vdfs/list`）；递归形态用
  `vdfs/tree`（只下钻访问位含 `t` 的目录）。
- **每一项**：`name` / `title` / `description` / `ext` / `access` / `status` / `new_types`。
- **形态**：目录（`access` 含 `l`）→ 点击进入；文件 → 点击选中（右栏出详情）。
- **列表头**：按当前节点的 `new_types` 显示**添加按钮**（§5）；可写（`access` 含 `w`）
  时保留「新建目录」。
- **导航**：面包屑（`.vdfs` → … → 当前目录），路径即导航。

### 4.3 右栏：详情（按 ext 分发）

- **唯一分发键**：节点 `ext`（缺省由 `name` 推导）。分层不变：
  `schemas/vdfs.ts`（契约）→ `registry/vdfsTypes.ts`（ext → 渲染器标识，零组件）
  → `registry/vdfsRenderers.ts`（标识 → 组件，唯一装配点）。
- **既有渲染器**：`form`（定义驱动表单，解析 `node.schema`）、`session`（会话工作区）、
  `markdown` / `json` / `text`（文本类编辑器）、`appearance` / `about`（前端状态自持的
  专属面板）、`dir` / `fallback`（只读兜底）。
- **校验错误消费**：`write` 失败时按 `parseVdfsValidation` 还原字段级错误，
  逐字段高亮（`vdfs.md` §8）。

---

## 5. 可接受的新建类型（`new_types`）

### 5.1 语义

一个**目录节点**（含挂载点根）可以声明「**本目录可接受的新建元素类型**」——
一个类型清单，每项以**扩展名 `ext`** 标识（用户口径：「可接受的新建元素类型
（扩展名）」）：

- 清单**非空** → 中栏列表头显示**添加按钮**；
- 清单**多于一项** → 点击添加按钮后**先选类型**，再填写名称；
- 清单**恰好一项** → 直接进入该类型的命名；
- 清单**为空** → 不显示添加按钮（该目录由系统管理）。

### 5.2 域类型（后端）

```rust
/// 目录可接受的新建元素类型。
pub struct VdfsNewType {
    /// 新元素扩展名（决定创建后的详情渲染器）
    pub ext: String,
    /// 展示标题
    pub title: String,
    /// 语义说明（缺省不显示）
    pub description: Option<String>,
    /// 图标名（纯 UI 映射）
    pub icon: Option<String>,
}
```

- 挂在 `VdfsNode.new_types`（目录节点；文件节点为空、不序列化）。
- provider **根**的类型清单经 trait 方法 `root_new_types()` 声明（与
  `root_access` / `root_status` 同构），由容器在合成**挂载点节点**时回填。
- `VdfsMountInfo`（`vdfs/providers` 的使用方形状）同步下发 `new_types`。

### 5.3 创建动作（协议不变）

新建 = **对目标地址的一次 `vdfs/write`**（`create: true`）：

```
新建类型 ext 的新元素，名称 name，于当前目录 dir：
  目标地址 = vdfsJoin(dir, `${name}.${ext}`)
  vdfs/write { path: 目标地址, text: <默认内容，缺省空>, create: true }
```

- **创建语义由 provider 自持**：文件系统 provider 落为文件；会话 provider 落为
  会话；设置 provider 可拒绝（无 `w` 位 / 无类型声明）。
- 前端**不**认识任何具体类型——只负责「选类型 + 填名称 + 组装地址 + 发写请求」。
- 目录（无扩展名的结构节点）不属于「新建类型」，仍走 `vdfs/mkdir`。

### 5.4 前端职责

| 层 | 职责 |
|---|---|
| `schemas/vdfs.ts` | `VdfsNewType` 类型 + `new_types` 字段 + 解析 |
| `composables/useVdfs.ts` | 新建态机（选类型 → 填名 → 提交）+ 当前目录可新建类型（`creatableTypes`） |
| `views/VdfsView.vue` | 添加按钮可见性 + 类型选择器 + 命名输入 + 地址预览 |

---

## 6. 实时性

- **列表刷新靠订阅，非轮询**：视图订阅总线 `vdfs` 频道（`kind = "vdfs"`），
  按**挂载名**过滤、按**路径**精确刷新；订阅范围与当前目录严格绑定
  （切目录 = 解旧订阅 + 登新订阅）。
- **订阅地址用前端口径**（`.vdfs/session`），服务层翻译为线路 path（`/session`）。
- 后端对无实时能力的 provider 默认 no-op，前端无需按 provider 分流。
- 细粒度帧**不得**触发全量刷新；一律防抖重拉当前目录收敛。

---

## 7. 迁移路线（自顶向下，逐步推进）

原则：**先立页面规范（顶），再逐个迁移 provider（底）**；每步独立可验证、
可提交，绝不半途并存两套页面机制。

| 阶段 | 内容 | 产出 |
|---|---|---|
| **S1 页面规范地基** | 地址模型 `.vdfs`（§3）+ 导航元数据（§4.1）+ 可新建类型（§5）+ 添加/类型选择器 | 前端页面机制就位；后端 `new_types` 机制就位 |
| **S2 设置迁移** | `setting` 已是 provider；把设置入口从 `/settings`（entities）切到 `/vdfs/setting` | 设置页走 VDFS；`entities/setting` 退场 |
| **S3 会话迁移** | 新增 `session` provider：根 = 会话清单（`new_types = [会话]`）、节点 = 会话（`ext = session`）；read/write/delete 转发既有会话协议 | 会话页走 VDFS；聊天工作区作为 `session` 渲染器 |
| **S4 其余迁移** | 逐个新增 `model` / `agent` / `skill` / `mcp` provider，各自声明 `new_types` | 统一实体页按类型逐个退场 |
| **S5 下线旧协议** | 移除 `entities/*` 路由与前端实体页，导航完全由 `.vdfs` 驱动 | 一个协议、一个页面 |

每阶段的验收：`cargo check` + `cargo test` + `vitest run` 全绿；被迁移资源的
**新建 / 列出 / 详情 / 编辑 / 删除 / 实时** 六项行为与迁移前**等价**。

### 7.1 本文件对应的落地进度

- **S1 页面规范地基**：本文件 + 后端 `new_types` 机制 + 前端 `.vdfs` 地址模型与添加/类型选择
  （**已完成**，提交 `51c7f34` 后端机制 / `9d614f4` 前端页面规范）。
- **S2 设置迁移**（**已完成**）：
  - 设置入口 `navTargetOf('setting')` 由 `/settings` 切到 `/vdfs/setting`；旧地址
    `/settings` 改为 redirect 保兼容（书签 / 深链）。
  - 后端 setting provider 的 `section_node` 按**渲染器身份**声明 `ext`：
    定义驱动分区（`session` / `local` / `web` / `gateway`，携带 `schema`）→ `ext = form`；
    前端自持分区（`appearance` / `about`，无 `schema`）→ `ext = 分区 id`。
  - 前端新增渲染器标识 `appearance` / `about` 与 `ext → 渲染器` 映射，
    并在 `registry/vdfsRenderers.ts` 装配 `Appearance.vue` / `About.vue`
    （`setting:appearance` / `setting:about` 的实体 editor 注册随之退场）。
  - VDFS 列表项图标复用实体机制的**项级图标**（`kind + 节点名`，与 `config_type`
    项级分发同构），设置分区图标与迁移前等价。
  - VDFS 页「返回」在虚拟根处回主界面（本页整页替换主布局，避免困住用户）。
- **S3 会话迁移**（**已完成**）：
  - **后端 `session` provider**：根 = 会话清单（`new_types = [会话]`、`root_access = l`），
    节点 = 会话（`ext = session`、`kind = session`、`status` 反映 working、`name = 会话 id`、
    `title = display_title()`、`description = derive_session_summary`）。`read` 返回会话
    元数据 JSON；`write` 分两支——`create` 位 → 新建（**id 由 provider 生成**，路径名
    去扩展名作标题，`created_via = "vdfs"`），否则 → 合并 `metadata` / `title`（其余字段
    明确拒绝，不静默丢弃）；`delete` 转发 `delete_session_internal`。
  - **消息（转写）不经 VDFS**：聊天流仍由既有 chat 协议承载，VDFS 只承担「资源读写」
    这一层，避免出现两套写路径。
  - **机制补缺**：`vdfs/write` 的 `create` 位此前**未透传到 `VdfsContent`**，provider
    无法区分「新建 / 覆盖」；已补 `VdfsContent.create`（`#[serde(default, skip_serializing_if)]`
    + `with_create()`）+ `protocol.rs` 三处透传 + 单测。
  - **实时**：provider 持有 `broadcast::Sender<VdfsChange>`（变更源的持有者，**非轮询**），
    `watch` spawn 转发任务、`unwatch` abort；变更点在 `session/update`（`invoke_update`）
    与 `delete_session_internal`。
  - **前端**：`VdfsView` 新增 `embedded` 装配形态（左栏由应用外壳承担，本页只出中栏 + 右栏）；
    `/vdfs/:mount?` 移入 `MainLayout` 子路由；首页 `/` 重定向到 `/vdfs/session`；
    `navTargetOf('session')` 由 `/` 切到 `/vdfs/session`；旧 `session` 实体页实例（`path: ''`）
    退场。会话详情的**删除动作**经 `mechanismActions` 注入（判据 = 节点访问位 `w`）。
  - **已知取舍**：VDFS 新建会话是「立即创建 + 命名」（VDFS 的 `write { create }` 语义），
    与实体机制的「懒创建（发送首条消息才建）」不同；后者属聊天流优化，待 S5 后统一。
- **S4–S5**：待续。

---

## 8. 一致性要求

- 前端**不得**出现 `/` 口径的地址常量或拼接——一切路径运算经 `schemas/vdfs.ts`
  的路径代数，口径恒为 `.vdfs`。
- 地址翻译**只允许**出现在 `services/vdfs.ts` 一处；页面与渲染器不得感知线路口径。
- 前端**不得**硬编码资源类型、标签、能力或路径模板；只允许 `ext → 渲染器`、
  `挂载名 → 图标` 这类纯 UI 映射。
- 新建入口**只能**由节点声明的 `new_types` 驱动；前端不得凭 `kind` 或写死的类型表
  推断可新建性。
- 机制条款（trait、访问位、协议操作、分层）一律以 `vdfs.md` 为准，本文件不得
  与之冲突。
