//! 两件事：
//!
//! 1. 把 `assets/PiWebKeeper.ico` 编进 exe 的图标资源 —— 托盘图标直接从这份资源里取
//!    （见 `src/tray.rs`），与 C# 版的做法一致：exe 里只有一份图标。
//! 2. 把系统的 `msyh.ttc`（微软雅黑）复制到 `OUT_DIR`，好让 `src/font.rs` 用
//!    `include_bytes!` 把它编进 exe。程序运行时不再去系统里找字体。
//!
//! 字体为什么在构建期取、而不是放仓库里：它是 18.8 MB 的二进制，
//! 放进版本库只会让每次 clone 都拖上它；而它所要求的机器本来就是能构建 Windows 程序的机器。

use std::path::{Path, PathBuf};

/// 构建期要读的系统字体。
const FONT_FILE: &str = "Fonts\\msyh.ttc";

fn main() {
    println!("cargo:rerun-if-changed=assets/PiWebKeeper.ico");
    println!("cargo:rerun-if-changed=src/font.rs");

    embed_icon();
    stage_font();
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

/// 把系统里的微软雅黑复制到 `OUT_DIR`。
///
/// 找不到就直接让构建失败并说清原因 —— 这个程序是**内嵌**字体的，
/// 在没有这款字体的机器上编出来的 exe 会缺字，那比构建失败更难查。
fn stage_font() {
    let destination = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("msyh.ttc");

    let source: PathBuf = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join(FONT_FILE);

    if !source.exists() {
        panic!(
            "内嵌字体需要 {}，但它在当前机器上不存在。\n\
             换一台装了微软雅黑的 Windows 构建，或者改 src/font.rs 换一款字体。",
            source.display()
        );
    }

    // 源文件没变就不重复复制那 18.8 MB
    if is_up_to_date(&source, &destination) {
        return;
    }

    std::fs::copy(&source, &destination).unwrap_or_else(|error| {
        panic!(
            "复制字体 {} -> {} 失败：{error}",
            source.display(),
            destination.display()
        )
    });
}

/// 目标已存在且与源一样新、一样大。
fn is_up_to_date(source: &Path, destination: &Path) -> bool {
    let (Ok(source), Ok(destination)) = (source.metadata(), destination.metadata()) else {
        return false;
    };

    if source.len() != destination.len() {
        return false;
    }

    match (source.modified(), destination.modified()) {
        (Ok(source), Ok(destination)) => destination >= source,
        _ => false,
    }
}
