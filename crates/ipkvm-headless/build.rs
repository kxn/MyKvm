fn main() {
    println!("cargo::rerun-if-changed=web");
    println!("cargo::rerun-if-changed=../../third_party/novnc/1.7.0");
}
