//! System metrics collection.
//!
//! [`SystemMonitor`] is created **once** and reused for the lifetime of the
//! applet. This is important for two reasons:
//!
//! * `sysinfo` computes CPU load from the delta between two consecutive
//!   refreshes, so the [`sysinfo::System`] must persist across ticks.
//! * GPU discovery (sysfs scan + NVML init) is comparatively expensive and only
//!   needs to happen at startup.

use std::{fs, path::PathBuf};

/// A single snapshot of system metrics, produced by [`SystemMonitor::stats`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SystemStats {
    pub cpu_percent: f32,
    pub cpu_temp: Option<f32>,
    pub ram_used_gb: f32,
    pub ram_total_gb: f32,
    pub ram_percent: f32,
    pub gpus: Vec<GpuStats>,
}

/// Per-GPU metrics. `usage_percent`/`temp` are `None` when the backend cannot
/// report them, so the UI can distinguish "0%" from "unknown".
#[derive(Debug, Clone, PartialEq)]
pub struct GpuStats {
    pub name: String,
    pub usage_percent: Option<f32>,
    pub temp: Option<f32>,
}

const DRM_PATH: &str = "/sys/class/drm";
const HWMON_PATH: &str = "/sys/class/hwmon";
const BYTES_PER_GIB: f32 = 1024.0 * 1024.0 * 1024.0;

/// PCI vendor IDs as exposed by `/sys/class/drm/card*/device/vendor`.
const VENDOR_NVIDIA: &str = "0x10de";
const VENDOR_AMD: &str = "0x1002";
const VENDOR_INTEL: &str = "0x8086";

/// Maps a PCI vendor id to a human-readable name, if known.
fn vendor_label(vendor_id: &str) -> Option<&'static str> {
    match vendor_id.trim() {
        VENDOR_NVIDIA => Some("NVIDIA"),
        VENDOR_AMD => Some("AMD"),
        VENDOR_INTEL => Some("Intel"),
        _ => None,
    }
}

/// Parses a millidegree-Celsius hwmon value (e.g. `"45000"`) into Celsius.
fn parse_millicelsius(raw: &str) -> Option<f32> {
    raw.trim().parse::<f32>().ok().map(|m| m / 1000.0)
}

/// Computes `used / total` as a percentage, guarding against divide-by-zero.
fn percent_of(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (used as f32 / total as f32) * 100.0
    }
}

// ---------------------------------------------------------------------------
// GPU backends
// ---------------------------------------------------------------------------

/// A GPU exposed through the kernel DRM/hwmon sysfs interface (AMD, Intel, and
/// nouveau). Paths are resolved once during discovery.
#[derive(Debug, Clone)]
struct SysfsGpu {
    name: String,
    usage_path: Option<PathBuf>,
    temp_path: Option<PathBuf>,
}

impl SysfsGpu {
    /// Scans `/sys/class/drm` for GPUs. When `skip_nvidia` is set, NVIDIA cards
    /// are ignored here because they are handled by NVML (which reports usage
    /// that the proprietary driver does not expose via sysfs).
    fn discover(skip_nvidia: bool) -> Vec<Self> {
        let mut readers = Vec::new();

        let Ok(entries) = fs::read_dir(DRM_PATH) else {
            return readers;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // Match the primary node `cardN`, not render nodes or connectors.
            if !name.starts_with("card") || name.contains('-') {
                continue;
            }

            let device = path.join("device");
            if !device.is_dir() {
                continue;
            }

            let vendor_id = fs::read_to_string(device.join("vendor")).unwrap_or_default();
            if skip_nvidia && vendor_id.trim() == VENDOR_NVIDIA {
                continue;
            }
            let vendor = vendor_label(&vendor_id).unwrap_or("GPU");

            let usage_path = Self::find_usage_file(&device);
            let temp_path = Self::find_temp_file(&device);

            if usage_path.is_some() || temp_path.is_some() {
                readers.push(Self {
                    name: format!("{vendor} ({name})"),
                    usage_path,
                    temp_path,
                });
            }
        }

        readers.sort_by(|a, b| a.name.cmp(&b.name));
        readers.dedup_by(|a, b| a.usage_path == b.usage_path && a.usage_path.is_some());
        readers
    }

    fn find_usage_file(device: &std::path::Path) -> Option<PathBuf> {
        // amdgpu/i915 expose instantaneous busy percentage here.
        let candidate = device.join("gpu_busy_percent");
        candidate.exists().then_some(candidate)
    }

