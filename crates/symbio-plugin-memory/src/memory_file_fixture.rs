//! 测试夹具的宿主文件；实现都在 `memory_file_fixture.test.rs`。

#[path = "memory_file_fixture.test.rs"]
pub(crate) mod tests;

pub(crate) use tests::{absent, at};
