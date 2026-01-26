//! Behavior decision system for wildlife
//!
//! Evaluates entity needs, environment, and threats to select appropriate behavior.

use crate::types::*;
use crate::climate::Season;

/// Context about the environment for behavior decisions
pub struct EnvironmentContext {
    pub current_biome: BiomeType,
    pub biome_comfort: f64,
    pub time_of_day: f64,
    pub is_night: bool,
    pub season: Season,
    pub temperature: f64,
    pub nearby_threats: Vec<PerceivedEntity>,
    pub nearby_prey: Vec<PerceivedEntity>,
    pub nearby_food: Vec<PerceivedEntity>,
    pub nearby_water: Vec<WaterSource>,
    pub nearby_mates: Vec<PerceivedEntity>,
    pub nearby_hazards: Vec<PerceivedEntity>,
}

/// An entity perceived by wildlife
#[derive(Clone)]
pub struct PerceivedEntity {
    pub id: String,
    pub position: Vector3,
    pub distance: f64,
    pub size_class: SizeClass,
    pub diet_type: Option<DietType>,
    pub species_id: Option<String>,
    pub is_player: bool,
}

/// A water source location
pub struct WaterSource {
    pub position: Vector3,
    pub distance: f64,
}

/// Result of behavior selection
pub struct BehaviorDecision {
    pub behavior: BehaviorState,
    pub target_id: Option<String>,
    pub target_position: Option<Vector3>,
    pub priority: i32,
}

/// Select the best behavior for an entity given its state and environment
pub fn select_behavior(
    entity: &WildlifeEntity,
    species: &WildlifeSpecies,
    context: &EnvironmentContext,
) -> BehaviorDecision {
    let mut candidates = Vec::new();

    // Dead entities stay dead
    if !entity.is_alive {
        return BehaviorDecision {
            behavior: BehaviorState::Dead,
            target_id: None,
            target_position: None,
            priority: 100,
        };
    }

    // Evaluate each possible behavior
    // Climate/weather threats take highest priority
    if let Some(decision) = evaluate_hibernation(entity, species, context) {
        candidates.push(decision);
    }
    if let Some(decision) = evaluate_storm_fleeing(entity, context) {
        candidates.push(decision);
    }
    
    // Standard survival behaviors
    if let Some(decision) = evaluate_flee(entity, species, context) {
        candidates.push(decision);
    }
    if let Some(decision) = evaluate_cold_stress(entity, context) {
        candidates.push(decision);
    }
    if let Some(decision) = evaluate_heat_stress(entity, context) {
        candidates.push(decision);
    }
    if let Some(decision) = evaluate_desperate_hunting(entity, species, context) {
        candidates.push(decision);
    }
    if let Some(decision) = evaluate_hunt(entity, species, context) {
        candidates.push(decision);
    }
    if let Some(decision) = evaluate_drink(entity, species, context) {
        candidates.push(decision);
    }
    if let Some(decision) = evaluate_eat(entity, species, context) {
        candidates.push(decision);
    }
    if let Some(decision) = evaluate_forage(entity, species, context) {
        candidates.push(decision);
    }
    if let Some(decision) = evaluate_rest(entity, species, context) {
        candidates.push(decision);
    }
    if let Some(decision) = evaluate_mate(entity, species, context) {
        candidates.push(decision);
    }

    // Default behaviors
    candidates.push(BehaviorDecision {
        behavior: BehaviorState::Wandering,
        target_id: None,
        target_position: None,
        priority: 20,
    });
    candidates.push(BehaviorDecision {
        behavior: BehaviorState::Idle,
        target_id: None,
        target_position: None,
        priority: 10,
    });

    // Select highest priority
    candidates.sort_by(|a, b| b.priority.cmp(&a.priority));
    candidates.into_iter().next().unwrap()
}

fn evaluate_flee(
    entity: &WildlifeEntity,
    species: &WildlifeSpecies,
    context: &EnvironmentContext,
) -> Option<BehaviorDecision> {
    let nearest_threat = context.nearby_threats.first()?;

    let mut urgency = match species.diet_type {
        DietType::Prey => 90,
        DietType::Hybrid => 70,
        DietType::Predator => {
            // Predators only flee if hurt
            if entity.current_health < entity.max_health * 0.3 {
                60
            } else {
                return None;
            }
        }
    };

    // Closer threats are more urgent
    if nearest_threat.distance < species.sight_range * 0.3 {
        urgency += 15;
    } else if nearest_threat.distance < species.sight_range * 0.6 {
        urgency += 5;
    }

    // Already fleeing? Maintain
    if entity.current_behavior == BehaviorState::Fleeing {
        urgency += 10;
    }

    Some(BehaviorDecision {
        behavior: BehaviorState::Fleeing,
        target_id: Some(nearest_threat.id.clone()),
        target_position: Some(nearest_threat.position),
        priority: urgency,
    })
}

