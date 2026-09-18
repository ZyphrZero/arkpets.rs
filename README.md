# arkpets.rs

基于 Ark-Pets 与 ArkPets-Web 的原生桌面版本。Rust 负责行为、资源与界面，`eframe/egui 0.36.2` 提供原生窗口，`wgpu 30` 或 Glow 负责 GPU 绘制，官方 Spine 3.8 C 运行时负责现有模型的骨骼解析与动画。Windows 默认使用 DX12 + DirectComposition，也可选择 OpenGL；macOS 使用 Metal，Linux 使用 Vulkan。

已包含三个真实样例模型：佩佩、荒芜拉普兰德、Friston-3。支持模型搜索、收藏、本地导入、自动与手动动作、预览区拖拽和物理、显示与行为设置，以及透明、置顶、可拖动的桌宠窗口。桌宠窗口具有独立的重力、行走和边界转向，支持自动识别 Windows 任务栏、手动设置底部高度，以及任务栏和 Alt+Tab 显示开关。偏好保存在操作系统的应用用户目录。

启动器与桌宠共享一份模型、运行时和 GPU 纹理。打开桌宠后，由桌宠窗口独立推进动画；启动器停止定时重绘，模型预览仅在输入等事件发生时刷新。原始图集和系统 DPI 绘制比例保留，不通过缩小纹理或降低窗口像素密度节省资源。

## 运行

```sh
cargo run
cargo run --release
```

需要 Rust 1.95 或更新的 stable 工具链，以及 C 编译工具链。Windows 安装 Visual Studio C++ Build Tools，并安装支持所选 DX12 或 OpenGL 后端的显卡驱动；macOS 安装 Xcode Command Line Tools；Linux 安装 GCC/Clang 以及 `libxkbcommon-dev`、`libxkbcommon-x11-dev`、`libwayland-dev`、`libx11-dev`、`libxi-dev`、`libxcursor-dev`、`libxrandr-dev` 等系统开发包，并提供 X11 或 XWayland 桌面会话。日常使用和性能比较使用 release 构建。

程序依次查找 `ARKPETS_ASSETS` 环境变量、可执行文件旁的 `assets`、当前目录中的 `assets`。分发 release 可执行文件时，需要同时携带 `assets/`、`LICENSE`、`THIRD_PARTY_NOTICES.md` 与 Spine 许可证。

## 渲染器选择

Windows 的“行为与显示”页可选择 DirectX 12 或 OpenGL，选择后立即切换并保存偏好，无需点击应用或重启程序。切换在同一进程内重建窗口和 GPU 资源，窗口会短暂重建，当前页面、模型、动画、暂停状态和已打开的桌宠会保留。也可在启动时指定：

```sh
cargo run --release -- --renderer dx12
cargo run --release -- --renderer opengl
```

选择优先级为 `--renderer`、`ARKPETS_RENDERER` 环境变量、用户目录中的 `bootstrap.json`、平台默认值。macOS 接受 `metal`，Linux 接受 `vulkan`。无效名称或当前平台不支持的后端会报错，不会静默换用另一种渲染器。bootstrap 配置与普通界面偏好分离，Windows 位于 `%APPDATA%/ArkPets/data/bootstrap.json`。

两个 Windows 后端都使用原生透明窗口，不在正常绘制时逐帧读回 GPU 图像。DX12 使用 DirectComposition 和 Mailbox 呈现，避免双窗口重绘时 FIFO 队列积压造成内存增长和卡顿；OpenGL 使用 eframe 0.36.2 的透明子窗口修复。显卡驱动和系统合成器会影响内存、帧节奏和窗口效果，选择应以本机表现为依据。

## 动作与交互

“行为与显示”的“行为”区域提供以下设置，修改后立即生效并保存：

- **动作活跃度**：`0–16` 级，默认 `4`。数值越高，自动动作越少选择待机；它调整动作选择概率，不加速骨骼动画。`0` 仍会活动，停止自动切换请关闭自动动作。被禁用或模型不具备的动作不会因此启用。
- **行走速度**：`0–200` 逻辑像素/秒，默认 `30`，在预览和桌宠中均生效。`0` 停止行走位移，但仍可播放行走动画。逻辑像素随系统 DPI 缩放，与底部高度使用的物理像素不同。
- **交互式朝向翻转**：禁用、松开拖拽时（默认）、拖拽时、光标掠过时。角色跟随鼠标的水平移动方向，忽略微小抖动；“拖拽时”包含释放触发，“光标掠过时”包含拖拽和释放触发。关闭触摸和拖拽也会关闭交互翻转，屏幕边界的自动转向仍保留。

## 桌宠窗口、显示器与部署

“行为与显示”的“桌面窗口”区域提供以下设置，修改后立即生效并随偏好保存：

