//! The binary opens a device as a program, which is no finding; its own test module parses
//! `--headless` without spawning anything, which is none either. `draw`'s test module is.
mod draw;

fn main() {
    let instance = wgpu::Instance::default();
    let _ = pollster::block_on(instance.request_adapter(&Default::default()));
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses() {
        assert_eq!(["--headless"].len(), 1);
    }
}
