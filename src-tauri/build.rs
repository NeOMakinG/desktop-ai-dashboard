fn main() {
    let native = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = native.parent().expect("repository root");
    let resources = native.join("resources/managed-hermes");
    println!("cargo:rerun-if-changed=../scripts/managed-hermes");
    println!("cargo:rerun-if-changed=../runtime/forma_runtime");
    println!("cargo:rerun-if-changed=resources/managed-hermes");
    // Tauri's dev/build hooks prepare assets. Cargo itself never downloads or
    // silently falls back to a developer's Python/profile. Check the entire
    // inventory before allowing native compilation to consume these resources.
    let status = std::process::Command::new("node")
        .arg(root.join("scripts/managed-hermes/prepare.mjs"))
        .arg("--verify")
        .current_dir(root)
        .env("TARGET", std::env::var("TARGET").unwrap())
        .status()
        .expect("Node.js is needed to verify managed Hermes build resources");
    assert!(
        status.success(),
        "Managed Hermes resources missing/stale. Run pnpm runtime:prepare, or use pnpm desktop / pnpm desktop:build."
    );
    if std::env::var("PROFILE").as_deref() == Ok("debug") {
        println!(
            "cargo:rustc-env=FORMA_MANAGED_HERMES_DIR={}",
            resources.display()
        );
    }
    tauri_build::build();
}
