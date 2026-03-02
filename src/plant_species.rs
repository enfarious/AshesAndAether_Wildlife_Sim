#![allow(dead_code)]

//! Plant species definitions
//!
//! Defines vegetables, fruit trees, grasses, and herbs with their growth parameters.

use crate::climate_subscriber::Season;
use crate::types::{BiomeType, PlantGrowthStage};
use std::collections::HashMap;
use std::sync::LazyLock;

/// Type of plant
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlantType {
    Grass,
    Vegetable,
    Herb,
    FruitTree,
}

/// Configuration for a growth stage
#[derive(Debug, Clone)]
pub struct GrowthStageConfig {
    pub stage: PlantGrowthStage,
    /// How long this stage lasts as fraction of total growth time (0.0-1.0)
    pub duration_ratio: f64,
    /// Can herbivores eat this plant at this stage?
    pub can_be_eaten: bool,
    /// Can players harvest at this stage?
    pub can_be_harvested: bool,
    /// Food value if eaten at this stage
    pub food_value: f64,
}

/// Definition of a plant species
#[derive(Debug, Clone)]
pub struct PlantSpecies {
    pub id: &'static str,
    pub name: &'static str,
    pub plant_type: PlantType,

    /// Total time to grow from seed to mature (in game seconds)
    pub total_growth_time: f64,
    /// Growth stage configurations
    pub growth_stages: Vec<GrowthStageConfig>,

    /// Seasons when this plant can grow
    pub growing_seasons: Vec<Season>,
    /// Seasons when harvest is available (for fruit trees)
    pub harvest_seasons: Vec<Season>,
    /// Does the plant go dormant in winter?
    pub dormant_in_winter: bool,

    /// Item ID dropped when harvested
    pub harvest_item: &'static str,
    /// Min-max yield when harvested
    pub harvest_yield: (u32, u32),
    /// Does the plant regrow after harvest?
    pub regrows_after_harvest: bool,
    /// If regrows, what stage does it reset to?
    pub regrow_stage: PlantGrowthStage,

    /// Biomes where this plant naturally spawns
    pub preferred_biomes: Vec<BiomeType>,
    /// Spawn weight (higher = more common)
    pub spawn_weight: f64,
}

/// Global plant species registry
pub static PLANT_SPECIES: LazyLock<HashMap<&'static str, PlantSpecies>> = LazyLock::new(|| {
    let mut map = HashMap::new();

    map.insert("grass",       grass());
    map.insert("carrot",      carrot());
    map.insert("potato",      potato());
    map.insert("onion",       onion());
    map.insert("garlic",      garlic());
    map.insert("apple_tree",  apple_tree());
    map.insert("pear_tree",   pear_tree());
    map.insert("clover",      clover());
    map.insert("herb_sage",   herb_sage());
    map.insert("mushroom",    mushroom());
    map.insert("berry_bush",  berry_bush());

    map
});

pub fn get_plant_species(id: &str) -> Option<&'static PlantSpecies> {
    PLANT_SPECIES.get(id)
}

pub fn all_plant_species() -> impl Iterator<Item = &'static PlantSpecies> {
    PLANT_SPECIES.values()
}

// ============================================================================
// Plant Definitions
// ============================================================================

fn grass() -> PlantSpecies {
    PlantSpecies {
        id: "grass",
        name: "Grass",
        plant_type: PlantType::Grass,

        total_growth_time: 60.0, // 1 minute game time
        growth_stages: vec![
            GrowthStageConfig {
                stage: PlantGrowthStage::Sprout,
                duration_ratio: 0.3,
                can_be_eaten: false,
                can_be_harvested: false,
                food_value: 0.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Growing,
                duration_ratio: 0.4,
                can_be_eaten: true,
                can_be_harvested: false,
                food_value: 5.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Mature,
                duration_ratio: 0.3,
                can_be_eaten: true,
                can_be_harvested: false,
                food_value: 10.0,
            },
        ],

        growing_seasons: vec![Season::Spring, Season::Summer, Season::Fall],
        harvest_seasons: vec![],
        dormant_in_winter: true,

        harvest_item: "grass",
        harvest_yield: (1, 1),
        regrows_after_harvest: true,
        regrow_stage: PlantGrowthStage::Sprout,

        preferred_biomes: vec![BiomeType::Grassland, BiomeType::Forest, BiomeType::Coastal],
        spawn_weight: 10.0,
    }
}

