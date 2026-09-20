# 原项目调研与 Rust 迁移

## 来源与运行方式

- Ark-Pets：Java 17，LibGDX 1.11 + Spine 3.8.99.1，JavaFX 启动器，JNA / Win32 集成。初期调研路径为 `E:/GitHub/Ark-Pets`，版本为当时本地 `build.gradle` 中的 3.13.0；后续在 macOS 的本地 Ark-Pets 检出目录核对配置和渲染语义。参考仓库路径不参与 Rust 构建。
- ArkPets-Web：TypeScript，Spine WebGL，单个 Canvas 角色和右键菜单，包含三个可直接验证的本地模型；初期调研路径为 `E:/GitHub/ArkPets-Web`。
- 当前实现：Rust 1.95，eframe/egui 0.36.2 + winit 提供原生多窗口界面，wgpu 30 / Glow 负责 GPU 绘制。Windows 默认 DX12 + DirectComposition，并保留 OpenGL 选项；macOS 使用 Metal，Linux 使用 Vulkan。通过小型 FFI 适配层复用官方 Spine 3.8 C 运行时。

## 行为与模块映射

| 原实现 | 责任 | Rust 实现 |
| --- | --- | --- |
| `ModelItem`、`ModelsDataset` | 模型元数据、文件列表、类型目录 | `src/model.rs`，兼容 Web 清单、`storageDirectory` 与 `assetList`，以及旧版 `assetId` |
| Web `nextAction`、Java `StochasticMatrix` | 自动动作与可用动作过滤 | `src/animation.rs`，沿用 Web 权重，并根据设置和真实骨骼中的动作过滤 |
| Web `render` 中物理与拖拽 | 重力、阻尼、抛掷、边界 | `src/physics.rs`、`src/character.rs` |
| Java `Plane.updateVelocity` | 空气阻力、地面摩擦加速度和惯性速度上限 | `src/physics.rs`；`DesktopMotion` 提供屏幕并集的局部接地状态 |
| UI 与外部事件驱动 | 状态所有权、暂停、动作命令与处理预算 | `src/runtime.rs` 的 `PetRuntime` 与有界命令队列 |
| `ArkChar`、`SkeletonLoader` | 骨骼解码、混合、网格 | `src/spine.rs`、`src/spine_bridge.c`、`vendor/spine-c` |
| Web WebGL batcher | 图集与 GPU 三角网格 | `src/ui/renderer.rs`，按原图集分辨率上传纹理，复用骨骼输出缓冲；相邻同图集 slot 合并批次，保持顺序和普通 / 加法混合 |
| JavaFX Launcher | 模型库、行为与显示设置 | `src/ui/mod.rs`，搜索、收藏、导入、动作选择、缩放、主题、偏好持久化 |
| Win32 桌宠窗口 | 透明、置顶、拖动、鼠标穿透 | egui deferred viewport，DX12 / OpenGL 原生透明；`src/ui/platform.rs` 设置 Windows 11 边框和圆角属性 |
| 桌宠任务栏条目 | 保持显示和动画，同时控制系统条目 | `src/ui/shell_visibility.rs` 分派 Windows 工具窗口样式、Linux X11/EWMH 提示、macOS 应用激活策略 |
| 系统菜单入口 | 显示启动器、启动／收起桌宠、退出 | `src/ui/menu_bar.rs` 使用 macOS `NSStatusItem` / `NSMenu`；Windows/Linux 系统托盘仍未实现 |
| `setTransparentMode`、`TransitionFloat`、`EasingFunction` | 透明／鼠标穿透模式和整体不透明度过渡 | `src/ui/opacity.rs` 与 `Preferences.click_through`；四种原版缓动曲线、独立透明度参数、暂停下过渡与即时切换 |
| Java `BehaviorModule` 两项过渡下拉选项 | 四档时长与自定义配置值 | `src/config.rs` 的 `TransitionPreset`、`src/ui/mod.rs` 设置界面；`ConfigureTransitions` 提供进程内局部更新 |
| Java 整角色后处理与 `adjustCanvas` | 描边、阴影、整体 Alpha 与窗口边界 | `src/ui/effects.rs`、`src/ui/renderer/`；wgpu／Glow 合成与真实动画 Alpha 的 CPU 边界分析 |

## 选型依据

GPUI 是用户提出的可选方案。本次使用 eframe/egui + wgpu / Glow：启动器与透明桌宠共享一套原生窗口和纹理绘制接口，且不需要嵌入浏览器。这个选择不构成对 GPUI 平台支持能力的否定；当前没有实现 GPUI 前端。

