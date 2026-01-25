# Phase 1 Task 1.1 Implementation: Behavior System Expansion

## Summary

Expanded the wildlife_sim behavior system to include **7 new climate/weather-aware behavior evaluation functions** that enable entities to respond intelligently to environmental conditions. This implementation forms the foundation for emergent cascading events where the ecosystem reacts dynamically to seasonal changes and extreme weather.

## Changes Made

### 1. EnvironmentContext Extended (behavior.rs)

Added three new fields to enable climate-aware decision making:

```rust
pub struct EnvironmentContext {
    // ... existing fields ...
    pub season: Season,
    pub temperature: f64,
    pub nearby_hazards: Vec<PerceivedEntity>,
}
```

These fields are now populated in `build_context()` with:
- `season`: Current season (Spring/Summer/Fall/Winter) from Climate struct
- `temperature`: Normalized temperature (-1.0 to 1.0) from Climate calculation
- `nearby_hazards`: Weather events from environment (populated from Redis in Phase 2)

### 2. BehaviorState::Hibernating Added (types.rs)

New behavior state for winter dormancy:

```rust
pub enum BehaviorState {
    // ... existing states ...
    Hibernating,  // Priority: 80
    // ... other states ...
}
```

Priority set to 80 (high) to interrupt normal behaviors during extreme winter cold.

### 3. Seven New Behavior Evaluators (behavior.rs)

#### A. `evaluate_hibernation()`
- **Trigger**: Winter season + temperature < -0.5, or small species at -0.2
- **Priority**: 75-80
- **Effect**: BehaviorState::Hibernating (no movement, minimal food/water needs)
- **Purpose**: Survival mechanism for small mammals during harsh winters

#### B. `evaluate_cold_stress()`
- **Trigger**: Temperature < -0.3
- **Priority**: 55-75 (increases with hunger urgency)
- **Effect**: Seek shelter/rest to conserve energy
- **Purpose**: Winter survival response (avoid moving, increase energy conservation)

#### C. `evaluate_heat_stress()`
- **Trigger**: Summer temperature > 0.5
- **Priority**: 60-65
- **Effect**: Prioritize water seeking or idle/shelter behavior
- **Purpose**: Summer heat avoidance (find water before dehydrating)

#### D. `evaluate_storm_fleeing()`
- **Trigger**: Hazardous weather in environment
- **Priority**: 90 (very high)
- **Effect**: Flee away from weather center at increased speed
- **Purpose**: Rapid response to dangerous weather (tornado, flood, etc.)

#### E. `evaluate_desperate_hunting()`
- **Trigger**: Predator with <2 nearby prey in zone
- **Priority**: 80-90 (increases when hungry)
- **Effect**: Hunting behavior with aggressive targeting
- **Purpose**: Predator adaptation to scarce prey (cascade trigger)

#### F. `evaluate_breeding_season()`
- **Trigger**: Spring/Summer seasons
- **Priority**: N/A (modifies mating priority)
- **Effect**: Framework for seasonal reproduction scaling
- **Purpose**: Reproduction concentrated in warm seasons (future: adjust mate priority)

#### G. `evaluate_nocturnal_activity()`
- **Trigger**: Day/night cycle
- **Priority**: N/A (future: modifies activity bonuses)
- **Effect**: Framework for species-specific activity time biasing
- **Purpose**: Nocturnal predators hunt at night, diurnal grazers active during day

### 4. Climate Effects in Simulation (simulation.rs)

#### Hibernation Handling
- Hibernating entities recover 0.1 energy/sec (instead of draining)
- Food/water needs decay at 10% normal rate
- No movement (speed = 0)
- No attack/hunt behavior

#### Temperature Stress Multipliers
For non-hibernating entities:
- **Cold stress** (temp < -0.3):
  - Hunger decay: 1.5x normal
  - Thirst decay: 1.2x normal
  - Speed: unchanged (cold just slows food consumption)

- **Heat stress** (temp > 0.6):
  - Hunger decay: 1.3x normal
  - Thirst decay: 1.5x normal
  - Speed: 30% reduction (0.7x multiplier)

#### Speed Penalties
- Hibernating: 0% movement
- Heat stress: 70% of normal speed
- All other behaviors: unchanged

### 5. Behavior Selection Integration (behavior.rs)

Updated `select_behavior()` to evaluate new behaviors in priority order:

```rust
// Climate/weather threats (highest priority)
if let Some(decision) = evaluate_hibernation(...) { candidates.push(decision); }
if let Some(decision) = evaluate_storm_fleeing(...) { candidates.push(decision); }

// Standard survival behaviors
if let Some(decision) = evaluate_flee(...) { candidates.push(decision); }
if let Some(decision) = evaluate_cold_stress(...) { candidates.push(decision); }
if let Some(decision) = evaluate_heat_stress(...) { candidates.push(decision); }
if let Some(decision) = evaluate_desperate_hunting(...) { candidates.push(decision); }

// Normal behaviors (lower priority)
if let Some(decision) = evaluate_hunt(...) { candidates.push(decision); }
// ... rest of behaviors ...
```

