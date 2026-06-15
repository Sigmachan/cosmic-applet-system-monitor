use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Default)]
pub struct SystemStats {
    pub cpu_percent: f32,
    pub cpu_temp: Option<f32>,
    pub ram_used_gb: f32,
    pub ram_total_gb: f32,
    pub ram_percent: f32,
    pub gpus: Vec<GpuStats>,
}

#[derive(Debug, Clone)]
pub struct GpuStats {
    pub name: String,
    pub usage_percent: f32,
    pub temp: Option<f32>,
}

pub struct SystemMonitor {
    sys: sysinfo::System,
    gpu_readers: Vec<GpuReader>,
}

#[derive(Debug, Clone)]
struct GpuReader {
    name: String,
    usage_path: Option<PathBuf>,
    temp_path: Option<PathBuf>,
}

const DRM_PATH: &str = "/sys/class/drm";

impl GpuReader {
    fn discover() -> Vec<Self> {
        let mut readers = Vec::new();

        let Ok(entries) = fs::read_dir(DRM_PATH) else {
            return readers;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.starts_with("card") {
                continue;
            }
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            if !meta.is_dir() {
                continue;
            }

            let device = path.join("device");
            if !device.is_dir() {
                continue;
            }

            let usage_path = Self::find_usage_file(&device);
            let temp_path = Self::find_temp_file(&device);
            let vendor = Self::vendor_name(&device).unwrap_or_else(|| "GPU".to_string());

            if usage_path.is_some() || temp_path.is_some() {
                readers.push(Self {
                    name: format!("{vendor} ({name})"),
                    usage_path,
                    temp_path,
                });
            }
        }

        readers.sort_by(|a, b| a.name.cmp(&b.name));
        readers.dedup_by(|a, b| a.usage_path == b.usage_path);
        readers
    }

    fn vendor_name(device: &std::path::Path) -> Option<String> {
        let vendor = fs::read_to_string(device.join("vendor")).ok()?;
        match vendor.trim() {
            "0x10de" => Some("NVIDIA".into()),
            "0x1002" => Some("AMD".into()),
            "0x8086" => Some("Intel".into()),
            _ => None,
        }
    }

    fn find_usage_file(device: &std::path::Path) -> Option<PathBuf> {
        let candidate = device.join("gpu_busy_percent");
        if candidate.exists() {
            return Some(candidate);
        }
        None
    }

    fn find_temp_file(device: &std::path::Path) -> Option<PathBuf> {
        let hwmon_dirs = [
            device.join("hwmon/hwmon0"),
            device.join("hwmon/hwmon1"),
            device.join("hwmon/hwmon2"),
            device.join("hwmon/hwmon3"),
        ];

        for hwmon_base in &hwmon_dirs {
            if !hwmon_base.is_dir() {
                continue;
            }
            let Ok(entries) = fs::read_dir(hwmon_base) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if (name_str.starts_with("temp") && name_str.ends_with("_input"))
                    || name_str == "temp1_input"
                {
                    return Some(entry.path());
                }
            }
        }

        let temp_input = device.join("temp1_input");
        if temp_input.exists() {
            return Some(temp_input);
        }

        None
    }

    fn read_usage(&self) -> Option<f32> {
        let path = self.usage_path.as_ref()?;
        let raw = fs::read_to_string(path).ok()?;
        raw.trim().parse::<f32>().ok()
    }

    fn read_temp(&self) -> Option<f32> {
        let path = self.temp_path.as_ref()?;
        let raw = fs::read_to_string(path).ok()?;
        let millicelsius: f32 = raw.trim().parse().ok()?;
        Some(millicelsius / 1000.0)
    }
}

impl SystemMonitor {
    pub fn new() -> Self {
        let mut sys = sysinfo::System::new_all();
        sys.refresh_all();

        let gpu_readers = GpuReader::discover();
        tracing::info!(
            gpu_count = gpu_readers.len(),
            "GPU readers discovered"
        );

        Self { sys, gpu_readers }
    }

    pub fn stats(&mut self) -> SystemStats {
        self.sys.refresh_cpu_all();
        self.sys.refresh_memory();

        let cpu_percent = self.sys.global_cpu_usage();

        let total = self.sys.total_memory();
        let used = self.sys.used_memory();
        let ram_total_gb = total as f32 / 1024.0 / 1024.0 / 1024.0;
        let ram_used_gb = used as f32 / 1024.0 / 1024.0 / 1024.0;
        let ram_percent = if total > 0 {
            (used as f32 / total as f32) * 100.0
        } else {
            0.0
        };

        let cpu_temp = Self::read_cpu_temp();

        let gpus: Vec<GpuStats> = self
            .gpu_readers
            .iter()
            .map(|r| GpuStats {
                name: r.name.clone(),
                usage_percent: r.read_usage().unwrap_or(0.0),
                temp: r.read_temp(),
            })
            .collect();

        SystemStats {
            cpu_percent,
            cpu_temp,
            ram_used_gb,
            ram_total_gb,
            ram_percent,
            gpus,
        }
    }

    fn read_cpu_temp() -> Option<f32> {
        let Ok(entries) = fs::read_dir("/sys/class/hwmon") else {
            return None;
        };

        for entry in entries.flatten() {
            let Ok(name) = fs::read_to_string(entry.path().join("name")) else {
                continue;
            };
            let name = name.trim().to_lowercase();
            if !(name.contains("coretemp") || name.contains("k10temp") || name.contains("cpu")) {
                continue;
            }

            for i in 1..=10 {
                let Ok(label) =
                    fs::read_to_string(entry.path().join(format!("temp{i}_label")))
                else {
                    continue;
                };
                let label = label.trim().to_lowercase();
                if label.contains("package") || label.contains("tdie") || label.contains("tctl") {
                    let Ok(temp_str) =
                        fs::read_to_string(entry.path().join(format!("temp{i}_input")))
                    else {
                        continue;
                    };
                    if let Ok(temp) = temp_str.trim().parse::<f32>() {
                        return Some(temp / 1000.0);
                    }
                }
            }

            let input = entry.path().join("temp1_input");
            if let Ok(temp_str) = fs::read_to_string(&input) {
                if let Ok(temp) = temp_str.trim().parse::<f32>() {
                    return Some(temp / 1000.0);
                }
            }
        }

        None
    }
}
