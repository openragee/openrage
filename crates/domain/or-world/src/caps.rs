//! What a title's world has and what it can write back, as data: a panel section, a tool or a
//! verb whose capability is absent is hidden or greyed, and no shared code asks which game it serves.

/// One thing a world has or lacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Capability {
    /// A height of the ground under a position.
    Ground = 0,
    /// A collision query with hits and overlaps.
    Collision = 1,
    /// Placement rows of a map, and a search over them.
    Placements = 2,
    /// A weather that gates what streams.
    WeatherGate = 3,
    /// An edit store.
    Edits = 4,
    /// A base to compare with, put back to and merge against.
    Base = 5,
    /// Its own shader programs, where the plain pass is not the whole picture.
    OwnPrograms = 6,
}

impl Capability {
    pub const ALL: [Self; 7] = [
        Self::Ground,
        Self::Collision,
        Self::Placements,
        Self::WeatherGate,
        Self::Edits,
        Self::Base,
        Self::OwnPrograms,
    ];
}

/// The capabilities of one opened world, and what an instance of it carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorldCaps {
    held: u8,
    /// How many words of the title's own each instance of a selection carries.
    pub instance_words: usize,
}

impl WorldCaps {
    /// No capability, and instances that carry no word of the title's.
    pub const fn none() -> Self {
        Self {
            held: 0,
            instance_words: 0,
        }
    }

    /// These and `capability`.
    pub const fn with(mut self, capability: Capability) -> Self {
        self.held |= 1 << capability as u8;
        self
    }

    /// These, each instance carrying `words` words of the title's own.
    pub const fn with_instance_words(mut self, words: usize) -> Self {
        self.instance_words = words;
        self
    }

    pub const fn has(&self, capability: Capability) -> bool {
        self.held & (1 << capability as u8) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capability_is_held_only_once_given() {
        let caps = WorldCaps::none().with(Capability::Edits);
        for capability in Capability::ALL {
            assert_eq!(caps.has(capability), capability == Capability::Edits);
        }
    }
}
