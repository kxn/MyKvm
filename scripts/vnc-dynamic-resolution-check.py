#!/usr/bin/env python3
"""用 vncdotool 独立 VNC 客户端验证动态分辨率切换。

依赖：`python3 -m venv .venv && .venv/bin/pip install vncdotool`

用法：

    scripts/vnc-dynamic-resolution-check.py [--host 127.0.0.1] [--port 5900]

客户端持续请求增量更新，记录观察到的桌面尺寸；在超时前观察到至少两种
不同尺寸（例如 640x360 与 1280x720）即返回 0，否则返回 1。
"""

import argparse
import sys
import time

from vncdotool import api


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=5900)
    parser.add_argument("--timeout", type=float, default=30.0)
    args = parser.parse_args()

    observed_sizes = []
    deadline = time.monotonic() + args.timeout
    server = f"{args.host}::{args.port}"

    with api.connect(server, password=None, timeout=args.timeout) as client:
        while time.monotonic() < deadline:
            try:
                client.refreshScreen(incremental=True)
            except Exception as error:  # 连接断开或超时
                print(f"连接结束：{error}", file=sys.stderr)
                break

            size = client.screen.size
            if observed_sizes and size == observed_sizes[-1]:
                continue

            observed_sizes.append(size)
            print(f"观察尺寸：{size[0]}x{size[1]}", flush=True)
            if len(set(observed_sizes)) >= 2:
                break

    distinct = set(observed_sizes)
    if len(distinct) >= 2:
        print(f"通过：观察到 {len(observed_sizes)} 次尺寸变化，共 {len(distinct)} 种尺寸")
        return 0

    print(
        f"失败：超时前只观察到 {distinct}，未出现动态分辨率切换",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    exit_code = main()
    api.shutdown()
    sys.exit(exit_code)
