# arkpets.rs

基于 Ark-Pets 与 ArkPets-Web 的原生桌面版本。Rust 负责行为、资源与界面，`eframe/egui 0.36.2` 提供原生窗口，`wgpu 30` 或 Glow 负责 GPU 绘制，官方 Spine 3.8 C 运行时负责现有模型的骨骼解析与动画。Windows 默认使用 DX12 + DirectComposition，也可选择 OpenGL；macOS 使用 Metal，Linux 使用 Vulkan。

已包含三个真实样例模型：佩佩、荒芜拉普兰德、Friston-3。支持模型搜索、收藏、本地导入、自动与手动动作、预览区拖拽和物理、显示与行为设置，以及透明、置顶、可拖动的桌宠窗口。桌宠窗口具有独立的重力、行走和边界转向，支持自动识别 Windows 任务栏、手动设置底部高度，以及任务栏和 Alt+Tab 显示开关。偏好保存在操作系统的应用用户目录。

桌宠可以独立运行，无需连接 Agent。当前优先完善原版桌宠的交互、过渡、物理和渲染设置；这些配置已接入运行行为和保存，具体差异与平台验证范围见 [配置迁移计划](docs/MIGRATION_PLAN.md)。

启动器与桌宠共享一份模型、运行时和 GPU 纹理。模型库中的角色预览持续播放，无需移动鼠标；打开桌宠后，由桌宠窗口推进动画，预览按设置的帧率持续绘制同一份模型状态，不重复计算骨骼。主界面最小化、隐藏或被系统标记为完全遮挡时，以及切到设置页后，停止预览动画刷新；恢复可见后自动继续。原始图集和系统 DPI 绘制比例保留，不通过缩小纹理或降低窗口像素密度节省资源。

## 运行

```sh
cargo run
cargo run --release
```

需要 Rust 1.95 或更新的 stable 工具链，以及 C 编译工具链。Windows 安装 Visual Studio C++ Build Tools，并安装支持所选 DX12 或 OpenGL 后端的显卡驱动；macOS 需要 11 或更新版本并安装 Xcode Command Line Tools；Linux 安装 GCC/Clang 以及 `libxkbcommon-dev`、`libxkbcommon-x11-dev`、`libwayland-dev`、`libx11-dev`、`libxi-dev`、`libxcursor-dev`、`libxrandr-dev` 等系统开发包，并提供 X11 或 XWayland 桌面会话。日常使用和性能比较使用 release 构建。

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
- **允许触发交互动画**：默认开启。关闭后，点击和自动选择均不触发 `Interact`；仍可手动选择该动作。此开关不控制拖拽或朝向翻转。
- **允许拖拽**：默认开启，与交互动画独立。关闭时结束当前拖拽，仍可按其他设置点击互动或让角色行走。
- **交互式朝向翻转**：禁用、松开拖拽时（默认）、拖拽时、光标掠过时。角色跟随鼠标的水平移动方向，忽略微小抖动；“拖拽时”包含释放触发，“光标掠过时”包含拖拽和释放触发。关闭拖拽会关闭拖拽相关触发，悬停翻转与屏幕边界自动转向独立保留。

Windows 和 macOS 桌面拖拽直接读取系统全局光标位置，避免窗口移动与缓存鼠标坐标不同步造成抖动和误抛掷。松手时保留实际拖动产生的惯性，并受最大速度限制；鼠标停稳后释放不会产生抛掷速度。

## 过渡与物理

“行为与显示 → 过渡”提供动画交叉过渡、常规属性过渡和缓动函数。两项时长均使用原版四档下拉选项，默认“标准”：

| 选项 | 时长 |
| --- | --- |
| 禁用 | 0 秒 |
| 快速 | 0.1 秒（100 ms） |
| 标准 | 0.3 秒（300 ms） |
| 慢速 | 0.6 秒（600 ms） |

