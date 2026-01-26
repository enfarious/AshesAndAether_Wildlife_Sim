# Wildlife Sim - Development Roadmap
<!-- markdownlint-disable MD022 MD031 MD032 -->

A **living, chaotic world simulation** with modular tri-sim architecture. Each phase builds on emergent behavior, environmental cascades, and player-ecosystem interaction.

---

## Near-Term Ops

- [ ] Offline smoke: run climate_sim + weather_sim + wildlife_sim together with injected storm/tornado payloads; verify hazard caching and fleeing reactions.
- [ ] Refresh weather tick log message to reflect 0.1 Hz rate in weather_sim startup output.
- [ ] Silence remaining unused-variable warnings in wildlife_sim (prefix `_` or remove) to keep builds clean during test runs.

## Phase 1: Expand Behavior + Extract Climate (3-4 weeks)

Foundation for climate/weather-aware ecosystem responses and prepare for modular architecture.

### 1.1 Expand Behavior System - Climate & Weather Awareness

**Files**: `src/behavior.rs`, `src/types.rs`

**Behaviors to add**:

- [ ] **Hibernation**: Winter + temperature < -0.2 → Enter dormant state
  - Hibernating entities: stop eating, no movement, take 0 hunger decay
  - Trigger wake condition: spring season or temperature > 0
  - Apply to: rabbits (mild), bears/badgers (strong)

- [ ] **Heat stress**: Summer + temperature > 0.8 → Reduce activity
  - Decrease max speed by 30%
  - Increase hunger/thirst decay by 50%
  - Apply to: all species (affects behavior priority)

- [ ] **Cold stress**: Winter + temperature < -0.5 → Increase survival urgency
  - Increase hunger/thirst decay by 50%
  - Seek shelter/cover (avoid open areas)
  - Priority: fleeing + eating elevated

- [ ] **Storm fleeing**: When weather event within 100m → Panic behavior
  - Set priority to "FLEE" regardless of other needs
  - Move away from weather center (opposite of vector toward tornado)
  - Flee speed multiplier: +50%
  - Apply to: all herbivores strongly, predators slightly

- [ ] **Desperate hunting**: When food scarce (herbivore pop < 5 in zone)
  - Predators: increased attack range (+30%), increased damage (+30%), lower flee threshold
  - Herbivores: forage more aggressively (eat even damaged plants)
  - Trigger cascade events if combined with other factors

- [ ] **Breeding season windows**: Expand from "simple if warm"
  - Spring: +100% reproduction urge
  - Summer: +80% reproduction urge
  - Fall/Winter: -100% (blocked entirely)
  - Temperature modifier: urge scales with warmth

- [ ] **Nocturnal activity**: Better day/night handling
  - Nocturnal species (fox): much more active at night (speed +30%)
  - Diurnal species (rabbit): resting at night (wandering only)
  - Twilight hours: transition periods with activity penalties

**Behavior evaluation functions** to add:

```rust
fn evaluate_hibernation(&self, climate: &ClimateSnapshot, season: Season) -> BehaviorEvaluation
fn evaluate_heat_stress(&self, climate: &ClimateSnapshot) -> BehaviorEvaluation
fn evaluate_cold_stress(&self, climate: &ClimateSnapshot) -> BehaviorEvaluation
fn evaluate_storm_fleeing(&self, active_weather: &[WeatherEvent]) -> BehaviorEvaluation
fn evaluate_desperate_hunting(&self, zone_state: &ZoneState) -> BehaviorEvaluation
fn evaluate_breeding_season(&self, climate: &ClimateSnapshot, season: Season) -> BehaviorEvaluation
fn evaluate_nocturnal_activity(&self, climate: &ClimateSnapshot) -> BehaviorEvaluation
```

**Testing**:
- Offline mode: verify hibernating entities in winter don't move/eat
- Offline mode: spawn tornado, verify all entities flee
- Offline mode: reduce herbivore pop to 1, verify predators hunt desperately

### 1.2 Climate-Aware Growth Rates

**Files**: `src/simulation.rs`, `src/climate.rs`

