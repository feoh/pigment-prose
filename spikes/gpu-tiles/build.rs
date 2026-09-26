// Records the resolved wgpu version so benchmark output names it.
fn main() {
    let lock = std::fs::read_to_string("Cargo.lock").unwrap_or_default();
    let mut version = "unknown".to_string();
    let mut in_wgpu = false;
    for line in lock.lines() {
        if line == "name = \"wgpu\"" {
            in_wgpu = true;
        } else if in_wgpu {
            if let Some(v) = line.strip_prefix("version = ") {
                version = v.trim_matches('"').to_string();
            }
            in_wgpu = false;
        }
    }
    println!("cargo:rustc-env=WGPU_VERSION={version}");
    println!("cargo:rerun-if-changed=Cargo.lock");
}
