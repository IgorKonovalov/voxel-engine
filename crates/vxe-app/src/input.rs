//! Per-frame input state: keys held, keys and buttons pressed since the last frame, relative
//! mouse motion and scroll. The shell feeds it from `winit` events and clears the per-frame
//! parts after each frame; tests drive it through the same methods.

use std::collections::HashSet;

pub use winit::event::MouseButton;
pub use winit::keyboard::KeyCode;

#[derive(Debug, Default)]
pub struct Input {
    held: HashSet<KeyCode>,
    pressed: Vec<KeyCode>,
    clicked: Vec<MouseButton>,
    mouse_delta: [f64; 2],
    scroll: f32,
}

impl Input {
    /// `key` is down now.
    pub fn held(&self, key: KeyCode) -> bool {
        self.held.contains(&key)
    }

    /// `key` went down since the last frame (key repeat does not count).
    pub fn pressed(&self, key: KeyCode) -> bool {
        self.pressed.contains(&key)
    }

    /// `button` went down since the last frame.
    pub fn clicked(&self, button: MouseButton) -> bool {
        self.clicked.contains(&button)
    }

    /// Relative mouse motion since the last frame, in device units (x right, y down).
    pub fn mouse_delta(&self) -> [f64; 2] {
        self.mouse_delta
    }

    /// Scroll since the last frame, in lines; positive away from the user.
    pub fn scroll(&self) -> f32 {
        self.scroll
    }

    pub fn key_down(&mut self, key: KeyCode) {
        if self.held.insert(key) {
            self.pressed.push(key);
        }
    }

    pub fn key_up(&mut self, key: KeyCode) {
        self.held.remove(&key);
    }

    pub fn button_down(&mut self, button: MouseButton) {
        self.clicked.push(button);
    }

    pub fn add_mouse_delta(&mut self, dx: f64, dy: f64) {
        self.mouse_delta[0] += dx;
        self.mouse_delta[1] += dy;
    }

    pub fn add_scroll(&mut self, lines: f32) {
        self.scroll += lines;
    }

    /// Releases every held key, for when the window loses focus and key-up events stop.
    pub fn release_all(&mut self) {
        self.held.clear();
    }

    /// Clears what happened during the frame; held keys stay held.
    pub fn end_frame(&mut self) {
        self.pressed.clear();
        self.clicked.clear();
        self.mouse_delta = [0.0; 2];
        self.scroll = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_lasts_one_frame_and_a_hold_until_release() {
        let mut i = Input::default();
        i.key_down(KeyCode::KeyW);
        assert!(i.pressed(KeyCode::KeyW) && i.held(KeyCode::KeyW));
        i.end_frame();
        i.key_down(KeyCode::KeyW); // key repeat
        assert!(!i.pressed(KeyCode::KeyW) && i.held(KeyCode::KeyW));
        i.key_up(KeyCode::KeyW);
        assert!(!i.held(KeyCode::KeyW));
    }

    #[test]
    fn motion_scroll_and_clicks_accumulate_within_a_frame_only() {
        let mut i = Input::default();
        i.add_mouse_delta(3.0, -1.0);
        i.add_mouse_delta(2.0, 4.0);
        i.add_scroll(1.0);
        i.add_scroll(1.0);
        i.button_down(MouseButton::Left);
        assert_eq!(i.mouse_delta(), [5.0, 3.0]);
        assert_eq!(i.scroll(), 2.0);
        assert!(i.clicked(MouseButton::Left) && !i.clicked(MouseButton::Right));
        i.end_frame();
        assert_eq!(i.mouse_delta(), [0.0, 0.0]);
        assert_eq!(i.scroll(), 0.0);
        assert!(!i.clicked(MouseButton::Left));
    }
}
