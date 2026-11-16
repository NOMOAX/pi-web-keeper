//! 两件事：
//!
//! 1. 把 `assets/PiWebKeeper.ico` 编进 exe 的图标资源 —— 托盘图标直接从这份资源里取
//!    （见 `src/tray.rs`），与 C# 版的做法一致：exe 里只有一份图标。
//! 2. 扫描 `assets/fonts/`，把找到的字体文件生成成 `include_bytes!`。
//!    目录空着就生成 `None`，运行时退回系统字体（见 `src/font.rs`）。
//!
//! 字体为什么不在 `include_bytes!` 里直接写死路径：那个目录允许为空，
//! 而 `include_bytes!` 指向不存在的文件会直接编译失败。

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=assets/PiWebKeeper.ico");
    println!("cargo:rerun-if-changed=assets/fonts");

    embed_icon();
    generate_font_data();
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

/// 在 `assets/fonts/` 里挑第一个字体，生成 `EMBEDDED_FONT`。
fn generate_font_data() {
    let directory = Path::new("assets/fonts");
    let found = first_font_file(directory);

    let generated = match found {
        Some(path) => {
            let absolute = std::fs::canonicalize(&path).unwrap_or_else(|_| PathBuf::from(&path));
            let name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "embedded".to_owned());

            // 路径里的反斜杠在 Rust 字符串字面量里要转义
            let literal = absolute.to_string_lossy().replace('\\', "\\\\");

            format!(
                "/// 编进二进制的字体：显示名与字节。\npub const EMBEDDED_FONT: Option<(&str, &[u8])> = \
                 Some((\"{name}\", include_bytes!(\"{literal}\")));\n"
            )
        }
        None => {
            "/// 编进二进制的字体：显示名与字节。\n\
             ///\n\
             /// 现在为空 —— `assets/fonts/` 里没有字体文件，运行时会退回系统字体。\n\
             pub const EMBEDDED_FONT: Option<(&str, &[u8])> = None;\n"
                .to_owned()
        }
    };

    let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("font_data.rs");
    std::fs::write(&out, generated).expect("写入 font_data.rs 失败");
}

fn first_font_file(directory: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(directory).ok()?;

    let mut candidates: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            matches!(
                path.extension()
                    .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
                    .as_deref(),
                Some("ttf") | Some("otf")
            )
        })
        .collect();

    // 排序取第一个，避免文件系统枚举顺序让构建结果不稳定
    candidates.sort();
    candidates.into_iter().next()
}
