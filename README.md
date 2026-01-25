# Wildlife Sim - Chaotic Interconnected Ecosystem

A **Rust-based microservice** that simulates dynamic, interconnected ecosystems for the *Ashes & Aether* survival MMO. This is not a gentle simulation—the world is harsh and chaotic, with regional collapses, cascading disasters, and emergent recovery. Wildlife responds intelligently to climate and weather, populations boom and crash, and the line between survival and extinction shifts constantly.

## Vision

Create a **living, breathing world** where:

- **Climate drives everything**: Seasons trigger hibernation, breeding booms, migration. Winter starvation cascades can wipe regions clean.
- **Weather is dangerous**: Tornadoes, waterspouts, flash floods, dust devils deal damage and displace populations. Fast movers publish movement intents for smooth server-side interpolation.
- **Wildlife is intelligent**: Predators hunt desperately during famines, herbivores flee from storms and players, species adapt to conditions or die trying.
- **Cascading events emerge**: Population collapse → desperation → predator aggression → player danger. Massive wildfires → drought → migration. Abundance seasons → breeding booms → population explosion.
- **Recovery is real**: Extinct regions can be repopulated when conditions improve. Survivor populations breed and expand. The world resets and heals, never permanently broken.
- **Swappable modules enable worlds**: Different `climate_sim` = different planets (purple sun, 30-hour days, tides). Different `wildlife_sim` = alien fauna. Different `weather_sim` = magic storms vs realistic disasters. Same engine, infinite worlds.

---

## Architecture: Tri-Sim System

Wildlife_sim is one component of a **modular world simulation** that separates concerns by timescale and complexity:

### **Climate_Sim** (Global/Slow)
Manages world time, seasons, day/night cycles, temperature, tides, and weather probability.
- **Timescale**: Slow changes (seasons over weeks, temperature varies daily)
- **Scope**: Global or regional (entire zones share same season/time)
- **Data**: Publishes `climate:zone:{id}` state to Redis with ~1-10 second updates
- **Consumers**: All services query via Redis (wildlife needs seasonal growth rates, NPCs need day/night routines, visual client needs lighting)

### **Weather_Sim** (Regional/Fast)
Spawns and manages localized hazardous weather events with damage zones.
- **Timescale**: Fast-moving (tornadoes appear/disappear in 5-30 minutes)
- **Scope**: Per-zone (zone-42 has tornado, zone-43 has clear skies)
- **Data**: Publishes `weather:active:{zone}:{id}` events and movement intents to Redis
- **Behavior**: Damage is metadata only—game server calculates actual damage with player stats/armor/buffs
- **Movement**: Fast movers (tornadoes) publish velocity + position intents for client-side prediction

### **Wildlife_Sim** (Entity/Emergent)
Simulates flora and fauna ecosystems with intelligent behaviors responding to climate/weather.
- **Timescale**: Entity-driven (behavior decisions every 100ms, plant growth over hours/days)
- **Scope**: Per-zone (zone has 50-500 wildlife entities)
- **Data**: Subscribes to climate/weather events, publishes wildlife events (spawn/death/birth/hunt)
- **Behavior**: Hibernation in winter, fleeing from storms, breeding during abundance, desperate hunting during famine
- **Emergent**: Cascading events arise from weather + starvation + predation combinations—not scripted, not controlled

---

## Current State

### What Exists
- **2 wildlife species**: Rabbit (tiny prey, herbivore), Fox (small predator/omnivore, nocturnal)
- **7 plant species**: Grass, carrot, potato, onion, garlic, apple tree, pear tree
- **Embedded climate system**: Time/season/temperature tracking (will be extracted to climate_sim)
- **Needs-driven AI**: Hunger, thirst, energy, reproduction decay with behavior priority system
- **Predator-prey dynamics**: Foxes hunt rabbits, combat with health/damage
- **Reproduction**: Pregnancy timers, gestation periods, offspring counts
- **Age stages**: Juvenile (reduced stats), Adult, Elder (slightly better but aging)
- **Experience/leveling**: Stat bonuses for successful kills
- **Plant growth**: Multi-stage growth with seasonal dormancy and herbivore consumption
- **Redis integration**: Pub/sub messaging with game server for events and commands
- **WebSocket ready**: Can broadcast real-time wildlife hunting/fleeing events

