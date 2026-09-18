# arkpets.rs 项目指南

本文件适用于当前仓库。遵循上层规则，保留已有的暂存和未暂存修改；只修改当前任务涉及的文件。面向用户的说明默认使用简体中文。

## 项目定位与参考

- 本项目是 Ark-Pets 的 Rust 原生桌面实现，包含与 UI 无关的行为核心、命令行工具和透明桌宠窗口。
- 原项目：`E:\GitHub\Ark-Pets`，用于查阅 Java 桌面行为与平台集成。修改参考仓库需要属于用户明确要求的任务范围；本仓库不依赖这一绝对路径构建或运行。
- 先阅读 [README.md](README.md) 了解使用方式，按任务阅读 [架构说明](docs/ARCHITECTURE.md) 和 [迁移记录](docs/MIGRATION.md)。依赖与 feature 以 `Cargo.toml` 为准，CI 命令以 `.github/workflows/ci.yml` 为准，文档与实现不一致时先核对当前代码。
- 当前尚未完整替代原项目。系统托盘、开机启动、多宠 IPC、联网模型管理、Agent CLI 传输协议、消息气泡和养成系统等不能当作已有能力。

## 代码入口与职责

| 路径 | 职责 |
| --- | --- |
| `src/lib.rs` | 核心库模块与公开类型导出 |
| `src/main.rs` | `arkpets` 命令行：模型检查与无窗口物理演算 |
| `src/bin/arkpets-ui.rs`、`src/ui/mod.rs` | `arkpets-ui` 入口、启动器、设置、状态共享、持久化和帧调度 |
| `src/model.rs`、`src/config.rs` | 模型清单、资源路径、角色配置 |
| `src/animation.rs`、`src/physics.rs`、`src/character.rs` | 动作选择、物理、拖拽和角色状态 |
| `src/interaction.rs` | 与窗口无关的交互翻转模式和指针手势状态 |
| `src/runtime.rs` | `PetRuntime`、类型化命令、有界队列、暂停和命令执行结果 |
| `src/spine.rs`、`src/spine_bridge.c`、`build.rs` | Spine 3.8 C 运行时的构建、FFI 所有权、动画与裁剪网格输出 |
| `src/ui/renderer.rs`、`src/ui/backend.rs` | 图集与网格绘制；后端选择、校验和 bootstrap 配置 |
| `src/ui/desktop.rs`、`src/ui/desktop/regions.rs`、`src/ui/platform.rs` | 多屏部署与物理、屏幕并集边界；Windows 原生窗口集成 |
| `src/ui/shell_visibility.rs`、`src/ui/shell_visibility/` | 平台外壳可见性入口，以及 Linux X11、macOS 实现 |
| `src/ui/smoke.rs`、`src/ui/performance.rs`、`docs/scripts/` | 截图、真实窗口交互回归和性能采样 |
| `assets/`、`vendor/` | 样例模型与字体；随仓库维护的第三方代码及许可证 |

## 实现约束

- 保持核心库与 egui、窗口系统和外部通信协议解耦。新增外部集成应在适配层解析输入，再提交 `PetCommand`；保留队列容量、每帧处理预算、参数校验和失败反馈。入队成功不等于命令执行成功。
- 启动器与桌宠共享一份角色运行时、Spine 实例和图集纹理。桌宠打开时由桌宠回调推进动画，启动器按事件刷新；避免双窗口重复推进、无条件定时重绘和持有 egui Context 的引用环。
- 区分预览坐标、桌面坐标、逻辑点与物理像素。修改拖拽、工作区或底部高度时，保留 DPI 换算、边界裁剪和释放速度限制；覆盖零尺寸、极小窗口及非有限数值边界。
- 保留原图集分辨率和系统 DPI。网格批处理只合并相邻兼容批次，保持 slot 顺序、索引与混合语义；不通过纹理排序改变遮挡关系。
- 正常渲染使用原生透明窗口与 GPU 合成，不引入逐帧 GPU 读回、CPU 像素拷贝或 GDI 合成。截图读回仅用于显式诊断。
- Windows 支持 DX12 与 OpenGL，默认 DX12；macOS 使用 Metal，Linux 使用 Vulkan。后端优先级为 `--renderer`、`ARKPETS_RENDERER`、`bootstrap.json`、平台默认值。无效配置或不支持的后端必须报错，不能静默换用其他后端。
- 修改后端切换时保留同进程切换、角色与暂停状态，以及纹理重新绑定。修改 DX12 呈现时核对 DirectComposition、窗口创建标志和 Mailbox 设置，验证双窗口重绘下的内存与帧节奏。
- 外壳可见性通过平台模块处理。Windows 窗口样式、Linux X11/EWMH 和 macOS 应用激活策略具有不同作用范围；不能假定原生 Wayland 支持 X11 路径，也不能把 macOS 应用级 Dock 设置描述为独立窗口开关。
- Spine 仅处理可信本地 3.8 资源。修改 FFI 时检查分配与释放配对、指针有效期、错误传递和线程约束：包装实现 `Send`，不实现 `Sync`；载入锁保护 C 共享状态，不在每帧更新中增加全局锁。
- 修改 `vendor/egui-winit` 前先阅读 [PATCH.md](vendor/egui-winit/PATCH.md)。升级依赖时核对现有窗口创建补丁，变更补丁时同步说明；保留第三方许可证，不因升级覆盖本地必要改动。

