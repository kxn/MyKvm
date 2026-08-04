fn main() {
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_COMMIT={commit}");
    println!("cargo:rerun-if-changed=../../.git/HEAD");

    // 将正式 iced 桌面端图标嵌入 Windows exe；其它目标由 manifest_optional 跳过。
    embed_resource::compile("assets/icon.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("embed iced exe icon resource");
    println!("cargo:rerun-if-changed=assets/icon.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
