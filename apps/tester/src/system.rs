//! `system.json`: what a run's numbers need to be read: the tester's build, the OS, the CPU,
//! the RAM, the displays and every adapter `wgpu` enumerates on every backend, with the limits
//! the fly-through asks of it.
//!
//! Nothing that names the person or the machine is written: no user name, host name, path or
//! serial number. The OS readers are Linux's `/proc` and `/etc/os-release` and Windows'
//! `RtlGetVersion`, the registry's processor name, `GetSystemInfo` and `GlobalMemoryStatusEx`.

use std::fmt::Write as _;

use flythrough::report::write_json_string;
use vxe_app::wgpu;

/// The pool a streamed fly-through asks one storage binding and one buffer to hold
/// (`STREAM_POOL_BYTES` in the fly-through, 2 GiB); an adapter offering less spreads the pool
/// over shards.
pub const STREAM_POOL_BYTES: u64 = 2 << 30;

/// One adapter as `wgpu` reports it, with the limits the fly-through reads.
#[derive(Clone, Debug, PartialEq)]
pub struct AdapterRow {
    pub name: String,
    pub vendor: u32,
    pub device: u32,
    pub device_type: String,
    pub driver: String,
    pub driver_info: String,
    pub backend: String,
    pub max_buffer_size: u64,
    pub max_storage_buffer_binding_size: u64,
    pub max_storage_buffers_per_shader_stage: u32,
    /// The renderer's floor (`vxe_render::limits_within`) is met: the fly-through can start.
    pub meets_floor: bool,
    /// One binding holds the streamed pool whole, without shards.
    pub holds_pool: bool,
}

impl AdapterRow {
    /// The row of an adapter offering `limits`.
    pub fn new(info: &wgpu::AdapterInfo, limits: &wgpu::Limits) -> Self {
        Self {
            name: info.name.clone(),
            vendor: info.vendor,
            device: info.device,
            device_type: format!("{:?}", info.device_type),
            driver: info.driver.clone(),
            driver_info: info.driver_info.clone(),
            backend: vxe_app::backend_label(info.backend),
            max_buffer_size: limits.max_buffer_size,
            max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
            max_storage_buffers_per_shader_stage: limits.max_storage_buffers_per_shader_stage,
            meets_floor: vxe_render::limits_within(limits, STREAM_POOL_BYTES).is_ok(),
            holds_pool: limits.max_buffer_size >= STREAM_POOL_BYTES
                && limits.max_storage_buffer_binding_size >= STREAM_POOL_BYTES,
        }
    }
}

/// Every adapter of every backend this build has.
pub fn adapters() -> Vec<AdapterRow> {
    let desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    let instance = wgpu::Instance::new(desc);
    block_on(instance.enumerate_adapters(wgpu::Backends::all()))
        .iter()
        .map(|a| AdapterRow::new(&a.get_info(), &a.limits()))
        .collect()
}

/// Drives a future to completion on this thread; `wgpu`'s native futures are ready at once.
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    let mut f = std::pin::pin!(f);
    loop {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

/// A display as `winit` reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Display {
    pub size: [u32; 2],
    pub refresh_mhz: Option<u32>,
    pub scale: f64,
}

/// The machine, as `system.json` holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct SystemInfo {
    pub version: String,
    pub commit: String,
    pub os: String,
    pub os_version: String,
    /// Linux: `XDG_SESSION_TYPE`, and whether `WAYLAND_DISPLAY` is set.
    pub session: Option<(String, bool)>,
    pub cpu: String,
    pub logical_cores: u32,
    pub ram_bytes: Option<u64>,
    /// `None` where no window system was asked (`--system-json`).
    pub displays: Option<Vec<Display>>,
    pub adapters: Vec<AdapterRow>,
}

