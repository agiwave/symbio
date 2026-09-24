# VDFS 前端页面规范

状态：现行规范
范围：Symbio 前端的「三栏资源页」——一套 `vdfs/*` 协议驱动全部资源页的呈现
关联：
`docs/design/vdfs.md`（**机制规范，权威**：接口、访问位、协议操作、分层）、
`symbio/src/symbio_core/vdfs_provider.rs`（纯接口）、
`symbio/src/plugins/vdfs/protocol.rs`（线路信封）、
`tauri/src/schemas/vdfs.ts`（前端数据契约）、
`tauri/src/composables/useVdfs.ts`（页面逻辑）、
`tauri/src/views/VdfsView.vue`（页面装配）

> 本文件只写**前端页面规范**与**现行装配形态**（§7）。机制（trait、访问位、协议操作）
> 一律以 `vdfs.md` 为准，本文件不重复、不覆盖。凡与本文件冲突，以 `vdfs.md`
> 的机制条款为准。
>
> **迁移过程已归档**：S1–S17 每一步的改动、验收与提交号在
> [`docs/archive/vdfs-frontend-migration.md`](../archive/vdfs-frontend-migration.md)
> ——含以 `EntityProvider` / `EntityVdfsAdapter` / `EntityStore` 为主语的中间形态
> （那套抽象**已不存在**）。

---

## 1. 目标

**一个协议、一个页面、一处注册**——把「项目里所有的资源与数据」收敛为同一棵
虚拟树上的节点，让**前端与大语言模型用同一组操作、同一种寻址方式**访问，
从而让整个前端收敛为「通用资源状态查看 / 管理工具」：新增一类资源只需后端
新增一个 `VdfsProvider`，前端零页面开发。

**已达成**：`entities/*` 无任何路由——前后端只剩 `vdfs/*` 一个资源协议，
新能力一律接入 VDFS。迁移期两者并存的过程记录见
[`docs/archive/vdfs-frontend-migration.md`](../archive/vdfs-frontend-migration.md)。

不变量（承 `vdfs.md` §1，前端侧重申）：

1. **路径是唯一地址**：节点地址 = `<根>/<子目录>/<相对路径>`（前端口径，
   见 §3）。
2. **能力只看访问位**：前端**不得**按 `kind` 判定能力，只能依据 `access`（`r/w/l/t`）。
3. **ext 决定详情**：节点 `ext` 是前端选择详情渲染器的**唯一键**。
4. **前端零资源知识**：前端只持有 `ext → 渲染器`、`子目录名 → 图标` 这类纯 UI
   映射；**不得**硬编码资源类型清单、标签、路径模板、能力开关。
5. **实时靠订阅，禁止轮询**：列表刷新一律经事件总线 `vdfs` 频道 + `vdfs/watch`。

---

## 2. 分层与落点

### 2.1 后端

| 层 | 落点 | 说明 |
|---|---|---|
| 纯接口 | `symbio_core/vdfs_provider.rs` | `VdfsProvider` trait + 域类型 |
| 宿主桥 | `symbio_core/vdfs/host.rs` | 上下文注入 + 错误翻译 |
| 线路信封 | `plugins/vdfs/protocol.rs` | 14 个 `vdfs/*` 操作（闭集 `VDFS_OPS`；清单见 `docs/CURRENT.md` §3.2）+ 使用方形状 |
| 访问层 | `plugins/vdfs/fs.rs` + `host.rs` | `<根>`/物理分流（UnifiedFs）+ 翻译操作 + 树遍历 + 事件投递 |
| 拓扑 | `plugins/composite/vdfs.rs` | 逐子插件经 `Plugin::get_vfs_provider()` 查询聚合子目录 provider → 组合成包含子目录列表的 provider（系统链路）；`traverse` 中经 `register_vdfs_root` 登记供 LLM 工具取根 |
| LLM 工具 | `plugins/vdfs/tools/*` | `vdfs_*` 工具 + `ToolVdfs`（统一走 UnifiedFs） |
| 物理 / 虚拟 provider | `plugins/vdfs/physical.rs`、各自持插件 | 物理磁盘（含路径守卫）、设置分区、会话等 |

