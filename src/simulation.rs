//! Core simulation loop for wildlife and flora

use crate::behavior::*;
use crate::climate::Climate;
use crate::plant_species::{get_plant_species, PlantType};
use crate::species::get_species;
use crate::types::*;
use rand::Rng;
use std::collections::HashMap;

const ELDER_AGE_MULTIPLIER: f64 = 3.0;
const LEVEL_HEALTH_BONUS: f64 = 0.08;
const LEVEL_DAMAGE_BONUS: f64 = 0.05;

/// Configuration for spawn rates and limits
pub struct SpawnConfig {
    pub species_id: String,
    pub biomes: Vec<BiomeType>,
    pub spawn_chance: f64,
    pub max_per_zone: usize,
    pub min_distance_from_players: f64,
    pub min_distance_between: f64,
}

/// The main simulation state for a single zone
pub struct ZoneSimulation {
    pub zone_id: String,
    pub biome: BiomeType,
    pub bounds_min: Vector3,
    pub bounds_max: Vector3,

    // Climate system
    pub climate: Climate,

    // Entities
    pub wildlife: HashMap<String, WildlifeEntity>,
    pub plants: HashMap<String, PlantEntity>,

    // External state (from game server)
    pub player_positions: Vec<PlayerPosition>,

    // Water sources (static for now)
    pub water_sources: Vec<Vector3>,

    // Timing
    last_update_ms: i64,
    last_spawn_check_ms: i64,
    last_plant_spawn_ms: i64,

    // ID generation
    next_entity_id: u64,
    next_plant_id: u64,

    // Pending events to send
    pub pending_events: Vec<WildlifeEvent>,
}

impl ZoneSimulation {
    pub fn new(zone_id: String, biome: BiomeType, bounds_min: Vector3, bounds_max: Vector3) -> Self {
        Self::with_climate(zone_id, biome, bounds_min, bounds_max, Climate::default())
    }

    pub fn with_climate(
        zone_id: String,
        biome: BiomeType,
        bounds_min: Vector3,
        bounds_max: Vector3,
        climate: Climate,
    ) -> Self {
        Self {
            zone_id,
            biome,
            bounds_min,
            bounds_max,
            climate,

            wildlife: HashMap::new(),
            plants: HashMap::new(),
            player_positions: Vec::new(),
            water_sources: Vec::new(),

            last_update_ms: 0,
            last_spawn_check_ms: 0,
            last_plant_spawn_ms: 0,
            next_entity_id: 1,
            next_plant_id: 1,
            pending_events: Vec::new(),
        }
    }

    /// Get current time_of_day (for compatibility)
    pub fn time_of_day(&self) -> f64 {
        self.climate.time_of_day
    }

    /// Main update tick
    pub fn update(&mut self, now_ms: i64, delta_seconds: f64) {
        let mut rng = rand::thread_rng();

        // Advance climate/time
        self.climate.advance(delta_seconds);

        // Update all wildlife
        let entity_ids: Vec<String> = self.wildlife.keys().cloned().collect();
        for id in entity_ids {
            self.update_wildlife_entity(&id, now_ms, delta_seconds, &mut rng);
        }

        // Update all plants
        let plant_ids: Vec<String> = self.plants.keys().cloned().collect();
        for id in plant_ids {
            self.update_plant(&id, delta_seconds);
        }

        // Process attacks and interactions
        self.process_hunting(now_ms);

        // Check wildlife spawns periodically (every 30 seconds)
        if now_ms - self.last_spawn_check_ms > 30_000 {
            self.check_spawns(now_ms, &mut rng);
            self.last_spawn_check_ms = now_ms;
        }

        // Check plant spawns periodically (every 60 seconds)
        if now_ms - self.last_plant_spawn_ms > 60_000 {
            self.check_plant_spawns(now_ms, &mut rng);
            self.last_plant_spawn_ms = now_ms;
        }

        self.last_update_ms = now_ms;
    }