## 构建与运行

在仓库根目录执行。当前要求 Rust 1.95 或更新的 stable 工具链；默认 feature 为 `ui`，它启用 `spine` 和原生界面依赖。`spine` 通过 `cc` 编译随仓库提供的 C 源码，需要 C 编译工具链；平台系统依赖见 README 和 CI。

```sh
# 默认运行原生 UI；日常使用与性能比较采用 release
cargo run
cargo run --release

# 无 UI、无 Spine 的命令行检查
cargo run --no-default-features --bin arkpets -- inspect-model assets/models.json
cargo run --no-default-features --bin arkpets -- simulate assets/models.json 120

# Windows 显式选择后端
cargo run --release -- --renderer dx12
cargo run --release -- --renderer opengl
```

`simulate` 只演算物理状态，不能用来验证骨骼渲染、窗口透明或桌面交互。

## 验证要求

按修改范围从快到慢验证。纯文档修改核对路径、参数和差异即可；代码修改先运行相关模块测试，再覆盖涉及的 feature。测试通常与实现放在同一 `.rs` 文件的测试模块中。

```sh
# 核心逻辑
cargo test --locked --no-default-features

# Spine 与真实样例资源
cargo test --locked --no-default-features --features spine --lib

# 默认 UI feature 下的完整目标与 CI 检查
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release --bin arkpets-ui
```

涉及 UI、透明合成、桌面物理或渲染后端时，按变更选择 Windows 实机回归。先构建 debug 可执行文件；验证脚本默认使用 `target/debug/arkpets-ui.exe`，已有其他构建时可通过 `-Executable` 指定。以下脚本会打开真实窗口，交互脚本会操作鼠标，应顺序执行。

```powershell
cargo build --locked --bin arkpets-ui

# 真实系统合成：修改透明窗口或第三方窗口补丁时验证两个后端
powershell -NoProfile -File docs/scripts/Verify-Desktop.ps1 -Renderer dx12
powershell -NoProfile -File docs/scripts/Verify-Desktop.ps1 -Renderer opengl

# PowerShell 7：同进程后端切换、桌宠恢复与退出
pwsh -NoProfile -File docs/scripts/Verify-RendererSwitch.ps1 -Renderer dx12 -DesktopPet

# PowerShell 7：任务栏可见性和自动/手动底部边界
pwsh -NoProfile -File docs/scripts/Verify-Interaction.ps1 -Renderer dx12 -DesktopSettings

# 动作设置、多屏开关与初始部署的实际操作和保存
pwsh -NoProfile -File docs/scripts/Verify-Interaction.ps1 -Renderer dx12 -BehaviorSettings
```

后端切换和桌面交互变更也应按影响范围使用 `-Renderer opengl` 复测。透明验证必须检查系统合成后能看到受控背景，仅有 GPU 截图 Alpha 通道不足以证明原生窗口透明。

性能改动使用 release 构建与 `docs/scripts/Measure-Performance.ps1`；固定机器、渲染器、模型、缩放、目标帧率和窗口场景。`frames.json` 记录回调间隔，`memory.json` 记录进程内存，不能据此声称屏幕实际 FPS 或显存占用。采样命令和压力测试参数见 README。

CI 定义 Windows、macOS、Linux 三个平台的检查；本机通过不等于其他平台通过，编译通过也不等于桌面功能已实机验证。交付时说明实际执行的检查、失败原因和未验证的平台或场景。

## 资源、配置与文档维护

- 资源查找顺序为 `ARKPETS_ASSETS`、可执行文件旁的 `assets/`、当前目录中的 `assets/`。模型清单中的相对资源路径以清单所在目录为基准，直接导入 `.skel` 使用同名 `.atlas`。
- 保留三个真实样例模型，Spine 与渲染改动用真实资源验证，不能用伪造网格或成功结果替代。资源下载、跨版本转换和其他未实现的兼容能力需要明确实现后才能声明支持。
- 界面偏好位于操作系统的应用用户目录，后端配置独立保存为 `bootstrap.json`。诊断使用隔离目录；Windows 开发构建的 `ARKPETS_CAPTURE_DIR` 可触发截图检查，不能覆盖正常用户偏好。
- 分发可执行文件时同时携带 `assets/`、`LICENSE`、`THIRD_PARTY_NOTICES.md` 和 Spine 许可证。更换第三方代码、字体或模型时同步核对来源与声明。
- 用户可见行为、运行命令或设置变化同步更新 README；状态所有权、渲染和平台边界变化更新架构说明；迁移范围变化更新迁移记录。本文件保留开发入口与约束，详细流程放在对应文档中。
