# 第三方来源

- 原项目 Ark-Pets：Copyright (c) 2022-2026 Harry Huang；GPL-3.0。根目录 `LICENSE` 保留原项目许可证。本项目参考其架构、模型数据结构与行为接口。
- 描边与阴影的采样核改编自 Ark-Pets 的 `ComplexFragment.glsl`、`ComplexFragmentLow.glsl`：Copyright (c) 2022-2026 Harry Huang、Litwak913，GPL-3.0；Rust 的 WGSL／GLSL 实现改为预乘 Alpha 合成。动画边界 Alpha 权重与物理加速度语义亦依据 Ark-Pets 的对应实现迁移。
- ArkPets-Web：行为参数与样例模型来自本地 Web 项目。请参阅其项目来源 https://github.com/fuyufjh/ArkPets-Web 。
- Spine C Runtime 3.8：Copyright (c) Esoteric Software LLC。源代码固定在提交 `8b4844bd4b193ba9e54487ed397a777993cbad56`；见 `vendor/spine-c/LICENSE` 与各源文件版权头。Spine 运行时适用其自身许可证，不能仅以本项目 GPL 标注代替其条款。
- 思源黑体 Source Han Sans CN：Adobe 与相关字体作者，SIL Open Font License；字体来自原项目，见 `assets/fonts/LICENSE.txt`。
- `assets/models` 中明日方舟角色素材归鹰角网络及相关权利人所有，随附资源用于原模型兼容性验证，不转授素材版权。
- 阿米娅、史尔特尔、缄默德克萨斯、逻各斯的基建和正面战斗资源来自 [PuppiizSunniiz/Arknights-Spines](https://github.com/PuppiizSunniiz/Arknights-Spines/tree/ebc0d09b72c7543c0999355c7826ed3f9d435ac6/spineassets/character)，固定提交 `ebc0d09b72c7543c0999355c7826ed3f9d435ac6`。文件按原样保留，并核对上游 Git blob SHA-1 和大小；各目录 `SOURCE.md` 记录 SHA-256。自动基建动作权重参考 Ark-Pets 的 `StochasticMatrix.java`（Harry Huang、Litwak913，GPL-3.0），动作类型与循环修饰段规则参考其 `AnimClip.java` / `AnimClipGroup.java`。
- 荒芜拉普兰德正面战斗资源 `assets/models/1038_whitw2_battle` 来自 [PuppiizSunniiz/Arknights-Spines](https://github.com/PuppiizSunniiz/Arknights-Spines/tree/ebc0d09b72c7543c0999355c7826ed3f9d435ac6/spineassets/character/char_1038_whitw2/front)，固定提交 `ebc0d09b72c7543c0999355c7826ed3f9d435ac6`，保留原始 `.skel`、`.atlas` 和 `.png` 文件。素材版权仍归鹰角网络及相关权利人，不适用本项目代码的 GPL 授权；来源和校验值见该资源目录的 `SOURCE.md`。
- eframe / egui / wgpu / winit 与其他 Cargo 依赖的许可详见各 crate 的发布元数据。Phosphor 图标通过 `egui-phosphor` 字体提供。
- `vendor/egui-winit` 基于 egui-winit 0.36.2，MIT OR Apache-2.0；保留上游许可证，并补充 Windows DirectComposition 窗口创建配置。变更记录见 `vendor/egui-winit/PATCH.md`。

Spine 源码上游：https://github.com/EsotericSoftware/spine-runtimes/tree/8b4844bd4b193ba9e54487ed397a777993cbad56/spine-c
