# cosmic-applet-system-monitor

Applet for the [COSMIC desktop](https://github.com/pop-os/cosmic-epoch) panel
that displays CPU, GPU and RAM usage at a glance, with a detailed popup.

## Features

- **CPU** — usage percentage and package temperature
- **RAM** — used/total (GiB) and percentage
- **GPU** — usage and temperature, with **multi-GPU support**:
  - **NVIDIA** via [NVML] (real utilisation + temperature, including GPUs the
    proprietary driver does not expose through sysfs)
  - **AMD / Intel** via the kernel DRM/hwmon sysfs interface
    (`gpu_busy_percent`, hwmon `temp*_input`)
- Click the panel entry for a detailed popup with progress bars
- Refreshes every 2 seconds

On a hybrid system (e.g. an NVIDIA discrete GPU plus an AMD/Intel iGPU) **both
GPUs are listed**.

[NVML]: https://developer.nvidia.com/nvidia-management-library-nvml

## Requirements

- COSMIC desktop environment
- Rust toolchain (1.85+, edition 2024)
- Build deps: `libwayland`, `libxkbcommon`, `libudev`, `pkg-config`
- *(optional, runtime)* `libnvidia-ml` for NVIDIA GPU stats — absence is handled
  gracefully (NVIDIA cards simply fall back to sysfs temperature, or are hidden
  if they expose nothing)

## Build

```bash
cargo build --release
```

NVIDIA/NVML support is on by default. For a pure-sysfs build (no NVML link):

```bash
cargo build --release --no-default-features
```

## Install

Using [`just`](https://github.com/casey/just):

```bash
just install   # builds, installs binary + desktop entry (sudo)
```

Or manually:

```bash
sudo install -Dm0755 target/release/cosmic-applet-system-monitor /usr/bin/cosmic-applet-system-monitor
sudo install -Dm0644 resources/com.system76.CosmicAppletSystemMonitor.desktop \
  /usr/share/applications/com.system76.CosmicAppletSystemMonitor.desktop
```

Then add the applet to the panel via **Settings → Desktop → Panel → Applets**,
or edit `~/.config/cosmic/com.system76.CosmicPanel.Panel/v1/plugins_wings` and
restart the panel.

## Architecture

The metrics collector ([`SystemMonitor`](src/monitor.rs)) is created **once** and
reused for the applet's lifetime:

- **CPU** — `sysinfo` computes load from the delta between consecutive refreshes,
  so the `System` handle must persist across ticks (recreating it per tick yields
  bogus near-zero readings).
- **GPU** — sysfs paths and NVML handles are resolved at startup, not per refresh.

Sampling runs on a blocking task (`spawn_blocking`) behind an `Arc<Mutex<…>>`, so
the synchronous filesystem/NVML reads never block the UI event loop.

## Development

```bash
just check   # fmt --check + clippy -D warnings + tests
```

## License

GPL-3.0-only — see [LICENSE](LICENSE).