    /// Finds a temperature input under the device's hwmon node, preferring an
    /// edge/junction sensor when labelled.
    fn find_temp_file(device: &std::path::Path) -> Option<PathBuf> {
        let hwmon_root = device.join("hwmon");
        if let Ok(hwmon_entries) = fs::read_dir(&hwmon_root) {
            for hwmon in hwmon_entries.flatten() {
                let base = hwmon.path();
                // Prefer a labelled sensor (edge/junction/temp1) when present.
                let mut fallback = None;
                let Ok(entries) = fs::read_dir(&base) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let fname = entry.file_name();
                    let fname = fname.to_string_lossy();
                    if fname.starts_with("temp") && fname.ends_with("_input") {
                        fallback.get_or_insert_with(|| entry.path());
                    }
                }
                if let Some(path) = fallback {
                    return Some(path);
                }
            }
        }

        let temp_input = device.join("temp1_input");
        temp_input.exists().then_some(temp_input)
    }

    fn read_usage(&self) -> Option<f32> {
        let raw = fs::read_to_string(self.usage_path.as_ref()?).ok()?;
        raw.trim().parse::<f32>().ok()
    }

    fn read_temp(&self) -> Option<f32> {
        let raw = fs::read_to_string(self.temp_path.as_ref()?).ok()?;
        parse_millicelsius(&raw)
    }

    fn stats(&self) -> GpuStats {
        GpuStats {
            name: self.name.clone(),
            usage_percent: self.read_usage(),
            temp: self.read_temp(),
        }
    }
}

/// NVIDIA GPUs queried through NVML. The [`Nvml`](nvml_wrapper::Nvml) handle is
/// kept alive for the monitor's lifetime; devices are looked up by index on
/// each refresh (cheap) to avoid self-referential lifetimes.
#[cfg(feature = "nvidia")]
struct NvmlGpus {
    nvml: nvml_wrapper::Nvml,
    /// Cached `(index, display name)` pairs discovered at startup.
    devices: Vec<(u32, String)>,
}

#[cfg(feature = "nvidia")]
impl NvmlGpus {
    fn discover() -> Option<Self> {
        let nvml = match nvml_wrapper::Nvml::init() {
            Ok(nvml) => nvml,
            Err(err) => {
                tracing::debug!(%err, "NVML unavailable; NVIDIA GPUs via sysfs only");
                return None;
            }
        };

        let count = nvml.device_count().unwrap_or(0);
        let mut devices = Vec::new();
        for index in 0..count {
            // NVML names already include the brand (e.g. "NVIDIA GeForce RTX
            // 5090"), so only prefix when they don't to avoid "NVIDIA NVIDIA …".
            let name = nvml
                .device_by_index(index)
                .and_then(|d| d.name())
                .map(|n| {
                    if n.to_ascii_lowercase().contains("nvidia") {
                        n
                    } else {
                        format!("NVIDIA {n}")
                    }
                })
                .unwrap_or_else(|_| format!("NVIDIA #{index}"));
            devices.push((index, name));
        }

        if devices.is_empty() {
            return None;
        }
        Some(Self { nvml, devices })
    }

