#!/usr/bin/env bash
# Linux 桌面 release 启动冒烟：验证 ipkvm-desktop-iced release 二进制能启动、
# 持续存活并创建顶层窗口（与 scripts/verify-desktop-release.ps1 语义对齐，
# 见 issue #37）。
#
# 用法：
#   scripts/verify-desktop-release.sh [可执行文件路径] [启动超时秒数]
#
# 窗口检测工具优先级：xdotool → xwininfo → xlsclients；三者全缺时降级为
# 进程存活检查并输出警告（失败仍非零退出）。无 DISPLAY 时自动用 xvfb-run -a
# 重建虚拟显示，使应用与窗口检测工具处于同一显示。
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# 二进制默认路径：优先尊重 CARGO_TARGET_DIR（与 cargo/cargo-make 约定一致，
# 可能指向仓库外目标目录），否则用仓库内 target 目录。
TARGET_DIR="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}"
if [[ "${TARGET_DIR}" != /* ]]; then
    TARGET_DIR="${REPO_ROOT}/${TARGET_DIR}"
fi
EXECUTABLE_PATH="${1:-${TARGET_DIR}/release/ipkvm-desktop-iced}"
STARTUP_TIMEOUT_SECONDS="${2:-15}"
WINDOW_TITLE="my_ipkvm iced"
CHILD_PID=""

# 无 DISPLAY 时在 xvfb-run -a 的虚拟显示里重新执行本脚本（内层 DISPLAY 已由
# xvfb-run 设置，窗口检测工具与应用看到同一显示）。
if [[ -z "${DISPLAY:-}" ]] && command -v xvfb-run >/dev/null 2>&1; then
    echo "No DISPLAY set, re-running under xvfb-run -a"
    exec xvfb-run -a bash "${BASH_SOURCE[0]}" "$@"
fi

cleanup() {
    if [[ -n "${CHILD_PID}" ]] && kill -0 "${CHILD_PID}" 2>/dev/null; then
        # 优先按进程组终止（覆盖 xvfb-run 等派生子进程场景），失败回退单进程。
        kill -TERM -- "-${CHILD_PID}" 2>/dev/null || kill -TERM "${CHILD_PID}" 2>/dev/null || true
        for _ in $(seq 1 20); do
            kill -0 "${CHILD_PID}" 2>/dev/null || break
            sleep 0.1
        done
        kill -KILL -- "-${CHILD_PID}" 2>/dev/null || kill -KILL "${CHILD_PID}" 2>/dev/null || true
    fi
}
trap cleanup EXIT

if [[ ! -f "${EXECUTABLE_PATH}" ]]; then
    echo "Desktop release executable does not exist: ${EXECUTABLE_PATH}" >&2
    exit 1
fi

has_tool() { command -v "$1" >/dev/null 2>&1; }

WINDOW_TOOL=""
if has_tool xdotool; then
    WINDOW_TOOL="xdotool"
elif has_tool xwininfo; then
    WINDOW_TOOL="xwininfo"
elif has_tool xlsclients; then
    WINDOW_TOOL="xlsclients"
fi

if [[ -z "${WINDOW_TOOL}" ]]; then
    echo "WARNING: none of xdotool/xwininfo/xlsclients found; degraded to process-alive check only" >&2
fi

# 独立会话启动，便于按进程组清理（setsid 不可用时退化为普通后台进程）。
if has_tool setsid; then
    setsid "${EXECUTABLE_PATH}" &
else
    "${EXECUTABLE_PATH}" &
fi
CHILD_PID=$!

# 进程是否存活（僵尸进程视为已退出，kill -0 对未回收僵尸仍返回成功）。
process_alive() {
    kill -0 "${CHILD_PID}" 2>/dev/null || return 1
    if [[ -r "/proc/${CHILD_PID}/stat" ]]; then
        local state
        state="$(cut -d' ' -f3 "/proc/${CHILD_PID}/stat")"
        [[ "${state}" != "Z" ]]
        return
    fi
    return 0
}

window_found() {
    case "${WINDOW_TOOL}" in
        xdotool)
            xdotool search --name "${WINDOW_TITLE}" >/dev/null 2>&1
            ;;
        xwininfo)
            xwininfo -root -tree 2>/dev/null | grep -q "${WINDOW_TITLE}"
            ;;
        xlsclients)
            xlsclients -l 2>/dev/null | grep -q "${WINDOW_TITLE}"
            ;;
        *)
            return 1
            ;;
    esac
}

deadline=$(( $(date +%s) + STARTUP_TIMEOUT_SECONDS ))
found=0
while [[ $(date +%s) -lt ${deadline} ]]; do
    if ! process_alive; then
        echo "Desktop release exited (pid=${CHILD_PID}) before creating a top-level window" >&2
        exit 1
    fi
    if [[ -n "${WINDOW_TOOL}" ]] && window_found; then
        found=1
        break
    fi
    sleep 0.1
done

if [[ ${found} -eq 1 ]]; then
    echo "Desktop release startup passed: pid=${CHILD_PID}, window='${WINDOW_TITLE}' (via ${WINDOW_TOOL})"
elif [[ -z "${WINDOW_TOOL}" ]]; then
    # 降级模式：无窗口检测工具，超时内进程持续存活即通过。
    if process_alive; then
        echo "Desktop release startup passed (degraded, process alive): pid=${CHILD_PID}"
    else
        echo "Desktop release exited (pid=${CHILD_PID}) within ${STARTUP_TIMEOUT_SECONDS}s" >&2
        exit 1
    fi
else
    echo "Desktop release stayed alive but created no top-level window within ${STARTUP_TIMEOUT_SECONDS}s" >&2
    exit 1
fi
