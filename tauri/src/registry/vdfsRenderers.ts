/**
 * VDFS 渲染器装配 —— 「渲染器标识 → 组件」的唯一装配点（UI 资产，非数据契约）
 *
 * 机制分层：
 * - `schemas/vdfs.ts`   数据契约（与后端 protocol.rs 对齐，零组件知识）
 * - `registry/vdfsTypes.ts` 渲染器标识与 ext → 渲染器映射（纯 UI 映射，零组件导入）
 * - 本文件               把标识绑定到具体 Vue 组件（**唯一 import 组件的地方**）
 *
 * 这样 vdfsTypes 可被服务层/组合式安全引用而不牵连组件图；新增一种详情形态
 * 只需在此登记一行（未登记的渲染器由视图兜底，页面永不空白）。
 */

import { markRaw } from 'vue'
import VdfsFormDetail from '@/components/vdfs/VdfsFormDetail.vue'
import VdfsTextDetail from '@/components/vdfs/VdfsTextDetail.vue'
import VdfsReadonlyDetail from '@/components/vdfs/VdfsReadonlyDetail.vue'
import VdfsSessionDetail from '@/components/vdfs/VdfsSessionDetail.vue'
import Appearance from '@/components/settings/Appearance.vue'
import About from '@/components/settings/About.vue'
import { registerVdfsRenderer } from './vdfsTypes'

// 机制级呈现形态（与场景无关）
registerVdfsRenderer('form', markRaw(VdfsFormDetail))
registerVdfsRenderer('session', markRaw(VdfsSessionDetail))
registerVdfsRenderer('markdown', markRaw(VdfsTextDetail))
registerVdfsRenderer('json', markRaw(VdfsTextDetail))
registerVdfsRenderer('text', markRaw(VdfsTextDetail))
// 前端状态自持的专属面板（节点以语义类型名为 ext 声明，前端在此绑定组件）
registerVdfsRenderer('appearance', markRaw(Appearance))
registerVdfsRenderer('about', markRaw(About))
// 目录与未命中：机制级只读兜底（资源管理器永不空白的原因）
registerVdfsRenderer('dir', markRaw(VdfsReadonlyDetail))
registerVdfsRenderer('fallback', markRaw(VdfsReadonlyDetail))