- **动画交叉过渡**：控制 Spine 动作之间的混合。0 为瞬切，新时长从下一次动作切换开始使用；在暂停时修改也能正确应用。
- **常规属性过渡**：控制预览／桌宠的位置、水平翻转、不透明度和描边强度。0 为立即切换。物理计算和视觉平滑分离；拖拽位置直接跟手，从实际显示位置建立锚点，不会跳到尚未显示的物理目标。多屏凹口会沿合法拐点过渡，避免穿过屏幕空隙或卡在另一侧。
- **缓动函数**：线性、正弦缓出、三次缓出（默认）、五次缓出。目标或参数在过渡中改变时，从当前显示值继续；暂停动画后仍会完成属性过渡，完成后停止额外高频重绘。

配置仍使用以秒为单位的数值；已有或通过程序接口设置的自定义值（例如 0.137 秒）显示为“自定义”，不会在打开设置或保存时被归入最近档位。两项时长和缓动函数可分别设置；程序接口允许动画混合 0–2 秒、常规属性过渡任意有限非负秒数，详见 [命令入口](docs/ARCHITECTURE.md#命令入口)。

“行为与显示 → 物理”的五项参数使用逻辑像素，随系统 DPI 缩放，在预览和桌宠中实时生效：

| 参数 | 新配置／恢复默认后的值 | 设置范围 |
| --- | --- | --- |
| 重力加速度 | 800 逻辑像素/秒² | 0–2000 |
| 空气阻力加速度 | 100 逻辑像素/秒² | 0–2000 |
| 地面摩擦加速度 | 500 逻辑像素/秒² | 0–2000 |
| 最大水平运动速率 | 1000 逻辑像素/秒 | 0–2000 |
| 最大垂直运动速率 | 1000 逻辑像素/秒 | 0–2000 |

空气阻力对两轴惯性按时间减速；地面摩擦只在落地时额外减小水平惯性，速度减到零后不会反向。错位显示器按各自局部底边判断落地。最大速率限制自由运动与拖拽释放速度；**0 表示停止该轴惯性**，这与 Java 原版的 0 表示不限速不同。主动行走仍由“行走速度”单独控制。

旧偏好中的重力、惯性保留系数和停止阈值保持原值，新增摩擦不会叠加进去。调整空气阻力或地面摩擦后改用加速度减速；“恢复默认参数”可直接使用上表的新默认值。反弹系数继续保留。

## 透明模式与不透明度

模型库右侧或“行为与显示 → 桌面窗口”中的**透明模式（鼠标穿透）**开启后，鼠标可以操作桌宠后方的窗口，桌宠逐渐切换到独立的不透明度。关闭后恢复常规不透明度和鼠标交互。开启期间无法点击桌宠自身，可从主界面关闭；macOS 隐藏主界面后可通过菜单栏的“显示主界面”恢复入口。

“行为与显示 → 渲染”提供以下设置，随偏好保存：

- **常规不透明度**：10%–100%，默认 100%，同时用于模型库预览和普通桌宠。
- **透明模式不透明度**：10%–100%，默认 75%，仅用于开启鼠标穿透的桌宠；实际取它与常规不透明度的较小值。预览保持常规不透明度，仍可交互。

透明度使用“过渡”区域的常规属性过渡时长和缓动函数。

暂停动作时仍会完成透明度过渡，完成后恢复低频工作区检查，不持续刷新动画。切换模式不重新加载模型、不重新部署桌宠，也不重建窗口。重新打开桌宠时直接使用所保存模式的不透明度。“恢复默认参数”也会关闭透明模式。

不透明度统一作用于合成后的完整角色。预览和桌宠都先按原始部件颜色与遮挡顺序绘制到 GPU 纹理，再对整体应用一次不透明度，避免肢体重叠处变深或透出下层部件。离屏纹理按系统 DPI 绘制并复用，正常运行不读回 GPU 像素。

配置使用 `config.animationMixDuration`、`config.propertyTransitionDuration` 和 `config.easing`，最后一项保存 `LINEAR`、`EASE_OUT_SINE`、`EASE_OUT_CUBIC` 或 `EASE_OUT_QUINT`。旧字段 `transitionDuration` 仍按动画混合读取，`opacityTransitionDuration` / `opacityTransition` 分别作为常规属性过渡与缓动函数的兼容别名，旧值不会丢失。多形态动作偏移仍未迁移；暂不直接导入原始 Java 配置 JSON。

## 描边、阴影与采样

“行为与显示 → 渲染”提供以下配置，修改后即时生效并随偏好保存：

- **高亮描边**：普通状态和“操控中”分别选择关闭、拖拽时、鼠标按下时、窗口获得焦点时或始终开启。操控中指**手动动作模式**，与是否暂停、是否正在拖拽不同；焦点也不是鼠标悬停。普通默认拖拽时显示黄色描边，手动模式默认获得焦点时显示橙色描边。
- **描边颜色和宽度**：两种模式各有 RGBA 颜色，宽度可选 1–5 逻辑像素，默认 2，按系统 DPI 换算。开启／关闭强度按常规属性过渡变化，颜色即时切换。窗口四边留出随角色缩放变化的空间，以容纳描边与阴影。
- **阴影强度**：0–1，0 关闭；提供禁用、轻微、标准（默认）、重墨预设。它是轮廓周围的黑色阴影，不是地面投影。
- **高质量滤波**：默认开启，使用高斯邻域计算描边和阴影；关闭时改用圆周采样。它不控制图集放大过滤。
- **MipMap 抗锯齿**：默认开启。保留完整原图，并在缩小时使用三线性层级采样；关闭时使用原图基础层线性采样。仅载入模型或切换开关时准备图集，两个窗口共享。关闭的含义与 Java 的“保留 atlas 原设置”不同。

窗口边界提供“最宽／较宽／标准／较窄／最窄”和“极精确／精确／粗略／极粗略”，默认标准、精确。边界大小对应动画采样累计 Alpha 的裁剪阈值（0.45／0.65／0.8／0.9／0.95）；计算方式对应固定 30 FPS 基准下每 1／4／16／64 帧抽样一次。它们不是留白百分比或运行时刷新频率。

修改后点击 **重新计算边界**。计算会改变窗口裁剪范围，保持角色本身的缩放比例、当前动画、图集和暂停状态；窗口尺寸变更后会重新约束到可用屏幕。重新载入模型和恢复默认参数也会应用相应边界。界面显示姿态数，若半透明模型需要降低阈值才能获得有效范围，会显示实际采用的阈值；资源已在磁盘改变时，需要先重新载入模型。

边界在 CPU 上使用真实动画网格和原图 Alpha 分析，最长分析边为 256 像素，不读取 GPU。该分辨率仅用于分析，不降低实际图集或窗口渲染分辨率；结果与 Java 的 GPU 画布逐像素裁剪并非完全一致。最多分析 4096 个姿态，超出会明确提示改用更粗略的方式。同一进程缓存最多 32 组资源／参数结果，默认精度在三个样例上的 release 计算为几十毫秒量级；debug 计算更慢。

## 桌宠窗口、显示器与部署

“行为与显示”的“桌面窗口”区域提供以下设置，修改后立即生效并随偏好保存：

- **在任务栏和 Alt+Tab 中显示桌宠**：Windows、Linux 默认关闭，仅隐藏桌宠的系统条目，桌宠继续显示和播放动画。macOS 对应选项为“在 Dock 和应用切换器中显示”，默认关闭；它独立控制整个 ArkPets 应用的 Dock 入口，与桌宠是否打开无关。切换不重建 GPU 资源，不重置位置、动作或暂停状态。
- **自动检测任务栏高度**：Windows 默认开启，分别以各显示器的系统工作区为活动边界，定期刷新，适配任务栏和已向系统注册工作区的工具栏变化。
- **任务栏高度（下边界距离）**：关闭自动检测后可输入 `0–500 px`，表示距屏幕底边的物理像素距离；`0` 为屏幕底边。例如底部工具栏高 `80 px` 时设置为 `80`，桌宠会落在该高度并沿其行走。这个值替换系统底部预留，不与任务栏高度累加。
- **启用多显示屏**：默认开启。可拖拽到其他显示器，在有足够通行空间的相接屏幕间行走，并落到各自的底部边界。错位屏幕、屏幕之间的空隙会阻挡自动移动；拖拽可以直接放到另一块屏幕。关闭后限制在主显示器，拔掉显示器或改变布局时会把桌宠收回有效区域。

**初始部署位置**用水平、垂直百分比设置，默认均为 `20%`，在重新打开桌宠时生效。百分比相对于主显示器中扣除桌宠窗口尺寸后的可活动范围：水平 `0%/100%` 为最左/最右，垂直 `0%/100%` 为最上/最下。正常开启重力时，部署后仍会继续下落。修改设置不会立即移动当前桌宠。

手动高度适用于未向系统预留工作区的底部工具栏，不识别任意悬浮工具栏的局部形状。macOS/Linux 也可设置手动高度，自动工作区检测目前仅在 Windows 启用。

平台行为：

- Windows 使用工具窗口样式，启动器的任务栏条目不受影响。
- Linux 使用 X11 的 EWMH `SKIP_TASKBAR` / `SKIP_PAGER` 状态和 Utility 窗口类型；启动器不受影响，Alt+Tab 是否排除桌宠取决于窗口管理器。Wayland 会话下明确使用 XWayland，因为标准 `xdg-shell` 不提供任务栏排除或顶层窗口的全局定位接口。需要可用的 `DISPLAY` 和支持这些 EWMH 提示的窗口管理器；缺失时会报告错误。
- macOS 默认以 Accessory 激活策略启动，只保留菜单栏入口，不显示 Dock / Command+Tab 条目。开启 Dock 设置后使用 Regular 策略并显示应用自带的绿色爪印图标；再次关闭设置才会隐藏。启动、收起、再次启动桌宠均不切换此策略。修改 Dock 设置时会保留已有前台窗口及键盘焦点；隐藏或最小化的主界面不会因此弹出，桌宠不会因切换到其他应用而自动隐藏。

macOS 运行期间，顶部菜单栏会常驻一个**爪印图标**，使用系统 `pawprint.fill` 模板图像适配浅色／深色菜单栏，不显示项目名文字；悬停提示为“ArkPets 桌宠”。隐藏 Dock 图标或收起桌宠后仍保留入口。点击可选择“显示主界面”“启动桌宠／收起桌宠”和“退出 ArkPets”。“启动桌宠／收起桌宠”保留主界面的隐藏或最小化状态，只有“显示主界面”会恢复窗口并切到前台。菜单中的开关文字与实际桌宠状态同步。退出会正常保存偏好并移除菜单栏入口。

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

Spine 测试直接加载三个样例并验证动画顶点变化。GUI 测试覆盖不同窗口尺寸下的鼠标命中，避免初始化或最小化时出现 NaN；网格批处理测试检查绘制顺序、索引和混合颜色。运行时测试覆盖命令队列上限、每帧处理预算、参数校验和暂停。Linux 已在 WSL Debian 的 Xvfb/Openbox/合成器环境验证系统条目切换和动画；macOS 提供独立的原生窗口回归，用于验证 Dock 策略切换时的启动器可见性和焦点。

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

增加 `-BehaviorSettings` 可通过真实鼠标和键盘修改速度、活跃度、翻转方式、多屏开关与初始位置，检查保存结果，并关闭重力验证重新部署后的原生窗口坐标。多屏几何测试覆盖负坐标、相接和错位屏幕、空隙、屏幕移除与 DPI 重定位；实际多显示器热插拔和混合 DPI 设备仍需实机验证。macOS 已有下述原生回归，覆盖范围见 [迁移记录](docs/MIGRATION.md#验证策略)。

Linux 的回归脚本创建独立的 Xvfb/Openbox 桌面，检查窗口状态、连续切换时 XID 不变，以及启动器最小化后桌宠图像非空、动画像素继续变化。需要 `xvfb`、`xauth`、`openbox`、`xcompmgr`、`x11-utils`、`x11-xserver-utils`、`xdotool`、`wmctrl`、ImageMagick 和 Vulkan 驱动（无 GPU 时可用 `mesa-vulkan-drivers`）：

```sh
cargo build --bin arkpets-ui
bash docs/scripts/Verify-ShellVisibility-Linux.sh target/debug/arkpets-ui target/linux-shell
```

macOS 的回归直接调用应用使用的 AppKit 模块，检查反复切换 Dock 策略、关闭后再次创建窗口，以及已隐藏或最小化窗口的状态。销毁验证先恢复 Dock 策略、再销毁原生窗口，确认异步回调不会重新显示旧窗口，并覆盖桌宠持有焦点和 Dock 开关开启的关闭场景。脚本会打开测试窗口，并启动一个独立的临时窗口进程，验证应用处于后台时切换策略不会抢焦点；结束后自动关闭，不读写用户偏好。需要登录图形桌面，运行期间请勿切换应用或关闭测试窗口：

```sh
cargo run --locked --example verify-macos-shell-visibility
```

菜单栏回归会创建真实 `NSStatusItem`，触发原生菜单动作，检查显示／收起文字同步、隐藏 Dock 后入口仍可见，以及退出时释放菜单和回调。它同样需要图形桌面，且不读写用户偏好：

```sh
cargo run --locked --example verify-macos-menu-bar
```

实际 Metal UI 的生命周期回归覆盖主界面按钮和菜单栏两种入口，在 Dock 显示开／关两种设置下反复启动、收起和再次启动桌宠，并检查重新启动后的动画回调继续推进、Dock 策略及应用图标不变。它还会最小化、隐藏主界面，确认菜单只控制桌宠，且仅“显示主界面”会恢复主窗口。测试使用三个样例中的真实佩佩模型，并把偏好隔离到 `target/macos-menu-lifecycle/`：

```sh
cargo run --locked --example verify-macos-menu-lifecycle
```

预览回归不合成鼠标输入，检查桌宠打开时和单独预览时的连续回调与真实 GPU 动画像素，并覆盖最小化、隐藏、设置页、暂停及恢复。测试会打开原生窗口，请保持鼠标静止、不要操作测试窗口；截图和隔离配置保存在 `target/macos-preview/`：

```sh
cargo run --locked --example verify-macos-preview
```

macOS 拖拽回归会短暂打开并移动一个原生测试窗口，检查窗口内鼠标坐标尚未更新时，全局光标、桌宠位置和松手速度不受窗口移动影响。需要图形桌面，运行期间请保持鼠标静止；测试不移动系统鼠标，也不读写用户偏好：

```sh
cargo run --locked --example verify-macos-drag
```

透明模式回归使用三个真实模型，在 Metal 桌宠上切换常规、透明、恢复和零时长透明模式，逐像素检查 25% 不透明度下的 RGBA 是完整角色合成结果的四分之一（允许 2/255 的量化误差），并检查原生鼠标穿透、窗口身份及暂停时位置不变。它会打开测试窗口，配置与按模型命名的截图隔离到 `target/macos-opacity/`，不移动系统鼠标；请勿手动操作测试窗口。截图验证绘制结果，不替代受控背景下的系统透明合成检查：

```sh
cargo run --locked --example verify-macos-opacity
```

行为与过渡回归使用暂停的真实佩佩模型，检查 GPU 图像的连续水平翻转、原生窗口移动的中间位置与终点、零时长立即移动，以及过渡完成后的空闲重绘。它不移动系统鼠标，偏好与截图隔离到 `target/macos-behavior/`：

```sh
cargo run --locked --example verify-macos-behavior
```

渲染效果回归逐个运行三个真实模型，检查高／低质量描边和阴影、手动模式颜色、组合效果的 25% 整体透明度，以及缩小时开关 MipMap 的实际像素差异。它使用暂停姿态，并把截图和偏好隔离到 `target/macos-effects/`：

```sh
cargo run --locked --example verify-macos-effects
```

原生回归需要屏幕保持亮起；屏幕休眠时 macOS 的活动显示器枚举可能为空。测试期间请勿操作测试窗口，空闲重绘检查会记录额外输入事件。

## 性能测量

macOS 可用 Python 3 直接采样真实 Metal 窗口。默认场景为模型库和桌宠同时打开，`--page settings` 可比较设置页场景；需要登录图形桌面，采样时保持窗口可见且不要操作应用：

```sh
cargo build --locked --release --bin arkpets-ui
python3 docs/scripts/measure_macos_performance.py --page library
python3 docs/scripts/measure_macos_performance.py --page settings
```

脚本先预热 3 秒，再采样 12 秒，自动退出，将 `frames.json`、`process.json` 和日志保存到 `target/macos-performance/` 下的独立目录，偏好与日常运行隔离。可通过 `--executable`、`--model`、`--fps`、`--warmup`、`--duration` 和 `--output` 固定比较条件。`process.json` 使用 `ps` 的累计进程 CPU 时间计算采样期间的占用率（100% 表示一个逻辑核心），并记录进程常驻内存 RSS 和可执行文件哈希；RSS 不是显存占用。模型库场景同时播放预览与桌宠；设置页场景仅播放桌宠，两者绘制工作量不同。暂停时桌面工作区检查仍保持每秒一次，不在截止时间前反复唤醒。

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

## Agent 联动（可选扩展，尚未实现）

Agent 联动的用途是让桌宠随外部任务状态做出反应，例如开始工作时切换动作、等待用户回复时播放提示动作、任务完成后回到待机。具体动作需要从模型实际支持的动作中选择。这些是未来的联动场景，普通桌宠的播放、拖拽、行走、透明度和四档过渡设置均可独立使用。

目前已有程序内部的 Rust 命令接口，可播放动作、设置朝向和精确修改过渡参数；外部 Agent 尚不能直接连接桌宠。JSONL 是“每行一条 JSON”的消息格式，IPC 是让本地两个进程交换消息的通道，两者可组合使用，具体传输与消息协议尚未确定。

当前继续优先完善桌宠功能与平台验证，实际需要联动时再接入通信。开发接口及过渡参数示例见 [命令入口](docs/ARCHITECTURE.md#命令入口)，联动的实现边界见 [后续接入](docs/ARCHITECTURE.md#后续接入)。消息气泡和养成系统同样尚未实现。

## 迁移范围

当前是可运行的原生版本，尚未完全替代 Java 原版。macOS 已有菜单栏入口；Windows/Linux 系统托盘、开机启动、多宠 IPC、其他窗口边缘碰撞、联网模型管理、多形态动作链等仍未迁移。Linux 使用 X11/XWayland，尚不提供原生 Wayland 桌宠后端。完整调研、选型和功能边界见 [迁移记录](docs/MIGRATION.md)。

交互、过渡、物理及渲染设置清单的实现状态、原版字段对照和兼容差异见 [配置迁移计划](docs/MIGRATION_PLAN.md)。

本项目代码沿用 GPL-3.0；Spine 运行时、字体和明日方舟素材分别适用其自身许可与版权声明，见 [第三方说明](THIRD_PARTY_NOTICES.md)。
