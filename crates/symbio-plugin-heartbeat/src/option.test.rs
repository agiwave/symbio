//! `option` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）。
//!
//! 这里锁的是会话选项栏**本插件那一格**的声明。前三条随调度从
//! `plugins/session/options/tests.rs` 一起搬来（那条字段不再由会话声明，
//! 判据却一条没少——搬文件不等于搬掉回归）。

use super::*;
use crate::setting::HeartbeatConfig;

/// 心跳：字段自带子定义（`form` widget），且**基础设置恒可见**。
#[test]
fn heartbeat_field_carries_its_sub_definition_and_summary() {
    let f = heartbeat_field();
    assert_eq!(f.widget, "form");
    let form = f.form.expect("心跳子定义存在");

    let keys: Vec<&str> = form.sections[0]
        .fields
        .iter()
        .map(|x| x.key.as_str())
        .collect();
    assert_eq!(
        keys,
        vec!["enabled", "interval_seconds", "prompt", "include_history"]
    );
    // 基础设置必须恒可见（不得带 visible_when）：否则未启用时表单只剩开关，
    // 用户看不到任何可填参数（历史回归）。
    for sub in &form.sections[0].fields {
        assert!(
            sub.visible_when.is_none(),
            "字段 {} 不应带显隐门控",
            sub.key
        );
    }

    // 紧凑形态的一句话摘要：代表值取子定义的 title_from（enabled），
    // 再按本字段 options 查「值→标签」⇒ 未启用 = 未开启
    assert_eq!(form.title_from, vec!["enabled".to_string()]);
    let labels: Vec<(&str, &str)> = f
        .options
        .iter()
        .map(|o| (o.value.as_str(), o.label.as_str()))
        .collect();
    assert_eq!(labels, vec![("true", "已开启"), ("false", "未开启")]);

    // 未配置过心跳时紧凑形态要显示「未开启」而不是字段名：这靠 `default`
    // （子对象缺省值）给出 `enabled = false`，再按 `title_from` + `options` 查表。
    let d = f.default.as_ref().expect("必须给子对象缺省值");
    assert_eq!(d["enabled"], json!(false));
    assert_eq!(d["interval_seconds"], json!(DEFAULT_HEARTBEAT_INTERVAL));
    assert_eq!(d["prompt"], json!(""));
    assert_eq!(d["include_history"], json!(true));
}

/// 子表单里 `interval_seconds` 的 `default` 与子对象缺省值**同源**（都是
/// [`DEFAULT_HEARTBEAT_INTERVAL`]）：两处各写一个数字迟早会漂。
#[test]
fn heartbeat_interval_defaults_agree() {
    let form = heartbeat_field().form.expect("心跳子定义存在");
    let interval = form.sections[0]
        .fields
        .iter()
        .find(|x| x.key == "interval_seconds")
        .expect("interval_seconds 字段存在");
    assert_eq!(
        interval.default,
        Some(json!(DEFAULT_HEARTBEAT_INTERVAL)),
        "子表单字段缺省值必须与子对象缺省值一致"
    );
}

/// 子对象缺省值与**数据的 owner**（[`HeartbeatConfig::default`]）同源。
///
/// 这一条是切片之后新增的账：缺省值现在有两处定义（表单预填与结构体默认），
/// 两处不一致的表现是「会话从没配过心跳，选项栏却显示成一个不存在的间隔」。
#[test]
fn heartbeat_defaults_match_the_config_default() {
    let d: Value = heartbeat_defaults();
    assert_eq!(d, HeartbeatConfig::default().view());
}

/// 「立即心跳」按钮**已取消** —— 防回归。
///
/// 它原是一个独立命令选项（图标 `play`，点了直接调 `session/heartbeat/trigger`）。
/// 取消的理由是**它的作用与「在输入框里直接发一条消息」完全重复**——心跳的实质
/// 就是往会话发一轮提示词，想立刻做一次直接在输入框发即可。若哪天有人把它加回来，
/// 同一件事就又有了两个入口，且按钮那个绕开了对话本身。
///
/// 注意这里锁的是**两件事**：触发按钮没了、**心跳配置仍在**（取消的只是那个
/// 手动触发入口，不是心跳任务本身）。
#[test]
fn heartbeat_trigger_option_is_gone() {
    let f = heartbeat_field();
    assert_eq!(f.key, "heartbeat", "心跳任务**配置**必须保留");
    assert!(
        !f.key.contains("trigger"),
        "「立即心跳」按钮不得被加回来：它与「输入框里直接发一条消息」重复"
    );
    // 子定义里也不能藏一个触发动作：本文件的 actions 只有「保存」
    let form = f.form.expect("心跳子定义存在");
    let actions: Vec<&str> = form.actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(actions, vec!["save"]);
}

/// 号段 `60` 是登记在 `plugins/session/options/mod.rs` 模块文档里的**跨插件约定**
/// （插件之间不可见，只能靠号段）。撞车的表现是两行叠在一起，而不是编译错误。
#[test]
fn order_slot_is_the_registered_one() {
    assert_eq!(ORDER_HEARTBEAT, 60);
}