impl SystemInfo {
    /// This machine's OS, CPU and RAM, with `adapters` and `displays` as given.
    pub fn collect(adapters: Vec<AdapterRow>, displays: Option<Vec<Display>>) -> Self {
        let os = os::read();
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            commit: flythrough::report::COMMIT.to_owned(),
            os: os.name,
            os_version: os.version,
            session: os.session,
            cpu: os.cpu,
            logical_cores: os.cores.unwrap_or_else(|| {
                std::thread::available_parallelism().map_or(0, |n| n.get() as u32)
            }),
            ram_bytes: os.ram_bytes,
            displays,
            adapters,
        }
    }

    /// The JSON object, one field a line.
    pub fn to_json(&self) -> String {
        let mut o = String::from("{\n");
        let mut first = true;
        let mut field = |o: &mut String, name: &str| {
            if !first {
                o.push_str(",\n");
            }
            first = false;
            o.push_str("  ");
            write_json_string(o, name);
            o.push_str(": ");
        };
        field(&mut o, "schema");
        o.push('1');
        for (name, v) in [
            ("version", &self.version),
            ("commit", &self.commit),
            ("os", &self.os),
            ("os_version", &self.os_version),
            ("cpu", &self.cpu),
        ] {
            field(&mut o, name);
            write_json_string(&mut o, v);
        }
        field(&mut o, "session");
        match &self.session {
            None => o.push_str("null"),
            Some((kind, wayland)) => {
                o.push_str("{\"type\": ");
                write_json_string(&mut o, kind);
                let _ = write!(o, ", \"wayland_display\": {wayland}}}");
            }
        }
        field(&mut o, "logical_cores");
        let _ = write!(o, "{}", self.logical_cores);
        field(&mut o, "ram_bytes");
        match self.ram_bytes {
            Some(b) => {
                let _ = write!(o, "{b}");
            }
            None => o.push_str("null"),
        }
        field(&mut o, "displays");
        match &self.displays {
            None => o.push_str("null"),
            Some(d) => {
                o.push('[');
                for (i, d) in d.iter().enumerate() {
                    if i > 0 {
                        o.push_str(", ");
                    }
                    let _ = write!(
                        o,
                        "{{\"size\": [{}, {}], \"refresh_mhz\": {}, \"scale\": {}}}",
                        d.size[0],
                        d.size[1],
                        d.refresh_mhz.map_or("null".to_owned(), |r| r.to_string()),
                        d.scale
                    );
                }
                o.push(']');
            }
        }
        field(&mut o, "adapters");
        o.push('[');
        for (i, a) in self.adapters.iter().enumerate() {
            o.push_str(if i == 0 { "\n    {" } else { ",\n    {" });
            o.push_str("\"name\": ");
            write_json_string(&mut o, &a.name);
            let _ = write!(o, ", \"vendor\": {}, \"device\": {}", a.vendor, a.device);
            for (name, v) in [
                ("device_type", &a.device_type),
                ("driver", &a.driver),
                ("driver_info", &a.driver_info),
                ("backend", &a.backend),
            ] {
                o.push_str(", ");
                write_json_string(&mut o, name);
                o.push_str(": ");
                write_json_string(&mut o, v);
            }
            let _ = write!(
                o,
                ", \"max_buffer_size\": {}, \"max_storage_buffer_binding_size\": {}, \
                 \"max_storage_buffers_per_shader_stage\": {}, \"requested_pool_bytes\": {}, \
                 \"meets_floor\": {}, \"holds_pool\": {}}}",
                a.max_buffer_size,
                a.max_storage_buffer_binding_size,
                a.max_storage_buffers_per_shader_stage,
                STREAM_POOL_BYTES,
                a.meets_floor,
                a.holds_pool
            );
        }
        if !self.adapters.is_empty() {
            o.push_str("\n  ");
        }
        o.push_str("]\n}\n");
        o
    }
}

/// What the OS readers find.
struct OsFacts {
    name: String,
    version: String,
    session: Option<(String, bool)>,
    cpu: String,
    /// The logical cores the OS reports, `None` where it was not read.
    cores: Option<u32>,
    ram_bytes: Option<u64>,
}

#[cfg(target_os = "linux")]
mod os {
    use super::OsFacts;

