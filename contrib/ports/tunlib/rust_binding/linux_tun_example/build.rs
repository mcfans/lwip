fn main() {
    if cfg!(target_os = "macos") {
        return;
    }
    println!("cargo:rustc-rerun-if-changed=src/open_tun.c");

    cc::Build::new()
        .file("src/open_tun.c")
        .compile("open_tun");
}