### What's Planned

See [TODO.md](TODO.md) for detailed roadmap. Quick summary:

**Phase 1 (Immediate)**: Expand behavior to climate/weather-aware responses, extract climate_sim, create weather_sim  
**Phase 2 (Short-term)**: Build weather hazard system with cascading damage, add 30+ new species across biomes  
**Phase 3 (Medium-term)**: Multi-zone coordination, population collapses, recovery mechanisms  
**Phase 4 (Long-term)**: Advanced ecology (disease, scavenging, territory), player integration (taming, breeding, crafting)

---

## Integration with Game Server

The server treats wildlife_sim as a **black-box ecosystem provider**. Wildlife_sim:

1. **Listens** to player positions via Redis: `zone:players:update`
2. **Listens** to weather events via Redis: `weather:spawned`, `weather:moved`, `weather:despawned`
3. **Listens** to climate state via Redis: `climate:tick:{zone_id}`
4. **Publishes** wildlife events via Redis:
   - `wildlife:entity:spawn` - new entity appeared
   - `wildlife:entity:death` - entity died
   - `wildlife:entity:move` - position update
   - `wildlife:entity:attack` - predator attacked prey/player
   - `wildlife:entity:birth` - offspring born
   - `wildlife:plant:grow` - plant growth stage advanced
   - `wildlife:entity:damage` - entity taking environmental damage (weather, starvation)

The **server is authoritative** for:
- Final damage calculations (wildlife publishes damage events with metadata, server applies with player stats)
- Entity despawns due to player actions (players can kill wildlife, server validates)
- Collision detection (wildlife publishes moves, server validates collision)

Wildlife_sim is **authoritative** for:
- Entity state between server ticks (where is that fox right now?)
- Behavior logic (which entity should hunt/flee/breed?)
- Population dynamics (is the rabbit population growing?)

### Redis Channels (Publisher → Subscriber)

```
wildlife_sim → game_server:
  wildlife:events:{zone_id}          - JSON array of this tick's events
  wildlife:entity:spawned:{zone_id}  - new entity (includes position, stats)
  wildlife:entity:despawned:{zone_id} - entity died
  wildlife:danger:{zone_id}          - e.g. "predator_stalking_player_123"

game_server → wildlife_sim:
  zone:players:update:{zone_id}      - current player positions
  zone:weather:update:{zone_id}      - active weather events
  climate:tick:{zone_id}             - current climate state

game_server (general):
  zone:commands                      - player commands (for plant harvesting, etc.)
```

### WebSocket (Real-time Dangerous Events)

For **immediate danger** situations (predator hunting player), wildlife_sim can push directly via WebSocket:

```
wildlife_sim → client (via gateway):
{
  "type": "predator_alert",
  "predator_id": "fox_001",
  "predator_position": { "x": 100, "y": 50, "z": 200 },
  "predator_species": "fox",
  "threat_level": "critical",
  "distance_meters": 15.5,
  "attack_incoming_in_seconds": 1.5
}
```

This allows clients to render warnings and players to react.

### HTTP API (State Queries)

Wildlife_sim exposes HTTP endpoints for other services to query ecosystem state without polling Redis:

```
GET /api/zones/{zone_id}/population
  → { "rabbits": 42, "foxes": 3, "wolves": 1 }

GET /api/zones/{zone_id}/climate
  → { "season": "winter", "temperature": -0.5, "is_night": true }

GET /api/zones/{zone_id}/wildlife/{entity_id}
  → { "id": "fox_001", "species": "fox", "health": 78, "position": {...} }

GET /api/zones/{zone_id}/plants/{plant_id}
  → { "id": "apple_tree_001", "species": "apple_tree", "growth_stage": "mature", ... }
```

---

## Setup & Running

### Prerequisites

