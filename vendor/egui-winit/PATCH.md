# Local egui-winit 0.36.2 patch

Source: the crates.io `egui-winit` 0.36.2 release, licensed MIT OR Apache-2.0.
The license files are from the matching `emilk/egui` 0.36.2 tag.

The only source change exposes `set_windows_no_redirection_bitmap(bool)` and
passes its thread-local value to winit's `WindowAttributesExtWindows` when
creating windows. ArkPets enables this before `eframe::run_native` for DX12's
DirectComposition swapchains. OpenGL leaves it disabled. Other platforms are
unchanged.

The flag must be part of HWND creation. Applying it later with `SetWindowLongW`
fails with Windows error 87. Keeping it in winit's window attributes also
preserves it when changing topmost, mouse passthrough, visibility, or decorations.

Remove this patch when upstream exposes an equivalent per-renderer window
creation option. Verify both renderers with `docs/scripts/Verify-Desktop.ps1`:
GPU screenshots alone do not detect an opaque HWND background.
