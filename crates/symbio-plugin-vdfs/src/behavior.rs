//! `vdfs_*` 写族的**效果**：产物格补一句「这次写属于哪种动作」（DE-025 D-207）。
//!
//! ## 它替掉的是哪一段
//!
//! 工作目录变更账（`symbio_core::projection::changes`）过去按**工具名**认写族：
//! `match tool { "vdfs_write" => …, "vdfs_edit" => … }`。那张表住在 core，于是
//! 「改一个工具名」要动一个不属于任何插件的文件——正是要消灭的形态。
//!
//! 现在动作由**工具自己**说：写族四个工具在 `CapabilityMeta.behaviors` 里声明
//! `file_change`，本模块把声明翻译成一个字段补进产物格。读侧（`changes`）只认那个
//! 字段，不认识任何工具名。
//!
//! ## 为什么是**富化**而不是新格
//!
//! `artifact.added` 已经记下了这次调用（`tool` / `args` / `success` / 溯源），
//! 变更清单折得出来（D-169：清单是纯投影，不该有第二个写点）。缺的只是「哪种动作」
//! ——一个字段，不是一条事实。
//!
//! ## 只补成功的调用
//!
//! 失败 / 被拒的写没有改变工作目录。判据留在**这里**（而不是让读侧再判一次
//! `success`）：一条失败产物的载荷里出现 `file_op`，本身就是「这次写了」的谎。

use symbio_core::FileOp;
use symbio_plugin_api::behaviors;
use symbio_plugin_api::effects::{EffectInput, ToolBehavior};

/// 写族工具 → 动作。
///
/// 名字住本插件（这四个工具就是本插件的实现细节）：改一个工具名只改这一行，
/// core / loop 一个字都不用动。
fn op_of(tool: &str) -> Option<FileOp> {
    match tool.trim().rsplit('/').next().unwrap_or_default() {
        "vdfs_write" => Some(FileOp::Write),
        "vdfs_edit" => Some(FileOp::Edit),
        "vdfs_delete" => Some(FileOp::Delete),
        "vdfs_mkdir" => Some(FileOp::Mkdir),
        _ => None,
    }
}

/// 写族的产物富化：给产物格补 `file_op`。
pub struct FileChange;

impl ToolBehavior for FileChange {
    fn owner(&self) -> &'static str {
        // 与 `PLUGIN.yml` 的 `plugin_id` 同一串：owner 序即装配序。
        "vdfs"
    }

    fn behavior(&self) -> &'static str {
        behaviors::FILE_CHANGE
    }

    fn annotate(&self, input: &EffectInput) -> Vec<(&'static str, serde_json::Value)> {
        if !input.success {
            return Vec::new();
        }
        let Some(op) = op_of(&input.tool) else {
            return Vec::new();
        };
        // 值取 [`FileOp::as_str`]：线格式只有一处定义（core 的枚举），插件不抄一份。
        vec![("file_op", serde_json::json!(op.as_str()))]
    }
}

symbio_plugin_api::submit_behavior!(FileChange);

#[cfg(test)]
#[path = "behavior.test.rs"]
mod tests;
