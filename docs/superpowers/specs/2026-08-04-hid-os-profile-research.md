# HID 鼠标绝对/相对模式与 OS 兼容性阶段性调研

日期：2026-08-04

状态：阶段性调研记录，**不是最终 OS profile 兼容性结论表**。

## 1. 调研目标与结论边界

本次调研针对 CH9329 键鼠模拟链路中的两个问题：

- BIOS/UEFI 中绝对模式和相对模式都可以工作，但进入 Ubuntu 后绝对模式不移动；
- Windows 10 已实测绝对模式可用，且使用体验良好；
- Windows 7 当前表现为系统似乎没有识别设备，之后还出现了目标机或 CH9329 无响应/死机现象，暂时停止追查。

当前结论分为四种证据等级：

- **协议事实**：来自 USB HID、UEFI、Microsoft、Linux 或 WCH 文档；
- **仓库事实**：来自当前代码和已有自动化测试；
- **人工实测**：本项目当前硬件和目标 OS 上观察到的行为；
- **待验证假设**：根据输入栈行为推断出的原因，不能当作 OS 兼容性结论。

本文件先记录共性规律和已知现象。等 Windows、Linux、macOS、各种 BIOS/UEFI 和其他目标环境完成实测后，再单独整理 OS profile 的最终矩阵。本轮不把 Ubuntu 或 Windows 7 的现象扩展成所有 Linux 或所有 Windows 版本的结论。

## 2. 最重要的结论

### 2.1 Absolute/Relative 是 USB HID 的标准语义

Absolute 和 Relative 不是 CH9329 发明的两种鼠标协议，而是 USB HID 报告描述符中 `Input` 项的标准语义。

在 HID `Input` 项的属性位中，`Absolute` 表示报告值代表一个绝对位置，`Relative` 表示报告值代表相对于上一位置的增量。操作系统读取 HID 报告描述符后，通常会将其映射为不同类型的输入事件：

- 绝对轴通常对应 Linux `EV_ABS`、Windows 的 absolute pointing 数据或触摸屏/数位板类输入；
- 相对轴通常对应 Linux `EV_REL`、Windows 的 relative mouse 数据或传统鼠标移动。

因此，Absolute/Relative 本身是标准 HID 输入模型的一部分，但“桌面是否把这个设备当作普通鼠标光标”不是 HID 标准单独保证的。还要看 HID Usage、Application Collection、内核设备分类、桌面输入栈和窗口系统。

### 2.2 CH9329 的 `0x04`/`0x05` 是私有串口命令

CH9329 接收的串口命令不是 USB HID 报告本身。WCH 在 CH9329 串口协议中定义了自己的命令帧：

- `CMD 0x04`：发送绝对鼠标数据；
- `CMD 0x05`：发送相对鼠标数据。

芯片再把这些命令转换成 USB HID 键鼠设备的报告。也就是说，CH9329 的两个命令码是厂商私有 UART 协议，而设备最终呈现给 BIOS 或 OS 的 HID 描述符和 HID 报告，才决定目标端看到的是绝对轴还是相对轴。

这一区分很重要：同样发送了 CH9329 的绝对命令，不同目标环境可能出现不同结果；BIOS 能处理绝对轴，也不能推出 Linux 桌面一定会将它映射成普通桌面光标。

### 2.3 BIOS/UEFI 与桌面 OS 使用的输入栈不同

UEFI 本身同时定义了两类常见指针协议：

- `EFI_SIMPLE_POINTER_PROTOCOL`：相对移动、按键和滚轮；
- `EFI_ABSOLUTE_POINTER_PROTOCOL`：绝对坐标和按键。

因此，BIOS/UEFI 菜单同时支持 CH9329 的绝对和相对模式是合理的。BIOS 直接消费 UEFI 指针协议，并不等价于进入 OS 后仍由同一套代码处理。

