//! 把 `assets/PiWebKeeper.ico` 编进 exe 的图标资源 —— 托盘图标直接从这份资源里取
//! （见 `src/tray.rs`）：exe 里只有一份图标。
//!
//! 字体不在这里处理：它就在 `assets/fonts/` 里，`src/font.rs` 用相对路径
//! `include_bytes!` 直接引用，构建期没有额外的搬运步骤。

fn main() {
    println!("cargo:rerun-if-changed=assets/PiWebKeeper.ico");

    embed_icon();
}

fn embed_icon() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("assets/PiWebKeeper.ico");
    resource.set("ProductName", "Pi Web Keeper");
    resource.set("FileDescription", "Pi Web Keeper");
    resource.set("LegalCopyright", "MIT");

    /*
     * 资源编译失败不当成构建错误：图标缺失只是让托盘图标退回系统默认，
     * 不该让整个程序编译不过。
     */
    if let Err(error) = resource.compile() {
        println!("cargo:warning=图标资源编译失败：{error}");
    }
}
