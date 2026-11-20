//! Devices for the renderer's tests and the tests of crates that render, behind the
//! `test-support` feature.
//!
//! One rule for every helper: an adapter it cannot find is a printed skip (`None`), unless
//! `VXE_REQUIRE_SOFTWARE_ADAPTER=1`, where it panics; with that variable set the helpers for
//! asserting tests also take only the software Vulkan adapter (lavapipe, where goldens are
//! blessed), so a CI arm that promises image comparisons cannot pass by skipping them or by
//! running them elsewhere. `VXE_TEST_ADAPTER_FILTER`, when set, keeps only the adapters whose name holds
//! it (ignoring case): a filter that matches nothing stands in for a machine without the
//! adapter.

/// The variable that turns a missing software adapter from a skip into a panic.
pub const REQUIRE_SOFTWARE: &str = "VXE_REQUIRE_SOFTWARE_ADAPTER";

/// The variable that keeps only the adapters whose name holds its value.
pub const ADAPTER_FILTER: &str = "VXE_TEST_ADAPTER_FILTER";

/// Whether `VXE_REQUIRE_SOFTWARE_ADAPTER=1`.
pub fn software_required() -> bool {
    std::env::var(REQUIRE_SOFTWARE).as_deref() == Ok("1")
}

/// The adapters of `backends` that pass [`ADAPTER_FILTER`].
fn adapters(backends: wgpu::Backends) -> Vec<wgpu::Adapter> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let filter = std::env::var(ADAPTER_FILTER)
        .unwrap_or_default()
        .to_lowercase();
    pollster::block_on(instance.enumerate_adapters(backends))
        .into_iter()
        .filter(|a| a.get_info().name.to_lowercase().contains(&filter))
        .collect()
}

/// Skips (`None`, printed) or, under [`REQUIRE_SOFTWARE`], panics, naming what was `wanted`.
fn missing<T>(wanted: &str) -> Option<T> {
    let filter = std::env::var(ADAPTER_FILTER).unwrap_or_default();
    let by = if filter.is_empty() {
        String::new()
    } else {
        format!(" ({ADAPTER_FILTER}={filter})")
    };
    if software_required() {
        panic!("{REQUIRE_SOFTWARE}=1 but there is no {wanted}{by}");
    }
    println!("skipped: no {wanted}{by}");
    None
}

/// The software Vulkan adapter (lavapipe), or the skip-or-require rule's answer.
pub fn software_adapter() -> Option<wgpu::Adapter> {
    adapters(wgpu::Backends::VULKAN)
        .into_iter()
        .find(|a| a.get_info().device_type == wgpu::DeviceType::Cpu)
        .or_else(|| missing("software Vulkan adapter"))
}

/// A device on `adapter` with the frame's limits for a brick pool of `pool_bytes`
/// ([`crate::limits_for`]) and the features of `features` the adapter has; the adapter's name
/// and driver are printed. On a hardware adapter whose backend keeps a pipeline cache, every
/// compute pipeline the renderer builds on the device goes through the file cache of
/// [`pipeline_cache_file`] (opened here, written back when the calling thread ends or the next
/// device opens); a CPU adapter opens no cache file, since a software driver's cache holds no
/// compiled code.
pub fn device_on(
    adapter: &wgpu::Adapter,
    pool_bytes: u64,
    features: wgpu::Features,
) -> (wgpu::Device, wgpu::Queue) {
    let file = pipeline_cache_file(adapter);
    let extra = if file.is_some() {
        wgpu::Features::PIPELINE_CACHE
    } else {
        wgpu::Features::empty()
    };
    let (device, queue) = device_on_uncached(adapter, pool_bytes, features | extra);
    if let Some(file) = file {
        open_pipeline_cache(&device, file);
    }
    (device, queue)
}

/// [`device_on`] without the file cache: a device whose pipelines are built through no cache
/// unless the caller sets one (`debug::set_pipeline_cache`).
pub fn device_on_uncached(
    adapter: &wgpu::Adapter,
    pool_bytes: u64,
    features: wgpu::Features,
) -> (wgpu::Device, wgpu::Queue) {
    let info = adapter.get_info();
    println!(
        "adapter: {} ({:?}, driver {} {})",
        info.name, info.backend, info.driver, info.driver_info
    );
    let desc = wgpu::DeviceDescriptor {
        required_features: adapter.features() & features,
        required_limits: crate::limits_for(adapter, pool_bytes).expect("the frame's limits"),
        ..Default::default()
    };
    pollster::block_on(adapter.request_device(&desc)).expect("a device on the adapter")
}

/// The pipeline cache file [`device_on`] uses for `adapter`:
/// `<target>/vxe-pipeline-cache/<wgpu's key for the adapter>/<hash of the renderer's composed
/// shader sources>.bin`, or `None` for a CPU adapter, a backend without a pipeline cache, or a
/// test binary outside a cargo target directory. A changed shader names a new file, so a
/// stale cache is a cold start, never a load of old pipelines.
pub fn pipeline_cache_file(adapter: &wgpu::Adapter) -> Option<std::path::PathBuf> {
    use std::hash::{Hash, Hasher};
    let info = adapter.get_info();
    if info.device_type == wgpu::DeviceType::Cpu
        || !adapter.features().contains(wgpu::Features::PIPELINE_CACHE)
    {
        return None;
    }
    let key = wgpu::util::pipeline_cache_key(&info)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for s in crate::frame::shader_sources() {
        s.label.hash(&mut hasher);
        s.source.hash(&mut hasher);
        s.entry.hash(&mut hasher);
    }
    info.driver_info.hash(&mut hasher);
    Some(
        pipeline_cache_root()?
            .join(key)
            .join(format!("{:016x}.bin", hasher.finish())),
    )
}