进入 OS 后，设备需要经过 USB HID 类驱动、内核输入子系统、libinput/Xorg/Wayland 或 Windows 输入栈，最终才会影响桌面光标。Absolute 的定位语义通常适合触摸屏、触控笔、数位板、虚拟机绝对指针等设备；Relative 的定位语义更接近传统桌面鼠标。因此，同一个 USB HID 设备在固件和桌面环境中的表现可以不同。

## 3. 各平台当前研究结论

### 3.1 Windows 10：当前硬件已确认支持绝对模式

本项目实测 Windows 10 上 CH9329 绝对模式可用，且使用体验良好。这是当前最强的 OS 实测证据，说明：

- 当前 CH9329 模组的 USB HID 枚举结果能够被 Windows 10 接受；
- Windows 10 的输入栈能够消费该设备提供的绝对坐标；
- “CH9329 绝对模式在 OS 中通常完全不可用”这一概括是错误的。

Microsoft 的 HID 文档和 Windows 鼠标类驱动模型也明确区分 relative 和 absolute pointing data。Windows 10 的实测与该模型一致。

但这仍然是“当前设备、当前固件、当前 Windows 10 配置”的结论。不同 CH9329 固件、不同 USB 组合设备描述符、不同应用层消费方式，仍需分别验证。

### 3.2 Ubuntu：绝对轴可能被识别，但不一定控制普通桌面光标

当前现象是：

- BIOS 中绝对和相对都工作；
- 进入 Ubuntu 后，相对模式可以移动；
- 绝对模式对普通桌面光标没有效果；
- 当前相对模式的本地光标速度与目标机光标速度不一致；
- 现有桌面端相对模式还没有在所有路径上实现完整的“捕获输入、隐藏本地光标、避免撞到边界”的效果。

这与 Linux 输入栈的设备分类方式相符。Linux 内核可以把 HID absolute X/Y 报告成 `EV_ABS`，但 libinput 的绝对轴处理主要围绕触摸屏、触控板、触控笔、数位板等绝对设备建立。一个只提供普通鼠标 Usage、却发送绝对坐标的 HID 设备，可能在内核层有事件，在桌面层却没有被绑定为控制普通桌面指针的设备。

因此，Ubuntu 当前最可能的解释不是“绝对 HID 协议不标准”，而是“目标设备的 HID 描述符/分类与 Ubuntu 桌面输入栈的预期不匹配”。还需要区分以下环境：

- Ubuntu 使用 Xorg 还是 Wayland；
- GNOME、KDE 或其他桌面；
- 内核是否出现 `ABS_X`/`ABS_Y`；
- `libinput list-devices` 是否列出该设备以及设备类型；
- Xorg 是否通过 `xinput` 暴露该设备；
- CH9329 的键鼠 HID 集合是否被识别成 mouse、tablet、touchscreen 或其他类型。

在没有这些观测前，不能把“Ubuntu 桌面绝对模式不工作”简化成“Linux 不支持绝对鼠标”。更准确的说法是：**Linux 内核支持绝对输入事件，但 generic absolute pointer 是否驱动桌面光标取决于设备分类和桌面输入栈。**

### 3.3 Windows 7：暂不下结论

当前 Windows 7 现象是系统似乎完全没有识别 CH9329，之后还出现了 CH9329 或目标机无响应/死机。现阶段没有足够证据判断根因，因此本文件只做问题记录，不声称 Windows 7 支持或不支持 CH9329，也不声称 Windows 7 不支持绝对鼠标。

Windows 7 本身并非因为“没有 Absolute/Relative 概念”而天然排除。更值得优先排查的方向包括：

- 目标机 USB 2.0/USB 3.x 控制器及 xHCI 驱动；
- CH9329 的 USB 枚举和组合设备描述符；
- CH340 串口驱动、波特率和串口打开方式；
- 设备是否在异常命令或异常状态后需要重新上电；
- Windows 7 的设备管理器、事件查看器和 USB 抓包结果。

由于本次测试已经出现死机/无响应，后续调查应使用独立供电、可恢复的测试环境，先确认 USB 枚举，再逐步发送键盘、绝对鼠标和相对鼠标命令，不继续在当前状态下扩大测试范围。

