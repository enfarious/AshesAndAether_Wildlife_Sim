//! Species definitions for wildlife
//!
//! Each species is defined with its behaviors, stats, and habitat preferences.

use crate::types::*;
use std::collections::HashMap;
use std::sync::LazyLock;

/// Global species registry
pub static SPECIES: LazyLock<HashMap<String, WildlifeSpecies>> = LazyLock::new(|| {
    let mut map = HashMap::new();

    map.insert("rabbit".to_string(), rabbit());
    map.insert("fox".to_string(), fox());

    map
});

pub fn get_species(id: &str) -> Option<&'static WildlifeSpecies> {
    SPECIES.get(id)
}

pub fn all_species() -> impl Iterator<Item = &'static WildlifeSpecies> {
    SPECIES.values()
}

// ============================================================================
// Species Definitions
// ============================================================================

fn rabbit() -> WildlifeSpecies {
    WildlifeSpecies {
        id: "rabbit".to_string(),
        name: "Rabbit".to_string(),
        description: "A small, fluffy herbivore with long ears.".to_string(),

        diet_type: DietType::Prey,
        size_class: SizeClass::Tiny, // Tiny so foxes (Small) can hunt them

        base_speed: 2.0,
        flee_speed_multiplier: 2.5,
        swim_capable: false,
        climb_capable: false,

        attack_damage: 2.0,
        attack_range: 0.5,
        attack_cooldown: 2.0,
        max_health: 15.0,

        sight_range: 20.0,
        hearing_range: 40.0,
        smell_range: 15.0,

        need_decay_rates: NeedDecayRates {
            hunger: 0.15,
            thirst: 0.12,
            energy: 0.08,
            reproduction: -0.2,
        },
        preferred_food: vec![
            "grass".to_string(),
            "clover".to_string(),
            "carrot".to_string(),
            "herb_sage".to_string(),
        ],
        is_herbivore: true,
        is_carnivore: false,

        biome_preferences: vec![
            BiomePreference { biome: BiomeType::Grassland, comfort: 95.0, spawn_weight: 10.0 },
            BiomePreference { biome: BiomeType::Forest, comfort: 80.0, spawn_weight: 5.0 },
            BiomePreference { biome: BiomeType::Urban, comfort: 60.0, spawn_weight: 3.0 },
            BiomePreference { biome: BiomeType::Mountain, comfort: 40.0, spawn_weight: 1.0 },
            BiomePreference { biome: BiomeType::Coastal, comfort: 50.0, spawn_weight: 2.0 },
        ],
        nocturnal: false,
        social_behavior: SocialBehavior::Herd,
        pack_size_min: Some(3),
        pack_size_max: Some(8),

        gestation_time: 300.0,
        offspring_min: 2,
        offspring_max: 6,
        maturity_time: 600.0,

        loot_table: vec![
            LootEntry {
                item_id: "rabbit_meat".to_string(),
                chance: 1.0,
                quantity_min: 1,
                quantity_max: 2,
            },
            LootEntry {
                item_id: "rabbit_hide".to_string(),
                chance: 0.8,
                quantity_min: 1,
                quantity_max: 1,
            },
            LootEntry {
                item_id: "rabbit_foot".to_string(),
                chance: 0.1,
                quantity_min: 1,
                quantity_max: 1,
            },
        ],
    }
}

fn fox() -> WildlifeSpecies {
    WildlifeSpecies {
        id: "fox".to_string(),
        name: "Fox".to_string(),
        description: "A cunning small predator with russet fur.".to_string(),

        diet_type: DietType::Hybrid,
        size_class: SizeClass::Small,

        base_speed: 2.5,
        flee_speed_multiplier: 2.0,
        swim_capable: true,
        climb_capable: false,

        attack_damage: 8.0,
        attack_range: 1.0,
        attack_cooldown: 1.5,
        max_health: 35.0,

        sight_range: 35.0,
        hearing_range: 45.0,
        smell_range: 50.0,

        need_decay_rates: NeedDecayRates {
            hunger: 0.08,
            thirst: 0.10,
            energy: 0.06,
            reproduction: -0.2,
        },
        preferred_food: vec![
            "rabbit_meat".to_string(),
            "raw_meat".to_string(),
            "fish".to_string(),
        ],
        is_herbivore: false,
        is_carnivore: true,

        biome_preferences: vec![
            BiomePreference { biome: BiomeType::Forest, comfort: 95.0, spawn_weight: 8.0 },
            BiomePreference { biome: BiomeType::Grassland, comfort: 85.0, spawn_weight: 5.0 },
            BiomePreference { biome: BiomeType::Mountain, comfort: 60.0, spawn_weight: 2.0 },
            BiomePreference { biome: BiomeType::Urban, comfort: 55.0, spawn_weight: 2.0 },
        ],
        nocturnal: true,
        social_behavior: SocialBehavior::Solitary,
        pack_size_min: None,
        pack_size_max: None,

        gestation_time: 600.0,
        offspring_min: 2,
        offspring_max: 5,
        maturity_time: 900.0,

        loot_table: vec![
            LootEntry {
                item_id: "fox_pelt".to_string(),
                chance: 1.0,
                quantity_min: 1,
                quantity_max: 1,
            },
            LootEntry {
                item_id: "raw_meat".to_string(),
                chance: 0.9,
                quantity_min: 1,
                quantity_max: 2,
            },
            LootEntry {
                item_id: "fox_tail".to_string(),
                chance: 0.3,
                quantity_min: 1,
                quantity_max: 1,
            },
        ],
    }
}
