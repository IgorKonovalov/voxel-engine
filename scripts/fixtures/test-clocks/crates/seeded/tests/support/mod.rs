//! The seeded tree's stepped rig: each frame waits for the bake thread's step.

pub struct Q;

impl Q {
    pub fn step(&mut self) {
        self.frame.wait_bake_step();
    }
}
