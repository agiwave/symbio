fn main() {
    // 确保当模型文件发生变化时，包含 include_bytes! 的代码能重新编译
    println!("cargo:rerun-if-changed=src/plugins/agent/embedding/");
}
