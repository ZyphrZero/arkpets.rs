# 原项目调研与 Rust 迁移

## 来源与运行方式

- `E:/GitHub/Ark-Pets`：Java 17，LibGDX 1.11 + Spine 3.8.99.1，JavaFX 启动器，JNA / Win32 集成。调研版本为本地 `build.gradle` 中的 3.13.0。
- `E:/GitHub/ArkPets-Web`：TypeScript，Spine WebGL，单个 Canvas 角色和右键菜单，包含三个可直接验证的本地模型。
- 当前实现：Rust 1.95，eframe/egui 0.36.2 + winit 提供原生多窗口界面，wgpu 30 / Glow 负责 GPU 绘制。Windows 默认 DX12 + DirectComposition，并保留 OpenGL 选项；macOS 使用 Metal，Linux 使用 Vulkan。通过小型 FFI 适配层复用官方 Spine 3.8 C 运行时。

## 行为与模块映射

| 原实现 | 责任 | Rust 实现 |
| --- | --- | --- |
| `ModelItem`、`ModelsDataset` | 模型元数据、文件列表、类型目录 | `src/model.rs`，兼容 Web 清单、`storageDirectory` 与 `assetList`，以及旧版 `assetId` |
| Web `nextAction`、Java `StochasticMatrix` | 自动动作与可用动作过滤 | `src/animation.rs`，沿用 Web 权重，并根据设置和真实骨骼中的动作过滤 |
| Web `render` 中物理与拖拽 | 重力、阻尼、抛掷、边界 | `src/physics.rs`、`src/character.rs` |
| UI 与外部事件驱动 | 状态所有权、暂停、动作命令与处理预算 | `src/runtime.rs` 的 `PetRuntime` 与有界命令队列 |
| `ArkChar`、`SkeletonLoader` | 骨骼解码、混合、网格 | `src/spine.rs`、`src/spine_bridge.c`、`vendor/spine-c` |
| Web WebGL batcher | 图集与 GPU 三角网格 | `src/ui/renderer.rs`，按原图集分辨率上传纹理，复用骨骼输出缓冲；相邻同图集 slot 合并批次，保持顺序和普通 / 加法混合 |
| JavaFX Launcher | 模型库、行为与显示设置 | `src/ui/mod.rs`，搜索、收藏、导入、动作选择、缩放、主题、偏好持久化 |
| Win32 桌宠窗口 | 透明、置顶、拖动、鼠标穿透 | egui deferred viewport，DX12 / OpenGL 原生透明；`src/ui/platform.rs` 设置 Windows 11 边框和圆角属性 |
| 桌宠任务栏条目 | 保持显示和动画，同时控制系统条目 | `src/ui/shell_visibility.rs` 分派 Windows 工具窗口样式、Linux X11/EWMH 提示、macOS 应用激活策略 |

## 选型依据

GPUI 是用户提出的可选方案。本次使用 eframe/egui + wgpu / Glow：启动器与透明桌宠共享一套原生窗口和纹理绘制接口，且不需要嵌入浏览器。这个选择不构成对 GPUI 平台支持能力的否定；当前没有实现 GPUI 前端。

