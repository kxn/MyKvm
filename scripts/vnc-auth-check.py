#!/usr/bin/env python3
"""用 vncdotool 独立 VNC 客户端验证密码挑战互操作（issue #31 最小鉴权）。

依赖：`python3 -m venv .venv && .venv/bin/pip install vncdotool`

用法：

    scripts/vnc-auth-check.py --port 5900 --password abc12345

先启动启用了 VNC 密码的 headless 服务（如 `--vnc-password abc12345`），本
脚本以标准客户端完成密码挑战并发送一个指针事件。密码正确返回 0；密码错误
或挑战失败时 vncdotool 抛出认证异常，以非零码退出。
"""

import argparse
import sys

from vncdotool import api


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--password", required=True)
    parser.add_argument("--timeout", type=float, default=30.0)
    args = parser.parse_args()

    server = f"{args.host}::{args.port}"
    with api.connect(server, password=args.password, timeout=args.timeout) as client:
        client.mouseMove(0, 0)
    print("VNC 密码挑战互操作验证通过")
    return 0


if __name__ == "__main__":
    exit_code = main()
    api.shutdown()
    sys.exit(exit_code)