### 2.2 前端

| 文件 | 职责 |
|---|---|
| `schemas/vdfs.ts` | 数据契约 + 路径代数 |
| `services/vdfs.ts` | 路径 → 请求的机械翻译 |
| `composables/useVdfs.ts` | 页面逻辑（目录 / 选中 / 详情 / 实时）；**机制动作单点**（`mechanismActions`） |
| `registry/vdfsTypes.ts` | `ext → 渲染器标识`（零组件导入） |
| `registry/vdfsRenderers.ts` | `标识 → 组件`（唯一装配点） |
| `registry/messageTypes.ts` / `messageRenderers.ts` | 消息域同构的一对：`facets → 渲染器标识` / `标识 → 组件` |
| `registry/factory.ts` | 「标识 → 组件 + 兜底」注册表机制（上列两域各声明一次，机制只有一份） |
| `registry/vdfsIcons.ts` | 图标映射（`kind` / `kind:ext` → SVG）+ 动作图标 |
| `components/common/Workbench.vue` | **三栏容器原语**（纯插槽装配，无数据逻辑）：左导航 + 中列表 + 右详情 |
| `components/common/VdfsCard.vue` | 资源卡片（中栏列表项的呈现件） |
| `components/vdfs/DetailShell.vue` | 详情渲染器公共外壳（标题 + 动作区 + 错误条） |
| `components/vdfs/rendererContract.ts` | 渲染器统一契约（所有渲染器共用的 props 一份接口） |
| `components/vdfs/*.vue` | form / text / session / message / readonly 五个详情渲染器 |
| `components/vdfs/VdfsWorkbench.vue` | **VDFS 三栏工作台（全 App 唯一）**：绑定数据地址，自取左栏 / 中栏 / 详情 |
| `views/VdfsView.vue` | 路由宿主：浏览器地址 ↔ 数据地址换算 + 宿主件注入（返回键 / logo / 系统目录） |
| `router/index.ts` | `/vdfs/:dir(.*)*`（`MainLayout` 子路由；深链/返回键多级地址） |