fn evaluate_hunt(
    entity: &WildlifeEntity,
    species: &WildlifeSpecies,
    context: &EnvironmentContext,
) -> Option<BehaviorDecision> {
    // Only predators and hybrids hunt
    if species.diet_type == DietType::Prey {
        return None;
    }

    // Need to be hungry
    if entity.needs.hunger > thresholds::NEED_COMFORTABLE {
        return None;
    }

    // Don't hunt if threatened
    if !context.nearby_threats.is_empty() {
        let nearest = &context.nearby_threats[0];
        if nearest.distance < species.sight_range * 0.5 {
            return None;
        }
    }

    let prey = context.nearby_prey.first()?;

    let mut priority = 55;

    if entity.needs.hunger < thresholds::NEED_CRITICAL {
        priority += 25;
    } else if entity.needs.hunger < thresholds::NEED_LOW {
        priority += 15;
    }

    if prey.distance < species.sight_range * 0.3 {
        priority += 10;
    }

    let behavior = if prey.distance > species.attack_range * 3.0 {
        BehaviorState::Stalking
    } else {
        BehaviorState::Hunting
    };

    Some(BehaviorDecision {
        behavior,
        target_id: Some(prey.id.clone()),
        target_position: Some(prey.position),
        priority,
    })
}

fn evaluate_drink(
    entity: &WildlifeEntity,
    _species: &WildlifeSpecies,
    context: &EnvironmentContext,
) -> Option<BehaviorDecision> {
    if entity.needs.thirst > thresholds::NEED_LOW {
        return None;
    }

    let mut priority = 50;
    if entity.needs.thirst < thresholds::NEED_CRITICAL {
        priority = 85;
    }

    let water = context.nearby_water.first();

    match water {
        Some(w) if w.distance < 2.0 => Some(BehaviorDecision {
            behavior: BehaviorState::Drinking,
            target_id: None,
            target_position: Some(w.position),
            priority,
        }),
        Some(w) => Some(BehaviorDecision {
            behavior: BehaviorState::Wandering,
            target_id: None,
            target_position: Some(w.position),
            priority: priority - 5,
        }),
        None => Some(BehaviorDecision {
            behavior: BehaviorState::Wandering,
            target_id: None,
            target_position: None,
            priority: priority - 20,
        }),
    }
}

fn evaluate_eat(
    entity: &WildlifeEntity,
    _species: &WildlifeSpecies,
    context: &EnvironmentContext,
) -> Option<BehaviorDecision> {
    if entity.needs.hunger > thresholds::NEED_LOW {
        return None;
    }

    let food = context.nearby_food.first()?;
    if food.distance > 2.0 {
        return None;
    }

    let mut priority = 65;
    if entity.needs.hunger < thresholds::NEED_CRITICAL {
        priority = 80;
    }

    Some(BehaviorDecision {
        behavior: BehaviorState::Eating,
        target_id: Some(food.id.clone()),
        target_position: Some(food.position),
        priority,
    })
}

fn evaluate_forage(
    entity: &WildlifeEntity,
    species: &WildlifeSpecies,
    context: &EnvironmentContext,
) -> Option<BehaviorDecision> {
    if !species.is_herbivore {
        return None;
    }

    if entity.needs.hunger > thresholds::NEED_COMFORTABLE {
        return None;
    }

    let mut priority = 45;
    if entity.needs.hunger < thresholds::NEED_LOW {
        priority = 60;
    }
    if entity.needs.hunger < thresholds::NEED_CRITICAL {
        priority = 75;
    }

    let food = context.nearby_food.first();
    let (target_id, target_position) = match food {
        Some(f) => (Some(f.id.clone()), Some(f.position)),
        None => (None, None),
    };

    Some(BehaviorDecision {
        behavior: BehaviorState::Foraging,
        target_id,
        target_position,
        priority,
    })
}

fn evaluate_rest(
    entity: &WildlifeEntity,
    species: &WildlifeSpecies,
    context: &EnvironmentContext,
) -> Option<BehaviorDecision> {
    if entity.needs.energy > thresholds::NEED_LOW {
        return None;
    }

    // Don't rest if threatened
    if !context.nearby_threats.is_empty() {
        return None;
    }

    let mut priority = 30;
    if entity.needs.energy < thresholds::NEED_CRITICAL {
        priority = 50;
    }

    // Nocturnal creatures rest during day
    if species.nocturnal && !context.is_night {
        priority += 20;
    }
    // Diurnal creatures rest at night
    if !species.nocturnal && context.is_night {
        priority += 20;
    }

    Some(BehaviorDecision {
        behavior: BehaviorState::Resting,
        target_id: None,
        target_position: None,
        priority,
    })
}