    fn update_wildlife_entity(
        &mut self,
        entity_id: &str,
        now_ms: i64,
        delta_seconds: f64,
        rng: &mut impl Rng,
    ) {
        // First, gather immutable data we need
        let (species_id, is_alive, should_check_birth) = {
            let entity = match self.wildlife.get(entity_id) {
                Some(e) => e,
                None => return,
            };
            (
                entity.species_id.clone(),
                entity.is_alive,
                entity.is_pregnant && entity.pregnancy_ends_at.map_or(false, |t| now_ms >= t),
            )
        };

        if !is_alive {
            return;
        }

        let species = match get_species(&species_id) {
            Some(s) => s,
            None => return,
        };

        // Update needs (inline to avoid borrow issues)
        {
            let entity = self.wildlife.get_mut(entity_id).unwrap();
            let rates = &species.need_decay_rates;

            // Hibernating entities don't consume food/water and recover energy
            if entity.current_behavior == BehaviorState::Hibernating {
                // Slight energy recovery during hibernation
                entity.needs.energy = (entity.needs.energy + 0.1 * delta_seconds).min(100.0);
                // Hunger/thirst decay slowly
                entity.needs.hunger = (entity.needs.hunger - rates.hunger * 0.1 * delta_seconds).max(0.0);
                entity.needs.thirst = (entity.needs.thirst - rates.thirst * 0.1 * delta_seconds).max(0.0);
            } else {
                // Calculate temperature stress multipliers
                let hunger_mult = if self.climate.temperature() < -0.3 {
                    1.5 // Cold stress increases hunger
                } else if self.climate.temperature() > 0.6 {
                    1.3 // Heat stress increases thirst more than hunger
                } else {
                    1.0
                };

                let thirst_mult = if self.climate.temperature() > 0.6 {
                    1.5 // Heat stress increases thirst
                } else if self.climate.temperature() < -0.3 {
                    1.2
                } else {
                    1.0
                };

                entity.needs.hunger = (entity.needs.hunger - rates.hunger * hunger_mult * delta_seconds).max(0.0);
                entity.needs.thirst = (entity.needs.thirst - rates.thirst * thirst_mult * delta_seconds).max(0.0);

                let energy_drain = match entity.current_behavior {
                    BehaviorState::Fleeing | BehaviorState::Hunting => rates.energy * 3.0,
                    BehaviorState::Resting => -0.2,
                    _ => rates.energy,
                };
                entity.needs.energy = (entity.needs.energy - energy_drain * delta_seconds).clamp(0.0, 100.0);
            }

            // Reproduction urge increases when basic needs are met
            // Note: rates.reproduction is negative by convention, so we use abs()
            if entity.needs.hunger > 50.0 && entity.needs.thirst > 50.0 && entity.needs.energy > 40.0 {
                entity.needs.reproduction =
                    (entity.needs.reproduction + rates.reproduction.abs() * delta_seconds).min(100.0);
            }

            // Check starvation/dehydration
            if entity.needs.hunger <= 0.0 {
                entity.current_health -= 1.0 * delta_seconds;
            }
            if entity.needs.thirst <= 0.0 {
                entity.current_health -= 2.0 * delta_seconds;
            }
        }

        // Check if dead from starvation
        let should_die = {
            let entity = self.wildlife.get(entity_id).unwrap();
            entity.current_health <= 0.0
        };

        if should_die {
            self.kill_entity(entity_id.to_string(), None, "starvation".to_string());
            return;
        }

        // Build environment context (needs immutable borrow of self)
        let context = {
            let entity = self.wildlife.get(entity_id).unwrap();
            self.build_context(entity, species)
        };

        // Select behavior
        let decision = {
            let entity = self.wildlife.get(entity_id).unwrap();
            select_behavior(entity, species, &context)
        };

        // Apply behavior and movement
        let move_event = {
            let entity = self.wildlife.get_mut(entity_id).unwrap();
            entity.current_behavior = decision.behavior;
            entity.target_entity_id = decision.target_id;

            // Execute movement
            let mut speed = match entity.current_behavior {
                BehaviorState::Hibernating => 0.0, // No movement while hibernating
                BehaviorState::Fleeing => species.base_speed * species.flee_speed_multiplier,
                BehaviorState::Hunting | BehaviorState::SeekingMate => species.base_speed,
                BehaviorState::Stalking => species.base_speed * 0.5,
                BehaviorState::Foraging | BehaviorState::Wandering => species.base_speed * 0.7,
                _ => 0.0,
            };

            // Apply heat stress speed penalty
            if self.climate.temperature() > 0.6 && entity.current_behavior != BehaviorState::Hibernating {
                speed *= 0.7; // 30% slower in extreme heat
            }

            if speed > 0.0 {
                let heading = match (&decision.target_position, entity.current_behavior) {
                    (Some(target), BehaviorState::Fleeing) => {
                        flee_direction(&entity.position, target)
                    }
                    (Some(target), _) => {
                        approach_direction(&entity.position, target)
                    }
                    (None, _) => {
                        wander_direction(
                            &entity.position,
                            entity.home_position.as_ref(),
                            entity.heading,
                            rng,
                        )
                    }
                };

                let rad = heading.to_radians();
                let dx = rad.sin() * speed * delta_seconds;
                let dz = rad.cos() * speed * delta_seconds;

                entity.position.x += dx;
                entity.position.z += dz;
                entity.heading = heading;

                // Clamp to bounds
                entity.position.x = entity.position.x.clamp(self.bounds_min.x, self.bounds_max.x);
                entity.position.z = entity.position.z.clamp(self.bounds_min.z, self.bounds_max.z);

                Some(WildlifeEvent::Move {
                    entity_id: entity.id.clone(),
                    position: entity.position,
                    heading: entity.heading,
                    behavior: entity.current_behavior,
                })
            } else {
                None
            }
        };

        if let Some(event) = move_event {
            self.pending_events.push(event);
        }

        // Handle behavior-specific actions and aging
        let (should_mate, plant_to_eat, stage_changed) = {
            let entity = self.wildlife.get_mut(entity_id).unwrap();

            let mut plant_to_eat: Option<String> = None;
            let mut stage_changed = false;

            match entity.current_behavior {
                BehaviorState::Drinking => {
                    entity.needs.thirst = (entity.needs.thirst + 10.0 * delta_seconds).min(100.0);
                }
                BehaviorState::Eating => {
                    // Eating is handled separately - we need to consume the target
                    if let Some(target_id) = &entity.target_entity_id {
                        plant_to_eat = Some(target_id.clone());
                    }
                }
                BehaviorState::Foraging => {
                    // If close enough to food target, eat it
                    if let Some(target_id) = &entity.target_entity_id {
                        plant_to_eat = Some(target_id.clone());
                    }
                }
                BehaviorState::Resting => {
                    entity.needs.energy = (entity.needs.energy + 5.0 * delta_seconds).min(100.0);
                }
                _ => {}
            }

            // Update age
            entity.age += delta_seconds;
            if !entity.is_mature && entity.age >= species.maturity_time {
                entity.is_mature = true;
            }
            let next_stage = Self::age_stage_for(entity.age, species.maturity_time);
            if entity.age_stage != next_stage {
                entity.age_stage = next_stage;
                stage_changed = true;
            }

            entity.last_update_at = now_ms;

            // Check if mating should initiate pregnancy
            let should_mate = entity.current_behavior == BehaviorState::Mating
                && entity.sex == Sex::Female
                && !entity.is_pregnant
                && entity.target_entity_id.is_some();

            (should_mate, plant_to_eat, stage_changed)
        };

        if stage_changed {
            if let Some(entity) = self.wildlife.get_mut(entity_id) {
                self.apply_stat_scaling(entity, species);
            }
        }

        // Handle plant eating (separate block to avoid borrow issues)
        if let Some(plant_id) = plant_to_eat {
            self.eat_plant(entity_id, &plant_id);
        }

        // Handle mating (separate block to avoid borrow issues)
        if should_mate {
            self.process_mating(entity_id, now_ms, species.gestation_time);
        }

        // Check pregnancy (using the flag we computed earlier)
        if should_check_birth {
            self.give_birth(entity_id.to_string(), now_ms, rng);
        }
    }

