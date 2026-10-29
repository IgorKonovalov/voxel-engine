//! SEEDED: `test = true`, and its test module opens an adapter; not in the gpu filter.
fn main() {
    let _ = wgpu::Instance::default();
}

#[cfg(test)]
mod tests {
    #[test]
    fn renders() {
        let instance = wgpu::Instance::default();
        let _ = pollster::block_on(instance.request_adapter(&Default::default()));
    }
}
