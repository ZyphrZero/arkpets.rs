# Local egui-winit 0.36.2 patch

Source: the crates.io `egui-winit` 0.36.2 release, licensed MIT OR Apache-2.0.
The license files are from the matching `emilk/egui` 0.36.2 tag.

The Windows creation patch exposes `set_windows_no_redirection_bitmap(bool)` and
passes its thread-local value to winit's `WindowAttributesExtWindows` when
creating windows. ArkPets enables this before `eframe::run_native` for DX12's
DirectComposition swapchains. OpenGL leaves it disabled. Other platforms are
unchanged.

The flag must be part of HWND creation. Applying it later with `SetWindowLongW`
fails with Windows error 87. Keeping it in winit's window attributes also
preserves it when changing topmost, mouse passthrough, visibility, or decorations.

Remove the Windows creation patch when upstream exposes an equivalent per-renderer window
creation option. Verify both renderers with `docs/scripts/Verify-Desktop.ps1`:
GPU screenshots alone do not detect an opaque HWND background.

`monitors::MonitorSnapshot` also exposes winit's monitor bounds and primary-display
flag through temporary egui Context data. `update_viewport_info` refreshes it at
most once per second, on the native event-loop thread. Coordinates stay in physical
pixels so the application can rebase its desktop state when viewport DPI changes.
This is needed because eframe's public viewport input only exposes the current
monitor's size, not all monitor origins. It does not create an additional event
loop or retain windows. Remove this part when equivalent monitor input is public
upstream; verify negative origins, screen gaps, hotplug and mixed DPI when changing it.