- [ ] Plant growth modifier: scale by season + day length
  - Currently: `Season::Winter → 0.1x`
  - Add day-length effect: longer days = faster growth (photosynthesis)
  - Formula: `growth_rate = season_modifier * (0.5 + day_length/24)`

- [ ] Hibernation blocks plant consumption
  - Hibernating herbivores don't eat
  - Plants won't be harvested during dormancy

### 1.3 Extract Climate to Separate Module (Preparation)

**Goal**: Prepare structure for climate_sim extraction. No functionality change yet.

- [ ] Create `src/climate_provider.rs` trait:
```rust
pub trait ClimateProvider: Send + Sync {
    async fn get_climate(&self, zone_id: &str) -> Result<ClimateSnapshot>;
    async fn on_season_change(&self, zone_id: &str, season: Season);
}

pub struct LocalClimate { ... }  // Current implementation
pub struct RedisClimate { ... }  // Will use Redis in Phase 2
```

- [ ] Refactor `ZoneSimulation::climate` to use trait:
```rust
pub struct ZoneSimulation {
    climate_provider: Box<dyn ClimateProvider>,
    // ... rest
}
```

- [ ] Add caching layer:
```rust
struct ClimateCacheLayer {
    cached: Arc<RwLock<ClimateSnapshot>>,
    cache_age_ms: u64,
    max_age_ms: u64,
}
```

