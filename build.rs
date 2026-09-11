fn main() {
    println!("cargo:rerun-if-changed=native/winspd_shim.c");
    println!("cargo:rerun-if-env-changed=WINSPD_SDK");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || std::env::var_os("CARGO_FEATURE_WINSPD").is_none()
    {
        return;
    }
    let sdk = std::env::var_os("WINSPD_SDK")
        .expect("Set WINSPD_SDK to the pinned WinSpd SDK directory (inc and lib)");
    let sdk = std::path::PathBuf::from(sdk);
    cc::Build::new()
        .file("native/winspd_shim.c")
        .include(sdk.join("inc"))
        .compile("winluks_shim");
    println!(
        "cargo:rustc-link-search=native={}",
        sdk.join("lib").display()
    );
    println!("cargo:rustc-link-lib=winspd_x64");
    println!("cargo:rustc-link-lib=ole32");
}
