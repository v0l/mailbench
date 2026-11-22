fn main() {
    println!("cargo:rerun-if-changed=packaging/windows/icon.ico");
    println!("cargo:rustc-check-cfg=cfg(smime)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if std::env::var_os("CARGO_FEATURE_SMIME_OPENSSL").is_some() && os != "windows" && os != "android" {
        println!("cargo:rustc-cfg=smime");
    }
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("packaging/windows/icon.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon not embedded: {e}");
        }
    }
}