    fn build_context(&self, entity: &WildlifeEntity, species: &WildlifeSpecies) -> EnvironmentContext {
        let max_range = species.sight_range.max(species.hearing_range).max(species.smell_range);

        let mut threats = Vec::new();
        let mut prey = Vec::new();
        let mut mates = Vec::new();

        // Check other wildlife
        for other in self.wildlife.values() {
            if other.id == entity.id || !other.is_alive {
                continue;
            }

            let distance = entity.position.distance_to(&other.position);
            if distance > max_range {
                continue;
            }

            let other_species = match get_species(&other.species_id) {
                Some(s) => s,
                None => continue,
            };

            let perceived = PerceivedEntity {
                id: other.id.clone(),
                position: other.position,
                distance,
                size_class: other_species.size_class,
                diet_type: Some(other_species.diet_type),
                species_id: Some(other.species_id.clone()),
                is_player: false,
            };

            if is_threat(species, other_species.size_class, Some(other_species.diet_type), false) {
                threats.push(perceived.clone());
            }
            if is_prey(species, other_species.size_class) {
                prey.push(perceived.clone());
            }
            // Only opposite sex can be mates, and females can't be pregnant already
            if other.species_id == entity.species_id
                && other.is_mature
                && other.sex != entity.sex
                && (entity.sex == Sex::Male || !other.is_pregnant)
            {
                mates.push(perceived);
            }
        }

        // Check players (always potential threats)
        for player in &self.player_positions {
            let distance = entity.position.distance_to(&player.position);
            if distance > max_range {
                continue;
            }

            if is_threat(species, SizeClass::Medium, None, true) {
                threats.push(PerceivedEntity {
                    id: player.id.clone(),
                    position: player.position,
                    distance,
                    size_class: SizeClass::Medium,
                    diet_type: None,
                    species_id: None,
                    is_player: true,
                });
            }
        }

        // Sort by distance
        threats.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap());
        prey.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap());
        mates.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap());

        // Water sources
        let nearby_water: Vec<WaterSource> = self
            .water_sources
            .iter()
            .map(|pos| WaterSource {
                position: *pos,
                distance: entity.position.distance_to(pos),
            })
            .filter(|w| w.distance <= species.smell_range)
            .collect();

        // Food (plants for herbivores)
        let nearby_food: Vec<PerceivedEntity> = if species.is_herbivore {
            self.plants
                .values()
                .filter(|p| p.is_alive && matches!(p.current_stage, PlantGrowthStage::Mature | PlantGrowthStage::Growing | PlantGrowthStage::Flowering))
                .map(|p| PerceivedEntity {
                    id: p.id.clone(),
                    position: p.position,
                    distance: entity.position.distance_to(&p.position),
                    size_class: SizeClass::Tiny,
                    diet_type: None,
                    species_id: Some(p.species_id.clone()),
                    is_player: false,
                })
                .filter(|f| f.distance <= species.smell_range)
                .collect()
        } else {
            Vec::new()
        };

        let biome_comfort = species
            .biome_preferences
            .iter()
            .find(|p| p.biome == self.biome)
            .map(|p| p.comfort)
            .unwrap_or(30.0);

        EnvironmentContext {
            current_biome: self.biome,
            biome_comfort,
            time_of_day: self.climate.time_of_day,
            is_night: self.climate.is_night(),
            nearby_threats: threats,
            nearby_prey: prey,
            nearby_food,
            nearby_water,
            nearby_mates: mates,
            season: self.climate.season(),
            temperature: self.climate.temperature(),
            nearby_hazards: Vec::new(), // TODO: Populate from weather events via Redis
        }
    }

    fn process_hunting(&mut self, now_ms: i64) {
        let mut attacks: Vec<(String, String, f64)> = Vec::new();

        for entity in self.wildlife.values() {
            if !entity.is_alive {
                continue;
            }

            if entity.current_behavior != BehaviorState::Hunting {
                continue;
            }

            if now_ms < entity.attack_cooldown_until {
                continue;
            }

            let species = match get_species(&entity.species_id) {
                Some(s) => s,
                None => continue,
            };

            let target_id = match &entity.target_entity_id {
                Some(id) => id.clone(),
                None => continue,
            };

            let target = match self.wildlife.get(&target_id) {
                Some(t) if t.is_alive => t,
                _ => continue,
            };

            let distance = entity.position.distance_to(&target.position);
            if distance <= species.attack_range {
                attacks.push((
                    entity.id.clone(),
                    target_id,
                    entity.attack_damage,
                ));
            }
        }

        // Apply attacks
        for (attacker_id, target_id, damage) in attacks {
            let attacker = self.wildlife.get_mut(&attacker_id).unwrap();
            let species = get_species(&attacker.species_id).unwrap();
            attacker.attack_cooldown_until =
                now_ms + (species.attack_cooldown * 1000.0) as i64;

            let attacker_pos = attacker.position;

            let target = self.wildlife.get_mut(&target_id).unwrap();
            target.current_health -= damage;
            self.award_experience(&attacker_id, 2.0);

            self.pending_events.push(WildlifeEvent::Attack {
                attacker_id: attacker_id.clone(),
                target_id: target_id.clone(),
                damage,
                position: attacker_pos,
            });

            if target.current_health <= 0.0 {
                // Calculate food value from prey before killing it
                let prey_species = get_species(&target.species_id);
                let food_value = prey_species
                    .map(|s| {
                        // Food value based on prey size class
                        match s.size_class {
                            SizeClass::Tiny => 25.0,
                            SizeClass::Small => 40.0,
                            SizeClass::Medium => 60.0,
                            SizeClass::Large => 80.0,
                            SizeClass::Huge => 100.0,
                        }
                    })
                    .unwrap_or(30.0);
                let size_xp = prey_species.map(|s| Self::size_class_xp(s.size_class)).unwrap_or(4.0);

                self.kill_entity(target_id.clone(), Some(attacker_id.clone()), "predation".to_string());
                self.award_experience(&attacker_id, 8.0 + size_xp);

                // Predator eats the kill - gain hunger satisfaction
                if let Some(attacker) = self.wildlife.get_mut(&attacker_id) {
                    let old_hunger = attacker.needs.hunger;
                    attacker.needs.hunger = (attacker.needs.hunger + food_value).min(100.0);
                    attacker.target_entity_id = None; // Clear target after eating
                    tracing::info!(
                        "Predator {} ate prey, hunger {:.0} -> {:.0} (+{:.0})",
                        attacker_id,
                        old_hunger,
                        attacker.needs.hunger,
                        food_value
                    );
                }
            } else {
                // Target starts fleeing
                target.current_behavior = BehaviorState::Fleeing;
                target.fleeing_until = now_ms + 10_000;
                self.award_experience(&target_id, 1.0);
            }
        }
    }

    /// Handle herbivore eating a plant
    fn eat_plant(&mut self, entity_id: &str, plant_id: &str) {
        // Get plant info first
        let (food_value, plant_position) = {
            let plant = match self.plants.get(plant_id) {
                Some(p) if p.is_alive => p,
                _ => return,
            };

            let species = match get_plant_species(&plant.species_id) {
                Some(s) => s,
                None => return,
            };

            // Get food value for current stage
            let stage_config = species.growth_stages.get(plant.stage_index);
            let food_value = match stage_config {
                Some(config) if config.can_be_eaten => config.food_value,
                _ => return, // Can't eat this plant at this stage
            };

            (food_value, plant.position)
        };

        // Check if entity is close enough
        let entity_pos = {
            let entity = match self.wildlife.get(entity_id) {
                Some(e) if e.is_alive => e,
                _ => return,
            };
            entity.position
        };

        let distance = entity_pos.distance_to(&plant_position);
        if distance > 3.0 {
            return; // Too far to eat
        }

        // Apply food value to entity
        if let Some(entity) = self.wildlife.get_mut(entity_id) {
            entity.needs.hunger = (entity.needs.hunger + food_value).min(100.0);
            entity.target_entity_id = None; // Clear target after eating
        }

        // Damage/consume the plant
        let plant = self.plants.get_mut(plant_id).unwrap();
        plant.is_alive = false;
        plant.current_stage = PlantGrowthStage::Dead;

        // Emit event
        self.pending_events.push(WildlifeEvent::PlantEaten {
            plant_id: plant_id.to_string(),
            wildlife_id: entity_id.to_string(),
            food_value,
        });

        tracing::debug!(
            "Wildlife {} ate plant {} for {} hunger",
            entity_id,
            plant_id,
            food_value
        );
    }

    fn process_mating(&mut self, female_id: &str, now_ms: i64, gestation_time: f64) {
        // First, gather what we need (immutable borrow)
        let (mate_id, female_pos, female_name, species_id) = {
            let entity = match self.wildlife.get(female_id) {
                Some(e) => e,
                None => return,
            };
            let mate_id = match &entity.target_entity_id {
                Some(id) => id.clone(),
                None => return,
            };
            (mate_id, entity.position, entity.name.clone(), entity.species_id.clone())
        };

        // Check if mate is nearby (separate borrow)
        let mate_nearby = self.wildlife.get(&mate_id)
            .map(|mate| {
                mate.is_alive
                    && mate.sex == Sex::Male
                    && female_pos.distance_to(&mate.position) < 3.0
            })
            .unwrap_or(false);

        if !mate_nearby {
            return;
        }

        // Initiate pregnancy (mutable borrow)
        if let Some(entity) = self.wildlife.get_mut(female_id) {
            entity.is_pregnant = true;
            entity.pregnancy_ends_at = Some(now_ms + (gestation_time * 1000.0) as i64);
            entity.needs.reproduction = 0.0;
        }

        // Also reset the male's reproduction need
        if let Some(mate) = self.wildlife.get_mut(&mate_id) {
            mate.needs.reproduction = 0.0;
        }

        tracing::info!(
            "{} ({}) is now pregnant, due in {:.0}s",
            female_name,
            species_id,
            gestation_time
        );
    }

    fn kill_entity(&mut self, entity_id: String, killer_id: Option<String>, cause: String) {
        // Get killer species if applicable
        let killer_species = killer_id.as_ref().and_then(|kid| {
            self.wildlife.get(kid).map(|k| k.species_id.clone())
        });

        let entity = match self.wildlife.get_mut(&entity_id) {
            Some(e) => e,
            None => return,
        };

        let health_at_death = entity.current_health;
        let age = entity.age;
        let name = entity.name.clone();

        entity.is_alive = false;
        entity.current_behavior = BehaviorState::Dead;

        self.pending_events.push(WildlifeEvent::Death {
            entity_id: entity.id.clone(),
            species_id: entity.species_id.clone(),
            name,
            position: entity.position,
            zone_id: self.zone_id.clone(),
            killer_id,
            killer_species,
            cause,
            age,
            health_at_death,
        });
    }

    fn give_birth(&mut self, parent_id: String, now_ms: i64, rng: &mut impl Rng) {
        let parent = match self.wildlife.get_mut(&parent_id) {
            Some(p) => p,
            None => return,
        };

        let species = match get_species(&parent.species_id) {
            Some(s) => s,
            None => return,
        };

        let offspring_count =
            rng.gen_range(species.offspring_min..=species.offspring_max);

        let parent_pos = parent.position;
        let species_id = parent.species_id.clone();

        parent.is_pregnant = false;
        parent.pregnancy_ends_at = None;
        parent.needs.reproduction = 0.0;

        let mut offspring_ids = Vec::new();

        for _ in 0..offspring_count {
            let offset_x = rng.gen_range(-4.0..4.0);
            let offset_z = rng.gen_range(-4.0..4.0);

            let child = self.spawn_entity(
                &species_id,
                Vector3::new(
                    parent_pos.x + offset_x,
                    parent_pos.y,
                    parent_pos.z + offset_z,
                ),
                now_ms,
                false,
            );

            if let Some(mut child) = child {
                child.is_mature = false;
                child.age = 0.0;
                offspring_ids.push(child.id.clone());
                self.wildlife.insert(child.id.clone(), child);
            }
        }

        self.pending_events.push(WildlifeEvent::Birth {
            parent_id,
            offspring_ids,
            position: parent_pos,
            zone_id: self.zone_id.clone(),
        });
    }

    fn check_spawns(&mut self, now_ms: i64, _rng: &mut impl Rng) {
        // Only do emergency respawns when population drops critically low
        // Normal population growth should come from mating
        const MIN_POPULATION: usize = 3;
        const RESPAWN_MALES: usize = 2;
        const RESPAWN_FEMALES: usize = 5;

        let species_to_check = vec![
            ("rabbit", vec![BiomeType::Grassland, BiomeType::Forest]),
            ("fox", vec![BiomeType::Forest, BiomeType::Grassland]),
        ];

        for (species_id, biomes) in species_to_check {
            if !biomes.contains(&self.biome) {
                continue;
            }

            let alive_count = self
                .wildlife
                .values()
                .filter(|e| e.species_id == species_id && e.is_alive)
                .count();

            if alive_count < MIN_POPULATION {
                tracing::info!(
                    "Population of {} critically low ({} < {}), spawning reinforcements",
                    species_id,
                    alive_count,
                    MIN_POPULATION
                );
                self.spawn_population(species_id, RESPAWN_MALES, RESPAWN_FEMALES, now_ms);
            }
        }
    }

    fn find_spawn_position(&self, config: &SpawnConfig, rng: &mut impl Rng) -> Option<Vector3> {
        for _ in 0..10 {
            let pos = Vector3::new(
                rng.gen_range(self.bounds_min.x..self.bounds_max.x),
                0.0,
                rng.gen_range(self.bounds_min.z..self.bounds_max.z),
            );

            // Check player distance
            let too_close_to_player = self
                .player_positions
                .iter()
                .any(|p| pos.distance_to(&p.position) < config.min_distance_from_players);

            if too_close_to_player {
                continue;
            }

            // Check same-species distance
            let too_close_to_same = self
                .wildlife
                .values()
                .filter(|e| e.species_id == config.species_id && e.is_alive)
                .any(|e| pos.distance_to(&e.position) < config.min_distance_between);

            if too_close_to_same {
                continue;
            }

            return Some(pos);
        }

        None
    }

    fn spawn_entity(
        &mut self,
        species_id: &str,
        position: Vector3,
        now_ms: i64,
        as_adult: bool,
    ) -> Option<WildlifeEntity> {
        // Random sex
        let sex = if rand::thread_rng().gen_bool(0.5) {
            Sex::Female
        } else {
            Sex::Male
        };
        self.spawn_entity_with_sex(species_id, position, now_ms, as_adult, sex)
    }

    fn spawn_entity_with_sex(
        &mut self,
        species_id: &str,
        position: Vector3,
        now_ms: i64,
        as_adult: bool,
        sex: Sex,
    ) -> Option<WildlifeEntity> {
        let species = get_species(species_id)?;

        let id = format!("wildlife_{}_{}", species_id, self.next_entity_id);
        self.next_entity_id += 1;

        let mut entity = WildlifeEntity {
            id,
            species_id: species_id.to_string(),
            name: format!("a {}", species.name.to_lowercase()),

            position,
            heading: rand::thread_rng().gen_range(0.0..360.0),
            zone_id: self.zone_id.clone(),
            current_biome: self.biome,

            is_alive: true,
            current_health: species.max_health,
            max_health: species.max_health,
            attack_damage: species.attack_damage,
            needs: WildlifeNeeds::default(),

            current_behavior: BehaviorState::Idle,
            target_entity_id: None,
            home_position: Some(position),

            last_update_at: now_ms,
            attack_cooldown_until: 0,
            fleeing_until: 0,

            sex,
            is_pregnant: false,
            pregnancy_ends_at: None,
            age: if as_adult { species.maturity_time } else { 0.0 },
            is_mature: as_adult,
            age_stage: if as_adult { WildlifeAgeStage::Adult } else { WildlifeAgeStage::Juvenile },
            level: 1,
            experience: 0.0,
            experience_to_next: Self::experience_to_next(1),

            in_combat: false,
            last_hostile_at: 0,
        };

        self.apply_stat_scaling(&mut entity, &species);

        Some(entity)
    }

    /// Take pending events (clears the internal buffer)
    pub fn take_events(&mut self) -> Vec<WildlifeEvent> {
        std::mem::take(&mut self.pending_events)
    }

    /// Spawn an initial population for a species
    pub fn spawn_population(
        &mut self,
        species_id: &str,
        males: usize,
        females: usize,
        now_ms: i64,
    ) -> usize {
        let mut rng = rand::thread_rng();
        let mut spawned = 0;

        for _ in 0..males {
            let pos = Vector3::new(
                rng.gen_range(self.bounds_min.x..self.bounds_max.x),
                0.0,
                rng.gen_range(self.bounds_min.z..self.bounds_max.z),
            );
            if let Some(entity) = self.spawn_entity_with_sex(species_id, pos, now_ms, true, Sex::Male) {
                self.wildlife.insert(entity.id.clone(), entity);
                spawned += 1;
            }
        }

        for _ in 0..females {
            let pos = Vector3::new(
                rng.gen_range(self.bounds_min.x..self.bounds_max.x),
                0.0,
                rng.gen_range(self.bounds_min.z..self.bounds_max.z),
            );
            if let Some(entity) = self.spawn_entity_with_sex(species_id, pos, now_ms, true, Sex::Female) {
                self.wildlife.insert(entity.id.clone(), entity);
                spawned += 1;
            }
        }

        spawned
    }

    /// Update player positions from game server
    pub fn update_players(&mut self, players: Vec<PlayerPosition>) {
        self.player_positions = players;
    }

    /// Handle damage from player attack
    pub fn player_attacked(&mut self, target_id: &str, damage: f64, attacker_id: &str) {
        if let Some(entity) = self.wildlife.get_mut(target_id) {
            entity.current_health -= damage;
            entity.current_behavior = BehaviorState::Fleeing;
            entity.target_entity_id = Some(attacker_id.to_string());
            entity.fleeing_until = chrono::Utc::now().timestamp_millis() + 10_000;

            if entity.current_health <= 0.0 {
                self.kill_entity(
                    target_id.to_string(),
                    Some(attacker_id.to_string()),
                    "player".to_string(),
                );
            } else {
                self.award_experience(target_id, 3.0);
            }
        }
    }

    fn age_stage_for(age: f64, maturity_time: f64) -> WildlifeAgeStage {
        if age < maturity_time {
            WildlifeAgeStage::Juvenile
        } else if age < maturity_time * ELDER_AGE_MULTIPLIER {
            WildlifeAgeStage::Adult
        } else {
            WildlifeAgeStage::Elder
        }
    }

    fn experience_to_next(level: u32) -> f64 {
        50.0 + (level as f64 * 25.0)
    }

    fn apply_stat_scaling(&self, entity: &mut WildlifeEntity, species: &WildlifeSpecies) {
        let age_stage = Self::age_stage_for(entity.age, species.maturity_time);
        let (age_health_mult, age_damage_mult) = match age_stage {
            WildlifeAgeStage::Juvenile => (0.75, 0.7),
            WildlifeAgeStage::Adult => (1.0, 1.0),
            WildlifeAgeStage::Elder => (0.85, 0.9),
        };

        let level_index = entity.level.saturating_sub(1) as f64;
        let level_health_mult = 1.0 + LEVEL_HEALTH_BONUS * level_index;
        let level_damage_mult = 1.0 + LEVEL_DAMAGE_BONUS * level_index;

        let new_max = species.max_health * age_health_mult * level_health_mult;
        let new_damage = species.attack_damage * age_damage_mult * level_damage_mult;

        let ratio = if entity.max_health > 0.0 {
            (entity.current_health / entity.max_health).clamp(0.0, 1.0)
        } else {
            1.0
        };

        entity.max_health = new_max.max(1.0);
        entity.current_health = (entity.max_health * ratio).max(1.0);
        entity.attack_damage = new_damage.max(0.1);
        entity.age_stage = age_stage;
    }

    fn award_experience(&mut self, entity_id: &str, amount: f64) {
        let (species_id, level, exp, exp_next) = {
            let entity = match self.wildlife.get(entity_id) {
                Some(e) => e,
                None => return,
            };
            (entity.species_id.clone(), entity.level, entity.experience, entity.experience_to_next)
        };

        let species = match get_species(&species_id) {
            Some(s) => s,
            None => return,
        };

        let entity = match self.wildlife.get_mut(entity_id) {
            Some(e) => e,
            None => return,
        };

        let mut new_exp = exp + amount;
        let mut new_level = level;
        let mut next = exp_next;

        while new_exp >= next {
            new_exp -= next;
            new_level += 1;
            next = Self::experience_to_next(new_level);
        }

        entity.level = new_level;
        entity.experience = new_exp;
        entity.experience_to_next = next;

        self.apply_stat_scaling(entity, &species);
    }

    fn size_class_xp(size_class: SizeClass) -> f64 {
        match size_class {
            SizeClass::Tiny => 2.0,
            SizeClass::Small => 4.0,
            SizeClass::Medium => 6.0,
            SizeClass::Large => 8.0,
            SizeClass::Huge => 10.0,
        }
    }

    // ========================================================================
    // Plant Methods
    // ========================================================================

    fn update_plant(&mut self, plant_id: &str, delta_seconds: f64) {
        let plant = match self.plants.get(plant_id) {
            Some(p) => p,
            None => return,
        };

        if !plant.is_alive {
            return;
        }

        let species = match get_plant_species(&plant.species_id) {
            Some(s) => s,
            None => return,
        };

        let current_season = self.climate.season();

        // Check dormancy
        let should_be_dormant = species.dormant_in_winter
            && current_season == crate::climate::Season::Winter;

        // Get current stage config
        let stage_config = species.growth_stages.get(plant.stage_index);
        if stage_config.is_none() {
            return;
        }
        let stage_config = stage_config.unwrap();

        // Calculate growth for this tick
        let growth_rate = if should_be_dormant {
            0.0
        } else if species.growing_seasons.contains(&current_season) {
            self.climate.growth_rate()
        } else {
            0.1 // Minimal growth outside growing season
        };

        // Time needed for current stage
        let stage_duration = species.total_growth_time * stage_config.duration_ratio;
        let progress_per_second = if stage_duration > 0.0 {
            1.0 / stage_duration
        } else {
            1.0
        };

        let progress_delta = progress_per_second * delta_seconds * growth_rate;

        // Apply growth (need mutable borrow now)
        let plant = self.plants.get_mut(plant_id).unwrap();
        plant.is_dormant = should_be_dormant;

        if should_be_dormant {
            return;
        }

        plant.stage_progress += progress_delta;

        // Check stage transition
        if plant.stage_progress >= 1.0 {
            plant.stage_progress = 0.0;
            plant.stage_index += 1;

            if plant.stage_index < species.growth_stages.len() {
                let new_stage = species.growth_stages[plant.stage_index].stage;
                plant.current_stage = new_stage;

                self.pending_events.push(WildlifeEvent::PlantGrow {
                    plant_id: plant.id.clone(),
                    new_stage,
                });

                tracing::debug!(
                    "Plant {} ({}) grew to {:?}",
                    plant.id,
                    plant.species_id,
                    new_stage
                );
            } else {
                // Plant has completed growth cycle
                // For perennials (trees), check if it should wither then regrow
                if species.regrows_after_harvest && species.plant_type == PlantType::FruitTree {
                    // Trees cycle back to mature stage
                    plant.current_stage = species.regrow_stage;
                    plant.stage_index = species
                        .growth_stages
                        .iter()
                        .position(|s| s.stage == species.regrow_stage)
                        .unwrap_or(0);
                } else {
                    // Annual plants wither
                    plant.current_stage = PlantGrowthStage::Withering;
                    self.pending_events.push(WildlifeEvent::PlantGrow {
                        plant_id: plant.id.clone(),
                        new_stage: PlantGrowthStage::Withering,
                    });
                }
            }
        }
    }

    fn check_plant_spawns(&mut self, now_ms: i64, rng: &mut impl Rng) {
        // Target plant counts by type
        const TARGET_GRASS: usize = 50;
        const TARGET_VEGETABLES: usize = 15;
        const TARGET_TREES: usize = 4;

        let grass_count = self
            .plants
            .values()
            .filter(|p| p.is_alive && p.species_id == "grass")
            .count();

        let veggie_count = self
            .plants
            .values()
            .filter(|p| {
                p.is_alive
                    && matches!(
                        p.species_id.as_str(),
                        "carrot" | "potato" | "onion" | "garlic"
                    )
            })
            .count();

        let tree_count = self
            .plants
            .values()
            .filter(|p| {
                p.is_alive && matches!(p.species_id.as_str(), "apple_tree" | "pear_tree")
            })
            .count();

        // Spawn grass
        if grass_count < TARGET_GRASS {
            let to_spawn = (TARGET_GRASS - grass_count).min(10);
            for _ in 0..to_spawn {
                self.spawn_plant("grass", now_ms, rng);
            }
        }

        // Spawn vegetables (randomly pick one)
        if veggie_count < TARGET_VEGETABLES {
            let veggies = ["carrot", "potato", "onion", "garlic"];
            let species = veggies[rng.gen_range(0..veggies.len())];
            self.spawn_plant(species, now_ms, rng);
        }

        // Spawn trees (rare)
        if tree_count < TARGET_TREES {
            let trees = ["apple_tree", "pear_tree"];
            let species = trees[rng.gen_range(0..trees.len())];
            self.spawn_plant(species, now_ms, rng);
        }
    }

    fn spawn_plant(&mut self, species_id: &str, now_ms: i64, rng: &mut impl Rng) -> Option<String> {
        let species = get_plant_species(species_id)?;

        // Check biome compatibility
        if !species.preferred_biomes.contains(&self.biome) {
            return None;
        }

        let pos = Vector3::new(
            rng.gen_range(self.bounds_min.x..self.bounds_max.x),
            0.0,
            rng.gen_range(self.bounds_min.z..self.bounds_max.z),
        );

        let id = format!("plant_{}_{}", species_id, self.next_plant_id);
        self.next_plant_id += 1;

        // Start at first growth stage
        let initial_stage = species
            .growth_stages
            .first()
            .map(|s| s.stage)
            .unwrap_or(PlantGrowthStage::Seed);

        let plant = PlantEntity {
            id: id.clone(),
            species_id: species_id.to_string(),
            position: pos,
            zone_id: self.zone_id.clone(),

            current_stage: initial_stage,
            stage_started_at: now_ms,
            stage_progress: 0.0,
            stage_index: 0,

            is_alive: true,
            is_dormant: false,
            times_harvested: 0,
            last_harvested_at: None,

            spawned_at: now_ms,
        };

        self.plants.insert(id.clone(), plant);
        Some(id)
    }

    /// Spawn initial plants for the zone
    pub fn spawn_initial_plants(&mut self, now_ms: i64) {
        let mut rng = rand::thread_rng();

        // Spawn grass patches
        for _ in 0..30 {
            self.spawn_plant("grass", now_ms, &mut rng);
        }

        // Spawn some vegetables
        for _ in 0..5 {
            self.spawn_plant("carrot", now_ms, &mut rng);
        }
        for _ in 0..3 {
            self.spawn_plant("potato", now_ms, &mut rng);
        }
        for _ in 0..3 {
            self.spawn_plant("onion", now_ms, &mut rng);
        }
        for _ in 0..2 {
            self.spawn_plant("garlic", now_ms, &mut rng);
        }

        // Spawn a couple trees
        self.spawn_plant("apple_tree", now_ms, &mut rng);
        self.spawn_plant("pear_tree", now_ms, &mut rng);

        tracing::info!(
            "Spawned {} initial plants in zone {}",
            self.plants.len(),
            self.zone_id
        );
    }
}