fn carrot() -> PlantSpecies {
    PlantSpecies {
        id: "carrot",
        name: "Wild Carrot",
        plant_type: PlantType::Vegetable,

        total_growth_time: 300.0, // 5 minutes game time
        growth_stages: vec![
            GrowthStageConfig {
                stage: PlantGrowthStage::Seed,
                duration_ratio: 0.1,
                can_be_eaten: false,
                can_be_harvested: false,
                food_value: 0.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Sprout,
                duration_ratio: 0.2,
                can_be_eaten: false,
                can_be_harvested: false,
                food_value: 0.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Growing,
                duration_ratio: 0.4,
                can_be_eaten: true,
                can_be_harvested: false,
                food_value: 10.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Mature,
                duration_ratio: 0.3,
                can_be_eaten: true,
                can_be_harvested: true,
                food_value: 20.0,
            },
        ],

        growing_seasons: vec![Season::Spring, Season::Summer, Season::Fall],
        harvest_seasons: vec![Season::Summer, Season::Fall],
        dormant_in_winter: true,

        harvest_item: "carrot",
        harvest_yield: (1, 3),
        regrows_after_harvest: false,
        regrow_stage: PlantGrowthStage::Dead,

        preferred_biomes: vec![BiomeType::Grassland, BiomeType::Forest],
        spawn_weight: 3.0,
    }
}

fn potato() -> PlantSpecies {
    PlantSpecies {
        id: "potato",
        name: "Wild Potato",
        plant_type: PlantType::Vegetable,

        total_growth_time: 400.0,
        growth_stages: vec![
            GrowthStageConfig {
                stage: PlantGrowthStage::Seed,
                duration_ratio: 0.15,
                can_be_eaten: false,
                can_be_harvested: false,
                food_value: 0.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Sprout,
                duration_ratio: 0.2,
                can_be_eaten: false,
                can_be_harvested: false,
                food_value: 0.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Growing,
                duration_ratio: 0.35,
                can_be_eaten: false,
                can_be_harvested: false,
                food_value: 0.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Mature,
                duration_ratio: 0.3,
                can_be_eaten: false, // Underground, animals can't eat
                can_be_harvested: true,
                food_value: 0.0,
            },
        ],

        growing_seasons: vec![Season::Spring, Season::Summer],
        harvest_seasons: vec![Season::Summer, Season::Fall],
        dormant_in_winter: true,

        harvest_item: "potato",
        harvest_yield: (2, 5),
        regrows_after_harvest: false,
        regrow_stage: PlantGrowthStage::Dead,

        preferred_biomes: vec![BiomeType::Grassland, BiomeType::Forest],
        spawn_weight: 2.0,
    }
}

fn onion() -> PlantSpecies {
    PlantSpecies {
        id: "onion",
        name: "Wild Onion",
        plant_type: PlantType::Vegetable,

        total_growth_time: 350.0,
        growth_stages: vec![
            GrowthStageConfig {
                stage: PlantGrowthStage::Seed,
                duration_ratio: 0.1,
                can_be_eaten: false,
                can_be_harvested: false,
                food_value: 0.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Sprout,
                duration_ratio: 0.25,
                can_be_eaten: true,
                can_be_harvested: false,
                food_value: 5.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Growing,
                duration_ratio: 0.35,
                can_be_eaten: true,
                can_be_harvested: false,
                food_value: 10.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Mature,
                duration_ratio: 0.3,
                can_be_eaten: true,
                can_be_harvested: true,
                food_value: 15.0,
            },
        ],

        growing_seasons: vec![Season::Spring, Season::Summer, Season::Fall],
        harvest_seasons: vec![Season::Summer, Season::Fall],
        dormant_in_winter: true,

        harvest_item: "onion",
        harvest_yield: (1, 2),
        regrows_after_harvest: true,
        regrow_stage: PlantGrowthStage::Sprout,

        preferred_biomes: vec![BiomeType::Grassland, BiomeType::Forest, BiomeType::Mountain],
        spawn_weight: 2.5,
    }
}

fn garlic() -> PlantSpecies {
    PlantSpecies {
        id: "garlic",
        name: "Wild Garlic",
        plant_type: PlantType::Vegetable,

        total_growth_time: 450.0,
        growth_stages: vec![
            GrowthStageConfig {
                stage: PlantGrowthStage::Seed,
                duration_ratio: 0.15,
                can_be_eaten: false,
                can_be_harvested: false,
                food_value: 0.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Sprout,
                duration_ratio: 0.2,
                can_be_eaten: true,
                can_be_harvested: false,
                food_value: 3.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Growing,
                duration_ratio: 0.35,
                can_be_eaten: true,
                can_be_harvested: false,
                food_value: 8.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Mature,
                duration_ratio: 0.3,
                can_be_eaten: true,
                can_be_harvested: true,
                food_value: 12.0,
            },
        ],

        growing_seasons: vec![Season::Fall, Season::Spring], // Planted in fall, harvested in spring
        harvest_seasons: vec![Season::Spring, Season::Summer],
        dormant_in_winter: false, // Overwinters

        harvest_item: "garlic",
        harvest_yield: (1, 4),
        regrows_after_harvest: false,
        regrow_stage: PlantGrowthStage::Dead,

        preferred_biomes: vec![BiomeType::Forest, BiomeType::Grassland],
        spawn_weight: 1.5,
    }
}

