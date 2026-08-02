fn main() {
    // 把 git 短提交号注入 GIT_COMMIT 环境变量，窗口标题显示当前构建来源，
    // 便于确认用户运行的是哪个版本。
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_COMMIT={commit}");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
}
