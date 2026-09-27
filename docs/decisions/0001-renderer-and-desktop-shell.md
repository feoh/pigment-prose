# ADR 0001: Rust + wgpu renderer, eframe/egui desktop shell

**Status:** accepted 2026-09-26 (task 03). **Supersedes:** nothing. **Revisit if:** task 11 cannot meet the preview targets in [architecture.md](../architecture.md#responsiveness-targets-provisional), or task 21/22 finds a Direct3D 12 or Metal blocker.

## Context

Task 02 ([architecture spike](../architecture-spike.md)) showed on the Linux RTX 4070 Ti (Vulkan) that Rust + wgpu 30 compute painting over a 2.5D layered scene is feasible. Its tiled output was bit-identical to single-image output up to 16K, and an 8K export needed 27.4 MiB of renderer allocations. That meets the GPU and tiling requirements, so this ADR adopts the stack. It then chooses **one** desktop shell.

The shell must:

1. **Show GPU output** without a second graphics stack, ideally sharing the renderer's wgpu device.
2. **Accept asynchronous updates** from a render worker without blocking input.
3. **Open native file dialogs** for open, save and export.
4. **Expose accessibility** to screen readers: labelled controls, keyboard focus, value text.
5. **Package** for Linux first, then Windows and macOS.
6. Carry **no license terms** that conflict with the brief (no mandated attribution in outputs, no copyleft surprise).

## Options evaluated (versions current on crates.io, 2026-09-26)

| | eframe/egui 0.36.2 | iced 0.14.0 | Slint 1.18.1 | Tauri 2.12.0 |
| --- | --- | --- | --- | --- |
| wgpu version | `egui-wgpu` depends on **wgpu ^30.0**, the renderer's version | `iced_wgpu` depends on wgpu ^27.0 | optional wgpu ^30 / ^29 integration | none (system webview) |
| GPU texture sharing | `WgpuSetup::Existing` accepts our instance/adapter/device/queue; `Renderer::register_native_texture` shows a renderer-owned texture | would need a second wgpu 27 device or a downgrade | possible via its wgpu integration | no in-webview wgpu texture; needs readback + IPC image transfer or a separate native surface |
| Async updates | `egui::Context` is `Send + Sync`; a worker calls `request_repaint()` | message/subscription model | event loop + `invoke_from_event_loop` | JS events over IPC |
| Native dialogs | via `rfd` 0.17.2 (MIT) | via `rfd` | via `rfd` | built-in dialog plugin |
| Accessibility | AccessKit enabled by default (`accesskit` in eframe's default features) | no AccessKit dependency in iced 0.14's manifest | AccessKit via its winit backend | the webview's accessibility |
| Packaging | plain Rust binary; packager chosen in task 15 | plain binary | plain binary | mature bundler |
| License | MIT OR Apache-2.0 | MIT | GPL-3.0-only, royalty-free (requires attribution) or commercial | MIT OR Apache-2.0 |

Checked directly: the wgpu requirements (crates.io dependency metadata), eframe's default features and the `WgpuSetup::Existing` and `register_native_texture` APIs (the published `egui-wgpu` 0.36.2 and `eframe` 0.36.2 sources), iced 0.14's manifest, and Slint's license string. Tauri's texture-sharing limitation follows from its webview architecture and was not prototyped.

## Decision

- **Renderer:** Rust (edition 2024, toolchain pinned to 1.98) + **wgpu 30.0.1**. WebGPU core features only, device opened with `wgpu::Limits::default()`, WGSL compute passes evaluated per tile in whole-image coordinates. Backends: Vulkan (Linux, verified), Direct3D 12 (Windows, unverified), Metal (macOS, unverified).
- **Shell (task 11):** **eframe/egui 0.36** with the wgpu backend, sharing one `Arc<GpuContext>` through `egui_wgpu::WgpuSetup::Existing`. Native dialogs come from **rfd**. No second UI framework.
- **Preview display:** the MVP reads the preview tile back through the same `TileSink` path as export and uploads it as an egui texture. Preview and export then share one code path, and the spike's preview timings include readback (2.16 ms at 1280×720). Zero-copy display through `register_native_texture` stays available, because the shell and renderer share a wgpu version. Adopt it only if task 11 measures upload as a bottleneck.

## Consequences

- One wgpu version for UI and renderer. **Upgrading wgpu requires an egui release on the same major.** Pin both together.
- egui 0.36 needs Rust ≥ 1.95. The workspace pins 1.98 in `rust-toolchain.toml`.
- Immediate-mode UI repaints on demand. The render worker must call `request_repaint()` when a result arrives. An idle UI does not spin.
- A device loss affects the UI and the renderer together, because they share a device. Task 11 must handle `RenderError::DeviceLost` by recreating the context, or by restarting with a clear message.
- egui widgets have egui's own look, not the native toolkit's. That's acceptable for a painting studio. Screen-reader quality with Orca (Linux), Narrator (Windows) and VoiceOver (macOS) has **not** been tested, and task 11 must check it.
- Slint was not chosen: its free licenses are GPL-3.0 or royalty-free with attribution, and its wgpu integration is optional. Tauri was not chosen because showing GPU frames needs readback and IPC. iced was not chosen because it is on wgpu 27 and lacks AccessKit in 0.14.

## Unverified at the time of this decision

No shell code exists yet. Direct3D 12 and Metal are unverified, as are packaging on any OS, rfd's Linux backend on the user's desktop session, and the accessibility behaviour listed above.
