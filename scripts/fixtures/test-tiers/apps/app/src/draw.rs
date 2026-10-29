//! SEEDED: a module of the binary whose test module opens the software adapter.
pub fn draw() {}

#[cfg(test)]
mod tests {
    #[test]
    fn draws() {
        let _ = crate::software_device();
    }
}