/// `<target>/vxe-pipeline-cache`: under `CARGO_TARGET_DIR` when it is set, else beside the
/// profile directory the running test binary sits in (`<target>/<profile>/deps/<binary>`).
pub fn pipeline_cache_root() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("CARGO_TARGET_DIR") {
        return Some(std::path::PathBuf::from(dir).join("vxe-pipeline-cache"));
    }
    let exe = std::env::current_exe().ok()?;
    let parent = exe.ancestors().find(|p| {
        p.file_name()
            .is_some_and(|n| n == "deps" || n == "examples")
    })?;
    Some(parent.parent()?.parent()?.join("vxe-pipeline-cache"))
}

/// Writes the open file cache's data back when dropped: held in a thread-local of the thread
/// that opened the device, so a test's thread writes it at its end.
struct CacheWriteBack;

impl Drop for CacheWriteBack {
    fn drop(&mut self) {
        write_back_pipeline_cache();
    }
}

thread_local! {
    static WRITE_BACK: CacheWriteBack = const { CacheWriteBack };
}

/// The file the cache [`crate::gpu_util::set_pipeline_cache`] holds came from, while
/// [`device_on`] opened it.
static CACHE_FILE: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// Opens `file` as `device`'s pipeline cache (an absent, unreadable or refused file is an
/// empty cache: `fallback`), after writing back the cache of a previous device.
fn open_pipeline_cache(device: &wgpu::Device, file: std::path::PathBuf) {
    write_back_pipeline_cache();
    let data = std::fs::read(&file).ok();
    // SAFETY: the data is a file this helper wrote from `PipelineCache::get_data` on an
    // adapter with the same wgpu key and shader sources; wgpu checks its header against the
    // adapter and driver, and `fallback` makes a refused or truncated file an empty cache.
    let cache = unsafe {
        device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
            label: Some("test pipeline cache"),
            data: data.as_deref(),
            fallback: true,
        })
    };
    println!(
        "pipeline cache: {} ({})",
        file.display(),
        match &data {
            Some(d) => format!("{} B read", d.len()),
            None => "cold".to_owned(),
        }
    );
    crate::gpu_util::set_pipeline_cache(Some(cache));
    *CACHE_FILE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(file);
    WRITE_BACK.with(|_| ());
}

/// Writes the open file cache's data to its file (through a temporary file and a rename, so
/// a reader never sees half a file) and clears the cache; nothing when none is open.
pub fn write_back_pipeline_cache() {
    let Some(file) = CACHE_FILE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    else {
        return;
    };
    let data = crate::gpu_util::pipeline_cache_data();
    crate::gpu_util::set_pipeline_cache(None);
    let Some(data) = data else { return };
    let Some(dir) = file.parent() else { return };
    let tmp = file.with_extension(format!("{}.tmp", std::process::id()));
    let written = std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&tmp, &data))
        .and_then(|()| std::fs::rename(&tmp, &file));
    if let Err(e) = written {
        println!("pipeline cache: not written to {}: {e}", file.display());
    }
}

/// The file [`device_on`]'s cache is open from, if one is.
pub fn open_pipeline_cache_file() -> Option<std::path::PathBuf> {
    CACHE_FILE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// A device on the software Vulkan adapter ([`software_adapter`], [`device_on`]): the adapter
/// goldens and exact pixel counts are blessed on.
pub fn software_device(
    pool_bytes: u64,
    features: wgpu::Features,
) -> Option<(wgpu::Device, wgpu::Queue)> {
    let adapter = software_adapter()?;
    Some(device_on(&adapter, pool_bytes, features))
}

/// The software adapter where there is one, else (without [`REQUIRE_SOFTWARE`]) the first
/// adapter of any backend: for tests whose assertions hold on any adapter.
pub fn any_adapter() -> Option<wgpu::Adapter> {
    if software_required() {
        return software_adapter();
    }
    let all = adapters(wgpu::Backends::all());
    let software = all.iter().position(|a| {
        let i = a.get_info();
        i.device_type == wgpu::DeviceType::Cpu && i.backend == wgpu::Backend::Vulkan
    });
    match software {
        Some(i) => all.into_iter().nth(i),
        None => all.into_iter().next().or_else(|| missing("adapter")),
    }
}

/// A device on [`any_adapter`] ([`device_on`]).
pub fn any_device(
    pool_bytes: u64,
    features: wgpu::Features,
) -> Option<(wgpu::Device, wgpu::Queue)> {
    let adapter = any_adapter()?;
    Some(device_on(&adapter, pool_bytes, features))
}

/// The fastest Vulkan adapter (a discrete GPU, then an integrated one, then any), or the
/// skip-or-require rule's answer: for readings, which name the adapter they ran on.
/// `VK_DRIVER_FILES` naming one driver's manifest leaves the loader that driver's adapter.
pub fn high_performance_adapter() -> Option<wgpu::Adapter> {
    let rank = |a: &wgpu::Adapter| match a.get_info().device_type {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::VirtualGpu | wgpu::DeviceType::Other => 2,
        wgpu::DeviceType::Cpu => 3,
    };
    adapters(wgpu::Backends::VULKAN)
        .into_iter()
        .min_by_key(rank)
        .or_else(|| missing("Vulkan adapter"))
}

/// A device on [`high_performance_adapter`] ([`device_on`]) and the adapter's name.
pub fn high_performance_device(
    pool_bytes: u64,
    features: wgpu::Features,
) -> Option<(wgpu::Device, wgpu::Queue, String)> {
    let adapter = high_performance_adapter()?;
    let (device, queue) = device_on(&adapter, pool_bytes, features);
    Some((device, queue, adapter.get_info().name))
}
