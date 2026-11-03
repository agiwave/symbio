//! 第三端口验收（DE-048 D-302）：**真实插件端口**（`symbio-plugin-telegram` 的
//! `notify`）接入生产绑定表。
//!
//! 断言的落点在 [`tests`](self::tests)：生产冻结表 seal 通过、端口与执行体声明在表上、
//! 选债器按 `request(port=notify)` 选中插件声明的行。

#[cfg(test)]
#[path = "notify_port_acceptance.test.rs"]
mod tests;
