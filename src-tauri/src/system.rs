//! The optional "System" section: CPU load per thread and in total, memory, and temperatures when the machine
//! tells them. CPU and memory come from sysinfo (it needs two samples for a load, so the System lives between
//! calls). Temperatures: sysinfo's sensors (Linux and macOS have them; Windows usually only for an elevated
//! process, so there it is often absent and the widget just shows none). An NVIDIA card (load, memory,
//! temperature) is read with nvidia-smi when it is installed, at most every few seconds.

use serde::Serialize;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use sysinfo::{Components, CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Gpu {
    pub name: String,
    pub load: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub temp: Option<f32>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    /// 0..100
    pub cpu: f32,
    /// one per logical CPU (thread), 0..100
    pub threads: Vec<f32>,
    /// one per physical core: the mean of its threads (SMT siblings are numbered next to each other); equals
    /// `threads` when the counts don't pair up
    pub cores: Vec<f32>,
    /// how many threads each entry of `cores` stands for (1 or 2)
    pub per_core: usize,
    /// bytes
    pub mem_used: u64,
    pub mem_total: u64,
    /// °C, the hottest CPU sensor — None when the system doesn't tell
    pub cpu_temp: Option<f32>,
    pub gpu: Option<Gpu>,
}

pub struct Sampler {
    sys: System,
    components: Option<Components>,
    gpu: Option<Gpu>,
    gpu_at: Option<Instant>,
    /// nvidia-smi isn't there (or failed): don't try again
    no_gpu: bool,
    /// physical cores (sysinfo computes it on every call, so once)
    physical: Option<usize>,
}

const GPU_EVERY: Duration = Duration::from_secs(4);

impl Sampler {
    pub fn new() -> Self {
        let sys = System::new_with_specifics(
            RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing().with_cpu_usage()).with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        let physical = sys.physical_core_count();
        Sampler { sys, components: None, gpu: None, gpu_at: None, no_gpu: false, physical }
    }

    pub fn sample(&mut self, gpu: bool) -> Stats {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        let threads: Vec<f32> = self.sys.cpus().iter().map(|c| c.cpu_usage().clamp(0.0, 100.0)).collect();
        // the sensor list is built once; later only their values are refreshed
        let components = self.components.get_or_insert_with(Components::new_with_refreshed_list);
        components.refresh(false);
        let cpu_temp = components
            .iter()
            .filter(|c| {
                let l = c.label().to_lowercase();
                l.contains("cpu") || l.contains("core") || l.contains("package") || l.contains("tctl") || l.contains("tdie")
                    || l.contains("acpi") || l.contains("thermal")
            })
            .filter_map(|c| c.temperature())
            .filter(|t| *t > 1.0 && *t < 125.0)
            .fold(None, |m: Option<f32>, t| Some(m.map_or(t, |m| m.max(t))));
        if gpu && !self.no_gpu && self.gpu_at.is_none_or(|at| at.elapsed() >= GPU_EVERY) {
            self.gpu_at = Some(Instant::now());
            match nvidia() {
                Some(g) => self.gpu = Some(g),
                None => {
                    self.no_gpu = true;
                    self.gpu = None;
                }
            }
        }
        let per_core = match self.physical {
            Some(p) if p > 0 && threads.len() == p * 2 => 2,
            _ => 1,
        };
        let cores = threads.chunks(per_core).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
        Stats {
            cpu: self.sys.global_cpu_usage().clamp(0.0, 100.0),
            threads,
            cores,
            per_core,
            mem_used: self.sys.used_memory(),
            mem_total: self.sys.total_memory(),
            cpu_temp,
            gpu: if gpu { self.gpu.clone() } else { None },
        }
    }
}

/// One NVIDIA card through nvidia-smi: "name, load %, memory used MiB, memory total MiB, temperature °C".
fn nvidia() -> Option<Gpu> {
    let mut cmd = Command::new("nvidia-smi");
    cmd.args(["--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu", "--format=csv,noheader,nounits"]);
    #[cfg(windows)]
    {
        // no console window flashing up every few seconds
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok().filter(|o| o.status.success())?;
    let line = String::from_utf8_lossy(&out.stdout).lines().next()?.to_string();
    let f: Vec<&str> = line.split(',').map(str::trim).collect();
    if f.len() < 5 {
        return None;
    }
    let mib = |s: &str| s.parse::<u64>().ok().map(|v| v * 1024 * 1024).unwrap_or(0);
    Some(Gpu {
        name: f[0].to_string(),
        load: f[1].parse().unwrap_or(0.0),
        mem_used: mib(f[2]),
        mem_total: mib(f[3]),
        temp: f[4].parse().ok(),
    })
}

pub type Shared = Mutex<Sampler>;