- **在任务栏和 Alt+Tab 中显示桌宠**：Windows、Linux 默认关闭，仅隐藏桌宠的系统条目，桌宠继续显示和播放动画。macOS 对应选项为“在 Dock 和应用切换器中显示”。切换不重建 GPU 资源，不重置位置、动作或暂停状态。
- **自动检测任务栏高度**：Windows 默认开启，分别以各显示器的系统工作区为活动边界，定期刷新，适配任务栏和已向系统注册工作区的工具栏变化。
- **任务栏高度（下边界距离）**：关闭自动检测后可输入 `0–500 px`，表示距屏幕底边的物理像素距离；`0` 为屏幕底边。例如底部工具栏高 `80 px` 时设置为 `80`，桌宠会落在该高度并沿其行走。这个值替换系统底部预留，不与任务栏高度累加。
- **启用多显示屏**：默认开启。可拖拽到其他显示器，在有足够通行空间的相接屏幕间行走，并落到各自的底部边界。错位屏幕、屏幕之间的空隙会阻挡自动移动；拖拽可以直接放到另一块屏幕。关闭后限制在主显示器，拔掉显示器或改变布局时会把桌宠收回有效区域。

**初始部署位置**用水平、垂直百分比设置，默认均为 `20%`，在重新打开桌宠时生效。百分比相对于主显示器中扣除桌宠窗口尺寸后的可活动范围：水平 `0%/100%` 为最左/最右，垂直 `0%/100%` 为最上/最下。正常开启重力时，部署后仍会继续下落。修改设置不会立即移动当前桌宠。

手动高度适用于未向系统预留工作区的底部工具栏，不识别任意悬浮工具栏的局部形状。macOS/Linux 也可设置手动高度，自动工作区检测目前仅在 Windows 启用。

平台行为：

- Windows 使用工具窗口样式，启动器的任务栏条目不受影响。
- Linux 使用 X11 的 EWMH `SKIP_TASKBAR` / `SKIP_PAGER` 状态和 Utility 窗口类型；启动器不受影响，Alt+Tab 是否排除桌宠取决于窗口管理器。Wayland 会话下明确使用 XWayland，因为标准 `xdg-shell` 不提供任务栏排除或顶层窗口的全局定位接口。需要可用的 `DISPLAY` 和支持这些 EWMH 提示的窗口管理器；缺失时会报告错误。
- macOS 使用 AppKit 的 Accessory 激活策略。Dock 和 Command+Tab 按应用管理，因此桌宠打开且此选项关闭时，整个 ArkPets 应用的系统入口都会隐藏，但启动器和桌宠窗口仍可见，动画照常运行。开启选项或收起桌宠后恢复 Regular 策略；桌宠不会因切换到其他应用而自动隐藏。

## 模型与命令行

模型库旁的加号可载入本地 `.skel` 或模型 JSON 清单。清单兼容 Web 版数组、单角色对象、原版 `models_data.json` 的 `storageDirectory` / `data` / `assetList` 结构。资源路径相对于清单所在目录解析；直接导入 `.skel` 时使用同名 `.atlas`。

```sh
cargo run --bin arkpets -- --help
cargo run --bin arkpets -- inspect-model assets/models.json
cargo run --bin arkpets -- simulate assets/models.json 120
```

`simulate` 是无窗口的物理状态演算，不渲染骨骼。Spine 解析器仅用于可信本地 3.8.x 资源；远程下载和跨版本转换尚未实现。

## 检查

