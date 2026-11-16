//! 准备三样东西：exe 的图标资源、窗口图标的像素、版本信息。
//!
//! 图标只有一份源文件 `assets/PiWebKeeper.ico`，从它这里出两条路：
//!   - 编进 exe 资源，托盘（`src/tray.rs`）与 Explorer 读这一份；
//!   - 解出最大那一帧的 RGBA 写进 `OUT_DIR`，窗口图标（`src/ui.rs`）读这一份 ——
//!     eframe 的图标接口要的是像素，不是资源句柄。
//!
//! 字体不在这里处理：它就在 `assets/fonts/` 里，`src/font.rs` 用相对路径
//! `include_bytes!` 直接引用，构建期没有额外的搬运步骤。

fn main() {
    println!("cargo:rerun-if-changed=assets/PiWebKeeper.ico");

    embed_icon();
    extract_window_icon();
}

/// 把 `.ico` 里最大的一帧解成 RGBA，写到 `OUT_DIR/window-icon.rgba`。
///
/// 为什么不直接 `LoadImage` + `WM_SETICON`：eframe 在第一帧之后会按 viewport 的图标设置
/// 再设一次窗口图标，viewport 里没有图标就等于清掉，手工装上去的会被它抹掉（见 `src/ui.rs`）。
fn extract_window_icon() {
    let bytes = std::fs::read("assets/PiWebKeeper.ico").expect("读不到 assets/PiWebKeeper.ico");
    let rgba = largest_frame_as_rgba(&bytes);
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("没有 OUT_DIR")).join("window-icon.rgba");

    std::fs::write(&out, rgba).expect("写不出 window-icon.rgba");
}

/// 从 ICO 里挑最大的一帧，解码成 RGBA8。
fn largest_frame_as_rgba(ico: &[u8]) -> Vec<u8> {
    assert_eq!(&ico[0..4], &[0, 0, 1, 0], "不是 ICO 文件");

    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;

    let mut best: Option<(u32, &[u8])> = None;

    for index in 0..count {
        let entry = &ico[6 + index * 16..6 + (index + 1) * 16];

        // 目录项里 0 表示 256
        let width = if entry[0] == 0 { 256 } else { entry[0] as u32 };
        let size = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
        let offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;

        if best.map(|(best_width, _)| width > best_width).unwrap_or(true) {
            best = Some((width, &ico[offset..offset + size]));
        }
    }

    let (width, frame) = best.expect("ICO 里一帧都没有");

    assert_eq!(
        &frame[0..8],
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        "最大的一帧不是 PNG，图标需要重做成 PNG 帧的 ICO"
    );

    let mut decoder = png::Decoder::new(std::io::Cursor::new(frame));
    decoder.set_transformations(png::Transformations::normalize_to_color8());

    let mut reader = decoder.read_info().expect("PNG 解不开");
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).expect("PNG 帧读不出来");
    buffer.truncate(info.buffer_size());

    assert_eq!(info.width, width, "PNG 帧与目录项声明的尺寸对不上");

    // 统一成 RGBA8：带 alpha 的位图、grayscale 都要补成四通道
    let mut rgba = Vec::with_capacity((info.width * info.height * 4) as usize);

    match info.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(&buffer),
        png::ColorType::Rgb => {
            for pixel in buffer.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for pixel in buffer.chunks_exact(2) {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        png::ColorType::Grayscale => {
            for value in &buffer {
                rgba.extend_from_slice(&[*value, *value, *value, 255]);
            }
        }
        png::ColorType::Indexed => panic!("调色板 PNG 未被展开，ICO 的图标帧需要是 RGBA"),
    }

    rgba
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
