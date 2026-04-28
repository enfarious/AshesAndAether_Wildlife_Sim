#![allow(dead_code)]

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
    map.insert("fox".to_string(),    fox());
    map.insert("deer".to_string(),   deer());
    map.insert("wolf".to_string(),   wolf());
    map.insert("boar".to_string(),   boar());

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

        base_speed: 3.5,
        flee_speed_multiplier: 3.0,    // Rabbits are explosive sprinters
        swim_capable: false,
        climb_capable: false,

        attack_damage: 2.0,
        attack_range: 0.5,
        attack_cooldown: 2.0,
        max_health: 15.0,

        sight_range: 50.0,
        hearing_range: 80.0,   // Huge ears — excellent hearing
        smell_range: 30.0,

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

        base_speed: 4.0,
        flee_speed_multiplier: 2.2,
        swim_capable: true,
        climb_capable: false,

        attack_damage: 8.0,
        attack_range: 1.0,
        attack_cooldown: 1.5,
        max_health: 35.0,

        sight_range: 80.0,
        hearing_range: 90.0,
        smell_range: 100.0,   // Excellent nose

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

fn deer() -> WildlifeSpecies {
    WildlifeSpecies {
        id: "deer".to_string(),
        name: "Deer".to_string(),
        description: "A graceful herbivore with large brown eyes. Timid but swift.".to_string(),

        diet_type: DietType::Prey,
        size_class: SizeClass::Medium,

        base_speed: 5.0,
        flee_speed_multiplier: 2.8,    // 14 m/s flee (~50 km/h, close to real deer)
        swim_capable: true,
        climb_capable: false,

        attack_damage: 12.0,
        attack_range: 1.5,
        attack_cooldown: 2.5,
        max_health: 60.0,

        sight_range: 120.0,   // Deer spot movement from far away
        hearing_range: 100.0,
        smell_range: 60.0,

        need_decay_rates: NeedDecayRates {
            hunger: 0.07,
            thirst: 0.09,
            energy: 0.05,
            reproduction: -0.012,
        },
        preferred_food: vec![
            "grass".to_string(),
            "clover".to_string(),
            "berry".to_string(),
            "apple".to_string(),
            "pear".to_string(),
            "carrot".to_string(),
        ],
        is_herbivore: true,
        is_carnivore: false,

        biome_preferences: vec![
            BiomePreference { biome: BiomeType::Forest,    comfort: 95.0, spawn_weight: 9.0 },
            BiomePreference { biome: BiomeType::Grassland, comfort: 85.0, spawn_weight: 7.0 },
            BiomePreference { biome: BiomeType::Mountain,  comfort: 60.0, spawn_weight: 2.0 },
            BiomePreference { biome: BiomeType::Swamp,     comfort: 40.0, spawn_weight: 1.0 },
            BiomePreference { biome: BiomeType::Coastal,   comfort: 45.0, spawn_weight: 2.0 },
            BiomePreference { biome: BiomeType::Tundra,    comfort: 35.0, spawn_weight: 1.0 },
        ],
        nocturnal: false,
        social_behavior: SocialBehavior::Herd,
        pack_size_min: Some(2),
        pack_size_max: Some(6),

        gestation_time: 900.0,
        offspring_min: 1,
        offspring_max: 2,
        maturity_time: 1200.0,

        loot_table: vec![
            LootEntry { item_id: "venison".to_string(),     chance: 1.0, quantity_min: 3, quantity_max: 6 },
            LootEntry { item_id: "deer_hide".to_string(),   chance: 0.9, quantity_min: 1, quantity_max: 2 },
            LootEntry { item_id: "deer_antler".to_string(), chance: 0.4, quantity_min: 1, quantity_max: 2 },
            LootEntry { item_id: "deer_sinew".to_string(),  chance: 0.6, quantity_min: 1, quantity_max: 3 },
        ],
    }
}