```sh
cargo test --no-default-features
cargo test --no-default-features --features spine --lib
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

Spine 测试直接加载三个样例并验证动画顶点变化。GUI 测试覆盖不同窗口尺寸下的鼠标命中，避免初始化或最小化时出现 NaN；网格批处理测试检查绘制顺序、索引和混合颜色。运行时测试覆盖命令队列上限、每帧处理预算、参数校验和暂停。Linux 已在 WSL Debian 的 Xvfb/Openbox/合成器环境验证系统条目切换和动画；macOS 新增 AppKit 模块已通过 Apple Silicon、Intel 目标的类型检查，仍需实机验证窗口效果。

开发构建支持 `ARKPETS_CAPTURE_DIR` 环境变量，Windows 下设置后自动运行三个模型、两个主题、透明桌宠下落与行走的 GPU 截图检查，保存图片后退出。测试偏好写入该目录，不覆盖正常运行的偏好。以下脚本会放置受控背景，检查系统合成后的透明像素确实显示后方窗口，同时确认模型可见；仅验证 GPU 图片带 Alpha 无法排除原生窗口白底问题。

```powershell
powershell -NoProfile -File docs/scripts/Verify-Desktop.ps1 -Renderer dx12
powershell -NoProfile -File docs/scripts/Verify-Desktop.ps1 -Renderer opengl
```

Windows 下可用 PowerShell 7 运行渲染后端切换回归。脚本从独立 PowerShell 进程启动真实窗口，通过设置页连续切换，检查 PID 保持不变、桌宠重新显示、实际 GPU 后端和保存偏好一致，以及关闭后终端命令正常返回；配置与日志保存在 `target/renderer-switch/`。

```powershell
pwsh -NoProfile -File docs/scripts/Verify-RendererSwitch.ps1 -Renderer dx12 -DesktopPet
pwsh -NoProfile -File docs/scripts/Verify-RendererSwitch.ps1 -Renderer opengl -Shell powershell
```

任务栏和高度设置的 Windows 交互回归会检查真实 HWND 样式、切换时窗口保持不变，以及自动边界和多个手动高度下的实际落地位置：

```powershell
pwsh -NoProfile -File docs/scripts/Verify-Interaction.ps1 -Renderer dx12 -DesktopSettings
pwsh -NoProfile -File docs/scripts/Verify-Interaction.ps1 -Renderer opengl -DesktopSettings
```

增加 `-BehaviorSettings` 可通过真实鼠标和键盘修改速度、活跃度、翻转方式、多屏开关与初始位置，检查保存结果，并关闭重力验证重新部署后的原生窗口坐标。多屏几何测试覆盖负坐标、相接和错位屏幕、空隙、屏幕移除与 DPI 重定位；实际多显示器热插拔、混合 DPI 设备及 macOS 桌面仍需实机验证。

Linux 的回归脚本创建独立的 Xvfb/Openbox 桌面，检查窗口状态、连续切换时 XID 不变，以及启动器最小化后桌宠图像非空、动画像素继续变化。需要 `xvfb`、`xauth`、`openbox`、`xcompmgr`、`x11-utils`、`x11-xserver-utils`、`xdotool`、`wmctrl`、ImageMagick 和 Vulkan 驱动（无 GPU 时可用 `mesa-vulkan-drivers`）：

```sh
cargo build --bin arkpets-ui
bash docs/scripts/Verify-ShellVisibility-Linux.sh target/debug/arkpets-ui target/linux-shell
```

## 性能测量

Windows 可用以下命令记录同一场景下的桌宠回调间隔和进程内存：

```powershell
cargo build --release --bin arkpets-ui
powershell -NoProfile -File docs/scripts/Measure-Performance.ps1 -Configuration release -Model pepe -Fps 60 -WarmupSeconds 2 -DurationSeconds 8
```

脚本自动打开设置页和桌宠，在预热后采样，退出时将 `frames.json`、`memory.json` 和日志写入 `target/performance/` 下的独立目录。`frames.json` 提供回调间隔的 p50/p95/p99；`memory.json` 每 100 ms 采样工作集和私有内存。回调频率不是屏幕实际呈现 FPS，进程工作集也不是显存占用。比较不同版本时应使用同一机器、渲染后端、模型、缩放、帧率限制和构建配置；可通过 `ARKPETS_RENDERER` 为采样指定后端。具体口径见 [架构说明](docs/ARCHITECTURE.md#性能与验证)。

DX12 内存回归可增加持续的主窗口重绘，并设置私有内存峰值上限；超过上限时脚本保留报告并报错。上限是测试预算，可按模型与测试机器调整：

```powershell
$env:ARKPETS_RENDERER = 'dx12'
powershell -NoProfile -File docs/scripts/Measure-Performance.ps1 -DurationSeconds 60 -RepaintStress -MaxPrivateMemoryMiB 1024
```

## 扩展入口

`src/runtime.rs` 提供与 UI 无关的 `PetRuntime`、`PetCommand`、`CommandSender` 和命令处理结果。命令队列容量为 64，每帧最多处理 16 条；提交成功后可唤醒前端，暂停等 UI 操作已经使用同一运行时。后续 Agent CLI 适配器可以在边界解析 JSONL / IPC 消息，再提交类型化命令。

当前尚未实现 Agent CLI 传输协议、消息气泡、对外 IPC 或养成系统。模块职责和这些功能的接入位置见 [架构说明](docs/ARCHITECTURE.md)。

## 迁移范围

当前是可运行的原生版本，尚未完全替代 Java 原版。系统托盘、开机启动、多宠 IPC、其他窗口边缘碰撞、联网模型管理、描边和阴影等仍未迁移。Linux 使用 X11/XWayland，尚不提供原生 Wayland 桌宠后端。完整调研、选型和功能边界见 [迁移记录](docs/MIGRATION.md)。

本项目代码沿用 GPL-3.0；Spine 运行时、字体和明日方舟素材分别适用其自身许可与版权声明，见 [第三方说明](THIRD_PARTY_NOTICES.md)。