- [ ] Add Redis subscription setup (won't connect yet):
```rust
async fn subscribe_climate_updates(&self, zone_id: &str) -> Result<()> {
    // TODO: Will implement in Phase 2
    Ok(())
}
```

---

## Phase 2: Build Climate_Sim & Weather_Sim (4-5 weeks)

Extract climate into standalone service, build weather hazard system with cascading effects.

### 2.1 Create Climate_Sim Microservice

**New project**: `climate_sim/`

A standalone Rust service that:
- Manages world time, calendar, seasons
- Calculates day/night cycles per latitude
- Computes temperature variations
- Calculates tide heights (if aquatic)
- Publishes climate state to Redis

**Key components**:

```rust
pub struct ClimateEngine {
    zones: HashMap<String, ZoneClimate>,
}

pub struct ZoneClimate {
    zone_id: String,
    day_of_year: u16,
    time_of_day: f64,
    latitude: f64,
    current_season: Season,
    // Computed values:
    is_night: bool,
    day_length_hours: f64,
    temperature: f64,  // -1.0 to 1.0
    precipitation_chance: f64,
}
```

**Responsibilities**:
- [ ] Time advancement loop (every tick, advance by delta_seconds × time_scale)
- [ ] Season detection (day_of_year → current_season)
- [ ] Day/night calculation (latitude + day_of_year → sunrise/sunset times)
- [ ] Temperature modeling (season modifier + day/night cycle + latitude)
- [ ] Precipitation chance (season + latitude)
- [ ] Tides (if applicable)

**Redis integration**:
- [ ] Publish `climate:tick:{zone_id}` every 1-10 seconds
  - Format: `{ day_of_year, time_of_day, season, is_night, temperature, day_length, precipitation_chance, timestamp }`
- [ ] Publish `climate:events:{zone_id}` on significant changes
  - `{ event_type: "season_change"|"dawn"|"dusk", data: {...} }`
- [ ] Allow zone servers to query `GET climate:zone:{zone_id}` for initial load

**Configuration**:
- [ ] Per-zone latitude, time_scale, hemisphere
- [ ] Customizable season dates (Earth: days 1-365; alien planet: days 1-400)
- [ ] Optional: orbital mechanics (if we want exoplanets)

**Testing**:
- [ ] Unit tests: verify season calculation for arbitrary dates
- [ ] Unit tests: verify day length varies by latitude
- [ ] Integration test: connect to Redis, publish updates
- [ ] Verify wildlife_sim can subscribe and use updates

### 2.2 Create Weather_Sim Microservice

**New project**: `weather_sim/`

A standalone Rust service that:
- Spawns localized weather hazards based on climate conditions
- Moves weather entities with physics
- Publishes damage zones and movement intents
- Manages hazard lifespans and expiry

**Key components**:

```rust
pub enum WeatherType {
    Tornado,       // High damage, fast movement
    Waterspout,    // Ocean vortex, medium damage
    DustDevil,     // Slow, disorienting
    FlashFlood,    // Area denial, drowning
    Blizzard,      // Slow, cold damage
    Hailstorm,     // Random impact points
    Wildfire,      // Spreading area
}

pub struct WeatherEvent {
    id: String,
    zone_id: String,
    weather_type: WeatherType,
    position: Vector3,
    velocity: Vector3,
    radius: f64,
    max_radius: f64,
    damage: DamageMetadata,
    created_at: u64,
    expires_at: u64,
    intensity: f64,  // 0-10
    lifecycle: WeatherLifecycle,  // spawning|active|decaying|despawned
}

pub struct DamageMetadata {
    base: f64,           // Base damage per tick
    damage_type: String, // "slashing"|"bludgeoning"|"cold"|"fire"|"water"
    period_seconds: f64, // Damage tick frequency
    effects: Vec<String>, // Optional: "stun"|"slow"|"blind"
}
```

**Responsibilities**:
- [ ] Spawn weather based on climate conditions
  - `if season == winter && precipitation_chance > 0.7 && rng < 0.1 → spawn blizzard`
  - `if zone_is_coastal && precipitation_chance > 0.5 && rng < 0.05 → spawn waterspout`
  - `if season == summer && precipitation_chance > 0.3 && rng < 0.02 → spawn tornado`

- [ ] Movement simulation (every 100-200ms tick):
  - Position += velocity × delta_time
  - Radius grows/shrinks based on intensity and age
  - Optional acceleration (tornado spinning faster as it ages)

- [ ] Collision/damage zones:
  - Circle/sphere of damage around position
  - Expanding radius (max 50-100m depending on type)

- [ ] Persistence to Redis:
  - Store in `weather:active:{zone_id}:{weather_id}` with TTL
  - TTL = event duration (5-30 minutes typical)

- [ ] Event publishing:
  - `weather:spawned:{zone_id}` when new event created
  - `weather:moved:{zone_id}:{weather_id}` every 200ms
  - `weather:despawned:{zone_id}:{weather_id}` before expiry

- [ ] Movement intent publishing (for client prediction):
  - `game:weather_movement:{zone_id}`
  - Format: `{ weather_id, position, velocity, acceleration, timestamp }`

**Biome-specific hazards**:
- [ ] Grassland: tornadoes (high spin, fast)
- [ ] Desert: dust devils (slow, disorienting), sandstorms
- [ ] Ocean/Coastal: waterspouts, tsunamis
- [ ] Mountain: avalanches, lightning storms
- [ ] Forest: wildfires (spreading), falling trees
- [ ] Tundra: blizzards, white-outs
- [ ] Swamp: flash floods (area denial)

**Randomness**:
- [ ] Weather types distributed by biome
- [ ] Intensity varies (tornado C0 vs C3)
- [ ] Duration varies (5-30 minute lifespans)
- [ ] Movement path somewhat random (not straight line)

**Testing**:
- [ ] Unit tests: verify spawn rates are reasonable
- [ ] Integration test: connect to Redis, publish weather updates
- [ ] Integration test: verify wildlife_sim receives and reacts
- [ ] Integration test: verify game server receives movement intents

### 2.3 Integrate Wildlife_Sim with Climate_Sim & Weather_Sim

**Files**: `src/main.rs`, `src/simulation.rs`, `src/behavior.rs`

- [ ] Remove `LocalClimate` from `ZoneSimulation`
- [ ] Implement `RedisClimateProvider`:
  - Subscribe to `climate:tick:{zone_id}`
  - Cache latest state with 1-second TTL
  - Provide `get_climate()` method for behavior evaluation

- [ ] Add weather event subscriptions:
  - Subscribe to `weather:spawned:{zone_id}`
  - Subscribe to `weather:moved:{zone_id}:*`
  - Subscribe to `weather:despawned:{zone_id}:*`
  - Query `weather:active:{zone_id}:*` every tick

- [ ] Implement weather damage:
  - For each active weather event, check which entities are in damage zone
  - Apply damage metadata: `damage_per_second = base * (damage_period / tick_time)`
  - Publish `wildlife:damage` events to game server

- [ ] Health checks:
  - Verify climate state freshness (warn if > 2 seconds old)
  - Fall back to default climate if weather_sim is offline
  - Graceful degradation if climate_sim is offline (use last known state)

---

## Phase 3: Cascading Events & Population Dynamics (3-4 weeks)

Implement emergent collapse scenarios and recovery mechanisms.

### 3.1 Cascade Event System

**Files**: `src/simulation.rs`, new file `src/cascade_events.rs`

**Purpose**: Detect conditions for population events and apply effects

**Cascade detection** (evaluated every 10-100 ticks):

- [ ] **Starvation cascade**: `IF (herbivore_pop < 10 AND total_food < 5) THEN trigger`
  - Effect: herbivore hunger_decay +100%, herbivores flee when hungry
  - Duration: 10-100 ticks
  - Recovery: food replenishes or population stabilizes

- [ ] **Predator desperation**: `IF (predator_pop > 0 AND prey_pop < 5 AND predator_hunger > 60) THEN trigger`
  - Effect: predator damage +30%, attack_range +50%, flee threshold reduced
  - Duration: as long as prey scarce
  - Recovery: prey population recovers

- [ ] **Population boom**: `IF (food_abundant AND season IN [spring, summer] AND health_avg > 70) THEN trigger`
  - Effect: reproduction_urge +100%, offspring count +25%
  - Duration: 50-200 ticks
  - Recovery: natural (population caps out when food limited)

- [ ] **Extinction cascade**: `IF (herbivore_pop = 0 AND predator_pop > 0) THEN trigger`
  - Effect: predator health decay accelerates (-10/tick), increased desperation
  - Duration: 50-200 ticks
  - Recovery: all predators die, respawn mechanism resets

- [ ] **Recovery cascade**: `IF (season = spring AND survivors_exist AND herbivore_pop < 50) THEN trigger`
  - Effect: spawn offspring (number based on survivor health)
  - Duration: 100-300 ticks
  - Recovery: population explodes, then stabilizes

**Cascade publication**:

```rust
pub struct CascadeEvent {
    event_type: CascadeType,     // starvation|desperation|boom|extinction|recovery
    zone_id: String,
    affected_species: Vec<String>,
    severity: f64,               // 0.0 to 1.0
    duration_ticks: u32,
    effects: Vec<CascadeEffect>,
    triggers: Vec<String>,       // Debug: what triggered this
}

enum CascadeEffect {
    HungerDecayModifier(f64),
    ThirstDecayModifier(f64),
    DamageModifier(f64),
    RangeModifier(f64),
    ReproductionModifier(f64),
    SpeedModifier(f64),
    FleeThresholdModifier(f64),
}
```

Publish to Redis: `cascade:events:{zone_id}` with JSON

Publish to game server as human-readable: `"A starvation event is sweeping zone_42: rabbit population critically low"`

### 3.2 Recovery Mechanisms

**Files**: `src/simulation.rs`

- [ ] **Emergency respawn**: When species population drops below 3
  - Spawn 1-2 adults (depending on zone capacity)
  - Apply "Recovery" tag to prevent immediate re-extinction
  - Delay before respawn: 10 ticks (allow natural recovery first)

- [ ] **Breeding boost**: When conditions improve (food abundant, mild weather)
  - Increase reproduction_urge by 50-100%
  - Increase offspring count per birth by 25-50%
  - Duration: 50-200 ticks (spring/early summer)

- [ ] **Seasonal migration** (Phase 3+): When conditions deteriorate
  - Predators move to adjacent zones looking for prey
  - Herbivores move to warmer zones in winter
  - Requires: multi-zone coordination (Phase 3)

### 3.3 Randomness Tuning

**Files**: new file `src/config.rs`

```rust
pub struct CascadeConfig {
    // Starvation triggers
    starvation_herbivore_threshold: usize,      // < 10
    starvation_food_threshold: usize,           // < 5 patches
    starvation_hunger_penalty: f64,             // +100%
    
    // Desperation triggers
    desperation_prey_threshold: usize,          // < 5
    desperation_predator_hunger_min: f64,       // > 60
    desperation_damage_boost: f64,              // +30%
    desperation_range_boost: f64,               // +50%
    
    // Boom triggers
    boom_food_required: usize,                  // > 50 patches
    boom_season_bonus: f64,                     // +100% urge
    boom_offspring_bonus: f64,                  // +25% count
    
    // Recovery
    recovery_respawn_delay_ticks: u32,          // 10 ticks
    recovery_respawn_count: (usize, usize),     // (1, 2)
    recovery_breeding_boost: f64,               // +50%
    
    // Difficulty modifiers (allow per-server tuning)
    chaos_level: f64,  // 0.0 (peaceful) to 1.0 (apocalyptic)
}
```

Allow configuration via environment or Redis config keys

---

## Phase 4: Multi-Zone Coordination & Migration (3-4 weeks)

Enable wildlife to migrate between zones, predators to track prey across regions.

### 4.1 Cross-Zone Communication

**Architecture**: All zones publish to shared Redis topics, subscribe to zone-local topics

- [ ] Implement zone-to-zone migration:
  - Track which zones are adjacent
  - When population pressure high, select some entities to migrate
  - Move entity: delete from zone A, create in adjacent zone B

- [ ] Predator tracking (advanced):
  - If prey population drops to 0 in zone, predators receive "prey_elsewhere_hint"
  - Predators drift toward adjacent zones with food
  - Requires: game server to handle cross-zone predator movement

### 4.2 Biome-Specific Populations

**Files**: `src/species.rs`, `src/simulation.rs`

- [ ] Species availability per biome:
  - Grassland: rabbits, deer, wolves
  - Forest: foxes, bears, squirrels
  - Desert: coyotes, scorpions, rattlesnakes
  - Ocean: sharks, dolphins, fish
  - Tundra: polar bears, seals, penguins

- [ ] Dynamic migration:
  - Winter: herbivores flee south/to warmer biomes
  - Spring: predators move north following prey
  - Summer: breeding in abundance zones
  - Fall: preparation for winter

### 4.3 Population Equilibrium

- [ ] Implement carrying capacity:
  - Calculate zone resource capacity (vegetation, water, shelter)
  - When population exceeds capacity, reduce birth rate or increase death rate
  - Balance predators:prey ratio naturally

---

## Phase 5: Advanced Ecology & Player Integration (4-6 weeks)

Deep ecosystem simulation and meaningful player interaction.

### 5.1 Advanced Behaviors

- [ ] Pack hunting (wolves, dogs)
  - Coordinate attacks on large prey
  - Share kills
  - Hierarchical pack structure (alpha, beta, omega)

- [ ] Parenting behaviors:
  - Adults protect offspring
  - Teach hunting/foraging skills
  - Some species: multi-generational family groups

- [ ] Territory & dominance:
  - Herbivores maintain feeding territories
  - Predators establish hunting ranges
  - Conflict when ranges overlap

- [ ] Scavenging:
  - Hyenas, vultures, rats eat corpses
  - Decomposition over time
  - Disease vectors (disease spreads from corpses)

### 5.2 Disease & Parasites (Optional)

- [ ] Disease mechanics:
  - Contagious illnesses spread between nearby entities
  - Affects health, movement speed, reproduction
  - Natural recovery or death

- [ ] Parasites:
  - Reduce effectiveness (damage output, movement speed)
  - Can be cured by certain plants/player actions

### 5.3 Player Interaction Systems

- [ ] **Taming**:
  - Reduce health of target animal
  - Use special items/foods to attract
  - Successful: animal becomes "companion"
  - Companions can be ridden, fight alongside player, bred

- [ ] **Breeding for traits**:
  - Selective breeding increases desired traits (speed, damage, health)
  - Requires: companion animals, special enclosures
  - Economic: bred animals valuable for trading

- [ ] **Harvesting products**:
  - Kill animal: receive materials (hide, bone, fur, meat, organs)
  - Each species has different loot tables
  - Materials used in crafting

- [ ] **Ecosystem impact**:
  - Player overhunting → cascade events
  - Player protecting population → recovery
  - World responds to player actions

### 5.4 Advanced Spawning

- [ ] Species-specific spawn locations:
  - Rabbits: grassland, fields
  - Wolves: forest, mountains
  - Fish: water sources
  - Etc.

- [ ] Dynamic spawn rates:
  - High population → low spawn rate
  - Low population → high spawn rate
  - Seasonal variations

- [ ] Rare spawns:
  - Legendary animals (higher stats, unique color)
  - Valuable to players

---

## Long-Term Roadmap

### Performance Optimization (Ongoing)

- [ ] **Spatial partitioning**: Octree for entity lookups (O(n log n) instead of O(n²))
- [ ] **Async movement**: Don't wait for Redis ACK per entity
- [ ] **Population culling**: When zone overcrowded, remove weakest entities
- [ ] **Batching**: Send multiple entity updates in single Redis message

### New Content (Ongoing)

- [ ] 50+ species: small mammals, large predators, birds, reptiles, insects, aquatic
- [ ] 30+ plant species: trees, flowers, mushrooms, crops, rare plants
- [ ] Weather hazards: wildfire spreading, avalanches, volcanic effects
- [ ] Biome-specific mechanics: swimming, flying, burrowing

### Advanced Features

- [ ] **World events**: Meteor strikes, volcanic eruptions, climate shifts
- [ ] **Genetic trait system**: Animals inherit/mutate stats
- [ ] **Tool use**: Some species can use simple tools
- [ ] **Language/communication**: Animals signal warnings, mating calls
- [ ] **Tool interaction**: Animals interact with traps, buildings, player-placed structures

### Visualization

- [ ] Population charts (species count over time, per zone)
- [ ] Heatmaps (where predators hunt, where prey hide)
- [ ] Event timeline (what cascades happened, when did extinctions occur)
- [ ] Replay system (rewind time, watch how populations changed)

---

## Quick Reference: File Changes by Phase

### Phase 1
- Modify: `src/behavior.rs`, `src/simulation.rs`, `src/climate.rs`, `src/types.rs`
- Create: `src/climate_provider.rs` (trait + implementations)
- No new projects

### Phase 2
- Create: `climate_sim/` (new Rust project)
- Create: `weather_sim/` (new Rust project)
- Modify: `src/main.rs`, `src/simulation.rs`, `src/behavior.rs`
- Modify: Integrate with Redis pub/sub for climate/weather

### Phase 3
- Create: `src/cascade_events.rs`
- Create: `src/config.rs`
- Modify: `src/simulation.rs` (add cascade detection)
- Add: Environment variables for tuning

### Phase 4
- Modify: `src/simulation.rs` (cross-zone coordination)
- Modify: `src/species.rs` (biome availability)
- Add: Zone adjacency mapping

### Phase 5
- Expand: `src/behavior.rs` (pack behavior, parenting, territory)
- Create: `src/disease.rs`
- Create: `src/breeding.rs`
- Modify: Redis protocol to include loot/item drops

---

## Testing Strategy

### Unit Tests
- Behavior evaluation: verify correct priority under various conditions
- Cascade detection: verify triggers fire at right thresholds
- Climate calculations: verify season/day-length/temperature math

### Integration Tests
- Redis connection: verify pub/sub messaging works
- Climate_sim ↔ wildlife_sim: verify cascade with climate changes
- Weather_sim ↔ wildlife_sim: verify entities flee from weather
- Game server ↔ wildlife_sim: verify events are properly formatted

### Stress Tests
- 1000 entities: verify tick completes in reasonable time
- Cascade chains: trigger multiple cascades, verify no deadlocks
- Network latency: simulate Redis lag, verify graceful degradation

### Manual Testing
- Offline mode: watch population dynamics over 1-hour simulation
- With server: hunt animals, trigger cascades, watch recovery
- Different biomes: verify species availability per zone

---

## Notes

- **Iteration is expected**: As we build and test, mechanics will shift
- **Metrics are key**: Track population curves, cascade frequency, player impact
- **Player feedback matters**: What feels "chaotic and fair" vs "unfair and scripted"?
- **Modular design enables experimentation**: Easy to swap weather_sim with "no weather" version for testing

---

## Priority for Next Session

1. Start Phase 1: Expand behavior.rs with climate/weather awareness
2. Complete climate_provider refactor (trait-based)
3. Test offline mode with new behaviors
4. Begin climate_sim skeleton (no functionality yet)
5. Plan weather_sim data structures