- Rust 1.70+ (via [rustup](https://rustup.rs/))
- Redis running and accessible (see env vars below)
- Optional: Game server running (for integrated testing)

### Environment Variables

```bash
# Redis connection
REDIS_URL=redis://localhost:6379

# Zone configuration
ZONE_ID=zone_42              # Which zone this instance manages
ZONE_BOUNDS_MIN_X=-1000
ZONE_BOUNDS_MIN_Y=0
ZONE_BOUNDS_MIN_Z=-1000
ZONE_BOUNDS_MAX_X=1000
ZONE_BOUNDS_MAX_Y=1000
ZONE_BOUNDS_MAX_Z=1000

# Biome for this zone (affects species availability)
ZONE_BIOME=grassland         # grassland|forest|desert|tundra|swamp|ocean|etc.
ZONE_LATITUDE=42.0           # Affects day/night cycle, season intensity

# Simulation parameters
TICK_RATE_HZ=10              # 10 simulation ticks per second
TIME_SCALE=60                # 60x real-time (1 real sec = 1 game min)
OFFLINE_MODE=false           # true = no Redis, print to stdout

# Logging
LOG_LEVEL=info               # debug|info|warn|error
EVENTS_LOG=true              # Log all events to events.jsonl
```

### Building

```bash
cd wildlife_sim
cargo build --release
```

### Running (Standalone/Offline)

For testing without the full server:

```bash
# Terminal 1: Start Redis
redis-server

# Terminal 2: Start wildlife_sim
OFFLINE_MODE=true cargo run --release
```

You'll see zone updates printed every 10 seconds:
```
2026-01-25T14:32:10 [zone_42] Tick 1200: 42 rabbits, 3 foxes, 150 grass patches
2026-01-25T14:32:11 [zone_42] Tick 1201: Fox_001 hunted Rabbit_042 (31 dmg dealt)
2026-01-25T14:32:12 [zone_42] Tick 1202: Rabbit_089 born (parents: Rabbit_015 + Rabbit_073)
```

### Running (Integrated with Server)

Start Redis and the game server first (see [server docs](/agents/server/AshesAndAether_Server/docs/DISTRIBUTED.md)), then:

```bash
REDIS_URL=redis://localhost:6379 \
ZONE_ID=zone_nyc_manhattan \
ZONE_BIOME=urban \
ZONE_LATITUDE=40.7 \
cargo run --release
```

The simulation will:
1. Subscribe to `zone:players:update:zone_nyc_manhattan`
2. Subscribe to `climate:tick:zone_nyc_manhattan`
3. Subscribe to `weather:spawned:zone_nyc_manhattan`
4. Begin publishing `wildlife:events:zone_nyc_manhattan` every tick
5. Process player attacks, plant harvests, and other commands from Redis

---

## How Chaos Works: Example Scenarios

### Scenario 1: Winter Starvation Cascade

```
Day 1 (Winter begins):
  - Climate_sim publishes season=winter, temperature=-0.5
  - Wildlife_sim receives: growth_rate=0.1 (plants barely grow)
  - All herbivores: hunger_decay increases by 50%
  
Days 2-5 (Food scarce):
  - Rabbits eating sparse grass → 30 rabbits down to 12
  - Foxes hunting desperately → attacking even with low health
  - Event cascade: "starvation_event" published to game_server
  
Day 6 (Collapse):
  - All rabbits dead (none survived
  - Foxes starving: -20 health/tick
  - Last 2 foxes die
  - Zone_42 is now: 0 rabbits, 0 foxes, bare ground
  
Days 7-89 (Dead zone):
  - Plants slowly regrow (winter=0.1x growth)
  - Eventually: 1 grass patch appears
  - Recovery seed sown
  
Day 90 (Spring arrives):
  - Climate_sim: season=spring, temp=0.3, growth=1.2x
  - Emergency respawn triggered (population < 3): spawn 2 rabbits
  - New rabbits breed: +4 offspring
  - Zone recovers
```

### Scenario 2: Tornado Wipes Predators

```
Sunny day:
  - Weather_sim spawns: tornado_C1 at position (100, 50, 200), radius=25
  - Publishes: weather:spawned with base_damage=20, damage_period=0.5s
  
Tornado movement (200ms ticks):
  - Moves toward position (150, 50, 300) with velocity=5.0 m/s
  - Any entity within 25m takes 20 damage every 0.5 seconds
  
Predator in zone:
  - Fox_001 at (110, 50, 210) → inside damage zone
  - Takes: 20 damage/0.5s = 40 DPS
  - Health: 78 → drops to 38 in 1 second
  - Flees away from tornado center
  - Cannot escape (tornado moves at 5 m/s, fox at 3 m/s)
  - Dies after 2 seconds
  
Server receives:
  - Event: wildlife:damage with { entity_id: "fox_001", damage: 40, source: "tornado" }
  - Visualizes fox ragdoll in tornado
  
Recovery:
  - Herbivores (safe) breed extensively without predator pressure
  - Population boom: 150 rabbits in 2 weeks
  - Eventually, new predators spawn from distant zones or breed
```

### Scenario 3: Player Interaction + Cascades

```
Player hunts rabbits for food:
  - Kills 10 rabbits in 1 hour
  - Population: 60 → 50
  - Foxes: still well-fed
  
Game event: massive storm + flooding
  - Weather_sim: flash_flood spawned, blocks movement zones
  - Predators isolated from prey zones
  - Herbivores: safe but trapped in muddy areas
  
Days later: drought follows
  - Climate_sim: precipitation_chance drops to 0.1
  - Plants wilt: growth_rate = 0.05
  - Foxes: desperately hunting → attack_damage +30%, attack_range +50%
  - Players: "This region is dangerous now" (predators more aggressive)
  
Eventually: Monsoon season
  - Precipitation: 0.8
  - Growth rate: 1.5x (above normal)
  - Plant explosion → herbivore boom
  - Predator breeding frenzy
  - Zone becomes: 200 rabbits, 15 foxes, 500 grass patches
  - Players: "This region is abundant now"
```

---

## Key Concepts

### Behavior Priority System

Wildlife makes decisions based on **accumulated urgency**. Higher priority = gets done first.

```
Tier 1 (Survival):
  - Fleeing (from threat) → always interrupts
  - Drinking (< 15 thirst) → critical
  - Eating (< 20 hunger) → critical
  
Tier 2 (Health):
  - Resting (low energy) → high priority
  - Seeking shelter (night time, nocturnal species) → medium
  
Tier 3 (Reproduction):
  - Seeking mate (reproduction > 60, all needs met) → low priority
  - Only triggers if survival needs are met
  
Tier 4 (Other):
  - Hunting (predator-specific, when fed) → medium
  - Foraging (herbivore-specific, when fed) → low
  - Wandering (idle) → fallback
```

### Cascade Triggers

Cascades are **probabilistic events** that spawn when conditions align:

```
IF (season = winter AND herbivore_population < 10 AND food_available < 5)
  THEN spawn "starvation_event" with base_damage affecting all herbivores

IF (weather_type = "tornado" AND entity_in_damage_zone)
  THEN entity takes damage based on tower_intensity and position

IF (predator_population = 0 AND herbivore_population > 50)
  THEN spawn "population_boom_event" (breeding multiplier +50%)

IF (date = spring AND survivors_exist)
  THEN spawn "recovery_event" (populate with offspring of survivors)
```

Cascades are **not scripted**, they emerge from the simulation. The same conditions can play out differently based on RNG (some rabbits escape, some foxes happen to be in safe zones).

### Climate-Aware Behaviors

Wildlife responds to climate state cached from climate_sim:

- **Hibernation** (winter, temp < -0.2): Enter sleep state, reduce metabolism, no hunting/breeding
- **Heat stress** (summer, temp > 0.8): Reduce activity, seek water, avoid daylight
- **Cold stress** (winter, temp < -0.5): Increase energy needs, seek shelter, reduce movement
- **Breeding season** (spring/summer): Reproduction urge increases, more frequent mating
- **Night activity** (nocturnal species, is_night=true): Predators hunt more, herbivores hide
- **Day activity** (diurnal species, is_night=false): Herbivores forage, predators rest

### Weather-Aware Behaviors

Wildlife reacts to active weather events:

- **Tornado nearby** (within 100m): Flee away from center with panic speed multiplier
- **Flood** (area denied): Avoid flooded zones, take alternate paths
- **Dust devil** (disorienting): Movement accuracy reduced, perception range reduced
- **Heavy rain** (precipitation > 0.7): Seek shelter, reduce activity
- **Clear + warm** (ideal conditions): Breeding urge increases, hunger/thirst decay slightly

---

## Extending the Simulation

### Adding a New Species

1. **Define** in [src/species.rs](src/species.rs):
```rust
pub fn register_species() {
    let mut species = HashMap::new();
    
    species.insert("wolf", WildlifeSpecies {
        diet_type: DietType::Predator,
        size_class: SizeClass::Large,
        spawn_weight: 0.1,
        preferred_biomes: vec![BiomeType::Forest, BiomeType::Mountain],
        nocturnal: false,
        // ... more fields
    });
}
```

2. **Configure behavior** in [src/behavior.rs](src/behavior.rs):
   - Add species-specific perception ranges
   - Add species-specific hunting strategies
   - Add species-specific reproduction windows

3. **Test** in offline mode to see behavior

### Adding a Weather Type

Once weather_sim is built:

1. **Define** in weather_sim:
```rust
enum WeatherType {
    Tornado, Waterspout, Dust Devil, Flash Flood, Blizzard,
    // New type:
    Tsunami,
}
```

2. **Spawn conditions** in weather_sim:
```rust
if zone_near_ocean && wave_height > 5.0 {
    spawn_weather("tsunami", intensity=8);
}
```

3. **Wildlife_sim reacts** automatically (flees, takes damage)

### Configuration

Currently hardcoded in source. Future versions will support:
- JSON/YAML config files for species parameters
- Redis-backed configuration (hot-reload)
- Per-zone species whitelists
- Difficulty modifiers (chaos level)

---

## Performance & Scaling

### Current Limits
- Single zone instance: ~500 wildlife entities before noticeable slowdown
- Tick rate: 10 Hz (100ms per tick)
- No spatial partitioning yet (O(n²) entity checks)

### Optimization Roadmap
1. **Spatial partitioning** (quadtree/octree) → O(n log n)
2. **Async movement** updates (don't wait for Redis ACK)
3. **Population limits** per zone (remove weakest when overcrowded)
4. **Caching** of climate/weather state (1-second TTL instead of querying per tick)

For multi-zone scaling, deploy separate wildlife_sim instances per region:
```
wildlife_sim_instance_1 → zones: [zone_1, zone_2, zone_3]
wildlife_sim_instance_2 → zones: [zone_4, zone_5, zone_6]
wildlife_sim_instance_3 → zones: [zone_7, zone_8, zone_9]
```
All connected to same Redis, no conflicts.

---

## Development Notes

### Project Structure
```
src/
  main.rs           - Entry point, CLI args, main loop
  simulation.rs     - ZoneSimulation, entity updates, needs decay
  species.rs        - WildlifeSpecies definitions (2 species currently)
  plant_species.rs  - PlantSpecies definitions (7 species currently)
  behavior.rs       - Decision-making AI, behavior priority system
  climate.rs        - Time/season/temperature tracking (will extract)
  types.rs          - Shared types for Redis protocol
  redis_bridge.rs   - Pub/sub messaging

Cargo.toml          - Dependencies (tokio, redis, serde, etc.)
```

### Testing
```bash
# Offline simulation (no Redis needed)
OFFLINE_MODE=true cargo run --release

# With logging
LOG_LEVEL=debug cargo run --release

# Unit tests
cargo test

# Integration test (needs Redis + server)
# See test files in src/tests/
```

### Debugging
Enable detailed logging:
```bash
LOG_LEVEL=debug cargo run --release 2>&1 | tee sim.log
```

Look for:
- `BEHAVIOR_DECISION` logs showing why entities chose actions
- `CASCADE_EVENT` logs showing population events
- `WEATHER_DAMAGE` logs showing environmental damage
- `STALE_STATE` warnings if climate/weather data is outdated

---

## Future Vision

This is the **foundation** for a living world. As we add:

- Climate_sim: Planets with different day/year lengths, orbital mechanics, tides
- Weather_sim: Fast-moving hazards with client-side prediction
- 50+ species: Arctic foxes, desert rattlesnakes, tropical parrots, ocean sharks
- Advanced ecology: Disease spreading, territorial behavior, tool use
- Player mechanics: Taming animals, breeding for traits, harvesting skins/bones
- Distributed simulation: Multiple climate/weather/wildlife instances coordinating

...the world becomes less a "game mechanic" and more a **persistent, evolving organism** that players inhabit and influence.

The goal is simple: **Create a world that doesn't care about you, but provides anyway.**

---

## License

Ashes & Aether codebase. Internal use.

## Contact

Questions or contributions? Reach out through the Ashes & Aether dev team.