fn apple_tree() -> PlantSpecies {
    PlantSpecies {
        id: "apple_tree",
        name: "Apple Tree",
        plant_type: PlantType::FruitTree,

        total_growth_time: 1800.0, // 30 minutes to mature
        growth_stages: vec![
            GrowthStageConfig {
                stage: PlantGrowthStage::Seed,
                duration_ratio: 0.05,
                can_be_eaten: false,
                can_be_harvested: false,
                food_value: 0.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Sprout,
                duration_ratio: 0.15,
                can_be_eaten: false,
                can_be_harvested: false,
                food_value: 0.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Growing,
                duration_ratio: 0.4,
                can_be_eaten: false,
                can_be_harvested: false,
                food_value: 0.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Mature,
                duration_ratio: 0.1, // Briefly mature before flowering
                can_be_eaten: false,
                can_be_harvested: false,
                food_value: 0.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Flowering,
                duration_ratio: 0.3, // Flowering = fruiting for trees
                can_be_eaten: true, // Animals can eat fallen fruit
                can_be_harvested: true,
                food_value: 25.0,
            },
        ],

        growing_seasons: vec![Season::Spring, Season::Summer, Season::Fall],
        harvest_seasons: vec![Season::Summer, Season::Fall],
        dormant_in_winter: true,

        harvest_item: "apple",
        harvest_yield: (3, 8),
        regrows_after_harvest: true,
        regrow_stage: PlantGrowthStage::Mature, // Returns to mature, flowers again next season

        preferred_biomes: vec![BiomeType::Forest, BiomeType::Grassland],
        spawn_weight: 0.5,
    }
}

fn pear_tree() -> PlantSpecies {
    PlantSpecies {
        id: "pear_tree",
        name: "Pear Tree",
        plant_type: PlantType::FruitTree,

        total_growth_time: 2000.0,
        growth_stages: vec![
            GrowthStageConfig {
                stage: PlantGrowthStage::Seed,
                duration_ratio: 0.05,
                can_be_eaten: false,
                can_be_harvested: false,
                food_value: 0.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Sprout,
                duration_ratio: 0.15,
                can_be_eaten: false,
                can_be_harvested: false,
                food_value: 0.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Growing,
                duration_ratio: 0.4,
                can_be_eaten: false,
                can_be_harvested: false,
                food_value: 0.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Mature,
                duration_ratio: 0.1,
                can_be_eaten: false,
                can_be_harvested: false,
                food_value: 0.0,
            },
            GrowthStageConfig {
                stage: PlantGrowthStage::Flowering,
                duration_ratio: 0.3,
                can_be_eaten: true,
                can_be_harvested: true,
                food_value: 22.0,
            },
        ],

        growing_seasons: vec![Season::Spring, Season::Summer, Season::Fall],
        harvest_seasons: vec![Season::Fall],
        dormant_in_winter: true,

        harvest_item: "pear",
        harvest_yield: (2, 6),
        regrows_after_harvest: true,
        regrow_stage: PlantGrowthStage::Mature,

        preferred_biomes: vec![BiomeType::Forest, BiomeType::Grassland],
        spawn_weight: 0.4,
    }
}

fn clover() -> PlantSpecies {
    PlantSpecies {
        id: "clover",
        name: "Clover",
        plant_type: PlantType::Herb,

        total_growth_time: 315.0,
        growth_stages: vec![
            GrowthStageConfig { stage: PlantGrowthStage::Seed,      duration_ratio: 0.14, can_be_eaten: false, can_be_harvested: false, food_value: 0.0 },
            GrowthStageConfig { stage: PlantGrowthStage::Sprout,    duration_ratio: 0.29, can_be_eaten: false, can_be_harvested: false, food_value: 0.0 },
            GrowthStageConfig { stage: PlantGrowthStage::Growing,   duration_ratio: 0.29, can_be_eaten: true,  can_be_harvested: false, food_value: 12.0 },
            GrowthStageConfig { stage: PlantGrowthStage::Mature,    duration_ratio: 0.20, can_be_eaten: true,  can_be_harvested: true,  food_value: 25.0 },
            GrowthStageConfig { stage: PlantGrowthStage::Flowering, duration_ratio: 0.08, can_be_eaten: true,  can_be_harvested: true,  food_value: 28.0 },
        ],

        growing_seasons: vec![Season::Spring, Season::Summer, Season::Fall],
        harvest_seasons: vec![Season::Summer],
        dormant_in_winter: true,

        harvest_item: "clover",
        harvest_yield: (2, 5),
        regrows_after_harvest: true,
        regrow_stage: PlantGrowthStage::Growing,

        preferred_biomes: vec![BiomeType::Grassland, BiomeType::Forest, BiomeType::Coastal],
        spawn_weight: 8.0,
    }
}