**实时面只有一条通道**：`event_bus` 的 `vdfs` 频道 + `vdfs/watch` 登记（§6）。
消息是 `<根>/session/<id>/message` 目录里的**文件**，流式输出 = 该文件内容的
**增长**（`updated` + `delta`）；会话运行态是会话节点 `<根>/session/<id>` 的
`status`（`updated`，无 `delta` ⇒ 回读）。判据与推导见
[ADR-025](../DECISIONS.md#adr-025-顺序是节点属性delta-是updated的传输形态)
与 `symbio/src/plugins/session/docs/node-state-streaming.md`。

---

## 3. 地址模型：`<根>`

> **`<根>` 是运行期锚点，不是常量**：根名是后端 vdfs 插件的挂载规则，前端不写死
> （审计规则 S-010 强制）。启动期 `main.ts` 挂载前调 `vdfs/root` 拿根地址登记进
> `schemas/vdfsRoot`（全前端唯一持有地），路径代数与换算一律读锚点。
> 后端改挂载名，前端零改动。未引导（引导失败）时锚点为空串——判虚拟恒假，
> 页面降级为只剩物理半，可诊断而不报错。

### 3.1 前端口径

前端以 **`<根>` 为虚拟根**，地址形如：

```
<根>                          # 首页地址：子目录清单（左栏导航同源）
<根>/session                  # 会话目录的列表（下一级）
<根>/session/<id>             # 会话详情
<根>/setting/appearance       # 设置分区
```

这与大语言模型侧 `ToolVdfs` 的**虚拟地址前缀**（`VIRTUAL_PREFIX = "<根>/"`）
**同源**：前端与 LLM 因此使用**同一种寻址方式**（`vdfs.md` 不变量 5 的落地）。

### 3.2 与线路协议的关系（关键）

**地址口径前后端同源**：`vdfs/*` 的 `path` 就是**展示地址**本身——
`<根>/...` 打头的是虚拟层，其余是物理层（`vdfs.md` §3.1、不变量 3）。
判别规则只有一条，且唯一实现在后端门面 `plugins/vdfs/fs.rs::UnifiedFs`：

```text
前端 / LLM 地址    <根>/session/abc   ──原样上线──▶  线路 path = "<根>/session/abc"
线路返回节点 path  "<根>/session/abc"  ──原样使用──▶  前端直接寻址 / 比对
```

- **前端服务层不做任何地址翻译**：`services/vdfs.ts` 只封装调用，
  没有 `toWirePath` / `toVdfsPath`（后端 `normalize_addr` 也已不强加前导 `/`，
  `<根>` 与工作目录相对地址、绝对路径三者形态各异，统一成段序列即可）。
- 页面逻辑、渲染器、路径代数**只认 `<根>` 口径**。
- **口径翻译唯一在后端一处**：`<根>/session/x` ↔ 树内 `session/x`
  （`UnifiedFs` 进出两处各一次）。provider 永远只见自己子树内的相对路径。

### 3.3 路径代数

`schemas/vdfs.ts` 的路径工具（`vdfsJoin` / `vdfsParent` / `vdfsBase`）是
**全部路径运算的唯一来源**，统一以 `<根>` 为根。
子目录名 = `<根>/` 之后的首段。

---

## 4. 三栏页面规范

沿用三栏结构（**左栏导航 / 中栏列表·树 / 右栏详情**），数据源与协议按本节调整。

### 4.1 左栏：导航（绑定地址的子目录清单）

- **数据源**：`vdfs/list('<绑定地址>')` 的目录内容——绑定
  `<根>` 时由访问层**合成**（见 [vdfs.md §13.3](./vdfs.md#133-vfs-插件vdfs访问层)），
  返回普通目录节点集合（`kind = dir`），前端只做形状映射，
  **不消费** `vdfs/providers` 端点（该端点已随 `VdfsMountInfo` 一并删除）。
- **每一项携带**：`name`（子目录名）、`title`（展示标题）、`icon`（纯 UI 映射）、
  `description`（语义说明）、`new_type`（可新建的那一种东西，§5）。
- **渲染**：图标 + 标题为主，`description` 作 tooltip；高亮 = 当前选中的子目录。
- **点击**：**就地切换**当前目录（中栏显示其列表），**不是路由跳转**。
- 子目录集合、顺序、标签**全部由后端下发**，前端零硬编码。
- **装配（控件自包含）**：三栏结构封装为唯一控件
  `components/vdfs/VdfsWorkbench.vue`——绑定一个**数据地址**（`addr`）自包含
  渲染：左栏 = 绑定地址的子目录清单、中栏 = 选中子目录内容、右栏 = 详情。
  **没有独立的导航数据层**，`useVdfs.ts` 是唯一数据层，
  首页与内部管理页只有绑定地址不同：
  - 首页绑 `<根>`（路由 `/vdfs`），左下角注入系统目录入口；
  - push 出来的地址页绑 `<根>/<dir…>`（如会话内部 `<根>/session/<id>`），
    左上角注入返回键（回 **push 来源页** = 浏览器历史 back；深链直开无来源时
    回首页）。控件的 `rail-header` / `rail-footer` 插槽即宿主件注入点。
- **默认选中**：左栏缺省选中第一个子目录（缺省会话），首页中栏因此显示
  会话列表而非 `<根>` 自身的子目录清单（与左栏零重复）；每个数据地址的
  选中项有记忆（往返 push / 返回后恢复）。

### 4.2 中栏：列表 / 树

- **数据源**：当前目录地址 `<根>/<…>` 的目录内容（`vdfs/list`）；递归形态用
  `vdfs/tree`（只下钻访问位含 `t` 的目录）。
- **每一项**：`name` / `title` / `description` / `ext` / `access` / `status` / `new_type`。
- **呈现口径（用户语义优先）**：列表项只渲染**用户语义**——`title`（缺省 `name`）、
  `description`、`status`、`meta_tags`、更新时间；**不渲染机制字段**：`ext`
  （渲染器键）、`access`（访问位 = 能力判据）、`path`、`kind`。徽标只给**目录**
  （子项数），文件不给徽标；「可写」这类由访问位派生的标签不出现——能不能保存 /
  删除，在详情页的动作上自会体现。机制字段只在**详情**保留（尤其只读兜底
  `VdfsReadonlyDetail`，它是结构浏览器 / 调试视图）。
- **状态点**：只有**声明了** `status` 的节点才画（`VdfsCard` 的 `showStatus`）。
  节点 `status` 的缺省值是 `active`，所以后端要用 `VDFS_STATUS_NONE`（空串）**显式
  声明**「本资源没有运行态」（设置分区 / 配置条目这类静态文档即是），列表据此不画点。
  状态点是**真实状态**的指示器，不是装饰——没有状态可言时画一个点等于凭空造信息。
  hover 提示走**文案映射**（`working` → 「进行中」），不把后端枚举漏给用户。
- **形态**：目录（`access` 含 `l`）→ 点击进入；文件 → 点击选中（右栏出详情）。
- **列表头**：按当前节点的 `new_type` 显示**新建按钮**（§5）。
  **不再显示「新建目录」按钮**——「新建」就是新建那一种资源；该类型若声明了两条
  入口（主入口 + 整包导入），用户在右栏详情区先选入口（选完**直接进入该类型的
  详情页**或**选文件**，没有命名输入）。
  目录结构由 provider 内部维护，前端不暴露 mkdir 入口。
- **导航**：**列表顶部不出现面包屑**。路径即导航：层级靠左栏就地切
  子目录、中栏点目录钻入（push 新地址页）或左上角返回键（回 push 来源页）；
  「浏览内部」（§4.4）是 push 进容器目录地址的页面。

### 4.3 右栏：详情（按 ext 分发）

- **唯一分发键**：节点 `ext`（缺省由 `name` 推导）。分层不变：
  `schemas/vdfs.ts`（契约）→ `registry/vdfsTypes.ts`（ext → 渲染器标识，零组件）
  → `registry/vdfsRenderers.ts`（标识 → 组件，唯一装配点）。
- **既有渲染器**：`form`（定义驱动表单，解析 `node.schema`）、`session`（会话工作区）、
  `markdown` / `json` / `text`（文本类编辑器）、`appearance` / `about`（前端状态自持的
  专属面板）、`dir` / `fallback`（只读兜底）。
- **校验错误消费**：`write` 失败时按 `parseVdfsValidation` 还原字段级错误，
  逐字段高亮（`vdfs.md` §8）。

### 4.4 「浏览内部」= push 页面

- **触发**：详情定义声明 `open-container` 动作（如会话「浏览内部」、
  agent 概览「管理内部实体」），点击后 **push 进容器同名目录的地址页**
  （`/vdfs/<容器>/<id>`），由**同一个 VdfsView** 承接——全 App 只有一份三栏实现，
  没有浮层组件。
- **形态**：与首页完全一致（同一个控件 `VdfsWorkbench`，只是绑定的数据地址
  不同）：左栏 = **绑定地址**（`<根>/<容器>/<id>`）的子目录导航、
  中栏 = 选中子目录内容、右栏 = 按 `ext` 分发的详情渲染器。
- **返回**：左上角返回键（宿主页在 `rail-header` 注入，非首页地址页可见），
  点击回 **push 来源页**（浏览器历史 back，不是父目录）；深链直开（无来源）
  时回首页。浏览器/WebView 历史同样可回退（钻入是 `router.push`）。
- **嵌套**：详情里再遇 `open-container` → 继续 push 更深的目录地址，返回键
  逐级回到出发点。
- **不做**：不提供 `mkdir` / 面包屑——与 §4.2 的约定一致。

---

## 5. 可新建的那一种东西（`new_type`）

### 5.1 语义

一个**目录节点**（含 `<根>` 的一级子目录）可以声明「**本目录可新建的那一种
东西**」——**至多一个**，缺省 = 不可新建：

- 声明了 → 中栏列表头显示**添加按钮**；
- 未声明 → 不显示添加按钮（该目录由系统管理）。

**为什么是「一个」而不是「清单」**：一个 provider = 一棵子树 = 一种资源，
「本目录可建的东西」自然只有一类。清单形态下，每个目录都要为「我到底能建几种」
维持一份判据，而使用方（前端）拿到清单后仍要按 `ext` 反查渲染器——两处知识必然
漂移。收敛为一个之后，「能不能建」是一个 `Option`，判据只有一个。

**类型与入口是两件事**。类型 = 「一类东西」（至多一个）；入口 = 「怎么把它造
出来」（至多两条）：

| 入口 | 判据 | 动作 |
|---|---|---|
| **主入口** | 恒有 | `source = file` → 选文件；否则 → 进该类型的详情页（草稿态） |
| **整包导入** | 类型声明了 `import` | 选文件（恒为 `source = file` 形态） |

于是：一条入口 → 点添加直接落到那条路；两条入口 → 先**选入口**（清单就是动作行），
选完落到上面两条之一。前端由 `vdfsNewEntries(newType)` 推导出这 1~2 条入口，
动作 id 固定为 `new:type` / `new:import`。

**选定主入口后：直接进入该类型的详情页**（草稿态）。
这也是本机制与「选中一项」共用一条通道的原因：

- **同一个渲染器**：草稿节点的 `ext` 与落成后**完全一致**；
- **同一张详情**：草稿用类型声明的 `schema` 渲染出同一张表单；
- **差别只有内容**：草稿**没有 id、也没有名字**（它还没落盘），选中一项则有；
- 因此没有「第二种新建形态」——除了 `source = file` 那一条。

名字**不由前端先问**：它是 provider 的私有知识（`id 归 provider`）。名字要么在
详情页里产生（会话由**最后一条用户消息**派生标题——列表名跟随「最近在聊什么」，
不是被第一句话钉住），要么在保存时由 provider 生成（写目录自身，见 §5.3）。

**两条键：`ext` 与 `node_ext`**。类型里的 `ext` 是**呈现扩展名**（地址末段
后缀，provider 用 `id_of` 按它剥条目 id），它**不**决定详情怎么渲染：配置型资源
（model / mcp / skill）落成后统一是 `ext = form`。所以类型必须另声明
`node_ext`（落成后的渲染器键，缺省 = `ext`）与 `schema`（表单定义）。漏了
`node_ext` 草稿会落到通用兜底，漏了 `schema` 会渲染出空表单。

**内容来源**（`source`）：类型还可声明「写进去的内容从哪来」——

- 缺省：在详情页里边看边填（先进入草稿详情，保存时一次写入）；
- `file`：内容取自**本地文件**——使用方给文件选择器而不是详情页，目标名由文件名
  推导，字节走 `vdfs/write` 的二进制（`b64`）通道。这是**唯一不进详情页**的形态，
  因为内容在打开详情页之前就已经齐备（没有「边看边填」的过程）。

于是**整包导入（zip）也是一种「新建」**，不新增协议操作。两种声明形态：

- **主入口即选文件**：`source = file`（agent 包——本目录只有这一种造法）；
- **导入是同一类型下的第二条入口**：`import = VdfsNewImport{ext, title, …}`
  （skill / mcp——主入口是表单新建，导入是备选）。

### 5.2 域类型（后端）

```rust
/// 目录可新建的**那一种**元素类型（至多一个；`None` = 不可新建）。
pub struct VdfsNewType {
    /// 新元素**呈现扩展名**（地址末段后缀；id_of 按它剥 id，**不是**渲染器键）
    pub ext: String,
    /// 展示标题
    pub title: String,
    /// 语义说明（缺省不显示）
    pub description: Option<String>,
    /// 图标名（纯 UI 映射）
    pub icon: Option<String>,
    /// 内容来源：`None` = 在详情页里填；`Some("file")` = 选择本地文件
    pub source: Option<String>,
    /// 新元素落成后的节点 ext（**渲染器键**）；缺省 = 与 ext 相同
    pub node_ext: Option<String>,
    /// 新元素的呈现描述（与 VdfsNode.schema 同义）——草稿详情页据此渲染
    pub schema: Option<Value>,
    /// **备选的第二条入口**（整包导入）；缺省 = 只有主入口
    pub import: Option<VdfsNewImport>,
}

/// 同一类型下的整包导入入口（「怎么造出来」的第二种形态）。
pub struct VdfsNewImport {
    /// 包地址末段后缀（`newFileNameOf` 按它剥建议名）
    pub ext: String,
    /// 导入入口展示标题（如「技能包」）
    pub title: String,
    /// 语义说明（缺省不显示）
    pub description: Option<String>,
}
```

- 挂在 `VdfsNode.new_type`（目录节点；缺省不序列化）。
- provider **根**的类型经 `VdfsProvider::root_new_type()`（async）声明，由容器在
  合成**子目录节点**时现场取（它是挂载点自述里唯一动态的部分——session 的 schema
  需运行期汇流，故不在同步的 `PluginMeta` 上）。

### 5.3 创建动作（协议不变）

新建 = **对目录自身的一次 `vdfs/write`**（`create: true`，不给名字）：

```
新建类型 ext 的新元素，于当前目录 dir：
  目标地址 = dir                      // 目录自身：使用方不说叫什么
  vdfs/write { path: dir, text: <详情页填好的字段>, create: true }
  → provider 生成 id，并在 VdfsWriteResponse.path 里给出新节点
  → 使用方刷新后选中那一项（详情页从草稿态变成带内容的那一页）

source = file（主入口或 import 入口）：名称来自文件名
  目标地址 = vdfsJoin(dir, newFileNameOf(file.name, ext))   // 主干 + 入口扩展名
  vdfs/write { path: 目标地址, b64: <文件字节 base64>, create: true }
```

- **创建语义由 provider 自持**：文件系统 provider 落为文件；会话 provider 落为
  会话；设置 provider 可拒绝（无 `w` 位 / 无类型声明）。
- **`create` 只回答「目标不存在时怎么办」**：`true` → 建；`false` → `NotFound`。
  它**不改变内容的处理方式**——详情页填好的字段原样落盘。唯一例外是内容为空
  （「先建一个，随后再填」），此时 provider 落一份最小合法内容。
- **具名写**（`path` 指向具体名字，如 LLM 直接写 `<根>/model/foo`）：名字就是
  那个名字，**不存在则创建**——这就是「有名字但文件不存在则自动创建」。
- 前端**不**认识任何具体类型——只负责「选入口 + 进入草稿详情（或选文件）+
  组装地址 + 发写请求」。
- 目录（无扩展名的结构节点）不属于「新建类型」，仍走 `vdfs/mkdir`。

### 5.4 前端职责

| 层 | 职责 |
|---|---|
| `schemas/vdfs.ts` | `VdfsNewType`（含 `node_ext` / `schema` / `source` / `import`）+ `new_type` 字段 + `vdfsNewEntries`（类型 → 1~2 条入口）+ `newFileNameOf` |
| `composables/useVdfs.ts` | `creatableType` / `newEntries` / `canCreate`；新建 = `startNew(type)` 选中一张**草稿节点**（`path === ''`）；`write` 对草稿打到 `cwd` 并带 `create: true`；`draftSeq` 是草稿的临时身份（`:key` 用） |
| `composables/useVdfsPrompt.ts` | 提示态状态机：`entry`（选入口）/ `file`（选文件），两者共占详情槽 |
| `components/vdfs/VdfsWorkbench.vue` | 添加按钮可见性 + 把提示态接到详情槽（**没有命名输入**） |

**草稿节点**的形状（`draftNodeOf`）：`path: ''`、`name: ''`、`access: 'w'`，
加上 `ext = type.node_ext ?? type.ext`、`schema = type.schema`、`kind = type.ext`
（图标键）。其余呈现字段留空——「还没有的东西」不假装有内容。

两条配套约束：

- `select()` 对草稿**直接返回**（没有内容可读，去读只会落到目录地址上）；
- `refresh()` 清理选中态时**跳过草稿**（它本来就不在清单里）。

---

## 6. 实时性

- **列表刷新靠订阅，非轮询**：视图订阅总线 `vdfs` 频道（`kind = "vdfs"`），
  按**路径**精确刷新；订阅范围与当前目录严格绑定
  （切目录 = 解旧订阅 + 登新订阅）。
- **订阅地址用前端口径**（`<根>/session`），服务层翻译为线路 path（`/session`）。
- 后端对无实时能力的 provider 默认 no-op，前端无需按 provider 分流。
- 细粒度帧**不得**触发全量刷新；一律防抖重拉当前目录收敛。

---

## 7. 现行形态（前端装配）

> 迁移过程（S1–S17 每一步的改动、验收与提交号）已归档：
> [`docs/archive/vdfs-frontend-migration.md`](../archive/vdfs-frontend-migration.md)。
> 本节只写**现在是什么**。

- **一个三栏控件**：`components/vdfs/VdfsWorkbench.vue` 是**全 App 唯一**的三栏实现，
  绑定一个**数据地址**（`addr` prop）自包含渲染；钻入只 `emit('open', addr)`，
  呈现方式由宿主决定（`rail-header` / `rail-footer` 插槽是宿主件注入点）。
- **页面 = 路由宿主**：`views/VdfsView.vue` 只做「浏览器地址 ↔ 数据地址」换算与宿主件
  注入，不持三栏结构；`views/MainLayout.vue` 只剩 `RouterView` + 全局 Toast + 全局初始化。
- **数据地址 ≠ 浏览器地址**：控件与数据层（`composables/useVdfs.ts`）只认数据地址；
  换算只发生在页面层**一处**（§3.1）。**首页与内部管理页零差别**——同一个控件、同一份
  数据逻辑，唯一区别是绑定的地址（`<根>` / `<根>/session/<id>` / `<根>/agent/<id>`）。
- **左栏属于控件**：外壳不持有导航数据层，左栏恒为**绑定地址**的子目录清单。
- **路由**：`/` 重定向到 `/vdfs`（= 地址 `<根>`）；`vdfs/:dir(.*)*` 承接 `<根>` 之下的
  多级深链。每个地址是一个**可回退的页面**——「浏览内部」= push 进容器目录地址，不是浮层。
- **后端**：资源存储只有 `providers/vdfs_service` 的三个 `VdfsProvider` 实现
  （单文件 / 目录 / 内存）；`EntityProvider` / `EntityVdfsAdapter` / `EntityStore` /
  `StorageService` 一套实体与存储抽象**已不存在**（见 [vdfs.md](./vdfs.md) §13.4）。

---

## 8. 一致性要求

- 前端**不得**出现 `/` 口径的地址常量或拼接——一切路径运算经 `schemas/vdfs.ts`
  的路径代数，口径恒为 `<根>`。
- 地址翻译**只允许**出现在 `services/vdfs.ts` 一处；页面与渲染器不得感知线路口径。
- 前端**不得**硬编码资源类型、标签、能力或路径模板；只允许 `ext → 渲染器`、
  `子目录名 → 图标` 这类纯 UI 映射。
- 新建入口**只能**由节点声明的 `new_type` 驱动（入口清单由 `vdfsNewEntries` 从它
  推导）；前端不得凭 `kind` 或写死的类型表推断可新建性。
- 新建类型的**详情呈现只能**由类型自己声明的 `node_ext` / `schema` 决定；前端不得
  按 `ext` 猜渲染器（`ext` 是地址后缀，不是渲染器键），也不得为某类资源写死表单。
- 草稿节点（`path === ''`）与已落盘节点走**同一条详情通道**：同一个 `ext` 解析出
  同一个渲染器。前端不得为「新建」另立第二种详情形态（`source = file` 除外）。
- 动作入口**只能**由详情定义声明的 `actions` 驱动；前端只认**载荷形状**
  （如文件载荷 `filename` + `b64`），不认具体动作标识。
- 列表项**不得**渲染机制字段（`ext` / `access` / `path` / `kind`）：它们是分发键、
  能力判据与寻址信息，由机制消费，不是给用户的类型名或权限标签。用户可见的呈现
  只来自 `title` / `description` / `status` / `meta_tags` / 时间（§4.2）。
- ⚠️ **不得为了列表好看而在后端删这些字段**：详情渲染、能力判据与渲染器分发都要
  用它们；该收敛的地方是**渲染层**，不是数据源。同理，provider 也**不要**把机制
  说明写进 `description`——那个字段只出现在用户看的列表里。
- 机制条款（trait、访问位、协议操作、分层）一律以 `vdfs.md` 为准，本文件不得
  与之冲突。