## 4. 当前仓库实现事实

### 4.1 CH9329 报告层

`crates/ipkvm-core/src/ch9329/report.rs` 当前将命令映射为：

- `MouseAbsolute` -> `CMD 0x04`；
- `MouseRelative` -> `CMD 0x05`；
- 绝对坐标限制在 `0..=4095`；
- 相对位移、滚轮字段按 CH9329 单字段范围处理，不接受 `-128`。

当前协议核心知道如何生成两种 CH9329 报告，但它不负责决定目标 OS 应该使用哪种模式。模式选择属于桌面/Web 入口和后续 OS profile 层。

本项目 2026-08-02 的硬件实测还发现：CH9329 绝对帧中的按钮位在目标端只产生了 `ABS_X`/`ABS_Y`，没有产生 `BTN_*` 事件；按钮通过零位移相对帧发送才可靠。这说明绝对按钮位可能受芯片或目标输入栈影响，不能只看协议字段名称推断按钮一定有效。2026-08-09 起当前输入核心先恢复严格语义：绝对模式全部走绝对报告，相对模式全部走相对报告；混用策略只作为后续 OS/device profile 的显式候选，不作为默认行为。

2026-08-09 对相对按钮链路补充验证：`RelativeMouseReport` 左键 press/release 已按 WCH 协议示例固定为 `CMD 0x05`，数据分别为 `[0x01,0x01,0,0,0]` 和 `[0x01,0,0,0,0]`；RFB/桌面使用的相对按钮掩码最终映射到 CH9329 位序为左 `0x01`、右 `0x02`、中 `0x04`。`ch9329_probe click-rel left` 可用于真实目标机上直接验证零位移相对按钮报告。如果 macOS 目标仍只接受相对移动而不接受该按钮报告，应按目标 OS 兼容性继续调查，而不是先假定本地 RFB 位序或 CH9329 相对报告字段写反。

### 4.2 Iced 桌面端

Iced 路径的绝对模式来自窗口的 `CursorMoved`，再将窗口中的视频坐标映射到视频帧坐标和 CH9329 的 `0..=4095` 范围。

Iced 路径的相对模式使用 Windows Raw Input：

- `RegisterRawInputDevices(RIDEV_INPUTSINK)` 注册鼠标源；
- 从 `RAWMOUSE.lLastX/lLastY` 读取相对增量；
- 当前通过约 33 ms 的采样/节流合并输入；
- 还会叠加 DPI、视频比例和相对灵敏度换算。

Iced 的光标控制目前是 Windows `ShowCursor` 加 `ClipCursor`。`ClipCursor` 只把光标限制在前台窗口矩形内，没有实现标准 pointer lock 常见的“到边缘后回到中心或窗口内部并继续产生增量”的完整闭环；`ShowCursor` 还是进程级计数 API，单独维护本程序的可见性 gate 也不能保证外部代码对系统计数的影响完全可见。

因此，当前 Iced 相对模式出现速度差异或撞边界，不应归因于 CH9329 的相对协议本身。至少有三段速度/坐标变换需要分别测量：

1. 主控系统物理鼠标到 Raw Input 的 `lLastX/lLastY`；
2. 本地程序的 DPI、视频比例、采样和灵敏度换算；
3. 目标 OS 对 CH9329 相对 HID 鼠标应用的加速度和速度曲线。

### 4.3 egui 桌面端和网页端

当前 egui 路径已经调用 egui 的 `CursorGrab::Locked` 和 `CursorVisible(false)`，其语义比简单裁剪更接近真正的窗口级 pointer lock，但仍需在不同 Windows、窗口管理器和输入设备上做人工验证。

网页端已经使用浏览器 Pointer Lock API，并在锁定后通过 `movementX/movementY` 发送相对指针扩展消息。noVNC 的本地修改把这类增量映射到仓库扩展的 RFB 相对指针消息 `0x08`。该路径更接近浏览器平台定义的相对输入模型，但 Pointer Lock 仍受用户手势、焦点、浏览器策略和 `Esc` 退出行为约束。

