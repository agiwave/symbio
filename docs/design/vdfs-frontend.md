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
>
> **现状说明（2026-09-15 起）**：§7 中以 `EntityProvider` / `EntityVdfsAdapter` /
> `EntityStore` 为主语的条目是**进度档案**，原样保留不改写。后端资源存储的**当前**
> 形态是 `providers/vdfs_service` 的三个 `VdfsProvider` 集中实现（单文件 / 目录 /
> 内存），对外地址、节点形状与协议操作**不变**；记录见 §7 的 S17 与
> `vdfs.md` §13.4。

---

## 1. 目标

**一个协议、一个页面、一处注册**——把「项目里所有的资源与数据」收敛为同一棵
虚拟树上的节点，让**前端与大语言模型用同一组操作、同一种寻址方式**访问，
从而让整个前端收敛为「通用资源状态查看 / 管理工具」：新增一类资源只需后端
新增一个 `VdfsProvider`，前端零页面开发。

最终目标：**替代前后端现有的几乎全部既有协议（`entities/*` 等），全面
拥抱 VDFS**。**已达成**——`entities/*` 自 S11 起无任何路由（迁移期两者并存的
局面已结束，过程记录见 §7）；新能力一律接入 VDFS。

不变量（承 `vdfs.md` §1，前端侧重申）：

1. **路径是唯一地址**：节点地址 = `<根>/<子目录>/<相对路径>`（前端口径，
   见 §3）。
2. **能力只看访问位**：前端**不得**按 `kind` 判定能力，只能依据 `access`（`r/w/l/t`）。
3. **ext 决定详情**：节点 `ext` 是前端选择详情渲染器的**唯一键**。
4. **前端零资源知识**：前端只持有 `ext → 渲染器`、`子目录名 → 图标` 这类纯 UI
   映射；**不得**硬编码资源类型清单、标签、路径模板、能力开关。
5. **实时靠订阅，禁止轮询**：列表刷新一律经事件总线 `vdfs` 频道 + `vdfs/watch`。

---

## 2. 现状评估

### 2.1 已实现（后端，已暂存）

| 层 | 落点 | 状态 |
|---|---|---|
| 纯接口 | `symbio_core/vdfs_provider.rs` | `VdfsProvider` trait + 域类型 |
| 宿主桥 | `symbio_core/vdfs/host.rs` | 上下文注入 + 错误翻译 |
| 线路信封 | `plugins/vdfs/protocol.rs` | 13 个 `vdfs/*` 操作 + 使用方形状 |
| 访问层 | `plugins/vdfs/fs.rs` + `host.rs` | `<根>`/物理分流（UnifiedFs）+ 翻译操作 + 树遍历 + 事件投递 |
| 拓扑 | `plugins/composite/vdfs.rs` | 逐子插件经 `Plugin::get_vfs_provider()` 查询聚合子目录 provider → 组合成包含子目录列表的 provider（系统链路）；`traverse` 中经 `register_vdfs_root` 登记供 LLM 工具取根 |
| LLM 工具 | `plugins/vdfs/tools/*` | `vdfs_*` 工具 + `ToolVdfs`（统一走 UnifiedFs） |
| 物理 / 虚拟 provider | `plugins/vdfs/physical.rs`、各自持插件 | 物理磁盘（含路径守卫）、设置分区、会话等 |

### 2.2 已实现（前端，已落地）

| 文件 | 职责 | 评估 |
|---|---|---|
| `schemas/vdfs.ts` | 数据契约 + 路径代数 | **保留**（扩展） |
| `services/vdfs.ts` | 路径 → 请求的机械翻译 | **保留**（扩展） |
| `composables/useVdfs.ts` | 页面逻辑（目录 / 选中 / 详情 / 实时）；**机制动作单点**（`mechanismActions`） | **保留**（扩展） |
| `registry/vdfsTypes.ts` | `ext → 渲染器标识`（零组件导入） | **保留** |
| `registry/vdfsRenderers.ts` | `标识 → 组件`（唯一装配点） | **保留** |
| `registry/messageTypes.ts` / `messageRenderers.ts` | 消息域同构的一对：`facets → 渲染器标识` / `标识 → 组件` | **保留** |
| `registry/factory.ts` | 「标识 → 组件 + 兜底」注册表机制（上列两域各声明一次，机制只有一份） | **保留** |
| `registry/vdfsIcons.ts` | 图标映射（`kind` / `kind:ext` → SVG）+ 动作图标 | **保留** |
| `components/common/Workbench.vue` | **三栏容器（唯一实现）**：左导航 + 中列表 + 右详情（插入槽） | **保留** |
| `components/common/VdfsCard.vue` | 资源卡片（中栏列表项的呈现件） | **保留** |
| `components/vdfs/DetailShell.vue` | 详情渲染器公共外壳（标题 + 动作区 + 错误条） | **保留** |
| `components/vdfs/rendererContract.ts` | 渲染器统一契约（所有渲染器共用的 props 一份接口） | **保留** |
| `components/vdfs/*.vue` | form / text / session / message / readonly 五个详情渲染器 | **保留** |
| `components/vdfs/VdfsWorkbench.vue` | **三栏工作台控件（唯一实现）**：绑定数据地址，自取左栏 / 中栏 / 详情（S15 新增） | **保留**（S15 新增） |
| `views/VdfsView.vue` | 路由宿主：浏览器地址 ↔ 数据地址换算 + 宿主件注入（返回键 / logo / 系统目录） | **保留**（扩展） |
| `router/index.ts` | `/vdfs/:dir(.*)*`（`MainLayout` 子路由；深链/返回键多级地址） | **保留** |

结论：前端既有改动**方向正确、结构合理**，与目标一致的部分**整体保留**，
G1–G3 三处差距已按 §3–§6 补齐（见 §7.1 的 S1）。

> **现状校正（针对 §7.1 的 S3）**：§7.1 记「消息（转写）不经 VDFS」，那是当时
> 的形态。**现在消息已在 VDFS 上**——`<根>/session/<id>/消息/<mid>`，节点
> `ext = message`，转写经 `services/vdfsTranscriptSync.ts` 同步，详情由
> `components/vdfs/VdfsMessageDetail.vue` 呈现。§7 是进度档案、按约定不改写，
> 但读 §7.1 时请以本条为准。

### 2.3 与目标的差距

| # | 差距 | 现状 | 目标 | 状态 |
|---|---|---|---|---|
| G1 | **地址口径** | 前端以 `/` 为根 | 以 `<根>` 为根，与 LLM 地址模型统一（§3） | **已补齐** |
| G2 | **可接受的新建类型** | 后端无此概念；前端只有「新建目录」 | 节点 / 挂载点声明可新建类型；前端据此出添加按钮与类型选择器（§5） | **已补齐** |
| G3 | **导航元数据** | 左栏仅图标 + 标题 | 导航项携带名字 / 标题 / 图标 / 描述 / 可新建类型（§4.1） | **已补齐** |

### 2.4 取舍

