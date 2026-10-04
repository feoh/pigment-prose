# Pigment Prose for Linux (x86_64)

A per-user package: two programs, a menu entry, an icon, and the license, notices and user guide. It installs into your home directory without root and uninstalls cleanly.

## Requirements

- **x86_64 Linux with glibc 2.44 or newer** (a current rolling distribution; `BUILD-INFO.txt` states the exact minimum for this build). Tested on CachyOS (Arch-based) only.
- **A hardware GPU with a Vulkan driver** and the Vulkan loader (`libvulkan.so.1`). Tested on NVIDIA GeForce RTX 4070 Ti with the NVIDIA 615.71.09 driver, and an Intel Raptor Lake-S iGPU with Mesa. CPU-only rendering is not supported.
- **A Wayland or X11 desktop.** Tested on KDE Plasma (Wayland). The window uses `libwayland-client` and `libxkbcommon` on Wayland, or `libX11`, `libxcb`, `libXcursor` and `libXi` on X11.
- **For the Open, Save and Export dialogs:** a running XDG desktop portal with a backend for your desktop (for example `xdg-desktop-portal-kde` or `-gnome`) and D-Bus. Without one the dialogs cannot open and behave as cancelled; the `pigment-prose` command-line export still works.

On Arch-based systems these are `vulkan-icd-loader`, your GPU's Vulkan driver (`nvidia-utils`, or `vulkan-intel`/`vulkan-radeon`), `wayland`, `libxkbcommon`, `libx11`, `libxcursor`, `libxi`, `libglvnd`, `dbus` and `xdg-desktop-portal` with your desktop's backend. Other distributions have not been tried.

## Install

```sh
tar --zstd -xf pigment-prose-0.2.0-x86_64-linux.tar.zst
cd pigment-prose-0.2.0-x86_64-linux
./install.sh                 # or: ./install.sh --prefix /some/dir
```

This puts `pigment-studio` and `pigment-prose` in `~/.local/bin`, the menu entry and icon under `~/.local/share`, and the documents in `~/.local/share/doc/pigment-prose/`. Installing again replaces the previous install. Start **Pigment Prose** from your application menu, or run `pigment-studio`.

## Uninstall

```sh
~/.local/share/pigment-prose/uninstall.sh
```

It removes exactly the files the installer listed. Your recipes and exported images are never touched: the program keeps no settings or files of its own.

## Documents

- `share/doc/pigment-prose/user-guide.md`: how to use it.
- `share/doc/pigment-prose/LICENSE`: Pigment Prose is MIT-licensed.
- `share/doc/pigment-prose/THIRD-PARTY-NOTICES.md`: the third-party code and fonts in the programs and their licenses. They concern the software only; images you make carry no watermark or attribution.
- `share/doc/pigment-prose/BUILD-INFO.txt`: the source commit, toolchain and dependency lock this build came from.