## 5. 为什么 Windows 10 与 Ubuntu 可能不同

不能只用“都支持 USB HID”来推导相同结果。至少存在以下差异层：

1. **HID 描述符分类**：设备可能报告为普通 mouse、digitizer、tablet 或组合设备；
2. **内核事件映射**：Absolute 可能成为 `EV_ABS`，Relative 可能成为 `EV_REL`；
3. **桌面设备选择**：桌面环境可能只把被分类为 pointer 的设备接入普通光标；
4. **坐标范围与校准**：绝对设备通常需要有效的轴范围、屏幕映射和校准信息；
5. **窗口系统语义**：Wayland、Xorg、Windows Desktop Window Manager 对输入设备的接入路径不同；
6. **目标端加速度和过滤**：相对鼠标一般会经过桌面鼠标加速度，而绝对设备通常采用坐标映射。

所以“Windows 10 绝对模式好用、Ubuntu 绝对模式无效”并不矛盾。它说明不同 OS 对同一个 HID 设备的分类和桌面消费方式不同，也说明 OS profile 需要记录的不只是一个 `mouse_mode` 字段。

## 6. OS profile 的设计方向（暂不定稿）

后续 profile 不应只保存 `absolute` 或 `relative` 两个值。至少需要评估以下字段是否进入 profile：

- 输入发送模式：absolute、relative 或按场景切换；
- 绝对坐标范围、原点、轴方向和坐标换算方式；
- 绝对移动时按钮是否单独用相对帧发送；
- 相对增量的采样周期、合并策略、单帧拆分和灵敏度；
- 主控端光标是否隐藏、捕获、锁定、回位，以及失焦/退出时的恢复策略；
- 目标 OS 是否需要桌面侧校准、设备分类配置或额外驱动；
- 滚轮单位、按钮位序和拖拽期间的状态保持；
- 设备枚举失败、模式无效或输入栈不接受时的回退模式。

其中“主控端光标策略”和“目标端 HID 模式”是两个不同层次：

- 主控端需要解决用户如何持续提供输入，以及如何避免本地光标撞到窗口边界；
- 目标端需要解决 OS 如何消费 CH9329 产生的 HID 报告。

profile 可以同时配置这两层，但不能用主控端隐藏光标来修复目标 OS 不接受 `EV_ABS` 的问题。

## 7. 后续实测记录要求

最终 OS profile 表暂不在本文件中创建。后续每个 OS 至少记录以下信息，避免把“能枚举”“能产生内核事件”和“能控制桌面光标”混为一谈：

- OS 版本、桌面环境、Xorg/Wayland 或固件版本；
- CH9329 模组、USB 线、USB 控制器和串口参数；
- 设备是否成功枚举，以及设备名称、VID/PID 和 HID 集合信息；
- 绝对模式是否产生坐标事件，事件类型和坐标范围；
- 绝对模式是否真的移动普通桌面光标；
- 相对模式是否产生移动、按钮和滚轮事件；
- 拖拽、窗口边缘、失焦、重新聚焦和退出时的行为；
- 需要的驱动、校准、桌面设置或特殊配置；
- 是否出现设备重置、串口异常、系统无响应或死机。

Linux 目标机后续优先保存以下诊断输出，再讨论桌面适配：

```text
lsusb -v
udevadm info --query=all --name=/dev/input/eventN
evtest /dev/input/eventN
libinput list-devices
```

若使用 Xorg，再补充 `xinput list` 和对应设备属性；若使用 Wayland，则需要记录桌面环境和 compositor 的设备识别结果。这样可以确认问题是在 USB HID、内核事件、libinput 分类还是桌面光标映射层。

## 8. 当前阶段的明确结论

