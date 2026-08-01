#!/usr/bin/env bash
# 下载并转换演示素材：两个不同分辨率的 Big Buck Bunny 片段，输出为 Y4M。
#
# 用法：
#
#     scripts/fetch-demo-assets.sh [输出目录]
#
# 默认输出到仓库根目录下的 .cache/demo-assets。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ASSET_DIR="${1:-$REPOSITORY_ROOT/.cache/demo-assets}"

if ! command -v ffmpeg >/dev/null 2>&1; then
    echo "未找到 ffmpeg，请先安装（例如 apt install ffmpeg）" >&2
    exit 1
fi

mkdir -p "$ASSET_DIR"

BASE_URL="https://test-videos.co.uk/vids/bigbuckbunny/mp4/h264"
echo "==> 下载 640x360 样本"
curl -fsSL -o "$ASSET_DIR/bbb_360.mp4" \
    "$BASE_URL/360/Big_Buck_Bunny_360_10s_1MB.mp4"
echo "==> 下载 1280x720 样本"
curl -fsSL -o "$ASSET_DIR/bbb_720.mp4" \
    "$BASE_URL/720/Big_Buck_Bunny_720_10s_5MB.mp4"

echo "==> 转换为 Y4M（640x360）"
ffmpeg -y -hide_banner -loglevel error \
    -i "$ASSET_DIR/bbb_360.mp4" \
    -an -pix_fmt yuv420p -r 10 -t 3 -f yuv4mpegpipe \
    "$ASSET_DIR/bbb_360.y4m"

echo "==> 转换为 Y4M（1280x720）"
ffmpeg -y -hide_banner -loglevel error \
    -i "$ASSET_DIR/bbb_720.mp4" \
    -an -pix_fmt yuv420p -r 10 -t 3 -f yuv4mpegpipe \
    "$ASSET_DIR/bbb_720.y4m"

ls -lh "$ASSET_DIR"/*.y4m
echo "素材就绪：$ASSET_DIR"