Windows DX12 使用 `Dx12BackendOptions.presentation_system = Dx12SwapchainKind::DxgiFromVisual`。这条路径由 wgpu 创建 DirectComposition visual 和 DXGI composition swapchain，支持预乘 alpha；普通 HWND swapchain 仍不提供相同透明能力。该功能首次引入于 [wgpu 27 官方发布说明](https://github.com/gfx-rs/wgpu/releases/tag/v27.0.0)，当前使用 wgpu 30。

OpenGL 使用 eframe 0.36.2 的 [Windows 透明子窗口修复 #8423](https://github.com/emilk/egui/pull/8423)：保留原生子窗口的透明请求，避免 glutin 的配置能力判断把请求覆盖为不透明。两个后端正常绘制时都没有逐帧 GPU 像素读回、DIB 拷贝或应用自己维护的分层窗口合成器；主动截图验证会按需读回。`platform.rs` 管理 Windows 任务栏样式，并针对支持的 Windows 11 系统设置无边框颜色、无圆角属性，不承担渲染或像素合成。

后端通过设置页选择并重启，保存到独立 `bootstrap.json`；命令行 `--renderer` 和 `ARKPETS_RENDERER` 可显式覆盖。依赖按平台启用对应后端，并将 DX12 的 `wgpu-hal` 与 `gpu-allocator` 统一到 `windows 0.62.2`，避免同名 COM 类型来自不同版本造成编译失败。

启动器和桌宠共享单个 `PetRuntime`、Spine 实例和纹理集合。桌宠使用 deferred viewport，打开后由其独立推进动画和物理，启动器按事件刷新。网格批次仅合并相邻同图集 slot，避免纹理排序改变遮挡关系；不缩小原始 PNG，不降低系统 DPI 对应的窗口像素密度。完整所有权、调度与扩展边界见 [架构说明](ARCHITECTURE.md)。

原模型的二进制头为 Spine `3.8.99`。已核实的 `rusty_spine 0.8.0` API 文档声明支持 Spine 4.2，因此不能直接替换运行时。这里固定官方 3.8 分支提交 `8b4844bd4b193ba9e54487ed397a777993cbad56`，保留其源代码和许可证。仅适配资源所有权、动画入口和绘制输出，骨骼解析、约束、蒙皮与裁剪继续使用上游实现。

Spine 模型通过原生 C 解析器载入，只应用于可信本地资源；这不是面向恶意二进制的沙箱。当前对 3.8 以外的版本明确报错。不同 Spine 版本的兼容转换尚未迁移。

Spine 包装只实现 `Send`，不实现 `Sync`：模型状态由唯一拥有者访问，可以转移所有权，但不允许无保护并发调用。同进程的载入操作通过锁串行化，以保护上游 JSON 错误指针、attachment ID 和空动画初始化等共享 C 状态；逐帧更新不持有该载入锁。

## 当前边界

已经实现并可运行的工作流：启动原生 UI、加载三个原始模型、动画播放和切换、预览区拖拽与物理、局部模型导入、收藏、行为与显示设置、主题切换、透明置顶桌宠窗口及其原生拖动。

系统条目隐藏支持 Windows、Linux X11/XWayland 和 macOS，切换不重建桌宠渲染器。Linux 使用 EWMH 提示，Alt+Tab 筛选遵循窗口管理器策略；原生 Wayland 后端尚未实现。macOS 的 Dock/Command+Tab 开关影响整个应用，窗口仍显示，收起桌宠后恢复启动器入口。底部高度可手动指定，自动工作区检测目前仅支持 Windows。

已迁移动作活跃度（0–16，默认 4）、四档交互式朝向翻转、多显示器移动与主屏初始部署比例；行走速度设置扩展至 0–200 逻辑像素/秒。多屏移动遵守各屏幕区域的并集，支持跨相接屏幕行走、跨屏拖拽、各屏落地和布局变更后的有效位置恢复。部署比例默认 20%/20%，按当前角色窗口尺寸扣除可活动范围，重新打开时应用。

活跃度没有照搬原 Java `GeneralBehavior.getMatrix` 对 IDLE 整行权重同比缩放的实现：整行同比缩放通常不改变归一化概率，16 级还会令整行归零。Rust 改为缩放待机目的状态权重，使级别越高越少选择待机。继续沿用现有可用动作过滤，不声称已经迁移全部 Java 行为矩阵和多形态过渡链。

以下内容仍未完成，不能视作原项目完整功能对等：

- 系统托盘、开机启动、启动器与多个独立桌宠进程的 IPC。
- Agent CLI 的 JSONL / IPC 适配器、消息气泡和养成系统。当前已提供类型化命令与有界队列，但没有发布外部传输协议或实现这些产品功能。
- 其他桌面窗口边缘碰撞、多宠互斥。
- 模型库联网下载、压缩包导入、更新服务、公告和原项目遥测。
- Java 原版的多形态动画组合、过渡链与全部行为权重规则。
- 描边、阴影、双色 tint，以及 Multiply / Screen 混合。后二者遇到时明确报错，不伪造兼容渲染。
- 原始 Java 配置 JSON 的全部字段兼容；当前 Rust 参数保存在应用用户目录。

## 验证策略

单元测试覆盖动画选择、拖拽、边界碰撞、快照、模型清单，以及命令的校验、背压和暂停。Spine 集成测试直接加载三个真实 `.skel/.atlas/.png` 资源，检查网格索引合法并比较动画推进前后的顶点变化，同时覆盖运行时所有权转移和并行载入。GUI 测试覆盖正常、小窗口和极小初始化尺寸的鼠标命中测试，防止不可用绘制区域产生 NaN；批处理测试检查同图集合并时不会改变 slot 顺序、索引和混合颜色。

新增测试覆盖活跃度对动作分布的影响、即时速度更新、翻转触发方式与抖动过滤、旧配置默认值、初始部署边界、负坐标、错位屏幕、空隙、跨屏落地及 DPI 重定位。Windows 的 `Verify-Interaction.ps1 -BehaviorSettings` 还会实际修改并保存这些设置，关闭重力后核对重新部署的 HWND 坐标。几何测试不能替代实际多显示器热插拔和混合 DPI 验证；macOS 桌面效果仍需实机检查。

Windows 的构建、透明合成、重绘节奏和真实鼠标操作需要分别验证；提供的脚本不代替所有交互场景的实机检查。Linux 已在 WSL Debian 的 Xvfb/Openbox/合成器环境运行构建、单元测试、Clippy，以及真实窗口状态和动画像素回归，并将图形回归加入 CI。macOS 新增 AppKit 模块已通过 Apple Silicon、Intel 目标类型检查，完整应用仍需 macOS 构建和实机窗口验证。不同桌面环境的窗口切换器行为不能仅凭 GUI 库的跨平台能力保证一致。

性能验证使用 release 构建和 `docs/scripts/Measure-Performance.ps1`，记录预热后的桌宠回调间隔、长间隔分布、主窗口更新情况、进程工作集和私有内存。该脚本不测量屏幕实际呈现 FPS 或显存。状态栏的 CPU 时间是 eframe 报告的上一帧应用耗时，也不是端到端 GPU 帧时间或性能保证。具体测量口径见 [架构说明](ARCHITECTURE.md#性能与验证)。