Windows DX12 使用 `Dx12BackendOptions.presentation_system = Dx12SwapchainKind::DxgiFromVisual`。这条路径由 wgpu 创建 DirectComposition visual 和 DXGI composition swapchain，支持预乘 alpha；普通 HWND swapchain 仍不提供相同透明能力。该功能首次引入于 [wgpu 27 官方发布说明](https://github.com/gfx-rs/wgpu/releases/tag/v27.0.0)，当前使用 wgpu 30。

OpenGL 使用 eframe 0.36.2 的 [Windows 透明子窗口修复 #8423](https://github.com/emilk/egui/pull/8423)：保留原生子窗口的透明请求，避免 glutin 的配置能力判断把请求覆盖为不透明。两个后端正常绘制时都没有逐帧 GPU 像素读回、DIB 拷贝或应用自己维护的分层窗口合成器；主动截图验证会按需读回。`platform.rs` 管理 Windows 任务栏样式，并针对支持的 Windows 11 系统设置无边框颜色、无圆角属性，不承担渲染或像素合成。

Windows 后端通过设置页选择后在同一进程内切换，重建窗口和 GPU 资源，保留角色、暂停和桌宠状态，保存到独立 `bootstrap.json`；命令行 `--renderer` 和 `ARKPETS_RENDERER` 可显式覆盖。依赖按平台启用对应后端，并将 DX12 的 `wgpu-hal` 与 `gpu-allocator` 统一到 `windows 0.62.2`，避免同名 COM 类型来自不同版本造成编译失败。

启动器和桌宠共享单个 `PetRuntime`、Spine 实例和纹理集合。桌宠使用 deferred viewport，打开后由其推进动画和桌面物理；可见的模型库预览按设定帧率持续绘制同一姿态，不重复推进骨骼，也不依赖鼠标事件。主界面隐藏、最小化或切到设置页时停止预览动画刷新。网格批次仅合并相邻同图集 slot，避免纹理排序改变遮挡关系；不缩小原始 PNG，不降低系统 DPI 对应的窗口像素密度。完整所有权、调度与扩展边界见 [架构说明](ARCHITECTURE.md)。

原模型的二进制头为 Spine `3.8.99`。已核实的 `rusty_spine 0.8.0` API 文档声明支持 Spine 4.2，因此不能直接替换运行时。这里固定官方 3.8 分支提交 `8b4844bd4b193ba9e54487ed397a777993cbad56`，保留其源代码和许可证。仅适配资源所有权、动画入口和绘制输出，骨骼解析、约束、蒙皮与裁剪继续使用上游实现。

Spine 模型通过原生 C 解析器载入，只应用于可信本地资源；这不是面向恶意二进制的沙箱。当前对 3.8 以外的版本明确报错。不同 Spine 版本的兼容转换尚未迁移。

Spine 包装只实现 `Send`，不实现 `Sync`：模型状态由唯一拥有者访问，可以转移所有权，但不允许无保护并发调用。同进程的载入操作通过锁串行化，以保护上游 JSON 错误指针、attachment ID 和空动画初始化等共享 C 状态；逐帧更新不持有该载入锁。

## 当前边界

已经实现并可运行的工作流：启动原生 UI、加载三个原始模型、动画播放和切换、预览区拖拽与物理、局部模型导入、收藏、行为与显示设置、主题切换、透明置顶桌宠窗口及其原生拖动。

另外收录荒芜拉普兰德的正面战斗模型（Spine 3.8.99），可从原始动作列表手动播放普通攻击。原有三个基建模型保留。技能骨骼分段可以手动播放，但完整技能动作链、独立战斗特效和伤害逻辑尚未实现；此项不代表 Java 多形态行为迁移完成。

系统条目隐藏支持 Windows、Linux X11/XWayland 和 macOS，切换不重建桌宠渲染器。Linux 使用 EWMH 提示，Alt+Tab 筛选遵循窗口管理器策略；原生 Wayland 后端尚未实现。macOS 默认只显示菜单栏入口，Dock/Command+Tab 开关独立控制整个应用；启动和收起桌宠不改变 Dock 策略，开启 Dock 后使用应用自带的绿色爪印图标。底部高度可手动指定，自动工作区检测目前仅支持 Windows。

macOS 顶部菜单栏提供独立的爪印图标入口，运行期间保持可用，隐藏 Dock 或收起桌宠不会移除。菜单启动／收起桌宠保留主界面的隐藏或最小化状态，显示主界面是独立菜单操作。菜单动作复用已有应用状态，正常退出时保存偏好并释放系统菜单；菜单项文字随实际桌宠状态更新。原生回归覆盖菜单动作、Dock 策略独立性、重复开关与清理，实际 Metal UI 回归覆盖隐藏／最小化主界面时的桌宠生命周期。

已迁移动作活跃度（0–16，默认 4）、四档交互式朝向翻转、多显示器移动与主屏初始部署比例；行走速度设置扩展至 0–200 逻辑像素/秒。多屏移动遵守各屏幕区域的并集，支持跨相接屏幕行走、跨屏拖拽、各屏落地和布局变更后的有效位置恢复。部署比例默认 20%/20%，按当前角色窗口尺寸扣除可活动范围，重新打开时应用。

活跃度没有照搬原 Java `GeneralBehavior.getMatrix` 对 IDLE 整行权重同比缩放的实现：整行同比缩放通常不改变归一化概率，16 级还会令整行归零。Rust 改为缩放待机目的状态权重，使级别越高越少选择待机。继续沿用现有可用动作过滤，不声称已经迁移全部 Java 行为矩阵和多形态过渡链。

已迁移透明模式的 `opacity_normal` / `opacity_dim` 行为：原版 `ArkPets.setTransparentMode` 在开启鼠标穿透时选择 dim，关闭时选择 normal，未实现悬停自动变暗。Rust 复用既有鼠标穿透开关，默认常规 100%、透明模式 75%，实际 dim 不超过 normal；预览仍保持普通模式。过渡默认 0.3 秒、`EASE_OUT_CUBIC`，并提供原版 `LINEAR`、`EASE_OUT_SINE`、`EASE_OUT_QUINT`；切换中可连续反向，0 秒立即完成，暂停时也能结束过渡。

常规属性过渡已覆盖位置、水平翻转、不透明度和描边强度，使用 Rust 字段 `propertyTransitionDuration` / `easing`；多形态动作偏移仍未迁移。骨骼交叉过渡使用独立 `animationMixDuration`，在 Spine 创建新动画 track entry 前应用，修正先切换、后改默认混合时长造成的延迟。旧 Rust `transitionDuration` 仍按动画混合读取，`opacityTransitionDuration` / `opacityTransition` 分别作为常规过渡和缓动的兼容别名，JSON 与实际 RON 持久化均有回归。

两项过渡时长已按 Java `BehaviorModule` 恢复四档下拉选项：禁用 0 秒、快速 0.1 秒、标准 0.3 秒、慢速 0.6 秒，默认标准。自定义数值仍保留并明确显示，不自动吸附到档位；新增类型化命令 `ConfigureTransitions` 可独立修改时长与缓动参数，校验失败整条拒绝，不重置动作或其他配置。该入口用于后续 Agent 适配器，尚不是外部 IPC 协议。

交互动画开关与拖拽开关独立；关闭交互动画后禁止点击和自动选择 `Interact`，保留显式手动播放和朝向设置。物理界面补齐重力、空气阻力、地面摩擦、最大水平／垂直惯性速率。新默认加速度为 800／100／500，最大速率为 1000／1000；摩擦按每秒线性减速，保留原图与 DPI 对应的逻辑坐标。旧指数阻尼偏好保留原值且不叠加新摩擦，调整阻力或恢复默认后采用加速度减速。最大速率 0 沿用 Rust 的停止该轴惯性语义，与 Java 的不限速不同，主动行走仍由行走速度独立控制。

透明度采用与 Java 相同的先合成、后整体淡化顺序：原始 slot Alpha 在 GPU 离屏纹理中完成遮挡和混合，最终预乘 RGBA 统一乘以整体不透明度，避免重叠部件变深。wgpu 与 Glow 均复用原始图集及按系统 DPI 分配的离屏纹理，不引入正常运行时的像素读回。

渲染设置已迁移普通／手动模式描边条件与 RGBA 颜色、描边宽度、阴影强度、高质量后处理和 MipMap。描边／阴影沿用 24 点高斯或 16 点圆周采样核，转换为预乘合成；宽度按逻辑像素适配 DPI，效果参数实时生效。MipMap 保留原图和所有层级，关闭时强制基础层采样，与 Java 的“保留 atlas 配置”不同。

窗口边界大小与计算方式已接入真实动画 Alpha 的 CPU 预计算，通过显式按钮应用。最长边 256 的分析网格、浮点累积及全部动画采样与 Java GPU／形态分组算法不同，但不再用初始姿态几何包围盒冒充覆盖阈值。边界变化不改变骨骼缩放，运行中重算不重置动画或暂停。缓存按资源版本失效，累计 Alpha 超过 4096 姿态或完全透明时明确报错；阈值回退会显示实际值。战斗动作另有独立绘制范围采样，物理范围与画布分离。详细字段、默认值、性能口径和兼容差异见 [配置迁移计划](MIGRATION_PLAN.md)。

桌宠功能和平台验证是当前优先方向。外部 Agent 联动属于可选扩展，可用于让角色响应任务开始、等待回复和结束；普通播放、拖拽及配置调整均独立运行。已有 `PetCommand` 是进程内 API，通信连接和动作状态映射尚未实现。

以下原版迁移项仍未完成，不能视作原项目完整功能对等：

- Windows/Linux 系统托盘、开机启动、启动器与多个独立桌宠进程的 IPC。
- 其他桌面窗口边缘碰撞、多宠互斥。
- 模型库联网下载、压缩包导入、更新服务、公告和原项目遥测。
- Java 原版的多形态动画组合、过渡链与全部行为权重规则。
- 多形态动作垂直偏移的属性过渡及退出淡出。
- 双色 tint，以及 Multiply / Screen 混合。后二者遇到时明确报错，不伪造兼容渲染。
- 原始 Java 配置 JSON 的全部字段兼容；当前 Rust 参数保存在应用用户目录。

另有尚未实现的扩展：外部 Agent 联动、消息气泡和养成系统。JSONL 只是一种候选消息格式，IPC 是进程间通信机制，具体协议与连接方式均未确定；这些扩展不作为上述配置迁移的前置条件。

## 验证策略

交互动画、拖拽、常规属性过渡、加速度物理和后续渲染设置的逐项验收见 [配置迁移计划](MIGRATION_PLAN.md)。新增测试覆盖交互／拖拽独立性、加速度摩擦跨帧率一致性、局部屏幕地面、显示位置与物理位置分离、L 形内角连续移动、DPI 变化中保留过渡进度、零时长与非法输入。`verify-macos-behavior` 在 Apple M2 / Metal 验证暂停骨骼下的真实翻转图像、原生窗口移动中间点／终点、零时长移动及结束后的低频刷新；不代替混合 DPI 和 Windows/Linux 实机验证。

单元测试覆盖动画选择、拖拽、边界碰撞、快照、模型清单，以及命令的校验、背压和暂停。Spine 集成测试直接加载三个真实 `.skel/.atlas/.png` 资源，检查网格索引合法并比较动画推进前后的顶点变化，同时覆盖运行时所有权转移和并行载入。GUI 测试覆盖正常、小窗口和极小初始化尺寸的鼠标命中测试，防止不可用绘制区域产生 NaN；批处理测试检查同图集合并时不会改变 slot 顺序、索引和混合颜色。

新增测试覆盖活跃度对动作分布的影响、即时速度更新、翻转触发方式与抖动过滤、旧配置默认值、初始部署边界、负坐标、错位屏幕、空隙、跨屏落地及 DPI 重定位。Windows 的 `Verify-Interaction.ps1 -BehaviorSettings` 还会实际修改并保存这些设置，关闭重力后核对重新部署的 HWND 坐标。几何测试不能替代实际多显示器热插拔和混合 DPI 验证；macOS 已验证的单机原生场景不能外推到这些设备场景。

透明模式测试覆盖四种曲线、反向切换、实时参数调整、零时长、旧配置兼容、非法时长、三个真实模型保留 slot 颜色并独立提交整体 Alpha，以及暂停后过渡完成时停止高频刷新。`verify-macos-opacity` 已在 Apple M2 / Metal 运行三个模型的真实窗口回归：25% 不透明度下逐像素验证 RGBA 等于正常合成结果的四分之一，允许 2/255 的量化误差，并确认原生鼠标穿透、窗口身份、暂停和位置保持。双窗口预览回归也已通过。Glow 离屏模块另在 macOS OpenGL 原生窗口验证重叠矩形的整体 Alpha 与尺寸变更；该精确图形测试是补充验证，不代替真实 Spine 资源测试。上述回归不是受控背景的系统合成截图，Windows DX12/OpenGL 和 Linux 的新合成路径仍需独立实机验证。

Windows 的构建、透明合成、重绘节奏和真实鼠标操作需要分别验证；提供的脚本不代替所有交互场景的实机检查。Linux 既有版本已在 WSL Debian 的 Xvfb/Openbox/合成器环境运行构建、单元测试、Clippy，以及真实窗口状态和动画像素回归，并将图形回归加入 CI；这些结果不覆盖本批新增的全部效果。macOS 26.5.2 / Apple M2 已完成完整目标测试、Clippy、release 构建，并通过菜单栏、Dock 和桌宠生命周期回归。早期通用 GPU 截图自检曾出现预览动画像素变化为 0，后续独立 `verify-macos-preview` 已覆盖无鼠标输入下的持续播放、隐藏／最小化／设置页／暂停与恢复；未据此声明旧通用自检全部通过。Intel Mac、其他 macOS 版本及新设置的 Windows/Linux 实机效果仍需实测。

性能验证使用 release 构建和 `docs/scripts/Measure-Performance.ps1`，记录预热后的桌宠回调间隔、长间隔分布、主窗口更新情况、进程工作集和私有内存。该脚本不测量屏幕实际呈现 FPS 或显存。状态栏的 CPU 时间是 eframe 报告的上一帧应用耗时，也不是端到端 GPU 帧时间或性能保证。具体测量口径见 [架构说明](ARCHITECTURE.md#性能与验证)。

## 本批验证记录（2026-09-20）

四档过渡选项与局部命令更新完成后的本机检查：

| 检查 | 结果 |
| --- | --- |
| `cargo test --locked --no-default-features` | 42 项通过 |
| `cargo test --locked --all-targets` | 核心／Spine 51 项、UI 69 项，共 120 项通过 |
| `cargo fmt --all -- --check`、`git diff --check` | 通过 |
| `cargo clippy --locked --all-targets -- -D warnings` | 通过 |
| `cargo build --locked --release --bin arkpets-ui` | 通过 |

新增覆盖四档准确映射、自定义 0.137／0.423 秒不取整、未指定字段保留、运行状态不重置、非法参数在提交和执行入口整条拒绝，以及界面同步和实际 RON 持久化。此前同批渲染／行为迁移已运行上述 macOS 原生回归；四档控件与命令入口的最后一次修改没有重新执行原生窗口交互或 Windows/Linux 实机验证。本次纯文档同步仅核对引用、命令和文档差异，未重复运行代码测试。

## 攻击动作验证（2026-09-20，Windows）

新增荒芜拉普兰德战斗模型与攻击入口后，通过核心测试 44 项、Spine 测试 54 项、默认全部目标测试 121 项（核心／Spine 54 项、UI 67 项），以及格式检查、Clippy（拒绝警告）、debug 和 release UI 构建。测试涵盖真实攻击网格变化与循环完成事件、初始待机、活跃度、自动攻击后回到待机，以及手动技能分段不进入自动候选。

AMD Radeon 780M 上顺序运行 `Verify-Desktop.ps1 -Renderer dx12` 和 `-Renderer opengl`，两个后端均通过：原有基建预览、主题、下落／行走和受控背景系统透明合成；新增战斗模型的两帧攻击预览有显著像素差异，桌宠攻击截图包含角色和透明背景。新增截图为 `10-whitw2-attack.png`、`11-whitw2-attack-motion.png`、`12-whitw2-attack-desktop.png`。截图自检通过进程内动作入口播放攻击，未合成鼠标点击攻击按钮；新增战斗模型未在 macOS/Linux 实机复测，也未验证完整技能链或游戏独立战斗特效。

后续修复攻击武器裁切：保留原 Alpha 物理边界，单独扩大绘制画布，并按偏移换算原生位置、DPI 和拖拽起点，避免透明留白影响自动／手动任务栏高度。追加回归以 480 Hz 检查完整攻击和三种技能循环，以及左右朝向、0.5／1／1.7 倍缩放；物理测试覆盖 1／1.5／2 倍 DPI、系统工作区和多个手动高度下的落地与静止拖拽。默认全部目标测试 124 项通过（核心／Spine 54 项、UI 70 项），Clippy、格式和 debug／release 构建通过。

修复后的 Windows DX12／OpenGL 原生回归均通过最宽攻击姿态、左右翻转、描边与窗口边缘透明像素检查。新增 `13-whitw2-attack-left.png`、`14-whitw2-floor-auto.png`、`15-whitw2-floor-120.png`、`16-whitw2-floor-240.png`；直接核对原生窗口坐标加上物理区域偏移后的落地底边。两个后端在 1080 像素高屏幕上结果一致：自动工作区目标 1032、手动 120 像素目标 960、手动 240 像素目标 840；实际物理底边分别为 1031.52、959.52、839.52，像素对齐误差小于 1。此版本尚未在 macOS/Linux 和实际混合 DPI 多屏实测。

## 模型落地基准修复（2026-09-20，Windows）

对照本地 Java 原项目：`ArkChar.setCanvas` 将骨骼纵向位置初始化为 0，`render` / `renderAdditive` 在调整画布后仍保持该纵向基准；`DynamicOrthographicCamara.getFittedInsert` 计算画布尺寸，`ArkPets.refreshMonitorInfo` 独立设置屏幕底部预留。此前 Rust 将所有动画的累计 Alpha 裁剪底边加效果边距作为物理底边，导致不同模型的负 Y 附件抬高整只角色。真实模型失败回归在 height=180、padding=12、DPI=1 时测得站立留空：佩佩 14.27、基建拉普兰德 54.50、Friston-3 12.93、战斗拉普兰德 12.03 逻辑像素。

`DesktopLayout` 现以骨骼 `y=0` 为物理底边，保留独立的完整绘制画布、原有缩放和翻转中心。新增测试用真实图集 Alpha 检查站立底部，并检查两种裁剪设置、三档缩放、三档 DPI、三档效果留白及两个朝向下实际提交的网格坐标。窗口底边不再用于证明接地：原生交互脚本读取诊断布局并结合真实客户区屏幕坐标计算物理区域，暂停透明度测试也改用真实布局尺寸。

本机 `cargo test --locked --all-targets` 125 项通过（核心／Spine 54、UI 71）；Clippy（`-D warnings`）、格式检查、差异检查及 debug／release UI 构建通过。两种后端分别运行 `Verify-Desktop.ps1`，四个模型的自动任务栏高度与手动 120 像素高度均通过原生位置和实际 GPU 像素底部检查，站立轮廓相对骨骼地面线误差均小于 1 个物理像素；同时通过受控背景系统透明合成、攻击、下落和行走回归。新增截图为 `17-pepe-ground-auto.png` 至 `24-battle-ground-120.png`。

DX12／OpenGL 分别运行 `Verify-Interaction.ps1 -DesktopSettings -BehaviorSettings`，均通过悬停、拖拽、收起／重开、任务栏与 Alt+Tab 样式切换、手动 0／80／120 像素高度、恢复自动工作区、80%／10% 初始部署及 RON 偏好持久化。本批未运行 macOS/Linux 实机回归或实际混合 DPI 多屏测试。

## 更多模型与自动战斗循环（2026-09-20，Windows）

新增阿米娅、史尔特尔、缄默德克萨斯、逻各斯的基建和正面战斗版本各一，共 8 个，清单总数为 12。全部为原始 Spine 3.8.99 资源，固定上游提交并逐文件核对 Git blob SHA-1、文件大小和 SHA-256；没有改写骨骼或降低图集分辨率。下载中部分 Raw 文件发生连接重置，后改从同一 GitHub 仓库的固定 blob API 获取并验证相同校验值，未使用替代模型或占位文件。

已对照 Java 原版的随机矩阵、动作分组、Loop 优先规则和出场语义，分析见 [自动动作与战斗循环](ANIMATION_BEHAVIOR.md)。基建权重改用 Java 矩阵，保留本项目的随机战斗扩展；攻击和技能正确选择循环主体，排除起止段、技能待机和出场片段。自动攻击可连续多轮保持同一 track，不再每轮强制回待机。界面增加实际可用的攻击／技能循环菜单，原始片段仍可手动选择，Start 与起止段单次播放。动作识别也统一用于初始待机、行走开关和战斗画布预留。

本批核心无默认 feature 测试 48 项、Spine 测试 59 项、全部目标测试 130 项（核心／Spine 59、UI 71）通过；Clippy（`-D warnings`）、格式、差异检查与 debug／release 构建通过。真实资源测试逐一验证所有战斗候选至少完成两轮、网格发生变化，以及 Start 只完成一次。随机行为测试覆盖 Java 基建权重与左右行走分布、活跃度、禁用动作、Loop 优先、循环主体持续播放及过渡／出场排除。

顺序运行 DX12／OpenGL 的 `Verify-Desktop.ps1`，每个后端 40 张截图及系统透明合成回归通过；新增 8 个模型的桌宠截图均有可见角色及真实动画像素变化。战斗样例分别播放阿米娅 `Attack`、史尔特尔 `Skill_3_Loop`、缄默德克萨斯 `Skill_2_Loop`、逻各斯 `Skill_2_Loop`。原有四模型落地、攻击、下落和行走回归继续通过。未在 macOS/Linux 实机复测新增资源和随机行为；本批也没有重复执行鼠标交互脚本或完整技能链验证。

## 手动模式键盘操控（2026-09-20，Windows）

对照 Java `MemberTrayImpl.onKeepAnimEn`、`ArkPets.render` 和 `ArkPets.onKeyDown`，补齐手动模式的左右键临时行走、Ctrl 双倍速度、上下键循环切换非行走动作，以及松开后恢复所选动作。鼠标下拉框和攻击菜单继续可用，选定动作后将键盘焦点交回预览控制区域。进入手动模式先回待机；战斗模型沿用 Loop 候选，缺少 Move 时左右键仅转向并在界面提示。没有新增全局按键钩子。

`ui/keyboard.rs` 维护当前控制视口和临时动作；未获焦的另一个视口不会取消控制，文本／设置控件的输入也不会操控角色。失焦、禁用控制、暂停、拖拽、关闭控制窗口、模型切换和后端重建会清理临时状态。核心物理只接收临时方向及加速标志，配置速度不被改写；边缘碰撞不改变按住的方向。对比修复前，新增真实 UI 输入回归原本无法从 Sit 切换到 Move，修复后验证按住和松开的动作切换。

无默认 feature 核心测试 49 项、默认全部目标测试 136 项（核心／Spine 60、UI 76）通过，Clippy（`-D warnings`）、格式、差异检查及 debug／release 构建通过。新增测试覆盖加速与配置保持、边界朝向、同时按键与释放、非行走主体选择、按键重复抑制、失焦、控件占用、模式停用、鼠标更新所选动作，以及双视口控制所有权。

两种后端均运行 `Verify-Interaction.ps1 -KeyboardControls -DesktopSettings -BehaviorSettings`。真实键盘测试中，约 700 ms 持键后 DX12 普通／加速位移为 23／44 物理像素，OpenGL 为 22／44；松开均恢复 Sit，按住方向键切走焦点后停止，预览窗口输入均使共享桌宠移动 23 物理像素。数字包含原生事件调度与位置过渡的采样时机，不是精确的速度测量。任务栏样式、手动 0／80／120 像素底边、恢复自动工作区、80%／10% 部署及 RON 保存也全部通过。本批未执行 macOS/Linux 实机键盘回归或实际混合 DPI 多屏测试。

## 同角色形态联动与独立位移（2026-09-20，Windows）

12 套模型通过 `characterId`、`skinId`、`form` 显式配对，模型库合并为 7 个角色条目。新增自动切换、固定基建、固定战斗及自动模式下的待机形态偏好，按角色与皮肤保存；旧选中模型 ID 和收藏仍可读取，无配对信息的导入资源保持独立。角色载入时准备完整配对组，关联资源缺失则保留当前角色并报告错误，不留下半组资源。

方向键位移不再要求 Move 动画。自动形态可在普通行走时借用基建 Move，松开后恢复所选战斗待机；攻击／技能循环可以同时移动，固定战斗形态也可使用当前姿态位移。只有一份 PetRuntime 和一个活动 Spine 时钟，缓存至多一个非活动形态；切换不重置命令队列、速度、朝向、暂停或按键状态，不播放 Start。跨骨骼直接切换，不新增淡入淡出。桌面与预览均按骨骼地面线和脚底中心重定位，平移已有属性过渡并保留进度；工作区边界仍可能要求投影。

核心无默认 feature 测试 51 项、默认全部目标测试 142 项（核心／Spine 62、UI 80）通过；格式、差异检查、Clippy（`-D warnings`）及最终 debug／release 构建通过。新增回归覆盖无 Move 的战斗位移、移动中切换技能、显式角色／皮肤配对与非法元数据、暂停下的真实双形态往返、资源缓存复用、运行时／命令队列保留、缺失配对资源的原子失败，以及形态偏好的 JSON 和实际 eframe RON 序列化。几何测试覆盖 1／1.5／2 倍 DPI、负屏幕坐标、脚底锚点和速度保持。

DX12／OpenGL 各完成 46 张 GPU 截图和系统透明合成回归。新增 `41-texas-forms-library.png` 至 `46-texas-fixed-battle-loop.png`，验证合并后的界面、自动战斗→基建→战斗、固定基建和固定战斗技能。暂停时从原生窗口位置还原的脚底锚点在两套资源间误差小于 1 个物理像素；原有 12 套真实资源动画与透明合成检查也通过。构建中曾因运行中的普通 debug 程序占用可执行文件而失败，截图验证使用独立命名的相同源码构建，随后普通 debug／release 均已重新构建成功。

本批新增 `Verify-Interaction.ps1 -FormControls -FormMode auto|battle`，用于验证真实按键下的借用行走、松开恢复、固定战斗位移和移动攻击，并支持 `-Executable` 指定构建。实际尝试未取得通过结果：分别遇到鼠标按下、窗口尚未可见、指针命中其他应用，以及一次悬停检查位置变化。启动可见性等待和可点击区域选择已改为根据真实原生状态判断；没有关闭运行中的用户程序、向其他应用发送点击，或把失败标记为成功。这项额外输入回归仍需在无输入干扰的桌面复验，以区分会话操作干扰与潜在交互问题。macOS/Linux 和实际混合 DPI 多屏尚未实机验证本批改动。

## 行走优先修正（2026-09-20，Windows）

按后续需求调整上一版的移动规则：主动行走必须播放真实 Move，攻击／技能作为松开方向键后的恢复目标。行走期间通过上下键、鼠标或命令选择的新动作只更新恢复目标，在 Spine track 安装前拦截，不切入攻击、不重启步态。固定战斗形态缺少 Move 时也临时借用配对基建的 Move，其他非行走动作仍受固定模式限制；整组没有 Move 的模型不进行按键位移，界面明确提示。行走位置直接同步物理目标，避免松开恢复攻击后仍有位置缓动。该规则取代上一节的“边移动边攻击”和“无 Move 滑行”。

核心无默认 feature 测试 51 项、全部目标测试 144 项（核心／Spine 62、UI 82）、格式、差异检查、Clippy（`-D warnings`）以及 debug／release 构建通过。新增和调整的测试覆盖攻击被 Move 覆盖、行走中选择技能后延迟恢复、无 Move 时禁止滑行，以及固定战斗模式借用基建后保持偏好和脚底位置。真实缄默德克萨斯回归还检查了行走期间收到新的技能命令后，活动形态仍为基建，Spine 网格姿态保持不变，释放后才恢复目标战斗技能。

DX12／OpenGL 各通过 48 张 GPU 截图与系统透明合成检查，新增 `47-texas-fixed-battle-move.png` 和 `48-texas-fixed-battle-resume.png` 验证固定战斗设置下的 Move 借用和恢复，脚底锚点误差小于 1 个物理像素。真实按键脚本的断言已同步为“移动期间 Move、松开恢复技能”，本批未再次执行鼠标／键盘注入脚本，也未进行 macOS/Linux 或混合 DPI 多屏实机复验。

## 动作逻辑精简（2026-09-21）

本节取代前述自动战斗、动画阶段解析和跨形态借用 Move 的运行规则。保留全部资源、显式形态选择、缓存、物理与键盘操控；删除全局 AnimationKind／Phase 分类、Loop 优先筛选、随机战斗列、攻击专用菜单、点击回退攻击、合并动作目录和按名字寻找资源。普通播放只处理当前骨骼，换形态只由显式选择触发。

基建保留精确 Relax／Sit／Sleep／Move／Special 自动权重，其他骨骼保持当前动作。手动动画统一使用原始列表，除既有 Interact／Special 外循环播放，不再根据后缀猜测单次播放。上下键也保留原始片段；缺少当前形态 Move 时不位移。旧 mode／preferred 偏好兼容读取，auto 固定为其保存的偏好形态。详见 [动作播放与形态选择](ANIMATION_BEHAVIOR.md)。

相对本轮开始时的工作区，`animation.rs`、`ui/forms.rs`、`ui/mod.rs` 合计净减少 541 行（含测试），未增加依赖。核心测试 48 项、含 Spine 的测试 59 项、默认全部目标 141 项（核心／Spine 59、UI 82）、Clippy（`-D warnings`）、格式／差异检查和 debug／release 构建通过。新增回归检查原始片段不触发推断、不存在的当前形态动作被拒绝、显式换形态保持脚底／速度／暂停／运行时、缓存复用与旧 auto 偏好兼容。真实资源测试播放原始动画，单次播放仍通过显式 Spine API 验证。

Windows DX12／OpenGL 各通过 48 张 GPU 截图与受控背景系统透明合成检查，显式基建／战斗往返的原生脚底锚点误差小于 1 像素。最后的旧选中模型 ID 偏好保存补充与截图文件名调整后，重跑相关测试及构建，未重复截图流程。最终 DX12 `Verify-Interaction.ps1 -FormControls -FormMode battle` 全部通过。OpenGL 曾通过战斗按键检查但在脚本的 RON 转义断言失败；实际保存文件正确，断言已修正并用真实文件核验。顺序复跑时，OpenGL 战斗在拖拽位移检查失败（期望 10/-8，观察到 48/0），OpenGL 基建因焦点停留主窗口拒绝发键，DX12 基建因悬停阶段检测到鼠标按下退出；未将这些完整输入回归标记通过，拖拽失败的环境／应用归因尚未确定。日志分别位于 `target/cleanup-input-final-*/`。本轮未在 macOS/Linux 或实际混合 DPI 多屏实机复测。