1. Absolute/Relative 是 USB HID 报告描述符的标准输入语义。
2. CH9329 `0x04`/`0x05` 是 WCH 私有 UART 命令，不是跨 OS 的标准命令码。
3. BIOS/UEFI 同时支持两种模式是合理的，因为固件有相对和绝对指针协议；不能据此推断桌面 OS 行为。
4. Windows 10 已由本项目实测确认绝对模式可用，且效果良好。
5. Ubuntu 当前绝对模式不驱动普通桌面光标，最可能涉及 generic absolute HID 设备分类与桌面输入栈之间的不匹配；尚未完成底层事件和桌面环境分层验证。
6. Windows 7 当前只记录为未识别/异常和暂停调查，不下“Windows 7 不支持 CH9329”的结论。
7. 当前相对模式的速度不一致和本地光标捕获不完整，是主控端 Raw Input、采样/换算、目标端鼠标加速度以及光标锁定策略共同作用的问题，不能简单归因于 CH9329 协议。
8. 最终 OS profile 应在各 OS 实测完成后再设计和填写，不在本阶段凭研究资料提前定表。

## 9. 参考资料

### USB HID 与 CH9329

- USB-IF HID 总览：https://www.usb.org/hid
- USB HID 1.11 规范：https://www.usb.org/sites/default/files/documents/hid1_11.pdf
- USB HID Usage Tables 1.7：https://usb.org/document-library/hid-usage-tables-17
- WCH CH9329 数据手册：https://www.wch-ic.com/downloads/CH9329DS1_PDF.html
- WCH CH9329 串口协议（本地资料）：`docs/references/CH9329-serial-protocol-wch-20190508.pdf`
- WCH CH9329 协议下载页：https://www.wch-ic.com/download/file?id=277

### Windows

- HID over USB：https://learn.microsoft.com/en-us/windows-hardware/drivers/hid/hid-over-usb
- Keyboard and Mouse HID Client Drivers：https://learn.microsoft.com/en-us/windows-hardware/drivers/hid/keyboard-and-mouse-hid-client-drivers
- Raw Input：https://learn.microsoft.com/en-us/windows/win32/inputdev/raw-input
- `ShowCursor`：https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showcursor
- `ClipCursor`：https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-clipcursor

### Linux、UEFI 与浏览器

- Linux Input Event Codes：https://www.kernel.org/doc/html/latest/input/event-codes.html
- libinput Absolute Axes：https://wayland.freedesktop.org/libinput/doc/latest/absolute-axes.html
- Linux HID input 映射实现：https://codebrowser.dev/linux/linux/drivers/hid/hid-input.c.html
- UEFI `SimplePointer` Protocol：https://github.com/tianocore/edk2/blob/master/MdePkg/Include/Protocol/SimplePointer.h
- UEFI `AbsolutePointer` Protocol：https://github.com/tianocore/edk2/blob/master/MdePkg/Include/Protocol/AbsolutePointer.h
- Pointer Lock API：https://developer.mozilla.org/en-US/docs/Web/API/Pointer_Lock_API

### 本仓库实现与 RFB

- CH9329 报告实现：`crates/ipkvm-core/src/ch9329/report.rs`
- Iced 输入入口：`crates/ipkvm-desktop-iced/src/app.rs`
- Windows Raw Input：`crates/ipkvm-desktop-iced/src/platform/windows.rs`
- Iced 光标控制：`crates/ipkvm-desktop-iced/src/platform/cursor.rs`
- egui 光标捕获：`crates/ipkvm-desktop/src/app.rs`
- Web Pointer Lock：`crates/ipkvm-headless/web/modules/pointer.js`
- noVNC 相对指针接入：`third_party/novnc/1.7.0/core/rfb.js`
- RFB 相对指针社区扩展：`docs/references/rfbproto-community-spec.rst`

## 2026-08-04 #159 边界更新

profile、连接参数、设备选择引用和输入会话语义保持不变；与 UI 无关的配置、探测抽象、
会话控制器和帧转换已迁入 `ipkvm-desktop-core`。`ipkvm-desktop` 只保留真实相机、CH9329
串口和剪贴板 adapter，`ipkvm-desktop-iced` 通过 core 使用这些能力。该拆分不改变本文
记录的 HID、Raw Input、绝对/相对指针和键盘映射结论。
