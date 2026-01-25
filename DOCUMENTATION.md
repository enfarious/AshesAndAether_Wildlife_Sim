# Wildlife Sim Documentation Summary

**Created**: January 25, 2026  
**Status**: Complete documentation for tri-sim chaotic ecosystem architecture

---

## What Was Created

### 1. [README.md](README.md) - Comprehensive Project Documentation

A **7,000+ word** guide covering:

- **Vision**: A harsh, chaotic world where climate/weather drive behavior cascades
- **Tri-Sim Architecture**: Climate_sim (slow/global), Weather_sim (fast/local), Wildlife_sim (entity/emergent)
- **Current State**: 2 species, 7 plants, needs-driven AI, predator-prey dynamics
- **Integration**: Redis pub/sub, WebSocket for real-time danger, HTTP API for queries
- **Behavior System**: Priority-based decision making, cascade triggers, climate/weather awareness
- **Example Scenarios**: Winter starvation cascades, tornado wipes region, player interaction effects
- **Setup Instructions**: Standalone vs integrated with game server
- **Performance Notes**: Scaling strategy, optimization roadmap
- **Extending the Sim**: How to add species, weather types, custom mechanics

**Key Concepts Documented**:
- Cascade triggers (probabilistic events from aligned conditions)
- Climate-aware behaviors (hibernation, heat stress, breeding windows)
- Weather-aware behaviors (fleeing from tornadoes, avoiding floods)
- Emergent vs scripted design (world doesn't care about you)
- Recovery mechanisms (extinct regions can repopulate)
- Swappable architecture (different planets, different fauna, different weather)

---

### 2. [TODO.md](TODO.md) - Development Roadmap

A **5-phase development plan** (15+ weeks total):

#### **Phase 1: Expand Behavior + Extract Climate (3-4 weeks)**
- Add climate/weather-aware behaviors (hibernation, heat stress, cold stress, storm fleeing, desperate hunting, breeding windows, nocturnal activity)
- Improve plant growth rates (season × day-length multiplier)
- Create `ClimateProvider` trait for modular architecture
- Add caching layer for climate state

**Deliverable**: Behavior system responds to climate/weather, prepared for modular extraction

#### **Phase 2: Build Climate_Sim & Weather_Sim (4-5 weeks)**
- New `climate_sim/` microservice: time/seasons/temperature/tides with Redis pub/sub
- New `weather_sim/` microservice: hazard spawning with movement intents and damage metadata
- Integrate wildlife_sim with both via Redis subscriptions
- Health checks and graceful degradation

**Deliverable**: Standalone services enable swappable worlds

#### **Phase 3: Cascading Events & Population Dynamics (3-4 weeks)**
- Cascade event system: starvation, desperation, booms, extinction, recovery
- Emergency respawn mechanism
- Breeding boost mechanics
- Randomness tuning (chaos levels)

**Deliverable**: Emergent population collapses and recovery

#### **Phase 4: Multi-Zone Coordination (3-4 weeks)**
- Cross-zone migration
- Biome-specific species availability
- Predator tracking across zones
- Population equilibrium mechanics

**Deliverable**: World-scale ecosystem dynamics

#### **Phase 5: Advanced Ecology & Player Integration (4-6 weeks)**
- Pack hunting, parenting, territory, scavenging
- Disease/parasite systems
- Player taming, breeding, harvesting
- Genetic trait inheritance

**Deliverable**: Deep simulation + meaningful player interaction

---

## Architecture Highlights

### Modular Design
```
World = Climate_Sim + Weather_Sim + Wildlife_Sim + Game Server

- Swap Climate_Sim → alien planet with 30-hour days
- Swap Wildlife_Sim → alien fauna with 6 legs
- Swap Weather_Sim → magic storms instead of tornadoes
- Same world engine, infinite worlds
```

### Communication Flow
```
Game Server (authoritative for combat):
  ↓ publishes player positions
  ↓ publishes zone configs
  ↓ publishes player commands
  
Wildlife_Sim (authoritative for behavior):
  → listens to player positions
  → listens to climate state
  → listens to weather events
  → publishes wildlife events (spawn/death/move/attack)
  
Climate_Sim (authoritative for time):
  → publishes time/season/temperature
  → listened to by wildlife_sim and game server
  
Weather_Sim (authoritative for hazards):
  → publishes weather events + movement intents
  → listened to by wildlife_sim and game server
```

### Emergent Behavior (Not Scripted)
```
Cascade conditions are probabilistic:
  IF (winter AND herbivores < 10 AND food < 5) 
  THEN spawn starvation_event

Same conditions can play out differently:
  - Run 1: All herbivores die (extinction)
  - Run 2: Half starve, half migrate (recovery)
  - Run 3: Slow decline, predators also starve (double collapse)

Players see fresh world every time despite same mechanics
```

---

## Key Design Decisions

### Climate Stays Embedded (Initially)
- Too many per-tick accesses (10Hz) for external service
- Will extract to climate_sim in Phase 2 with proper caching
- Trait-based design (`ClimateProvider`) prepared now for extraction

### Damage Calculation Split
- **Wildlife_sim**: "Entity took damage from tornado"
- **Game Server**: Apply damage with player armor/buffs/resistances
- Prevents duplicate logic, server stays authoritative

### Cascades Are Emergent, Not Scripted
- No hardcoded quest "Kill all wolves"
- Conditions check → events spawn → world reacts
- Same trigger can have different outcomes (RNG)
- Players see world as alive, not as system

### Swappable Modules Enable Variety
- Climate config: `{ day_length: 30_hours, year_days: 400, tides: true }`
- Wildlife data: Load species from JSON, no hardcoding
- Weather rules: Easy to switch between realistic/fantasy/sci-fi

---

## Next Steps to Implement

### Immediate (Start Phase 1)
1. Expand `src/behavior.rs` with hibernation/storm-fleeing logic
2. Create `src/climate_provider.rs` trait (no functional change, just refactoring)
3. Test offline mode with new behaviors
4. Document the behavior changes in code comments

### Short-term (Phase 2)
1. Create skeleton `climate_sim/` project
2. Create skeleton `weather_sim/` project
3. Implement Redis pub/sub plumbing
4. Test basic messaging

### Medium-term (Phase 3)
1. Build cascade event detection
2. Tune randomness via config
3. Test starvation → extinction → recovery cycles

---

## Files Created

```
wildlife_sim/
├── README.md                    (NEW - 7000+ words)
├── TODO.md                      (NEW - 5000+ words)
├── src/
│   ├── main.rs                  (existing - will modify Phase 1)
│   ├── behavior.rs              (existing - will expand Phase 1)
│   ├── simulation.rs            (existing - will modify Phase 1)
│   ├── climate.rs               (existing - will extract Phase 2)
│   ├── species.rs               (existing - will expand)
│   ├── plant_species.rs         (existing - will expand)
│   ├── types.rs                 (existing - unchanged for now)
│   ├── redis_bridge.rs          (existing - will modify Phase 2)
│   ├── climate_provider.rs      (TODO - Phase 1 - trait-based abstraction)
│   └── cascade_events.rs        (TODO - Phase 3 - cascade detection)
│
└── Future projects:
    ├── climate_sim/             (TODO - Phase 2)
    └── weather_sim/             (TODO - Phase 2)
```

---

## Design Philosophy

**"Create a world that doesn't care about you, but provides anyway."**

- **Harsh**: Winter starvation, predators hunt you, disasters strike randomly
- **Fair**: Resources abundant in other seasons, can be avoided with knowledge
- **Generous**: Enough animals to hunt, plants to gather, always a path to survival
- **Alive**: Population booms and crashes naturally, not scripted quests
- **Responsive**: Your actions ripple through ecosystem (overhunt → cascade)
- **Recoverable**: Even extinct zones repopulate when conditions improve

---

## Expected Chaos Levels

### Peaceful (chaos_level=0.2)
- Cascades rare, mild
- Predators docile
- Recovery fast
- Good for learning

### Normal (chaos_level=0.5, default)
- Cascades seasonal
- Predators opportunistic
- Recovery balanced
- Good for gameplay

### Harsh (chaos_level=0.8)
- Cascades frequent, severe
- Predators desperate
- Extinction common
- Good for hardcore

### Apocalyptic (chaos_level=1.0)
- Cascades constant
- Entire regions collapse
- Recovery slow
- Good for sandbox/storytelling

---

## Questions for User

1. **Weather Update Frequency**: Should weather_sim run at 10Hz (smooth movement) or slower like 1-5Hz?

2. **Damage Responsibility**: Should damage calculation live in weather_sim or entirely in game server? (Recommend: game server for consistency)

3. **Configuration Storage**: Should species/weather/planet configs live in JSON files (restart to change) or Redis (hot-reload)?

4. **Difficulty Server Setting**: Should each server instance have a `chaos_level` dial, or is chaos always the same?

5. **Multi-Zone First**: Should Phase 4 (multi-zone migration) be moved earlier for bigger world feel?

---

## Documentation Status

✅ **Complete**:
- Architecture overview
- Integration patterns
- Phase 1-5 roadmap with specific tasks
- Example scenarios
- Design philosophy
- Extension guide

🟡 **Ready for Implementation**:
- Phase 1 behavior tasks (specific functions to add)
- Climate_provider trait design
- Cascade event detection conditions

⏳ **Deferred to Implementation**:
- Specific Redis message schemas (will finalize during coding)
- Performance profiling (will measure as we code)
- Client visualization (will design with Unity team)

---

## Success Criteria

✅ **For Documentation**: 
- Explains tri-sim architecture clearly
- Shows how each service connects
- Provides concrete examples of cascades
- Gives clear implementation roadmap

✅ **For Initial Implementation (Phase 1)**:
- Wildlife behaviors respond to climate
- Wildlife flees from weather
- Cascade events trigger correctly
- Offline testing shows realistic behavior

✅ **For Full System (all 5 phases)**:
- World feels alive and responsive
- Players see consequences of their actions
- Regions can collapse and recover
- Swappable modules enable world variety

---

**Ready to iterate and build! The documentation provides both vision and concrete steps.**
