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

    /*
     * 版本号从 Cargo.toml 来，不在两处各写一份：
     * 右键 exe → 属性 → 详细信息里的「文件版本」应当和发布标签一致。
     */
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_owned());

    resource.set("ProductName", "Pi Web Keeper");
    resource.set("FileDescription", "Pi Web Keeper");
    resource.set("LegalCopyright", "MIT");
    resource.set("FileVersion", &version);
    resource.set("ProductVersion", &version);

    // VERSIONINFO 的二进制字段：MAJOR << 48 | MINOR << 32 | PATCH << 16 | BUILD
    if let Some(packed) = pack_version(&version) {
        resource
            .set_version_info(winresource::VersionInfo::FILEVERSION, packed)
            .set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed)
            .set_version_info(winresource::VersionInfo::FILEOS, 0x0004_0004)
            .set_version_info(winresource::VersionInfo::FILETYPE, 0x0000_0001);
    }

    /*
     * 资源编译失败不当成构建错误：图标缺失只是让托盘图标退回系统默认，
     * 不该让整个程序编译不过。
     */
    if let Err(error) = resource.compile() {
        println!("cargo:warning=图标资源编译失败：{error}");
    }
}

/// 把 `0.4.0` 这样的版本号压成 VERSIONINFO 的四个 16 位字段。
///
/// 缺的部分按 0 算；`-pre` 之类的后缀在这里没有位置，忽略。
fn pack_version(version: &str) -> Option<u64> {
    let mut parts = version.split('.').map(|part| {
        part.split(['-', '+'])
            .next()
            .and_then(|digits| digits.parse::<u64>().ok())
    });

    let major = parts.next()??;
    let minor = parts.next().flatten().unwrap_or(0);
    let patch = parts.next().flatten().unwrap_or(0);
    let build = parts.next().flatten().unwrap_or(0);

    Some((major << 48) | (minor << 32) | (patch << 16) | build)
}
