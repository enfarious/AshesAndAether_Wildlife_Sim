//! Core types for the wildlife simulation
//!
//! These mirror the TypeScript definitions in the game server
//! to ensure protocol compatibility.

use serde::{Deserialize, Serialize};


// ============================================================================
// Spatial Types
// ============================================================================

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct Vector3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vector3 {
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn distance_to(&self, other: &Vector3) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }

    pub fn distance_2d(&self, other: &Vector3) -> f64 {
        let dx = self.x - other.x;
        let dz = self.z - other.z;
        (dx * dx + dz * dz).sqrt()
    }
}

// ============================================================================
// Biome Types
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BiomeType {
    Forest,
    Grassland,
    Desert,
    Tundra,
    Swamp,
    Mountain,
    Coastal,
    Freshwater,
    Ocean,
    Urban,
    Underground,
}

// ============================================================================
// Wildlife Classification
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DietType {
    Predator,
    Prey,
    Hybrid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sex {
    Male,
    Female,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeClass {
    Tiny = 1,
    Small = 2,
    Medium = 3,
    Large = 4,
    Huge = 5,
}

impl SizeClass {
    pub fn value(&self) -> i32 {
        *self as i32
    }
}

// ============================================================================
// Wildlife Needs
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WildlifeNeeds {
    pub hunger: f64,      // 0-100, 0 = starving
    pub thirst: f64,      // 0-100, 0 = dehydrated
    pub energy: f64,      // 0-100, 0 = exhausted
    pub safety: f64,      // 0-100, perceived safety
    pub reproduction: f64, // 0-100, urge to mate
}

impl Default for WildlifeNeeds {
    fn default() -> Self {
        Self {
            hunger: 70.0,
            thirst: 70.0,
            energy: 80.0,
            safety: 70.0,
            reproduction: 20.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeedDecayRates {
    pub hunger: f64,
    pub thirst: f64,
    pub energy: f64,
    pub reproduction: f64, // Negative = increases over time
}

impl Default for NeedDecayRates {
    fn default() -> Self {
        Self {
            hunger: 0.1,
            thirst: 0.15,
            energy: 0.05,
            reproduction: -0.02,
        }
    }
}

// ============================================================================
// Behavior States
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BehaviorState {
    Idle,
    Wandering,
    Foraging,
    Hunting,
    Stalking,
    Fleeing,
    Drinking,
    Eating,
    Resting,
    SeekingMate,
    Mating,
    Defending,
    Attacking,
    Hibernating,
    Dying,
    Dead,
}

impl BehaviorState {
    /// Priority for behavior selection (higher = more urgent)
    pub fn priority(&self) -> i32 {
        match self {
            Self::Dead => 100,
            Self::Dying => 99,
            Self::Hibernating => 80,
            Self::Fleeing => 90,
            Self::Attacking => 85,
            Self::Defending => 80,
            Self::Drinking => 70,
            Self::Eating => 65,
            Self::Foraging => 60,
            Self::Hunting => 55,
            Self::Stalking => 50,
            Self::SeekingMate => 40,
            Self::Mating => 35,
            Self::Resting => 30,
            Self::Wandering => 20,
            Self::Idle => 10,
        }
    }
}

// ============================================================================
// Wildlife Species Definition
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiomePreference {
    pub biome: BiomeType,
    pub comfort: f64,      // 0-100
    pub spawn_weight: f64, // Relative spawn chance
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LootEntry {
    pub item_id: String,
    pub chance: f64,
    pub quantity_min: u32,
    pub quantity_max: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WildlifeSpecies {
    pub id: String,
    pub name: String,
    pub description: String,

    // Classification
    pub diet_type: DietType,
    pub size_class: SizeClass,

    // Movement
    pub base_speed: f64,
    pub flee_speed_multiplier: f64,
    pub swim_capable: bool,
    pub climb_capable: bool,

    // Combat
    pub attack_damage: f64,
    pub attack_range: f64,
    pub attack_cooldown: f64,
    pub max_health: f64,

    // Perception
    pub sight_range: f64,
    pub hearing_range: f64,
    pub smell_range: f64,

    // Needs
    pub need_decay_rates: NeedDecayRates,
    pub preferred_food: Vec<String>,
    pub is_herbivore: bool,
    pub is_carnivore: bool,

    // Habitat
    pub biome_preferences: Vec<BiomePreference>,
    pub nocturnal: bool,
    pub social_behavior: SocialBehavior,
    pub pack_size_min: Option<u32>,
    pub pack_size_max: Option<u32>,

    // Reproduction
    pub gestation_time: f64,
    pub offspring_min: u32,
    pub offspring_max: u32,
    pub maturity_time: f64,

    // Loot
    pub loot_table: Vec<LootEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SocialBehavior {
    Solitary,
    Pair,
    Pack,
    Herd,
}

// ============================================================================
// Wildlife Entity (Runtime Instance)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WildlifeEntity {
    pub id: String,
    pub species_id: String,
    pub name: String,

    // Position
    pub position: Vector3,
    pub heading: f64,
    pub zone_id: String,
    pub current_biome: BiomeType,

    // State
    pub is_alive: bool,
    pub current_health: f64,
    pub max_health: f64,
    pub attack_damage: f64,
    pub needs: WildlifeNeeds,

    // Behavior
    pub current_behavior: BehaviorState,
    pub target_entity_id: Option<String>,
    pub home_position: Option<Vector3>,

    // Timers (milliseconds since epoch)
    pub last_update_at: i64,
    pub attack_cooldown_until: i64,
    pub fleeing_until: i64,

    // Reproduction
    pub sex: Sex,
    pub is_pregnant: bool,
    pub pregnancy_ends_at: Option<i64>,
    pub age: f64,
    pub is_mature: bool,
    pub age_stage: WildlifeAgeStage,
    pub level: u32,
    pub experience: f64,
    pub experience_to_next: f64,

    // Combat
    pub in_combat: bool,
    pub last_hostile_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WildlifeAgeStage {
    Juvenile,
    Adult,
    Elder,
}

// ============================================================================
// Plant Types
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlantGrowthStage {
    Seed,
    Sprout,
    Growing,
    Mature,
    Flowering,
    Withering,
    Dead,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlantEntity {
    pub id: String,
    pub species_id: String,
    pub position: Vector3,
    pub zone_id: String,

    pub current_stage: PlantGrowthStage,
    pub stage_started_at: i64,
    /// Progress through current stage (0.0 to 1.0)
    pub stage_progress: f64,
    /// Index into the species growth_stages array
    pub stage_index: usize,

    pub is_alive: bool,
    pub is_dormant: bool,
    pub times_harvested: u32,
    pub last_harvested_at: Option<i64>,

    pub spawned_at: i64,
}

// ============================================================================
// Events (sent to game server via Redis)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WildlifeEvent {
    Spawn {
        entity_id: String,
        species_id: String,
        position: Vector3,
        zone_id: String,
    },
    Death {
        entity_id: String,
        species_id: String,
        name: String,
        position: Vector3,
        zone_id: String,
        killer_id: Option<String>,
        killer_species: Option<String>,
        cause: String,
        age: f64,
        health_at_death: f64,
    },
    Move {
        entity_id: String,
        position: Vector3,
        heading: f64,
        behavior: BehaviorState,
    },
    Attack {
        attacker_id: String,
        target_id: String,
        damage: f64,
        position: Vector3,
    },
    Birth {
        parent_id: String,
        offspring_ids: Vec<String>,
        position: Vector3,
        zone_id: String,
    },
    PlantGrow {
        plant_id: String,
        new_stage: PlantGrowthStage,
    },
    PlantEaten {
        plant_id: String,
        wildlife_id: String,
        food_value: f64,
    },
}

// ============================================================================
// Messages from Game Server
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerPosition {
    pub id: String,
    pub position: Vector3,
    pub zone_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneInfo {
    pub id: String,
    pub biome: BiomeType,
    pub time_of_day: f64, // 0-24
    pub bounds_min: Vector3,
    pub bounds_max: Vector3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GameServerMessage {
    /// Player positions update (sent periodically)
    PlayersUpdate { players: Vec<PlayerPosition> },

    /// Zone information
    ZoneInfo { zone: ZoneInfo },

    /// Player attacked wildlife
    PlayerAttack {
        player_id: String,
        target_id: String,
        damage: f64,
    },

    /// Player harvested plant
    PlantHarvest {
        plant_id: String,
        player_id: String,
    },

    /// Request current state (for sync)
    StateRequest { zone_id: String },
}

// ============================================================================
// Thresholds (matching TypeScript constants)
// ============================================================================

pub mod thresholds {
    pub const NEED_CRITICAL: f64 = 15.0;
    pub const NEED_LOW: f64 = 30.0;
    pub const NEED_COMFORTABLE: f64 = 60.0;
    pub const NEED_FULL: f64 = 90.0;

    pub const SAFETY_PANIC: f64 = 20.0;
    pub const SAFETY_NERVOUS: f64 = 40.0;
    pub const SAFETY_ALERT: f64 = 60.0;
    pub const SAFETY_RELAXED: f64 = 80.0;

    pub const SPECIAL_CHARGE_MAX: u32 = 5;
}
