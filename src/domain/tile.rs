/// Tile types and their properties.
/// Properties are queried via methods, not stored as flags,
/// so tile semantics are centralized here.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tile {
    Empty,
    Brick,        // Solid + Diggable
    Concrete,     // Solid only
    Ladder,       // Climbable
    Rope,         // Hangable (horizontal bar)
    Gold,         // Pickup target
    HiddenLadder, // Appears when all gold collected
    TrapBrick,    // Looks like Brick, collapses when stepped on
}

impl Tile {
    /// Can an entity stand on top of this tile? (i.e. is it a floor)
    pub fn is_solid(self) -> bool {
        matches!(self, Tile::Brick | Tile::Concrete | Tile::TrapBrick)
    }

    /// Can this tile be dug?
    pub fn is_diggable(self) -> bool {
        matches!(self, Tile::Brick)
    }

    /// Can an entity climb (move up/down) on this tile?
    pub fn is_climbable(self) -> bool {
        matches!(self, Tile::Ladder | Tile::HiddenLadder)
    }

    /// Can an entity hang and move horizontally on this tile?
    pub fn is_hangable(self) -> bool {
        matches!(self, Tile::Rope)
    }

    /// Is this tile passable (entity can occupy this cell)?
    pub fn is_passable(self) -> bool {
        !self.is_solid()
    }

    /// Is this a gold pickup?
    #[allow(dead_code)]
    pub fn is_gold(self) -> bool {
        matches!(self, Tile::Gold)
    }

    /// Is this a trap brick? (looks like brick but collapses)
    #[allow(dead_code)]
    pub fn is_trap(self) -> bool {
        matches!(self, Tile::TrapBrick)
    }
}

impl Default for Tile {
    fn default() -> Self {
        Tile::Empty
    }
}
