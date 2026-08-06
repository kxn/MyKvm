## 关联 issue

Refs #35（FU-4：dirty rects 开关 + 帧源串联）

## 改动摘要

让 E（#21）的 dirty rects 管线真正生效——帧源采集循环持有 `DirtyRectDetector`，开启时每帧检测填 `VideoFrame.dirty_rects`。之前 E 只搭了管线（detector + 多矩形编码 + driver 串联），但 `dirty_rects` 始终 None，优化未生效。

### ipkvm-video
- `FileVideoSource::new_with_dirty_rects(assets, fps, tile_size)` + `LoopingVideoSource` 同名构造器：spawn 闭包内持 `DirtyRectDetector`，每帧 `detect()` 后填 `frame.dirty_rects`。
- `new()`（旧签名）作 wrapper 传 None（关闭）。

### headless config
- CLI `--dirty-rects`（开关）+ `--dirty-rect-tile-size <N>`（默认 32）。
- config `[video] dirty_rects` / `dirty_rect_tile_size` 字段。
- `build_source` 开启时传 tile_size 到 `FileVideoSource`。

### 当前覆盖
- file_source + looping（demo/测试路径）：已生效。
- camera（Windows DirectShow）：留后续（采集循环更复杂，需在 COM 线程内加 detector）。

## 测试

`cargo fmt --all --check` 干净；42 套件 705 测试 0 失败。

Refs #35
