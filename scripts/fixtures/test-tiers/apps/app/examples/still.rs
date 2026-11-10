//! SEEDED: `test = true`, and its test module opens an adapter; its name lacks the `gpu_` prefix.
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