    /// `/etc/os-release`'s `PRETTY_NAME`, the kernel's release, the session type, the first
    /// `model name` of `/proc/cpuinfo` and its count of `processor` lines, and `MemTotal` of
    /// `/proc/meminfo`.
    pub(super) fn read() -> OsFacts {
        let pretty = std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                    .map(|v| v.trim_matches('"').to_owned())
            })
            .unwrap_or_else(|| "Linux".to_owned());
        let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|s| s.trim().to_owned())
            .unwrap_or_default();
        let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        let cpu = cpuinfo
            .lines()
            .find(|l| l.starts_with("model name"))
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().to_owned())
            .unwrap_or_default();
        let cores = cpuinfo
            .lines()
            .filter(|l| l.split(':').next().map(str::trim) == Some("processor"))
            .count() as u32;
        let ram_bytes = std::fs::read_to_string("/proc/meminfo").ok().and_then(|s| {
            let l = s.lines().find(|l| l.starts_with("MemTotal:"))?;
            let kb: u64 = l
                .trim_start_matches("MemTotal:")
                .trim()
                .trim_end_matches("kB")
                .trim()
                .parse()
                .ok()?;
            Some(kb * 1024)
        });
        OsFacts {
            name: pretty,
            version: kernel,
            session: Some((
                std::env::var("XDG_SESSION_TYPE").unwrap_or_default(),
                std::env::var_os("WAYLAND_DISPLAY").is_some(),
            )),
            cpu,
            cores: (cores > 0).then_some(cores),
            ram_bytes,
        }
    }
}

#[cfg(windows)]
mod os {
    use super::OsFacts;
    use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    use windows_sys::Win32::System::SystemInformation::{
        GetSystemInfo, GlobalMemoryStatusEx, MEMORYSTATUSEX, OSVERSIONINFOW, SYSTEM_INFO,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// `RtlGetVersion`'s major, minor and build, the registry's `ProcessorNameString`,
    /// `GetSystemInfo`'s processor count and `GlobalMemoryStatusEx`'s total.
    pub(super) fn read() -> OsFacts {
        // SAFETY: `RtlGetVersion` fills the structure whose size field says how much it may
        // write; it is zeroed and sized here.
        let version = unsafe {
            let mut v: OSVERSIONINFOW = std::mem::zeroed();
            v.dwOSVersionInfoSize = size_of::<OSVERSIONINFOW>() as u32;
            if RtlGetVersion(&mut v) == 0 {
                format!(
                    "{}.{}.{}",
                    v.dwMajorVersion, v.dwMinorVersion, v.dwBuildNumber
                )
            } else {
                String::new()
            }
        };
        let key = wide(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0");
        let value = wide("ProcessorNameString");
        let mut buf = [0u16; 256];
        let mut len = size_of_val(&buf) as u32;
        // SAFETY: the key and value names are NUL-terminated UTF-16; `buf` and `len` give the
        // output's room in bytes, which `RegGetValueW` does not exceed.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut len,
            )
        };
        let cpu = if status == 0 {
            let n = (len as usize / 2).min(buf.len());
            String::from_utf16_lossy(&buf[..n])
                .trim_end_matches('\0')
                .trim()
                .to_owned()
        } else {
            String::new()
        };
        // SAFETY: the structure is zeroed with its length set, as the call requires.
        let ram_bytes = unsafe {
            let mut m: MEMORYSTATUSEX = std::mem::zeroed();
            m.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
            (GlobalMemoryStatusEx(&mut m) != 0).then_some(m.ullTotalPhys)
        };
        // SAFETY: `GetSystemInfo` fills the zeroed structure and cannot fail.
        let cores = unsafe {
            let mut i: SYSTEM_INFO = std::mem::zeroed();
            GetSystemInfo(&mut i);
            i.dwNumberOfProcessors
        };
        OsFacts {
            name: "Windows".to_owned(),
            version,
            session: None,
            cpu,
            cores: (cores > 0).then_some(cores),
            ram_bytes,
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod os {
    use super::OsFacts;

    pub(super) fn read() -> OsFacts {
        OsFacts {
            name: std::env::consts::OS.to_owned(),
            version: String::new(),
            session: None,
            cpu: String::new(),
            cores: None,
            ram_bytes: None,
        }
    }
}
