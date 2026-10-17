//! Behavior decision system for wildlife
//!
//! Evaluates entity needs, environment, and threats to select appropriate behavior.

use crate::types::*;
use crate::climate::Season;

/// Context about the environment for behavior decisions
#[allow(dead_code)]
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
    /// Nearest shelter position (damaged/collapsed structures) from terrain.
    pub nearby_shelter: Option<Vector3>,
    /// Whether the entity is currently on a road cell.
    pub on_road: bool,
    /// Whether the entity is near dense vegetation or ruins (cover from threats).
    pub near_cover: bool,
    /// Wall clock (ms) — drives the idle-bout rhythm in select_behavior.
    pub now_ms: i64,
}

/// Idle bouts: animals in the wild spend most of their time standing —
/// grazing in place, watching, chewing — not pacing. Each animal runs its own
/// cycle (period + phase hashed from its id, so a herd doesn't freeze and
/// unfreeze in unison); inside the "still" part of the cycle Idle outranks
/// casual grazing and wandering, but never real needs (low hunger, thirst,
/// threats, sleep). Hunting depends on this: a target that never stops is
/// one you can't reach.
const BOUT_PERIOD_MIN_S: u64 = 30;
const BOUT_PERIOD_SPREAD_S: u64 = 25;
const BOUT_STILL_FRACTION: f64 = 0.6;
const BOUT_IDLE_PRIORITY: i32 = 62; // > comfortable grazing (60), < low hunger (80)