fn wolf() -> WildlifeSpecies {
    WildlifeSpecies {
        id: "wolf".to_string(),
        name: "Wolf".to_string(),
        description: "A powerful pack hunter with grey fur and piercing yellow eyes.".to_string(),

        diet_type: DietType::Predator,
        size_class: SizeClass::Medium,

        base_speed: 5.5,
        flee_speed_multiplier: 1.6,
        swim_capable: true,
        climb_capable: false,

        attack_damage: 18.0,
        attack_range: 1.2,
        attack_cooldown: 1.2,
        max_health: 75.0,

        sight_range: 100.0,
        hearing_range: 120.0,
        smell_range: 150.0,   // Wolves track by scent from very far

        need_decay_rates: NeedDecayRates {
            hunger: 0.055,
            thirst: 0.07,
            energy: 0.045,
            reproduction: -0.01,
        },
        preferred_food: vec![
            "venison".to_string(),
            "rabbit_meat".to_string(),
            "raw_meat".to_string(),
        ],
        is_herbivore: false,
        is_carnivore: true,

        biome_preferences: vec![
            BiomePreference { biome: BiomeType::Forest,    comfort: 95.0, spawn_weight: 8.0 },
            BiomePreference { biome: BiomeType::Mountain,  comfort: 80.0, spawn_weight: 5.0 },
            BiomePreference { biome: BiomeType::Tundra,    comfort: 70.0, spawn_weight: 4.0 },
            BiomePreference { biome: BiomeType::Grassland, comfort: 65.0, spawn_weight: 3.0 },
            BiomePreference { biome: BiomeType::Swamp,     comfort: 40.0, spawn_weight: 1.0 },
        ],
        nocturnal: false,
        social_behavior: SocialBehavior::Pack,
        pack_size_min: Some(3),
        pack_size_max: Some(7),

        gestation_time: 1200.0,
        offspring_min: 2,
        offspring_max: 6,
        maturity_time: 1500.0,

        loot_table: vec![
            LootEntry { item_id: "wolf_pelt".to_string(), chance: 1.0, quantity_min: 1, quantity_max: 1 },
            LootEntry { item_id: "raw_meat".to_string(),  chance: 0.9, quantity_min: 2, quantity_max: 4 },
            LootEntry { item_id: "wolf_fang".to_string(), chance: 0.4, quantity_min: 1, quantity_max: 2 },
            LootEntry { item_id: "wolf_claw".to_string(), chance: 0.3, quantity_min: 1, quantity_max: 2 },
        ],
    }
}

fn boar() -> WildlifeSpecies {
    WildlifeSpecies {
        id: "boar".to_string(),
        name: "Wild Boar".to_string(),
        description: "A stocky, bristle-haired omnivore with curved tusks. Bad-tempered and fast.".to_string(),

        diet_type: DietType::Hybrid,
        size_class: SizeClass::Medium,

        base_speed: 3.5,
        flee_speed_multiplier: 2.2,
        swim_capable: true,
        climb_capable: false,

        attack_damage: 22.0,
        attack_range: 1.5,
        attack_cooldown: 1.8,
        max_health: 90.0,

        sight_range: 35.0,    // Bad eyesight (still their weakness)
        hearing_range: 70.0,
        smell_range: 110.0,   // Excellent nose — main detection sense

        need_decay_rates: NeedDecayRates {
            hunger: 0.09,
            thirst: 0.08,
            energy: 0.06,
            reproduction: -0.01,
        },
        preferred_food: vec![
            "potato".to_string(),
            "carrot".to_string(),
            "onion".to_string(),
            "apple".to_string(),
            "mushroom".to_string(),
            "raw_meat".to_string(),
        ],
        is_herbivore: false,
        is_carnivore: false, // Omnivore — uses preferred_food list directly

        biome_preferences: vec![
            BiomePreference { biome: BiomeType::Forest,    comfort: 95.0, spawn_weight: 9.0 },
            BiomePreference { biome: BiomeType::Swamp,     comfort: 75.0, spawn_weight: 5.0 },
            BiomePreference { biome: BiomeType::Grassland, comfort: 65.0, spawn_weight: 4.0 },
            BiomePreference { biome: BiomeType::Mountain,  comfort: 50.0, spawn_weight: 2.0 },
            BiomePreference { biome: BiomeType::Urban,     comfort: 30.0, spawn_weight: 1.0 },
            BiomePreference { biome: BiomeType::Coastal,   comfort: 40.0, spawn_weight: 1.0 },
        ],
        nocturnal: false,
        social_behavior: SocialBehavior::Solitary,
        pack_size_min: None,
        pack_size_max: None,

        gestation_time: 600.0,
        offspring_min: 2,
        offspring_max: 5,
        maturity_time: 900.0,

        loot_table: vec![
            LootEntry { item_id: "pork".to_string(),         chance: 1.0,  quantity_min: 2, quantity_max: 5 },
            LootEntry { item_id: "boar_hide".to_string(),    chance: 0.85, quantity_min: 1, quantity_max: 2 },
            LootEntry { item_id: "boar_tusk".to_string(),    chance: 0.45, quantity_min: 1, quantity_max: 2 },
            LootEntry { item_id: "boar_bristle".to_string(), chance: 0.6,  quantity_min: 1, quantity_max: 4 },
        ],
    }
}
