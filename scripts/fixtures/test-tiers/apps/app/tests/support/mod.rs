pub fn software_device(_pool: u64) -> Option<()> {
    None
}

pub fn rig() -> Option<()> {
    software_device(64 << 20)
}
