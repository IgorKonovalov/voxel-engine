//! `system.json` from this machine's OS readers and a fixed fake adapter list: its fields, and
//! nothing in it that names the person or the machine.

use flythrough::report::json::{Value, parse};
use vxe_tester::system::{AdapterRow, STREAM_POOL_BYTES, SystemInfo};

fn fake_adapters() -> Vec<AdapterRow> {
    vec![
        AdapterRow {
            name: "Fake Discrete 9000".to_owned(),
            vendor: 0x10de,
            device: 0x1234,
            device_type: "DiscreteGpu".to_owned(),
            driver: "fake".to_owned(),
            driver_info: "1.0".to_owned(),
            backend: "Vulkan".to_owned(),
            max_buffer_size: 4 << 30,
            max_storage_buffer_binding_size: 4 << 30,
            max_storage_buffers_per_shader_stage: 16,
            meets_floor: true,
            holds_pool: true,
        },
        AdapterRow {
            name: "Fake Integrated".to_owned(),
            vendor: 0x1002,
            device: 0x1638,
            device_type: "IntegratedGpu".to_owned(),
            driver: "fake".to_owned(),
            driver_info: "2.0".to_owned(),
            backend: "Dx12".to_owned(),
            max_buffer_size: 1 << 30,
            max_storage_buffer_binding_size: 128 << 20,
            max_storage_buffers_per_shader_stage: 8,
            meets_floor: false,
            holds_pool: false,
        },
    ]
}

/// The host name as the OS gives it, where it can be read.
fn host_name() -> Option<String> {
    let h = if cfg!(windows) {
        std::env::var("COMPUTERNAME").ok()
    } else {
        std::fs::read_to_string("/proc/sys/kernel/hostname")
            .ok()
            .or_else(|| std::env::var("HOSTNAME").ok())
    };
    h.map(|h| h.trim().to_owned()).filter(|h| h.len() >= 3)
}

#[test]
fn system_json_holds_the_machine_and_the_adapters() {
    let info = SystemInfo::collect(fake_adapters(), None);
    let json = info.to_json();
    let v = parse(&json).expect("system.json parses");
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    assert_eq!(v.get("schema").and_then(Value::as_f64), Some(1.0));
    assert_eq!(s("version").as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert!(s("commit").is_some_and(|c| !c.is_empty()));
    assert!(s("os").is_some_and(|o| !o.is_empty()), "os: {json}");
    assert!(s("os_version").is_some(), "os_version: {json}");
    assert!(s("cpu").is_some(), "cpu: {json}");
    assert!(
        v.get("logical_cores")
            .and_then(Value::as_f64)
            .is_some_and(|n| n >= 1.0),
        "cores: {json}"
    );
    let ram = v.get("ram_bytes").and_then(Value::as_f64);
    if cfg!(any(target_os = "linux", windows)) {
        assert!(ram.is_some_and(|b| b > 0.0), "ram: {json}");
        assert!(s("cpu").is_some_and(|c| !c.is_empty()), "cpu: {json}");
    }
    if cfg!(target_os = "linux") {
        let session = v.get("session").expect("session");
        assert!(session.get("type").and_then(Value::as_str).is_some());
        assert!(matches!(
            session.get("wayland_display"),
            Some(Value::Bool(_))
        ));
    }
    assert_eq!(v.get("displays"), Some(&Value::Null));

    let adapters = v
        .get("adapters")
        .and_then(Value::as_array)
        .expect("adapters");
    assert_eq!(adapters.len(), 2);
    let a = &adapters[1];
    assert_eq!(
        a.get("name").and_then(Value::as_str),
        Some("Fake Integrated")
    );
    assert_eq!(a.get("vendor").and_then(Value::as_f64), Some(4098.0));
    assert_eq!(a.get("device").and_then(Value::as_f64), Some(5688.0));
    assert_eq!(
        a.get("device_type").and_then(Value::as_str),
        Some("IntegratedGpu")
    );
    assert_eq!(a.get("backend").and_then(Value::as_str), Some("Dx12"));
    assert_eq!(a.get("driver_info").and_then(Value::as_str), Some("2.0"));
    assert_eq!(
        a.get("max_storage_buffer_binding_size")
            .and_then(Value::as_f64),
        Some(f64::from(128u32 << 20))
    );
    assert_eq!(
        a.get("requested_pool_bytes").and_then(Value::as_f64),
        Some(STREAM_POOL_BYTES as f64)
    );
    assert_eq!(a.get("meets_floor"), Some(&Value::Bool(false)));
    assert_eq!(adapters[0].get("holds_pool"), Some(&Value::Bool(true)));
}

#[test]
fn system_json_names_neither_the_user_nor_the_host() {
    let json = SystemInfo::collect(fake_adapters(), None).to_json();
    for var in ["USER", "USERNAME", "LOGNAME"] {
        if let Ok(user) = std::env::var(var)
            && user.len() >= 3
        {
            assert!(
                !json.contains(&user),
                "${var} ({user}) is in system.json:\n{json}"
            );
        }
    }
    if let Some(host) = host_name() {
        assert!(
            !json.contains(&host),
            "the host name is in system.json:\n{json}"
        );
    }
    if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
        let home = home.to_string_lossy().into_owned();
        if home.len() > 1 {
            assert!(!json.contains(&home), "the home path is in system.json");
        }
    }
}
