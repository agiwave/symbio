//! 读路径：存储视图。
//!
//! 读方法回答「存储里有什么」，与写侧 [`super::write`] 的「哪些消息能留在存储里」相对。

use super::*;

impl PersistentChatSession {
    /// 全量消息（按单调 `seq` 排序后的存储视图）。
    pub(crate) async fn get_messages(&self) -> Result<Vec<ChatNode>, PluginError> {
        let session = self.load_session().await?;
        let mut messages: Vec<_> = session.messages.to_vec();
        // 按**单调序号** `seq` 排序（稳定排序，缺失 seq 的旧数据排最后并保持插入顺序）。
        //
        // 不用 `sort_by_key(|m| m.timestamp)`，它有两个致命问题：
        //   1. `Option` 序是 `None < Some(_)`，缺失 timestamp 的消息被顶到最前面，
        //      会话顺序被打乱，父节点可能排到子节点之后；
        //   2. timestamp 是"时刻"不是"顺序"——同一毫秒批量落库的消息会并列，
        //      排序退化为依赖数组当前顺序，而数组顺序又会被上一次错误排序打乱。
        // 改用写入时分配的单调 seq 后，两者都被消除。
        messages.sort_by_key(|m| m.seq.unwrap_or(i64::MAX));
        // 建树策略投影：重建核心 config-free、从事件 `root_id` 一律重derive
        // 成 grouped（哪怕落库/消息文件写的是 flat）。`flat` 档在这里把本轮响应骨架的
        // 直接子节点升根并打 `turn_root` 记号——与 live 流式帧、`session_content` 呈现
        // 同一个投影，三格锁步。`grouped` 档下 `flatten_whole_tree` 立即返回，零改动。
        symbio_sdk::turn::node_build::flatten_whole_tree(
            &mut messages,
            self.cfg_or_default().presentation_tree(),
        );

        Ok(messages)
    }
}