- **保留继续**：§2.2 全部文件（含路径代数、渲染器分层、实时订阅、校验错误消费）。
- **放弃**：无（既有前端改动未发现与目标冲突者）。
- **待补**：无（G1–G3 已在 S1 落地）。

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

- **数据源（S11 严格 vdfs 化）**：`vdfs/list('<绑定地址>')` 的目录内容——绑定
  `<根>` 时由访问层**合成**（见 [vdfs.md §13.3](./vdfs.md#133-vfs-插件vdfs访问层)），
  返回普通目录节点集合（`kind = dir`），前端只做形状映射，
  **不消费** `vdfs/providers` 端点（该端点已随 `VdfsMountInfo` 一并删除）。
- **每一项携带**：`name`（子目录名）、`title`（展示标题）、`icon`（纯 UI 映射）、
  `description`（语义说明）、`new_types`（可接受的新建类型，§5）。
- **渲染**：图标 + 标题为主，`description` 作 tooltip；高亮 = 当前选中的子目录。
- **点击**：**就地切换**当前目录（中栏显示其列表），**不是路由跳转**。
- 子目录集合、顺序、标签**全部由后端下发**，前端零硬编码。
- **装配（S15 起，控件自包含）**：三栏结构封装为唯一控件
  `components/vdfs/VdfsWorkbench.vue`——绑定一个**数据地址**（`addr`）自包含
  渲染：左栏 = 绑定地址的子目录清单、中栏 = 选中子目录内容、右栏 = 详情。
  **没有独立的 Nav 数据逻辑**（原 `useNavRail.ts` 已删除），`useVdfs.ts` 是
  唯一数据层，首页与内部管理页只有绑定地址不同：
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
- **每一项**：`name` / `title` / `description` / `ext` / `access` / `status` / `new_types`。
- **呈现口径（用户语义优先）**：列表项只渲染**用户语义**——`title`（缺省 `name`）、
  `description`、`status`、`meta_tags`、更新时间；**不渲染机制字段**：`ext`
  （渲染器键）、`access`（访问位 = 能力判据）、`path`、`kind`。徽标只给**目录**
  （子项数），文件不给徽标；「可写」这类由访问位派生的标签不出现——能不能保存 /
  删除，在详情页的动作上自会体现。机制字段只在**详情**保留（尤其只读兜底
  `VdfsReadonlyDetail`，它是结构浏览器 / 调试视图，S14）。
- **状态点**：只有**声明了** `status` 的节点才画（`VdfsCard` 的 `showStatus`）。
  节点 `status` 的缺省值是 `active`，所以后端要用 `VDFS_STATUS_NONE`（空串）**显式
  声明**「本资源没有运行态」（设置分区 / 配置条目这类静态文档即是），列表据此不画点。
  状态点是**真实状态**的指示器，不是装饰——没有状态可言时画一个点等于凭空造信息。
  hover 提示走**文案映射**（`working` → 「进行中」），不把后端枚举漏给用户。
- **形态**：目录（`access` 含 `l`）→ 点击进入；文件 → 点击选中（右栏出详情）。
- **列表头（S11）**：按当前节点的 `new_types` 显示**新建按钮**（§5）。
  **不再显示「新建目录」按钮**——「新建」就是新建一种资源；需要多种元素类型时，
  由 `new_types` 清单一次性下发，用户在右栏详情区先选类型（S19：选完**直接进入
  该类型的详情页**，不再有命名输入）。
  目录结构由 provider 内部维护，前端不暴露 mkdir 入口。
- **导航（S11）**：**列表顶部不出现面包屑**。路径即导航：层级靠左栏就地切
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

### 4.4 「浏览内部」= push 页面（S11）

- **触发**：详情定义声明 `open-container` 动作（如会话「浏览内部」、
  agent 概览「管理内部实体」），点击后 **push 进容器同名目录的地址页**
  （`/vdfs/<容器>/<id>`），由**同一个 VdfsView** 承接——全 App 只有一份三栏实现，
  没有浮层组件（原 `VdfsContainerBrowser.vue` 已删除）。
- **形态**：与首页完全一致（同一个控件 `VdfsWorkbench`，只是绑定的数据地址
  不同）：左栏 = **绑定地址**（`<根>/<容器>/<id>`）的子目录导航、
  中栏 = 选中子目录内容、右栏 = 按 `ext` 分发的详情渲染器。
- **返回**：左上角返回键（宿主页在 `rail-header` 注入，非首页地址页可见），
  点击回 **push 来源页**（浏览器历史 back，不是父目录）；深链直开（无来源）
  时回首页。浏览器/WebView 历史同样可回退（钻入是 `router.push`）。
- **嵌套**：详情里再遇 `open-container` → 继续 push 更深的目录地址，返回键
  逐级回到出发点。
- **不做**：不提供 `mkdir` / 面包屑——与 §4.2 S11 约定一致。

---

## 5. 可接受的新建类型（`new_types`）

### 5.1 语义

一个**目录节点**（含 `<根>` 的一级子目录）可以声明「**本目录可接受的新建元素
类型**」——一个类型清单：

- 清单**非空** → 中栏列表头显示**添加按钮**；
- 清单**多于一项** → 点击添加按钮后**先选类型**；
- 清单**恰好一项** → 跳过类型选择，直接落到那一条路；
- 清单**为空** → 不显示添加按钮（该目录由系统管理）。

**选定类型后：直接进入该类型的详情页**（草稿态）。这是 S19 定下的交互口径，
也是本机制与「选中一项」共用一条通道的原因：

- **同一个渲染器**：草稿节点的 `ext` 与落成后**完全一致**；
- **同一张详情**：草稿用类型声明的 `schema` 渲染出同一张表单；
- **差别只有内容**：草稿**没有 id、也没有名字**（它还没落盘），选中一项则有；
- 因此没有「第二种新建形态」——除了下面 `source = file` 那一条。

名字**不由前端先问**：它是 provider 的私有知识（`id 归 provider`）。名字要么在
详情页里产生（会话由**最后一条用户消息**派生标题——列表名跟随「最近在聊什么」，
不是被第一句话钉住），要么在保存时由 provider 生成（写目录自身，见 §5.3）。

**两条键：`ext` 与 `node_ext`**。类型清单里的 `ext` 是**呈现扩展名**（地址末段
后缀，provider 用 `id_of` 按它剥条目 id），它**不**决定详情怎么渲染：配置型资源
（model / mcp / skill）落成后统一是 `ext = form`。所以类型必须另声明
`node_ext`（落成后的渲染器键，缺省 = `ext`）与 `schema`（表单定义）。漏了
`node_ext` 草稿会落到通用兜底，漏了 `schema` 会渲染出空表单。

**内容来源**（`source`，S12）：类型还可声明「写进去的内容从哪来」——

- 缺省：在详情页里边看边填（先进入草稿详情，保存时一次写入）；
- `file`：内容取自**本地文件**——使用方给文件选择器而不是详情页，目标名由文件名
  推导，字节走 `vdfs/write` 的二进制（`b64`）通道。这是**唯一不进详情页**的形态，
  因为内容在打开详情页之前就已经齐备（没有「边看边填」的过程）。

于是**整包导入（zip）也是一种「新建」**：`ext = zip` + `source = file`，
不新增协议操作（详见 §7 的 S12）。

### 5.2 域类型（后端）

```rust
/// 目录可接受的新建元素类型。
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
}
```

- 挂在 `VdfsNode.new_types`（目录节点；文件节点为空、不序列化）。
- provider **根**的类型清单经 trait 方法 `root_new_types()` 声明（与
  `root_access` / `root_status` 同构），由容器在合成**子目录节点**时回填。

### 5.3 创建动作（协议不变）

新建 = **对目录自身的一次 `vdfs/write`**（`create: true`，不给名字）：

```
新建类型 ext 的新元素，于当前目录 dir：
  目标地址 = dir                      // 目录自身：使用方不说叫什么
  vdfs/write { path: dir, text: <详情页填好的字段>, create: true }
  → provider 生成 id，并在 VdfsWriteResponse.path 里给出新节点
  → 使用方刷新后选中那一项（详情页从草稿态变成带内容的那一页）

source = file 的类型（整包导入）：名称来自文件名
  目标地址 = vdfsJoin(dir, newFileNameOf(file.name, ext))   // 主干 + 类型扩展名
  vdfs/write { path: 目标地址, b64: <文件字节 base64>, create: true }
```

- **创建语义由 provider 自持**：文件系统 provider 落为文件；会话 provider 落为
  会话；设置 provider 可拒绝（无 `w` 位 / 无类型声明）。
- **`create` 只回答「目标不存在时怎么办」**：`true` → 建；`false` → `NotFound`。
  它**不改变内容的处理方式**——详情页填好的字段原样落盘。唯一例外是内容为空
  （「先建一个，随后再填」），此时 provider 落一份最小合法内容。
- **具名写**（`path` 指向具体名字，如 LLM 直接写 `<根>/model/foo`）：名字就是
  那个名字，**不存在则创建**——这就是「有名字但文件不存在则自动创建」。
- 前端**不**认识任何具体类型——只负责「选类型 + 进入草稿详情（或选文件）+
  组装地址 + 发写请求」。
- 目录（无扩展名的结构节点）不属于「新建类型」，仍走 `vdfs/mkdir`。

### 5.4 前端职责

| 层 | 职责 |
|---|---|
| `schemas/vdfs.ts` | `VdfsNewType` 类型（含 `node_ext` / `schema` / `source`）+ `new_types` 字段 + `newFileNameOf` |
| `composables/useVdfs.ts` | 新建 = `startNew(type)` 选中一张**草稿节点**（`path === ''`）；`write` 对草稿打到 `cwd` 并带 `create: true`；`draftSeq` 是草稿的临时身份（`:key` 用） |
| `views/VdfsView.vue` | 添加按钮可见性 + 类型选择器（**没有命名输入**；`source = file` 才给文件选择器） |

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

## 7. 迁移路线（自顶向下，逐步推进）

原则：**先立页面规范（顶），再逐个迁移 provider（底）**；每步独立可验证、
可提交，绝不半途并存两套页面机制。

| 阶段 | 内容 | 产出 |
|---|---|---|
| **S1 页面规范地基** | 地址模型 `<根>`（§3）+ 导航元数据（§4.1）+ 可新建类型（§5）+ 添加/类型选择器 | 前端页面机制就位；后端 `new_types` 机制就位 |
| **S2 设置迁移** | `setting` 已是 provider；把设置入口从 `/settings`（entities）切到 `/vdfs/setting` | 设置页走 VDFS；`entities/setting` 退场 |
| **S3 会话迁移** | 新增 `session` provider：根 = 会话清单（`new_types = [会话]`）、节点 = 会话（`ext = session`）；read/write/delete 转发既有会话协议 | 会话页走 VDFS；聊天工作区作为 `session` 渲染器 |
| **S4 其余迁移** | 用一个通用 `EntityVdfsAdapter` 把既有 `EntityProvider` 接成挂载点（`model` / `skill` / `mcp` 可写、`agent` 只读）；外壳左栏切到 `<根>` 根 | 全部资源在 `<根>` 下可见可管；统一实体页按类型逐个退场 |
| **S5 下线旧协议** | 移除 `entities/*` 路由与前端实体页（`/entities/*` 仅留保兼容重定向），导航完全由 `<根>` 驱动 | 一个协议、一个页面 |
| **S6 会话内部重建** | 会话内部结构（子会话 / 工作目录树）改由 VDFS 同名目录承载，原容器实体页可替代 | S5 的阻塞解除 |
| **S7 容器子实体重建** | 通用适配器按 `container_kinds` 支持 `<id>/<子类别>/<条目>`；agent 目录内部（提示词 / 技能 / MCP）上 VDFS | 容器页最后一处不可替代能力消失 |
| **S8 会话清单上 VDFS** | 会话节点自带 `message_count` / `metadata` / `meta_tags`；`listSessions()` 改走 `vdfs/list`，`services/entities.ts` 删除 | 前端 `entities/*` 调用点归零 |
| **S9 导航可见性** | ~~机制层新增 `VdfsProvider::nav_visible()`~~ **已整体撤销**：物理层从 `plugins/local` 迁入 `plugins/vdfs`，本地文件不再是子目录，左栏自然回到「六类资源」，无需可见性标记（**2026-09-15 复核**：改造三之后 `web`/`local`/`gateway` 必须留在树里，改用机制级隐藏属性 `VdfsNode::hidden` + `root_hidden()`，见下） | 左栏 = 六类资源，无按名硬编码（**结果达成，机制未引入**；后来的隐藏属性是「存在但不列」这一独立问题的机制解） |
| **S10 节点动作** | 新增 `vdfs/action` 操作 + `VdfsProvider::action()`（默认 `NotImplemented`）；适配器把 `test` 接到 `EntityProvider::test_status`；前端把「测试连接」接回 | S5 后丢失的连通性自检回归 |
| **S11 下线 `entities/*` 协议** | 6 个插件不再路由 `entities/*`，`entities::dispatch` 与其请求/响应、zip 工具一并删除；网关只读白名单改列 `vdfs/*` 读操作 | 对外只剩 VDFS 一个资源协议 |
| **S12 整包导入** | `VdfsNewType.source`（`file`）+ `EntityProvider::import_zip` 钩子；适配器的二进制 `write` 承接导入（agent 走 `AgentDirStore::import`），agent 补上 `delete_item` | S5/S11 后丢失的 zip 导入回归，且**不新增协议操作** |
| **S12 清理** | 注册表去掉 `prefix` / `provider_name` / `compact_list` / `status_indicator` 与 `EntityCapabilities`（改由 `supports_import` 表达）；删协议时代的请求/响应与 `get_item`；`agent` 的旧协议路由（已被 VDFS 取代）只保留导出 | 历史冗余与被替换代码清空 |
| **S13 整包导出** | `VDFS_ACTION_EXPORT` 节点动作 + `EntityProvider::export_zip` 钩子（默认 `zip_dir`、agent 走 `AgentDirStore::export`）；结果按「文件载荷」（`filename` + `b64`）回传；`agent` 的旧协议路由整体下线 | 导入/导出在 VDFS 内闭环；`agent` 插件零自有路由 |
| **S16 收敛终局（废除实体机制）** | 删 `EntityProvider` trait / `provider_registry()` / `EntityVdfsAdapter`；`model` / `skill` / `agent` 各补一份 `impl VdfsProvider`（与已有的 `session` / `setting` / `mcp` 同构）；`entities.rs` 降为存储原语自由函数 | 后端只剩 VDFS 一套机制；**前端零改动**——挂载名与节点形状不变 |
| **S17 收敛存储层** | 删 `providers/storage_service` 与 `symbio_core` 的 `EntityStore` / `StorageService` / 存储原语，改为 `providers/vdfs_service` 的三个 `VdfsProvider` 集中实现（单文件 / 目录 / 内存）；`schemas/entities.rs` 收敛为 `DetailDefinition` 表单方言；事件总线只留 `kind = "vdfs"` | 资源存储讲的也是 VDFS 的话；磁盘布局不变，前端只退一个 `entity` 频道订阅 |

每阶段的验收：`cargo check` + `cargo test` + `vitest run` 全绿；被迁移资源的
**新建 / 列出 / 详情 / 编辑 / 删除 / 实时** 六项行为与迁移前**等价**。

### 7.1 本文件对应的落地进度

- **S1 页面规范地基**：本文件 + 后端 `new_types` 机制 + 前端 `<根>` 地址模型与添加/类型选择
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
    `title = display_title()`——`metadata.title` 优先，否则取**最后一条用户消息**；
    `description = derive_session_summary`——**最新一条助手回复**）。`read` 返回会话
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
    （**S19 已收敛**：新建 = 选中一张草稿节点，首条消息才真正 `vdfs/write`；
    id 由 provider 生成，见 §5。）
- **S4 其余迁移**（**已完成**）：`model` / `agent` / `skill` / `mcp` 四类不再各写一份
  provider，而是由**一个通用适配器**统一接入。
  - **通用适配器** `symbio_core/vdfs/entity_adapter.rs` 的 `EntityVdfsAdapter`：
    把**任意** `EntityProvider` 接成 VDFS 子目录。核心洞见——实体机制与 VDFS 是
    **同一批资源的两套寻址方式**，因此复用既有能力而非重写：
    子目录 label / order / 可写性 / `new_types` ← `entities::provider_registry()`；
    `list` ← `list_items`；`stat` / 节点呈现 ← `summarize` + `detail_definition`；
    `read` ← 摘要 `extra.config`（与实体详情页预填**同源**）；
    `write` / `delete` ← `entity_write` / `entity_delete`；`watch` ← provider 侧
    `broadcast::Sender<VdfsChange>`。**新增一种实体类型时，VDFS 侧零改动。**
  - **消除两条链路的逻辑分叉**：把 `entities/upload` 的 manifest 分支与 `entities/delete`
    抽成公开的 `entities::entity_write` / `entity_delete`，实体机制与 VDFS 共用同一份
    校验 / 写盘 / 事件发布。
  - **节点 `ext` 选取**：`detail_definition` 有定义 → `ext = form`（前端通用表单渲染器
    解析 `schema`）；无定义 → `ext = <kind>`（机制级只读视图）。`ext` 仍是详情渲染器的
    唯一分发键，适配器不参与渲染决策。
  - **新建语义由插件自持**：新增 `EntityProvider::new_entity_manifest(id, title)` 默认方法。
    实体机制走完整表单一次上传；VDFS `write { create }` 只给路径名，故插件各给一份
    **最小可用配置**：`model` 取预设首项 + `skip_validation`；`skill` 满足
    `name == id` 且 description ≥ 10 字；`mcp` 给 stdio 骨架（`type` / `command` / `args`）。
  - **可写性双重判定**：`writable()` = 注册表 `supports_upload` **且** provider 有
    `category()` + `manifest_file()`（EntityStore 型）。目录自管型 `agent`（
    走 `AgentDirStore`）因此**自动降级为只读**——避免「声明了可新建但落盘必失败」。
    `agent` 以**只读挂载**接入：列表 + 详情可用，新建仍走实体页的 zip 上传。
  - **导航顺序单一真相源**：新增 `entities::nav_meta_of(kind)`；`session` / `setting`
    自持 provider 的 `label` / `order` 改为从注册表读（原来硬编码 10 / 60，与注册表的
    1…6 不一致，会让左栏顺序错乱）。
  - **外壳左栏切到 `<根>` 根**：新建 `composables/useNavRail.ts`
    （`navTargetOf` 恒为 `/vdfs/{mount}`、挂载清单模块级单例 + `loadMounts` / `reloadMounts`、
    订阅 `vdfs` 总线重拉）；`useEntityProviders` 只保留实体注册表职责。
  - **`form` 渲染器补删除动作**：`VdfsFormDetail` 原先假设 `form` = 设置分区（增删无语义）；
    S4 起 `model` / `skill` / `mcp` 详情也走 `form`，故按访问位注入 `mechanismActions`
    （`w` ⇒ 可删），经 `@delete` 回页面层统一走 `vdfs/delete`（与 `VdfsSessionDetail` 同构）。
  - **已知取舍**：`local`（本地文件）当时同为 VDFS 子目录，故左栏曾出现第 7 项；
    该问题的最终解法不是可见性标记，而是把物理层从 `plugins/local` 迁入
    `plugins/vdfs`（见 `plugins/vdfs/physical.rs` 模块文档与 §7.1 的 S9 记录）。
    `agent` 的新建暂不支持 VDFS 路径（zip 语义）。
- **S5 下线旧协议**（**已完成**）：
  - **第一步（已完成）**：旧专项路由重定向改指 VDFS 页——
    `model-providers` → `/vdfs/model`、`mcp` → `/vdfs/mcp`、`skill` → `/vdfs/skill`、
    `agent` → `/vdfs/agent`（原先一律指向将被下线的 `/entities/{kind}`）。
    书签 / 深链从此直达 VDFS，不再经过统一实体页。
  - **阻塞已解除（S6）**：会话的「管理内部实体」能力（工作目录树 / 子会话）
    已在 VDFS 上重建，容器页不再是不可替代的入口。详见下面的 S6 记录。
  - **页面下线（已完成）**：移除三条旧路由与整簇页面代码——
    - `/entities/:types?` 改为**保兼容重定向**（单 kind 直达 `/vdfs/{kind}`，
      `all` / 多 kind 回落 `/vdfs/session`）；
    - 容器页 `/container/:kind/:id/entities`、`/agent/:agentId/entities`
      不再保留（能力已由 S7 的 `<id>/<子类别标签>/<条目>` 寻址承担）；
    - 随之删除 `views/WorkbenchView.vue`、`composables/useWorkbenchView.ts`、
      `composables/useWorkbench.ts`、`composables/useEntityProviders.ts`、
      `components/entities/EntityTree.vue` / `EntityTreeNode.vue` /
      `EntityDetailPanel.vue` 及其单测。**前端自此只剩一台三栏工作台**
      （外壳 `MainLayout` + 页面 `VdfsView`）。
  - **连带修复**：实体操作前缀（`worker/session` 等）原先由实体页挂载时
    拉取 `entities/providers` 填充，页面下线后无人加载 → 会话清单会打到
    错误地址。改为由 `services/entities.ts` **自持幂等加载**（首个 entities
    操作前拉一次，失败可重试）。
  - **剩余（已由 S8 完成）**：`services/session.ts` 的 `listSessions()` 曾仍走
    `entities/list`（需要 `message_count` / `metadata`，当时 VDFS 节点未
    下发这些字段）；S8 已迁到 `vdfs/list` 并整体删除 `services/entities.ts`。

- **S6 会话内部在 VDFS 上重建**（**已完成**）：会话保持**叶子**（点击 = 聊天
  详情，语义不变），其内部结构作为**会话同名目录**挂在其下：

  | 地址 | 语义 |
  | --- | --- |
  | `<id>` | 会话叶子（`ext = session`，聊天详情） |
  | `<id>/子会话[/<sub>]` | 子会话清单 / 单个子会话（查看 · 删除） |
  | `<id>/工作目录[/<rel>]` | 工作目录树（文件可查看 / 编辑） |

  - 入口：会话详情新增机制动作「浏览内部」（`VdfsSessionDetail` 注入，
    经 `@browse` 由页面层 `enter(node.path)` 完成）。
  - 后端：会话 `VdfsProvider` 重写 `list/stat/read/write/delete` 处理嵌套路径，
    场景实现复用既有 `workdir` 模块（两条链路同一份校验与 IO）：
    - 新增 `workdir::read_content`（另 `list_children` 去掉未用的 ctx 参数）；
    - `WorkdirWatchManager` 注入 VDFS 广播源，文件变化与机制变更**合流**，
      `<根>` 页面不另开监听；
    - `stat(<id>)` 按**目录视图**回答（只给 `l`）——`stat` 结果被分发层用作
      「当前目录节点」，其访问位决定是否给出新建入口。
  - 前端：`useVdfs.creatableTypes` 的子目录回退**仅限会话根**——否则每个
    子目录都会长出与其语义无关的新建入口（如工作目录里出现「新建会话」）。

- **S7 容器子实体在 VDFS 上重建**（**已完成**）：把 S6 的会话内部寻址**推广到
  通用适配器**——`EntityVdfsAdapter` 现在按 `container_kinds_for(kind)` 支持
  `<id>/<子类别>/<条目>` 三级寻址，agent 目录内部的提示词 / 技能 / MCP
  由此在 VDFS 上可见可编辑（原容器页的最后一处不可替代能力）。

  - 路径段用子类别**标签**（与 S6 同口径）；子实体的 `name` 是 agent 目录内
    相对路径（唯一，可含 `/`），`title` 是 basename（可读）。
  - **新建落位由 `path_hint` 决定**（`prompts/<name>.md` + 文件名 → 实际路径，
    缺省内容取 `default_content`）——路径模板仍是后端唯一真相源，前端零知识。
  - 入口复用既有 `open-container` 通道：由**详情定义**声明该动作（agent 已有
    定义），`VdfsFormDetail` 转发为 `browse`，页面层 `enter(node.path)`。
  - 前端修正：`VdfsFormDetail` 原先按 `w` 位**整表**剥掉定义动作，会把只读
    资源的「浏览内部」一并剥掉；现仅保留纯导航动作 `open-container`。
  - 通用规则：**条目有容器子实体且无详情定义 ⇒ 视为目录**（点进去浏览内部），
    有详情定义则仍是文档（详情优先 + 动作入口）。

- **S8 会话清单上 VDFS（`entities/*` 前端归零）**（**已完成**）：最后一处仍在
  调用 `entities/list` 的前端代码——`services/session.ts` 的 `listSessions()`
  ——改走 `vdfs/list`（`<根>/session`），`services/entities.ts` 整体删除。

  - **会话节点自带清单字段**：`session_node` 在 flatten 的 `attributes` 上挂
    `message_count` / `metadata` / `meta_tags`。它们是**场景数据**，VDFS 只透传；
    会话清单因此不必再为「拿 metadata」保留第二条链路。
  - `meta_tags` 由新增的 `session_meta_tags()` 产出，**实体机制与 VDFS 共用**
    （与 `display_title` / `derive_session_summary` 同口径），两条链路呈现一致。
  - 前端映射：`id` ← `name`、`name` ← `title`、`is_working` ← `status == working`。
  - `VdfsView.tagsOf` 新增渲染 `meta_tags`（后端声明、前端原样渲染，零类型知识），
    会话列表恢复「工作目录名 + 消息数」标签。
  - **前端自此不再有任何 `entities/*` 调用点**；`schemas/entities.ts` 保留
    （`DetailDefinition` 仍是 VDFS `ext = form` 的宿主方言），后端 `entities/*`
    协议与其注册表继续为 VDFS 适配器服务。

- **S9 导航可见性**（**已落地，但机制已整体撤销**）：左栏规范是「六类资源」，
  而 `local`（本地文件）当时是第 7 个子目录。S9 的做法是在机制层引入
  `VdfsProvider::nav_visible()` + 节点属性 + `VdfsMountInfo.nav_visible`
  + 前端过滤，把 `local` 藏起来。

  **该整套机制随后被删除**：物理文件层从 `plugins/local` 迁入 `plugins/vdfs`
  （见 `plugins/vdfs/physical.rs` 的模块文档），它不再是子目录、不参与 `<根>`
  的目录合成，左栏自然只剩六类资源。于是 `nav_visible`、`mount_node()`、
  `VdfsMountInfo`、`vdfs/providers` 端点、`plugins/local/vdfs.rs`
  与前端 `mountNavVisible()` 一并退场——**问题消失了，而不是被标记掩盖**。

  这是一次「先加机制、再发现机制不必存在」的记录：正确结论是
  **一个东西不该出现在左栏，就不该是子目录**，而不是给它打一个隐藏标记。

  **2026-09-15 追加（改造三之后）**：上面的结论要分两种情况读。S9 之所以能撤销机制，
  是因为 `local` 当时**本来就不必是子目录**（物理层已并入 `plugins/vdfs`）——
  「不该出现就不该是子目录」对那种情形成立。但改造三之后，`web` / `local` /
  `gateway` 三个目录**必须留在树里**：它们的全部内容就是一份配置文档，而这份文档的
  地址是**真实地址** `<根>/<插件>/PLUGIN.yml`（vdfs.md §3.4），删掉目录等于删掉
  配置的地址。「它存在、但不必出现在清单里」于是成了另一个问题，答案仍是**机制级**的：
  `VdfsNode::hidden`（与文件系统的隐藏属性同义，文件 / 目录通用：列表里不出现、
  可达性不受影响）+ `VdfsProvider::root_hidden()`（provider 的根也不过是一个目录
  节点，所以「它显示还是隐藏」由这条声明回答，容器合成该目录节点时回填）。

  它**不是** S9 那个 `nav_visible` 的复活：那套东西引入了 `VdfsMountInfo` /
  `mount_node()` / `vdfs/providers` 一整套「挂载清单」中间物，而这次没有任何新概念
  ——隐藏属性本来就该是节点的属性，过滤本来就该在产出列表的一方（容器过滤自己合成的
  子目录清单，以及任何子 provider 交回来的 `list` 结果）。另见 vdfs.md §3.2 / §13.2
  与 `docs/archive/vdfs-review.md` §7 的 S10（该评估已归档）。

  同时，设置页（`<根>/setting`）现在会列出这些配置文档——但那是**另一个目录的清单**，
  条目携带的是各自的真实地址，点开读写仍落在拥有者那份文件上：一处隐藏、一处列出，
  配置本身只有一个地址，没有重复。

- **S10 节点动作（`vdfs/action`）**（**已完成**）：S5 下线实体页时丢了一项能力——
  `model` / `mcp` 的「测试连接」仍在后端（`EntityProvider::test_status`），
  但**前端已无入口**（`VdfsFormDetail` 只接了 `@delete`，且 `capabilities.
  test_connection` 恒为 `false`）。本阶段把它补回 VDFS：

  | 层 | 改动 |
  | --- | --- |
  | 协议 | `vdfs/action`（`{ path, action, payload? }`），与 12 个既有操作同表分发 |
  | trait | `VdfsProvider::action()` 默认 `NotImplemented`；`VdfsMountTable` 只做路径转发 |
  | 适配器 | `EntityVdfsAdapter::action()`：`test` → `EntityProvider::test_status`（`connected` ⇒ `ok`），其余标识 `NotImplemented` |
  | 呈现 | 按钮仍由**详情定义**声明（`actions` + `cap.test_connection` 条件）；`VdfsFormDetail` 把「定义是否声明 test」翻译成能力位，不再硬编码 `false` |
  | 执行 | `useVdfs.runAction(id)` → `vdfs/action` → toast 结果；`testing` 忙态与 `saving` 分开 |

  - **动作 = provider 自持的动词**：VDFS 只透传 `(路径, 标识, 载荷)`，不解释语义；
    「是否支持」由 provider 回答（`NotImplemented`），「长什么样」由宿主方言决定
    ——与 `ext` 决定渲染器是同一个分层原则。
  - 只读节点仍保留 `test` 与 `open-container` 动作：二者都不依赖写权限
    （原先只读时只保留 `open-container`，会把只读资源的自检一并剥掉）。
  - 覆盖：`action_test_uses_entity_test_status`（成功 / 失败 / 未实现 / 不存在的条目 /
    非条目路径）、`action_forwards_verb_and_relative_path`（分层只转发）+ 前端
    `runVdfsAction` 三例（口径翻译、无载荷不发字段、载荷透传）。

- **S11 下线 `entities/*` 调用协议**（**已完成**）：前端（S8）与 LLM 早已零调用，
  剩下的只是**对外遗留面**——6 个插件仍把 `entities/*` 路由到
  `entities::dispatch`。本阶段拆除：

  | 位置 | 处理 |
  | --- | --- |
  | mcp / model / session / setting / skill | 删掉 `route()` 里的 `entities::dispatch` 分支 |
  | agent | 删掉 `entities/list|detail|get|upload|delete` 五个分支与 `entities_*` 三个处理函数（保留 `agent` 的导出路由） |
  | home | 删掉 `entities/providers`（资源类别改由 `<根>` 目录合成下发）及其 `provider_order_override` |
  | `symbio_core/entities.rs` | 删除 `dispatch` 与 7 个 `dispatch_*`、zip 工具、`providers_response*`；保留 `EntityProvider` trait + `entity_write` / `entity_delete`（适配器的唯一依赖） |
  | `schemas/entities.rs` | 删除 `ENTITIES_*` 路径常量与协议请求/响应（保留 `DetailDefinition`、`EntitySummary`、`EntityUploadResponse` / `EntityStatusResponse`） |
  | gateway | 只读白名单由 `entities/*` 换成 VDFS 读操作（`vdfs/list|tree|stat|read|search`） |

  - **实体机制退为内部抽象**：`EntityProvider` 仍在（它是「资源怎么存、怎么校验」的
    实现），但不再有对外地址；唯一消费者是 `EntityVdfsAdapter`。
  - **已知取舍**：随协议一并消失的两个入口，迁移前**已从 UI 不可达**（S5 下线实体页
    后就没有调用方）——① skill / agent 的 **zip 导入**（**已由 S12 以「新建类型
    `zip`」的形式回归**，见下）；② 服务器下发的**导航顺序覆盖**
    `symbio.provider_order`（顺序现由注册表 `order` 决定，不再提供配置覆盖）。
  - 净变化：核心 `entities.rs` 1467 → 634 行，协议契约 700 → 563 行。
  - **前端收尾**：`schemas/entities.ts` 删除协议时代类型（`ProviderInfo` /
    `ProvidersResponse` / `ContainerKindInfo` / `EntitiesListResponse` /
    `EntityUploadResponse` / `EntityStatusResponse` / `DetailDefinitionResponse` /
    `ENTITY_LABELS`），只留 VDFS `ext = form` 的宿主方言（`DetailDefinition` 及其
    附属形状）；`DetailForm` / `Session` 等组件里指向 `entities/*` 的注释改为
    指向 `vdfs/write` / `vdfs/delete` / `vdfs/action`。

- **S12 整包导入（zip 作为一种「新建类型」）**（**已完成**）：S5 下线实体页后
  zip 导入就没有入口，S11 删协议时又连后端实现一起删掉。回归的做法**不新增
  协议操作**——导入本就是「新建」，只是内容来自本地文件：

  | 层 | 内容 |
  | --- | --- |
  | 机制 | `VdfsNewType.source`（`VDFS_NEW_SOURCE_FILE = "file"`）：类型声明「内容取自本地文件」；`VDFS_EXT_ZIP = "zip"` 作为导入类型的扩展名 |
  | 后端 | `EntityProvider::import_zip(ctx, name, zip)` 钩子，默认实现 `entity_import_zip`（EntityStore 型通用解包，**整目录覆盖**）；agent 重写走 `AgentDirStore::import`（id 取自包内 manifest，同名替换） |
  | 适配器 | `root_new_types()` 按注册表 `supports_import` 追加 zip 类型；`write` 的**二进制分支**（`b64`）承接导入，只对挂载根下的条目有效，回 provider 给的 id 并广播变更 |
  | 前端 | `source = file` 的类型渲染**文件选择器**（而非命名输入），目标名由 `newFileNameOf(file.name, ext)` 推导；`arrayBufferToBase64` 分块编码后走既有 `writeVdfsBinary` |

  - 能力来源改为注册表显式声明：新增 `supports_import`（agent / skill / mcp 为
    true）。目录自管型 `agent` 这类类型也能导入——不必先有实体目录。
  - 顺带补上一个静默缺口：agent **没有** `delete_item` 钩子（目录自管型
    EntityStore 型），VDFS 删除 agent 目录会落到默认实现报 `NotImplemented`；
    现已重写为 `AgentDirStore::delete`。
  - 修掉一个历史缺陷：zip 解包先规范化再判隐藏文件，否则 `./a/b.txt` 会因首段
    `.` 被整条丢弃（原实现顺序相反）。

- **S13 整包导出（导入的逆动作）**（**已完成**）：导入回归后，导出是唯一还没
  有 VDFS 等价物的动作（`agent` 的导出动作因它暂留）。它与导入**共用同一
  个「整包往返」语义**，因此也不新增协议操作——做一个节点动作即可：

  | 层 | 内容 |
  | --- | --- |
  | 机制 | `VDFS_ACTION_EXPORT = "export"`：与 `test` 同为 `vdfs/action` 的动词取值 |
  | 后端 | `EntityProvider::export_zip(ctx, id)` 钩子，默认实现 `entity_export_zip`（`zip_dir` 打包整个实体目录，包内顶层目录 = id，与导入端 `strip_common_root` 配对）；agent 重写走 `AgentDirStore::export` |
  | 结果形状 | `EntityExport { id, filename, b64 }`——字段名与 `VdfsContent.b64` 同构，作为**文件载荷**随 `VdfsActionResult.data` 回传 |
  | 适配器 | `action()` 按标识分派 `action_test` / `action_export`；导出只对条目有效，先做存在性校验，不支持时透传 `NotImplemented` |
  | 声明 | agent / skill / mcp 的 `detail_definition` 各自声明 `export` 动作（`supports_import` 为真的三类） |
  | 前端 | `actionFileOf(data)` 按**形状**判定（`filename` + `b64`）而非按动作名——命中即 `downloadBlob`；`DetailForm` 未识别的动作原样 emit `action`，故新增动作前端零改动 |

  - 至此 `agent` 插件**不再有任何自有协议路由**：`route()` 直接返回 `NotFound`
    并指引到 `<根>/agent/…`，`host/handlers.rs` 整个删除。
  - 前端「动作忙态」统一：`DetailForm` 中除 `save` / `delete` 外的动作共用页面
    层的单一忙态标记（同一时刻只可能有一个动作在执行）。

- **S12 清理（历史冗余与被替换代码）**（**已完成**）：新机制稳定后，把随旧协议
  一起失去消费者的东西清掉：

  | 位置 | 处理 |
  | --- | --- |
  | `EntityProviderInfo` | 删 `prefix` / `provider_name` / `compact_list` / `status_indicator` / `capabilities`，导入能力改由 `supports_import` 表达（agent 的 `supports_upload` 随之修正为 `false`——它本就无法「最小 manifest 新建」） |
  | `EntityProvider` trait | 删无调用方的 `provider_name()` 与 `get_item()`（含 session 的重写） |
  | `schemas/entities.rs` | 删 `EntityCapabilities`（能力模型已换成访问位 + 注册表 + 声明的动作）与协议时代的请求/响应（`EntitiesList*` / `EntityUploadRequest` / `EntityGetRequest` / `EntityDeleteRequest` / `EntityStatusRequest` / `DetailDefinition*`）、`ContainerKindInfo`、`EntitySummary.provider` |
  | `agent` 旧协议路由 | 删已被 VDFS 取代的列表/读取/上传/删除/预览，只留尚无等价物的导出 |
  | 前端 | `EntityCapabilities` 收敛为表单渲染器真正消费的两项（`mutable` / `test_connection`），并注明它由渲染器按访问位自算、后端不再下发 |

- **S14 三栏唯一化（首页 = `<根>`，浏览内部 = push 页面）**（**已完成**）：
  一个 VdfsView 对应一个 vdfs 地址；三栏结构组件全 App **只有一份**。

  | 位置 | 处理 |
  | --- | --- |
  | 路由 | 首页 `/` 重定向改为 `/vdfs`（地址 `<根>` 本身，不再落到 `/vdfs/session`）；`vdfs/:mount?` 升级为 `vdfs/:dir(.*)*`（`<根>` 之下可多级深链） |
  | VdfsView | 目录变化由 `router.replace` 改为 `router.push`（每个地址 = 一个可回退的页面）；「浏览内部」/ open-container 由全窗口浮层改为 **push 进容器目录地址**；删除 `containerNode` / Teleport 装配 |
  | VdfsContainerBrowser | **整体删除**——它复制了 VdfsView 的列表 / 详情 / 新建状态机 / 图标徽标函数与几乎全部样式；push 后由同一个 VdfsView 承接，不存在第二份三栏实现 |
  | MainLayout | NavRail 头部新增**左上角返回键**（仅非首页地址页可见）：点击回上一级目录，逐级上溯到首页；导航高亮改前缀匹配（深地址仍高亮所属子目录）；去掉 `RouterView :key`（路由变化由 VdfsView 的 dirParam watch 在实例内消化，浏览器历史因此可用） |
  | useNavRail | `loadCategories` / `reloadCategories` 更名为 `loadNavDirs` / `reloadNavDirs`（`<根>` 子目录清单，无 category 概念） |
  | 测试 | 删除 `vdfsCategoryOf`（category 概念退场）的用例 |

  - **为什么删除浮层**：浮层与首页「同构」却各持一份实现（约 600 行重复），
    每次改三栏交互都要双写。push 页面后浮层的独立 `useVdfs` 实例不再必要——
    返回键即回到出发地址，外层状态天然保留（就是上一级目录的页面状态）。
  - **返回键放应用外壳**：三栏的左栏属于外壳；首页（`<根>`）无返回键，
    非首页地址页（push 出来）才有——与「一个 VdfsView 对应一个 vdfs 地址」
    的模型一致。

- **S15 三栏控件自包含（数据地址绑定；S14 的修正）**（**已完成**）：
  S14 把左栏留在外壳，导致 push 出来的地址页左栏仍是 `<根>` 子目录
  （而正确形态是**绑定地址**的子目录，如会话内部页显示 `<id>` 之下的
  子会话 / 工作目录）。三栏结构遂封装为**自包含控件**，宿主只注入宿主件：

  | 位置 | 处理 |
  | --- | --- |
  | `VdfsWorkbench.vue`（新增） | **三栏控件，唯一实现**：绑定一个**数据地址**（`addr` prop）自包含渲染——左栏 = 绑定地址的子目录导航、中栏 = 选中子目录内容、右栏 = 详情。控件不知道浏览器路由；钻入（点中栏目录 / 「浏览内部」）只 `emit('open', addr)`，呈现方式由宿主决定。`rail-header` / `rail-footer` 插槽 = 宿主件注入点 |
  | `useVdfs.ts` | 数据层绑定 `addr`（Ref）：左栏 / 选中 / 当前目录 / 详情全部由绑定地址驱动；每个地址的左栏选中项有记忆（往返 push / 返回后恢复） |
  | `VdfsView.vue` | 瘦身为**路由宿主**：浏览器地址 ↔ 数据地址换算（`/vdfs` ↔ `<根>`；`/vdfs/<dir…>` ↔ `<根>/<dir…>`，两个概念、一处换算）+ 宿主件注入——首页左下角系统目录入口、非首页左上角返回键（回 **push 来源页** = 浏览器历史 back，不是父目录；深链直开回首页）+ logo |
  | `useNavRail.ts` | **整体删除**——数据层面没有特殊 Nav 概念，外壳左栏与页面左栏是同一份获取逻辑，`useVdfs` 是唯一数据层 |
  | `MainLayout.vue` | 外壳三栏拆除：只剩 RouterView + 全局 Toast + 全局初始化（会话事件监听 / 工作区恢复）——三栏由路由页面自持 |
  | `HomedirEntry.vue`（新增） | 系统目录入口自足化（按钮 + 切换对话框 + 连接显示），谁嵌入谁拥有 |

  - **核心原则**：数据地址 ≠ 浏览器地址。控件/数据层只认数据地址
    （`<根>`、`<根>/session/<id>`），浏览器地址只是承载（甚至可以直接
    urlencode 数据地址）；两者在 `VdfsView` 一处换算。
  - **首页与内部管理页零差别**：同一个控件、同一份数据逻辑，唯一区别是
    绑定的数据地址（`<根>` vs `<根>/session/<id>` / `<根>/agent/<id>`）。

- **S16 收敛终局（废除实体机制，各插件直连 `VdfsProvider`）**（**已完成**）：
  S4 的适配器是**过渡装置**——它让「先有实体机制、再搬上 VDFS」两件事并行推进，
  代价是把每类资源的差异挤进一张 20 钩子的通用接口，再由 1500 行的适配器翻译。
  收敛期结束后整层删除：

  | 删除 | 取代者 |
  | --- | --- |
  | `EntityProvider` trait（20 钩子） | 各插件的 `impl VdfsProvider` |
  | `provider_registry()` / `EntityProviderInfo` | 各 provider 的 `label()` / `order()` / `icon()` / `root_new_types()` |
  | `EntityVdfsAdapter`（1504 行） | 无——插件直连，不需要适配器 |
  | `nav_meta_of()` | 各 provider 自持字面量 |

  - `model` / `skill` / `agent` 三个插件本次补上直连实现（`session` / `setting` /
    `mcp` 此前已直连）。**挂载名与节点形状完全不变**，故前端零改动。
  - `symbio_core/entities.rs` 降为**存储原语自由函数**（写盘 / 删除 / 导入 / 导出），
    没有 trait、没有注册表。
  - 变更广播从适配器实例提到 `vdfs/host.rs`（`notify_change` / `watch_changes` /
    `unwatch_changes`），**按 kind 全局持有**——同一 provider 每次 `traverse` 都会新构造，
    按实例持有会让订阅与投递配不上对。
  - **不变量**：`vdfs_provider.rs` 未因本次收敛做任何修改（它是核心机制）。

- **S17 收敛存储层（`storage_service` → `vdfs_service`，entity 词汇清零）**（**已完成**）：
  S16 删掉了「差异集中在一张 trait」的那一层，但**落盘那一层仍然讲着非 VDFS 的话**：
  磁盘资源由一套私有抽象（`StorageService` / `EntityStore`）表达，各插件再手翻成
  `VdfsNode` / `VdfsContent`——「一类资源 = 一份存储抽象 + 一份翻译」。本阶段把这套
  抽象换成**直接就是 `VdfsProvider` 的三个集中实现**：

  | 删除 | 取代者 |
  | --- | --- |
  | `providers/storage_service`（`StorageService` / `FileEntityStore` / `path_resolver::safe_id`，含一份从未参与编译的 `entity_store.rs`） | `providers/vdfs_service`：`SingleFileVdfs` / `DirVdfs` / `MemoryVdfs` |
  | `symbio_core::entities` 的存储原语自由函数 | `vdfs_service::entry`（寻址 + 原子落盘 + 广播）与 `vdfs_service::pack`（zip / base64）；「manifest 补 id」下沉为 model / mcp 各一份 `with_id` |
  | `symbio_core/providers/storage.rs`（trait + `categories` / `manifests` 常量） | 类别段名 = 插件名 = vdfs 子目录名（`PLUGIN_*` 常量），主文件名写在各插件内部的 `const MANIFEST` |
  | `schemas/entities.rs` 的 `EntitySummary` / `EntityUploadResponse` / `EntityExport` / `ENTITY_*` 常量 | 列表项与详情输入 = `VdfsNode`；写响应 = `VdfsWriteResponse`；导出载荷 = `VdfsPack`（线上字段与原 `EntityExport` 逐字一致） |
  | 事件总线第二条频道 `kind = "entity"`（`KIND_ENTITY` + `publish_entity_changed` / `publish_entity_status`） | 唯一一条 `kind = "vdfs"`：生命周期与运行时状态变化都经 `notify_change` 广播 |

  - **磁盘布局一个字没改**：仍是 `<homedir>/<category>/<id>/<manifest>`。
    三型只是三种**访问拓扑**（单文件不外露条目内部 / 目录可下钻 / 内存不落盘），
    换拓扑不动数据，因此**前端零改动**——地址、`ext`、`schema`、访问位全部原样。
  - **不是第二个抽象**：三个实现本身就是完整的 `impl VdfsProvider`（不是 trait、
    不是适配器、没有钩子表），差异由调用点以普通参数传入；`vdfs_provider.rs` 与
    `plugins/vdfs/*`（协议 / 访问层 / 物理层）**本次零改动**。
  - **前端连带改动**（唯一一处）：`services/eventBus.ts` 的 `KIND_ENTITY` 与实体
    生命周期 / 状态分支退场，清单与状态角标一律订阅 `kind = 'vdfs'`。
  - **明确代价**：`notify_change` 只报 `(kind, path, change)` 三元组、**不携带
    `node` / `content` 载荷**（扩展它等于改 core 协议，本次刻意不做）。因此
    `created` / `updated` / `deleted` 在前端**只能防抖重拉**；带载荷的增益投递
    只存在于 provider 自己实现的 `watch` 里（如会话消息的 `appended` + `delta`）。
    状态角标同理：收到该节点的一次 `updated` 后重读 `vdfs/stat`。

---

## 8. 一致性要求

- 前端**不得**出现 `/` 口径的地址常量或拼接——一切路径运算经 `schemas/vdfs.ts`
  的路径代数，口径恒为 `<根>`。
- 地址翻译**只允许**出现在 `services/vdfs.ts` 一处；页面与渲染器不得感知线路口径。
- 前端**不得**硬编码资源类型、标签、能力或路径模板；只允许 `ext → 渲染器`、
  `子目录名 → 图标` 这类纯 UI 映射。
- 新建入口**只能**由节点声明的 `new_types` 驱动；前端不得凭 `kind` 或写死的类型表
  推断可新建性。
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