    fn stats(&self) -> Vec<GpuStats> {
        use nvml_wrapper::enum_wrappers::device::TemperatureSensor;

        self.devices
            .iter()
            .map(|(index, name)| {
                let device = self.nvml.device_by_index(*index).ok();
                let usage_percent = device
                    .as_ref()
                    .and_then(|d| d.utilization_rates().ok())
                    .map(|u| u.gpu as f32);
                let temp = device
                    .as_ref()
                    .and_then(|d| d.temperature(TemperatureSensor::Gpu).ok())
                    .map(|t| t as f32);
                GpuStats {
                    name: name.clone(),
                    usage_percent,
                    temp,
                }
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// CPU temperature sensor
// ---------------------------------------------------------------------------

/// Resolved CPU temperature sensor path, discovered once at startup.
#[derive(Debug, Clone, Default)]
struct CpuTempSensor {
    path: Option<PathBuf>,
}

impl CpuTempSensor {
    fn discover() -> Self {
        Self {
            path: Self::find_sensor(),
        }
    }

    fn find_sensor() -> Option<PathBuf> {
        let entries = fs::read_dir(HWMON_PATH).ok()?;
        for entry in entries.flatten() {
            let dir = entry.path();
            let Ok(chip) = fs::read_to_string(dir.join("name")) else {
                continue;
            };
            let chip = chip.trim().to_lowercase();
            if !(chip.contains("coretemp") || chip.contains("k10temp") || chip.contains("cpu")) {
                continue;
            }

            // Prefer the package/Tdie/Tctl sensor when labelled.
            for i in 1..=16 {
                let Ok(label) = fs::read_to_string(dir.join(format!("temp{i}_label"))) else {
                    continue;
                };
                let label = label.trim().to_lowercase();
                if label.contains("package") || label.contains("tdie") || label.contains("tctl") {
                    let input = dir.join(format!("temp{i}_input"));
                    if input.exists() {
                        return Some(input);
                    }
                }
            }

            let input = dir.join("temp1_input");
            if input.exists() {
                return Some(input);
            }
        }
        None
    }

    fn read(&self) -> Option<f32> {
        let raw = fs::read_to_string(self.path.as_ref()?).ok()?;
        parse_millicelsius(&raw)
    }
}

// ---------------------------------------------------------------------------
// Monitor
// ---------------------------------------------------------------------------

/// Long-lived collector of system metrics. Construct once via [`new`]; call
/// [`stats`] on each refresh tick.
///
/// [`new`]: SystemMonitor::new
/// [`stats`]: SystemMonitor::stats
pub struct SystemMonitor {
    sys: sysinfo::System,
    cpu_temp: CpuTempSensor,
    sysfs_gpus: Vec<SysfsGpu>,
    #[cfg(feature = "nvidia")]
    nvml_gpus: Option<NvmlGpus>,
}

impl SystemMonitor {
    pub fn new() -> Self {
        let mut sys = sysinfo::System::new();
        // Prime the CPU counters; the first real load value is computed on the
        // next refresh in `stats()`.
        sys.refresh_cpu_usage();
        sys.refresh_memory();

        #[cfg(feature = "nvidia")]
        let nvml_gpus = NvmlGpus::discover();
        #[cfg(feature = "nvidia")]
        let skip_nvidia = nvml_gpus.is_some();
        #[cfg(not(feature = "nvidia"))]
        let skip_nvidia = false;

        let sysfs_gpus = SysfsGpu::discover(skip_nvidia);
        let cpu_temp = CpuTempSensor::discover();

        #[cfg(feature = "nvidia")]
        tracing::info!(
            sysfs_gpus = sysfs_gpus.len(),
            nvml_gpus = nvml_gpus.as_ref().map_or(0, |g| g.devices.len()),
            "GPU backends discovered"
        );
        #[cfg(not(feature = "nvidia"))]
        tracing::info!(sysfs_gpus = sysfs_gpus.len(), "GPU backends discovered");

        Self {
            sys,
            cpu_temp,
            sysfs_gpus,
            #[cfg(feature = "nvidia")]
            nvml_gpus,
        }
    }

    pub fn stats(&mut self) -> SystemStats {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();

        let cpu_percent = self.sys.global_cpu_usage();

        let total = self.sys.total_memory();
        let used = self.sys.used_memory();

        let mut gpus: Vec<GpuStats> = self.sysfs_gpus.iter().map(SysfsGpu::stats).collect();
        #[cfg(feature = "nvidia")]
        if let Some(nvml) = &self.nvml_gpus {
            gpus.extend(nvml.stats());
        }
        gpus.sort_by(|a, b| a.name.cmp(&b.name));

        SystemStats {
            cpu_percent,
            cpu_temp: self.cpu_temp.read(),
            ram_used_gb: used as f32 / BYTES_PER_GIB,
            ram_total_gb: total as f32 / BYTES_PER_GIB,
            ram_percent: percent_of(used, total),
            gpus,
        }
    }
}

impl Default for SystemMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_label_maps_known_ids() {
        assert_eq!(vendor_label("0x10de"), Some("NVIDIA"));
        assert_eq!(vendor_label("0x1002"), Some("AMD"));
        assert_eq!(vendor_label("0x8086"), Some("Intel"));
        assert_eq!(vendor_label("0x10de\n"), Some("NVIDIA")); // trailing newline
        assert_eq!(vendor_label("0xdead"), None);
    }

    #[test]
    fn parse_millicelsius_converts_and_trims() {
        assert_eq!(parse_millicelsius("45000"), Some(45.0));
        assert_eq!(parse_millicelsius("  60500\n"), Some(60.5));
        assert_eq!(parse_millicelsius("not-a-number"), None);
        assert_eq!(parse_millicelsius(""), None);
    }

    #[test]
    fn percent_of_guards_zero_total() {
        assert_eq!(percent_of(0, 0), 0.0);
        assert_eq!(percent_of(50, 100), 50.0);
        assert_eq!(percent_of(100, 100), 100.0);
    }

    #[test]
    fn discover_does_not_panic_on_real_system() {
        // Exercises the sysfs scan against whatever hardware runs the tests.
        let _ = SysfsGpu::discover(false);
        let _ = CpuTempSensor::discover();
    }
}
