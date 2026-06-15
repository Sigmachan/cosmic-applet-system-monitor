# cosmic-applet-system-monitor

Applet for the [COSMIC desktop](https://github.com/pop-os/cosmic-epoch) panel that displays CPU, GPU load and RAM usage.

## Features

- CPU usage percentage and temperature
- RAM used/total (GB) and percentage
- GPU usage and temperature (NVIDIA, AMD, Intel via DRM sysfs)
- Click to open detailed popup with progress bars
- Updates every 2 seconds

## Requirements

- COSMIC desktop environment
- Rust toolchain (rustc 1.96+, cargo)

## Build

```bash
cargo build --release
```

## Install

```bash
sudo cp target/release/cosmic-applet-system-monitor /usr/bin/
sudo cp resources/com.system76.CosmicAppletSystemMonitor.desktop /usr/share/applications/
```

Then add `com.system76.CosmicAppletSystemMonitor` to the panel config at
`~/.config/cosmic/com.system76.CosmicPanel.Panel/v1/plugins_wings`
and restart the panel:

```bash
pkill cosmic-panel && cosmic-panel &
```

## How it works

- **CPU**: load via `sysinfo::System::global_cpu_usage()`, temperature via `/sys/class/hwmon`
- **RAM**: via `sysinfo::System` (used/total memory)
- **GPU**: scans `/sys/class/drm/card*/device/` for `gpu_busy_percent` and `temp*_input` (hwmon). Vendor detected by PCI ID.

## License

GPL-3.0-only