fn evaluate_mate(
    entity: &WildlifeEntity,
    species: &WildlifeSpecies,
    context: &EnvironmentContext,
) -> Option<BehaviorDecision> {
    if !entity.is_mature || entity.is_pregnant {
        return None;
    }

    // Reproduction urge must be high enough (increases over time toward 100)
    if entity.needs.reproduction < thresholds::NEED_COMFORTABLE {
        return None;
    }

    // Basic needs must be met (hunger/thirst must be above low threshold)
    if entity.needs.hunger < thresholds::NEED_LOW || entity.needs.thirst < thresholds::NEED_LOW {
        return None;
    }

    // Don't mate if threatened
    if !context.nearby_threats.is_empty() {
        return None;
    }

    let priority = (40.0 + (entity.needs.reproduction - 60.0) * 0.3) as i32;

    match context.nearby_mates.first() {
        Some(mate) if mate.distance < 2.0 => Some(BehaviorDecision {
            behavior: BehaviorState::Mating,
            target_id: Some(mate.id.clone()),
            target_position: Some(mate.position),
            priority,
        }),
        Some(mate) => Some(BehaviorDecision {
            behavior: BehaviorState::SeekingMate,
            target_id: Some(mate.id.clone()),
            target_position: Some(mate.position),
            priority: priority - 5,
        }),
        None => Some(BehaviorDecision {
            behavior: BehaviorState::SeekingMate,
            target_id: None,
            target_position: None,
            priority: priority - 10,
        }),
    }
}

fn evaluate_hibernation(
    entity: &WildlifeEntity,
    species: &WildlifeSpecies,
    context: &EnvironmentContext,
) -> Option<BehaviorDecision> {
    // Only evaluate hibernation in winter
    if context.season != Season::Winter {
        return None;
    }

    // Only hibernators (small size, low cold tolerance) and in extreme cold
    if context.temperature < -0.5 {
        let priority = 80; // Very high priority - overrides most behaviors
        return Some(BehaviorDecision {
            behavior: BehaviorState::Hibernating,
            target_id: None,
            target_position: None,
            priority,
        });
    }

    // Softer hibernation trigger for small species in moderate cold
    if context.temperature < -0.2 && matches!(species.size_class, SizeClass::Tiny | SizeClass::Small) {
        let priority = 75;
        return Some(BehaviorDecision {
            behavior: BehaviorState::Hibernating,
            target_id: None,
            target_position: None,
            priority,
        });
    }

    None
}

fn evaluate_cold_stress(
    entity: &WildlifeEntity,
    context: &EnvironmentContext,
) -> Option<BehaviorDecision> {
    // Only in winter with moderate-to-severe cold
    if context.temperature >= -0.3 {
        return None;
    }

    // Energy depletion becomes urgent in cold
    let hunger_urgency = if entity.needs.hunger < thresholds::NEED_LOW { 15 } else { 5 };
    let energy_urgency = if entity.needs.energy < thresholds::NEED_LOW { 20 } else { 10 };
    let priority = 55 + hunger_urgency + energy_urgency;

    Some(BehaviorDecision {
        behavior: BehaviorState::Resting, // Seek shelter/rest to conserve energy
        target_id: None,
        target_position: None,
        priority,
    })
}

fn evaluate_heat_stress(
    entity: &WildlifeEntity,
    context: &EnvironmentContext,
) -> Option<BehaviorDecision> {
    // Only in summer heat
    if context.temperature < 0.5 {
        return None;
    }

    // Find and seek water or shade
    let priority = 65;

    // Prefer water over wandering in heat
    match context.nearby_water.first() {
        Some(water) => Some(BehaviorDecision {
            behavior: BehaviorState::Drinking,
            target_id: None,
            target_position: Some(water.position),
            priority,
        }),
        None => Some(BehaviorDecision {
            behavior: BehaviorState::Resting, // Stay idle/seek shelter
            target_id: None,
            target_position: None,
            priority: priority - 5,
        }),
    }
}

fn evaluate_storm_fleeing(
    entity: &WildlifeEntity,
    context: &EnvironmentContext,
) -> Option<BehaviorDecision> {
    // Check for hazardous weather in environment
    // (This would be populated by weather events from Redis)
    let dangerous_weather = context.nearby_hazards.first()?;

    // High priority to flee storms
    let priority = 90;

    Some(BehaviorDecision {
        behavior: BehaviorState::Fleeing,
        target_id: None,
        target_position: Some(dangerous_weather.position), // Flee away from hazard center
        priority,
    })
}

