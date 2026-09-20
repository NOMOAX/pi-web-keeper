"""把一张正方形 PNG 合成 Windows .ico（多尺寸）。

用法：
    python png_to_ico.py source.png
    python png_to_ico.py source.png 16 24 32 48 64 128 256

输出默认与源图同目录、同名，扩展名换成 .ico。

要点：
- 源图最好 256x256 或更大（PS 里按最大尺寸画，缩小交给这里）。
- Windows 会按场合挑最合适的一帧，所以多尺寸比单尺寸清晰得多。
- 非正方形源图会按比例缩放，生成非正方形帧；Windows 图标预期方形帧，所以源图请用正方形。
"""

import os
import sys
from PIL import Image


DEFAULT_SIZES = [16, 24, 32, 48, 64, 128, 256]


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2

    source_path = sys.argv[1]

    # 纯数字参数视为尺寸，其余视为输出路径（兼容旧的两参数写法）
    arguments = sys.argv[2:]
    sizes = [int(a) for a in arguments if a.isdigit()] or DEFAULT_SIZES
    explicit_output = [a for a in arguments if not a.isdigit()]

    output_path = explicit_output[0] if explicit_output else os.path.splitext(source_path)[0] + ".ico"

    image = Image.open(source_path).convert("RGBA")

    image.save(output_path, format="ICO", sizes=[(s, s) for s in sizes])
    print(f"已写出 {output_path}（{len(sizes)} 帧：{', '.join(map(str, sizes))}；源图 {image.width}x{image.height}）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