fn id_hash(id: &str) -> u64 {
    // FNV-1a: stable across runs (unlike std's RandomState).
    let mut h: u64 = 0xcbf29ce484222325;
    for b in id.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn in_idle_bout(entity: &WildlifeEntity, now_ms: i64) -> bool {
    let h = id_hash(&entity.id);
    let period = BOUT_PERIOD_MIN_S + h % BOUT_PERIOD_SPREAD_S;
    let phase = ((now_ms.max(0) as u64) / 1000 + (h >> 16)) % period;
    (phase as f64) < period as f64 * BOUT_STILL_FRACTION
}

/// An entity perceived by wildlife
#[allow(dead_code)]
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

/// Pick from a distance-sorted candidate list, preferring the target we have
/// already committed to.
///
/// A challenger must be BOTH `margin` (fraction) and `min_gain` (metres)
/// closer before it steals focus. Without that deadband, two candidates at
/// near-equal distance trade rank every tick as the entity drifts, and since
/// each carries a different `target_position` the animal re-aims left, right,
/// left before committing — the visible symptom this exists to kill.
///
/// PRECONDITION: `candidates` is sorted ascending by `distance`.
fn sticky_pick<'a>(
    candidates: &'a [PerceivedEntity],
    current_target: Option<&str>,
    margin: f64,
    min_gain: f64,
) -> Option<&'a PerceivedEntity> {
    let nearest = candidates.first()?;
    match current_target.and_then(|id| candidates.iter().find(|c| c.id == id)) {
        Some(cur) => {
            let gain = cur.distance - nearest.distance;
            if gain >= min_gain && nearest.distance < cur.distance * (1.0 - margin) {
                Some(nearest)
            } else {
                Some(cur)
            }
        }
        None => Some(nearest),
    }
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

    // Idle bout — stand still for a stretch unless something real is going on.
    // Not mid-meal or mid-drink (those are stationary anyway and own targets).
    if entity.current_behavior != BehaviorState::Eating
        && entity.current_behavior != BehaviorState::Drinking
        && in_idle_bout(entity, context.now_ms)
    {
        candidates.push(BehaviorDecision {
            behavior: BehaviorState::Idle,
            target_id: None,
            target_position: None,
            priority: BOUT_IDLE_PRIORITY,
        });
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

    // Incumbent bonus. The priority bands below are discrete jumps at the
    // NEED_* boundaries, so a need decaying across a threshold flips priority
    // by 15-25 points — and the behaviours on either side often carry
    // different targets. A small deadband stops that reading as twitching.
    // (This generalises what used to be a hand-rolled +10 inside
    // evaluate_flee, which is why that block is gone.)
    for c in candidates.iter_mut() {
        if c.behavior == entity.current_behavior {
            c.priority += 8;
        }
    }

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

    // NOTE: the "already fleeing? maintain" bonus that lived here is now the
    // general incumbent bonus applied in select_behavior.

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

    // Predators hunt opportunistically — only stop when nearly full
    if entity.needs.hunger > 85.0 {
        return None;
    }

    // Don't hunt if threatened
    if !context.nearby_threats.is_empty() {
        let nearest = &context.nearby_threats[0];
        if nearest.distance < species.sight_range * 0.5 {
            return None;
        }
    }

    // Wider deadband than foraging: a committed chase shouldn't be abandoned
    // because a different rabbit drifted 20 cm closer.
    let prey = sticky_pick(
        &context.nearby_prey,
        entity.target_entity_id.as_deref(),
        0.35,
        3.0,
    )?;

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
    // Animals drink opportunistically when water is available
    // Priority scales with thirst level
    let priority = if entity.needs.thirst < thresholds::NEED_CRITICAL {
        90  // Critical: desperate
    } else if entity.needs.thirst < thresholds::NEED_LOW {
        70  // Low: actively seeking
    } else if entity.needs.thirst < thresholds::NEED_COMFORTABLE {
        45  // Comfortable: will drink if nearby
    } else {
        20  // Full: low priority, but still opportunistic
    };

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
    // Herbivores eat opportunistically when food is nearby
    // They don't wait until starving - they graze constantly
    // Scan for anything actually within reach rather than testing only the
    // head of the list. This used to be `first()` + a distance gate, which
    // silently depended on nearby_food being sorted — it wasn't, so an
    // arbitrary far candidate could veto eating while the animal was standing
    // on a clover patch.
    let food = context.nearby_food.iter().find(|f| f.distance <= 2.0)?;

    let priority = if entity.needs.hunger < thresholds::NEED_CRITICAL {
        85  // Critical: starving
    } else if entity.needs.hunger < thresholds::NEED_LOW {
        70  // Low: actively foraging
    } else if entity.needs.hunger < thresholds::NEED_COMFORTABLE {
        50  // Comfortable: grazing behavior
    } else {
        25  // Full: still nibbling if food is right there
    };

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

    // Herbivores graze and forage for plants — priority scales with hunger.
    //
    // Bands are monotonic and all sit BELOW evaluate_flee's 90: a starving
    // rabbit should still run from a fox. The previous version computed a 95
    // for the critical case and then immediately overwrote it with 75 on the
    // next line, so the 95 branch was dead and the bands were non-monotonic
    // (critical 75 < low 80). 85 preserves the evident intent — outrank
    // everything except fleeing — without the dead code.
    let priority = if entity.needs.hunger < thresholds::NEED_CRITICAL {
        85  // Starving: outranks all but fleeing
    } else if entity.needs.hunger < thresholds::NEED_LOW {
        80  // Low: actively seeking food
    } else if entity.needs.hunger < thresholds::NEED_COMFORTABLE {
        60  // Comfortable: ordinary grazing
    } else {
        30  // Full: still drifts toward food, but yields easily
    };

    let food = sticky_pick(
        &context.nearby_food,
        entity.target_entity_id.as_deref(),
        0.25,
        1.5,
    );
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
    // Energy-based sleep priority
    let mut priority = if entity.needs.energy < thresholds::NEED_CRITICAL {
        100  // Exhausted: MUST sleep, even if threatened
    } else if entity.needs.energy < thresholds::NEED_LOW {
        80   // Tired: high priority
    } else if entity.needs.energy < thresholds::NEED_COMFORTABLE {
        40   // Getting tired: moderate priority
    } else {
        0    // Start with no base priority if energy is fine
    };

    // Don't rest if threatened UNLESS exhausted (< 15 energy)
    if !context.nearby_threats.is_empty() && entity.needs.energy >= thresholds::NEED_CRITICAL {
        return None;
    }

    // Day/night cycle strongly influences rest
    // Nocturnal creatures rest during day
    if species.nocturnal && !context.is_night {
        priority += 35;
    }
    // Diurnal creatures rest at night
    if !species.nocturnal && context.is_night {
        priority += 35;
    }

    // Animals rest opportunistically even when not tired if it's their sleep time
    if priority > 0 || (species.nocturnal && !context.is_night) || (!species.nocturnal && context.is_night) {
        return Some(BehaviorDecision {
            behavior: BehaviorState::Resting,
            target_id: None,
            target_position: None,
            priority,
        });
    }

    None

}

fn evaluate_mate(
    entity: &WildlifeEntity,
    _species: &WildlifeSpecies,
    context: &EnvironmentContext,
) -> Option<BehaviorDecision> {
    if !entity.is_mature || entity.is_pregnant {
        return None;
    }

    // Don't mate if threatened
    if !context.nearby_threats.is_empty() {
        return None;
    }

    // Don't mate if starving/dehydrated (critical survival needs)
    if entity.needs.hunger < thresholds::NEED_CRITICAL || entity.needs.thirst < thresholds::NEED_CRITICAL {
        return None;
    }

    // Animals mate opportunistically when they see a potential mate
    // Priority scales with reproduction urge
    let priority = if entity.needs.reproduction > thresholds::NEED_COMFORTABLE {
        60  // High urge: actively seeking
    } else if entity.needs.reproduction > thresholds::NEED_LOW {
        40  // Moderate urge: opportunistic
    } else {
        20  // Low urge: but still might mate if opportunity is right there
    };

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
    _entity: &WildlifeEntity,
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

    // Seek shelter if available — pathfind to nearest shelter structure
    if let Some(shelter) = context.nearby_shelter {
        Some(BehaviorDecision {
            behavior: BehaviorState::Wandering, // Move toward shelter, then rest
            target_id: None,
            target_position: Some(shelter),
            priority,
        })
    } else {
        Some(BehaviorDecision {
            behavior: BehaviorState::Resting, // No shelter — hunker down
            target_id: None,
            target_position: None,
            priority,
        })
    }
}