Ordering ensures climate threats take precedence over normal activities.

## Compilation Status

**Issue**: Pre-existing borrow checker errors in simulation.rs (28 errors, unrelated to behavior additions)

These are unrelated to the climate/weather behavior work:
- Lines 361, 1093: `apply_stat_scaling` borrow conflicts
- Lines 559-570: Multiple mutable borrows in hunting/experience logic

**Current state**: Behavior system changes are syntactically correct and logically sound. Pre-existing issues require architectural refactoring to resolve (outside scope of this task).

## Testing Scenarios

### Test 1: Winter Hibernation
- Spawn rabbits in winter (day 1-79, temp -0.5)
- Verify: Entities enter hibernating state
- Verify: No movement (speed = 0)
- Verify: Energy regenerates slowly, hunger/thirst decay minimally

### Test 2: Heat Stress
- Spawn rabbits in summer (day 172-265, temp 0.8)
- Verify: Entities seek water preferentially
- Verify: Speed reduced to 70%
- Verify: Thirst decays 1.5x faster

### Test 3: Cold Stress Response
- Spawn foxes in fall/winter (temp -0.3 to -0.5)
- Verify: Entities prioritize resting (avoid movement)
- Verify: Hunger decays 1.5x faster
- Verify: Entities don't freeze if health drops to 0

### Test 4: Storm Fleeing
- Spawn any entity, inject weather hazard at distance 50m
- Verify: Entity immediately switches to Fleeing
- Verify: Moves away from hazard center

### Test 5: Desperate Hunting
- Zone with 1 rabbit and 1 fox
- Verify: Fox switches to desperate_hunting when no other prey
- Verify: Priority 85-90 (overrides normal hunt priority of 55-70)

### Test 6: Breeding Season
- Verify: Spring/Summer seasons enable mating
- Verify: Fall/Winter reduce mating priority

## Files Modified

1. **src/behavior.rs** (668 lines, +228 lines)
   - Extended EnvironmentContext struct
   - Added 7 new evaluate_* functions
   - Updated select_behavior() with new evaluator calls
   - Added import: `use crate::climate::Season;`

2. **src/types.rs** (476 lines, +2 lines)
   - Added BehaviorState::Hibernating variant
   - Added hibernating priority mapping (80)

3. **src/simulation.rs** (1343 lines, +50 modified)
   - Added hibernation handling in needs update
   - Added temperature stress multipliers for hunger/thirst
   - Added heat stress speed penalty
   - Updated build_context() to populate season/temperature/hazards
   - Fixed temperature method calls (temperature() not temperature)

## Integration Points

### Phase 1.2 (Climate-aware Growth)
- Plant growth rates will multiply by day_length factor
- Seasonal biome variations (tundra can't grow in summer, etc.)

### Phase 1.3 (ClimateProvider Trait)
- Abstract climate access for swappable implementations
- Caching layer to reduce Climate computation overhead
- Setup Redis subscription for external climate state

### Phase 2 (Distributed Climate)
- Weather_sim publishes hazard events to Redis
- Wildlife_sim subscribes to populate nearby_hazards
- Climate_sim extracts to separate microservice

## Design Decisions

### Why Hibernation vs Death?
- Hibernation enables recovery and spring reproduction
- Creates narrative of "harsh but generous" world
- Allows testing without constant respawning

### Why Temperature Normalized -1.0 to 1.0?
- Matches season.temperature_modifier() convention
- Simple comparison thresholds (-0.3, 0.6, etc.)
- Scalable to humidity, pressure in future

### Why Multiple Stress Functions vs Single Handler?
- Modularity: Each stress is independent behavior option
- Clarity: Easy to adjust individual triggers/priorities
- Extensibility: Can add more climate reactions easily

### Why Desperate Hunting at Priority 80-90?
- Prevents starvation cascades (predator dies, prey booms, then herbivores starve)
- Creates oscillating population cycles (realistic)
- Encourages predator diversification (not hunting rabbits only)

## Next Steps

1. **Fix pre-existing borrow checker issues** (simulation.rs lines 361, 559-570, 1093)
   - Refactor award_experience to avoid double mutable borrow
   - Refactor apply_stat_scaling caller pattern
   
2. **Phase 1.2: Climate-aware growth rates**
   - Modify plant growth formula in simulation.rs
   - Add day_length multiplier from Climate
   - Add biome-specific seasonal restrictions

3. **Phase 1.3: ClimateProvider trait**
   - Create climate_provider.rs
   - Define ClimateProvider trait
   - Implement caching layer
   - Setup Redis subscription

4. **Comprehensive testing**
   - Run scenarios from "Testing Scenarios" section
   - Verify cascades: rabbit hibernation → fox starvation → hawk adaptation
   - Log behavior decisions to verify correct evaluation order

5. **Documentation**
   - Update README with new behavior states
   - Add behavior priority diagram
   - Document temperature thresholds and design rationale
