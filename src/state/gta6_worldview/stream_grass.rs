//! Where a camera stands on the grass: the height of the ground under it.

impl super::Viewer {
    /// The height of the ground at `x`, `y` by the install's height map, once it is read: where a camera stands on the grass.
    pub fn ground_under(&self, x: f32, y: f32) -> Option<f32> {
        super::water::ground(self.root.as_deref()?)?.ground(x, y)
    }
}