fn evaluate_heat_stress(
    _entity: &WildlifeEntity,
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
    _entity: &WildlifeEntity,
    context: &EnvironmentContext,
) -> Option<BehaviorDecision> {
    // Check for hazardous weather in environment
    let dangerous_weather = context.nearby_hazards.first()?;

    // High priority to flee storms — prefer shelter if available
    let priority = 90;

    if let Some(shelter) = context.nearby_shelter {
        // Head toward shelter rather than just fleeing blindly
        Some(BehaviorDecision {
            behavior: BehaviorState::Fleeing,
            target_id: None,
            target_position: Some(shelter),
            priority,
        })
    } else {
        Some(BehaviorDecision {
            behavior: BehaviorState::Fleeing,
            target_id: None,
            target_position: Some(dangerous_weather.position), // Flee away from hazard center
            priority,
        })
    }
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

#[allow(dead_code)]
fn evaluate_breeding_season(
    _entity: &WildlifeEntity,
    _species: &WildlifeSpecies,
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

#[allow(dead_code)]
fn evaluate_nocturnal_activity(
    _entity: &WildlifeEntity,
    _species: &WildlifeSpecies,
    _context: &EnvironmentContext,
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

/// Calculate flee direction (away from threat).
///
/// Uses `atan2(dx, dz)` to match the movement convention where
/// `x += sin(heading) * speed` and `z += cos(heading) * speed`.
pub fn flee_direction(entity_pos: &Vector3, threat_pos: &Vector3) -> f64 {
    let dx = entity_pos.x - threat_pos.x;
    let dz = entity_pos.z - threat_pos.z;
    let angle = dx.atan2(dz).to_degrees();
    (angle + 360.0) % 360.0
}

/// Calculate approach direction (toward target).
///
/// Uses `atan2(dx, dz)` to match the movement convention where
/// `x += sin(heading) * speed` and `z += cos(heading) * speed`.
pub fn approach_direction(entity_pos: &Vector3, target_pos: &Vector3) -> f64 {
    let dx = target_pos.x - entity_pos.x;
    let dz = target_pos.z - entity_pos.z;
    let angle = dx.atan2(dz).to_degrees();
    (angle + 360.0) % 360.0
}

/// Calculate a wander direction with home bias
pub fn wander_direction(
    entity_pos: &Vector3,
    home_pos: Option<&Vector3>,
    current_heading: f64,
    rng: &mut impl rand::Rng,
) -> f64 {
    // Base: gentle random offset from current heading for smooth wandering
    let random_offset = rng.gen_range(-15.0..15.0);
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

/// Rotate `current` toward `desired` by at most `max_delta_deg`, taking the
/// shortest arc. Degrees in, degrees out, normalised to [0, 360).
///
/// `approach_direction` and `flee_direction` return an ABSOLUTE bearing with
/// no reference to where the entity was already facing — unlike
/// `wander_direction`, which nudges the existing heading. That asymmetry is
/// why wandering looks smooth and target-directed movement snaps. Everything
/// that writes a heading should route through here.
pub fn turn_toward(current: f64, desired: f64, max_delta_deg: f64) -> f64 {
    // (x + 540) % 360 - 180 maps any angle difference onto the shortest
    // signed arc in (-180, 180]. The +540 keeps the dividend positive, since
    // Rust's % takes the sign of the dividend.
    let diff = ((desired - current + 540.0) % 360.0 - 180.0)
        .clamp(-max_delta_deg, max_delta_deg);
    (current + diff + 360.0) % 360.0
}

/// Maximum heading change in degrees per second.
///
/// Smaller animals pivot harder; a stalking predator turns deliberately;
/// panic overrides both. These are deliberately generous — the goal is to
/// remove the single-tick 180 snap, not to model real turning circles.
///
/// Worth watching: a fleeing deer (14 m/s at 450 deg/s) has a turning radius
/// of roughly 1.8 m, which is wider than the 1.5 m waypoint arrival radius in
/// simulation.rs. The arrival snap there covers it, but if you retune these
/// downward, check that fast movers don't start orbiting their waypoints.
pub fn max_turn_rate(species: &WildlifeSpecies, behavior: BehaviorState) -> f64 {
    let base = match species.size_class {
        SizeClass::Tiny   => 540.0,
        SizeClass::Small  => 420.0,
        SizeClass::Medium => 300.0,
        SizeClass::Large  => 220.0,
        SizeClass::Huge   => 160.0,
    };
    match behavior {
        BehaviorState::Fleeing  => base * 1.5,
        BehaviorState::Stalking => base * 0.6,
        _ => base,
    }
}
