fn main() {
    if std::env::var_os("CARGO_FEATURE_SYSTEM_SLEEF").is_none() {
        return;
    }

    if let Some(path) = std::env::var_os("SLEEF_LIB_DIR") {
        println!("cargo:rustc-link-search=native={}", path.to_string_lossy());
        println!("cargo:rerun-if-env-changed=SLEEF_LIB_DIR");
    }
}
