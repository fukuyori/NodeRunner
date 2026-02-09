/// Events emitted during a simulation step.
/// The presentation layer consumes these for animation/sound.

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum GameEvent {
    GoldPicked { x: usize, y: usize },
    HoleCreated { x: usize, y: usize },
    HoleFilled { x: usize, y: usize },
    GuardTrapped { id: usize, x: usize, y: usize },
    GuardKilled { id: usize, x: usize, y: usize },
    GuardRespawned { id: usize },
    GuardDroppedGold { x: usize, y: usize },
    PlayerKilled,
    PlayerFallStart,
    ExitEnabled,
    StageCleared,
    AllGoldCollected,
    TrapCollapsed { x: usize, y: usize },
}