fn herb_sage() -> PlantSpecies {
    PlantSpecies {
        id: "herb_sage",
        name: "Wild Sage",
        plant_type: PlantType::Herb,

        total_growth_time: 480.0,
        growth_stages: vec![
            GrowthStageConfig { stage: PlantGrowthStage::Seed,      duration_ratio: 0.12, can_be_eaten: false, can_be_harvested: false, food_value: 0.0 },
            GrowthStageConfig { stage: PlantGrowthStage::Sprout,    duration_ratio: 0.25, can_be_eaten: false, can_be_harvested: false, food_value: 0.0 },
            GrowthStageConfig { stage: PlantGrowthStage::Growing,   duration_ratio: 0.31, can_be_eaten: true,  can_be_harvested: false, food_value: 5.0 },
            GrowthStageConfig { stage: PlantGrowthStage::Mature,    duration_ratio: 0.19, can_be_eaten: true,  can_be_harvested: true,  food_value: 10.0 },
            GrowthStageConfig { stage: PlantGrowthStage::Flowering, duration_ratio: 0.13, can_be_eaten: true,  can_be_harvested: true,  food_value: 12.0 },
        ],

        growing_seasons: vec![Season::Spring, Season::Summer, Season::Fall],
        harvest_seasons: vec![Season::Summer, Season::Fall],
        dormant_in_winter: true,

        harvest_item: "herb_sage",
        harvest_yield: (1, 3),
        regrows_after_harvest: true,
        regrow_stage: PlantGrowthStage::Growing,

        preferred_biomes: vec![BiomeType::Grassland, BiomeType::Mountain, BiomeType::Forest],
        spawn_weight: 2.5,
    }
}

fn mushroom() -> PlantSpecies {
    PlantSpecies {
        id: "mushroom",
        name: "Forest Mushroom",
        plant_type: PlantType::Herb,  // Closest existing type; Mushroom type could be added later

        total_growth_time: 180.0,
        growth_stages: vec![
            GrowthStageConfig { stage: PlantGrowthStage::Sprout,    duration_ratio: 0.33, can_be_eaten: false, can_be_harvested: false, food_value: 0.0 },
            GrowthStageConfig { stage: PlantGrowthStage::Growing,   duration_ratio: 0.34, can_be_eaten: false, can_be_harvested: false, food_value: 0.0 },
            GrowthStageConfig { stage: PlantGrowthStage::Mature,    duration_ratio: 0.33, can_be_eaten: true,  can_be_harvested: true,  food_value: 25.0 },
        ],

        growing_seasons: vec![Season::Spring, Season::Summer, Season::Fall],
        harvest_seasons: vec![Season::Spring, Season::Summer, Season::Fall],
        dormant_in_winter: true,

        harvest_item: "mushroom",
        harvest_yield: (1, 3),
        regrows_after_harvest: true,
        regrow_stage: PlantGrowthStage::Sprout,

        preferred_biomes: vec![BiomeType::Forest, BiomeType::Swamp, BiomeType::Underground],
        spawn_weight: 4.0,
    }
}

fn berry_bush() -> PlantSpecies {
    PlantSpecies {
        id: "berry_bush",
        name: "Berry Bush",
        plant_type: PlantType::Herb,  // Bush type could be added later

        total_growth_time: 720.0,
        growth_stages: vec![
            GrowthStageConfig { stage: PlantGrowthStage::Sprout,    duration_ratio: 0.17, can_be_eaten: false, can_be_harvested: false, food_value: 0.0 },
            GrowthStageConfig { stage: PlantGrowthStage::Growing,   duration_ratio: 0.28, can_be_eaten: false, can_be_harvested: false, food_value: 0.0 },
            GrowthStageConfig { stage: PlantGrowthStage::Mature,    duration_ratio: 0.22, can_be_eaten: false, can_be_harvested: false, food_value: 0.0 },
            GrowthStageConfig { stage: PlantGrowthStage::Flowering, duration_ratio: 0.33, can_be_eaten: true,  can_be_harvested: true,  food_value: 20.0 },
        ],

        growing_seasons: vec![Season::Spring, Season::Summer, Season::Fall],
        harvest_seasons: vec![Season::Summer, Season::Fall],
        dormant_in_winter: true,

        harvest_item: "berries",
        harvest_yield: (3, 8),
        regrows_after_harvest: true,
        regrow_stage: PlantGrowthStage::Mature,

        preferred_biomes: vec![BiomeType::Forest, BiomeType::Grassland, BiomeType::Coastal],
        spawn_weight: 3.5,
    }
}