fn evaluate_desperate_hunting(
    entity: &WildlifeEntity,
    species: &WildlifeSpecies,
    context: &EnvironmentContext,
) -> Option<BehaviorDecision> {
    // Only predators hunt
    if species.diet_type == DietType::Prey {
        return None;
    }

    // Only when prey is extremely scarce in the region
    if context.nearby_prey.len() > 2 {
        return None;
    }

    // Must be hungry
    if entity.needs.hunger > thresholds::NEED_LOW {
        return None;
    }

    let prey = context.nearby_prey.first()?;
    
    // Aggressive hunting mode
    let mut priority = 80; // Much higher than normal hunt
    
    if entity.needs.hunger < thresholds::NEED_CRITICAL {
        priority = 90;
    }

    Some(BehaviorDecision {
        behavior: BehaviorState::Hunting,
        target_id: Some(prey.id.clone()),
        target_position: Some(prey.position),
        priority,
    })
}

fn evaluate_breeding_season(
    entity: &WildlifeEntity,
    species: &WildlifeSpecies,
    context: &EnvironmentContext,
) -> Option<BehaviorDecision> {
    // Only during spring/summer
    match context.season {
        Season::Spring | Season::Summer => {
            // This function doesn't return a behavior but modifies mating priority
            // in the select_behavior function. Return None here and handle in select_behavior
            None
        }
        _ => None,
    }
}

fn evaluate_nocturnal_activity(
    entity: &WildlifeEntity,
    species: &WildlifeSpecies,
    context: &EnvironmentContext,
) -> Option<BehaviorDecision> {
    // This function modifies activity priority based on time of day
    // Nocturnal species get bonuses at night, diurnal during day
    // Return None here - handle in select_behavior by modifying base priorities
    None
}

// ============================================================================
// Threat/Prey Evaluation
// ============================================================================

/// Check if another entity is a threat
pub fn is_threat(
    self_species: &WildlifeSpecies,
    other_size: SizeClass,
    other_diet: Option<DietType>,
    is_player: bool,
) -> bool {
    let self_size = self_species.size_class.value();
    let other_size_val = other_size.value();

    // Players are always potential threats (except to large predators)
    if is_player {
        return self_species.diet_type != DietType::Predator || self_size < 4;
    }

    let other_diet = match other_diet {
        Some(d) => d,
        None => return false,
    };

    match self_species.diet_type {
        DietType::Prey => other_size_val >= self_size && other_diet != DietType::Prey,
        DietType::Hybrid => other_size_val > self_size && other_diet != DietType::Prey,
        DietType::Predator => other_size_val > self_size + 1 && other_diet == DietType::Predator,
    }
}

/// Check if another entity is valid prey
pub fn is_prey(self_species: &WildlifeSpecies, other_size: SizeClass) -> bool {
    if self_species.diet_type == DietType::Prey {
        return false;
    }

    let self_size = self_species.size_class.value();
    let other_size_val = other_size.value();

    other_size_val < self_size
}

// ============================================================================
// Movement Helpers
// ============================================================================

/// Calculate flee direction (away from threat)
pub fn flee_direction(entity_pos: &Vector3, threat_pos: &Vector3) -> f64 {
    let dx = entity_pos.x - threat_pos.x;
    let dz = entity_pos.z - threat_pos.z;
    let angle = dz.atan2(dx).to_degrees();
    (angle + 360.0) % 360.0
}

/// Calculate approach direction (toward target)
pub fn approach_direction(entity_pos: &Vector3, target_pos: &Vector3) -> f64 {
    let dx = target_pos.x - entity_pos.x;
    let dz = target_pos.z - entity_pos.z;
    let angle = dz.atan2(dx).to_degrees();
    (angle + 360.0) % 360.0
}

/// Calculate a wander direction with home bias
pub fn wander_direction(
    entity_pos: &Vector3,
    home_pos: Option<&Vector3>,
    current_heading: f64,
    rng: &mut impl rand::Rng,
) -> f64 {
    use rand::Rng;

    // Base: random offset from current heading
    let random_offset = rng.gen_range(-45.0..45.0);
    let mut heading = (current_heading + random_offset + 360.0) % 360.0;

    // Bias toward home if far away
    if let Some(home) = home_pos {
        let dist = entity_pos.distance_2d(home);
        if dist > 50.0 {
            let home_dir = approach_direction(entity_pos, home);
            let bias = (dist / 200.0).min(0.5);
            heading = blend_angles(heading, home_dir, bias);
        }
    }

    heading
}

fn blend_angles(a: f64, b: f64, weight: f64) -> f64 {
    let a_rad = a.to_radians();
    let b_rad = b.to_radians();

    let x = a_rad.cos() * (1.0 - weight) + b_rad.cos() * weight;
    let y = a_rad.sin() * (1.0 - weight) + b_rad.sin() * weight;

    (y.atan2(x).to_degrees() + 360.0) % 360.0
}
