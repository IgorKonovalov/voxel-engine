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
/// and driver are printed.
pub fn device_on(
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
