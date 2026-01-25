# Architectural Decisions - Wildlife Sim

**Document**: Design rationale for tri-sim modular architecture  
**Last Updated**: January 25, 2026

---

## Core Decision: Tri-Sim Modular Architecture

### The Problem
A single monolithic "world simulation" would:
- Be tightly coupled (climate changes → must recompile wildlife_sim)
- Not scale horizontally (hard to add new features)
- Not be testable (can't test just one aspect)
- Not support variety (can't have different planets/weather systems)

### The Solution: Three Decoupled Microservices

```
┌─────────────────────────────────────────────────────────┐
│                    GAME SERVER                          │
│         (Authoritative for player actions)              │
└────────────────┬───────────────────────────────────────┘
                 │
        ┌────────┴────────┐
        │                 │
        ▼                 ▼
┌──────────────┐    ┌──────────────┐    ┌──────────────┐
│ CLIMATE_SIM  │    │ WEATHER_SIM  │    │ WILDLIFE_SIM │
│ (slow/global)│    │(fast/local)  │    │(entity/live) │
└──────────────┘    └──────────────┘    └──────────────┘
        ▲                 ▲                      ▲
        └─────────────────┼──────────────────────┘
                          │
                    ┌─────┴──────┐
                    │   REDIS    │
                    │   Pub/Sub  │
                    └────────────┘
```

### Why This Works

| Aspect | Benefit |
|--------|---------|
| **Separation of Concerns** | Each service owns its domain: time (climate), hazards (weather), behavior (wildlife) |
| **Independent Scaling** | Weather_sim needs more CPU for fast movement? Scale it separately. |
| **Testability** | Test climate math in isolation, test weather physics in isolation, etc. |
| **Swappability** | Different planet? Swap climate_sim. Different fauna? Swap wildlife_sim. |
| **Failure Isolation** | If weather_sim crashes, game continues (with graceful degradation) |
| **Horizontal Scaling** | Run multiple instances per zone without cross-service complexity |

---

## Decision 1: Climate Stays Embedded (Initially)

### The Trade-off

**Option A: Extract to climate_sim immediately**
- ✅ Enables world swapping now
- ❌ Extra network latency for every behavior decision (10 Hz)
- ❌ Requires caching layer anyway (defeats purpose)
- ❌ More complexity upfront

**Option B: Keep embedded, extract in Phase 2**
- ✅ No network overhead (in-process)
- ✅ Simple now, extractable later
- ✅ Can validate pattern with weather_sim first
- ❌ Delay world swapping to Phase 2

### Decision: Option B (Keep Embedded for Phase 1)

**Rationale**:
1. **Frequency**: Wildlife needs climate state every tick (10 Hz). External calls would hurt performance.
2. **Staleness tolerance**: 1-second staleness is OK for seasonal effects (growth rates change slowly).
3. **Prepare for extraction**: Design climate as a trait (`ClimateProvider`) now, extract to service in Phase 2 with caching layer.
4. **Risk mitigation**: Prove weather_sim pattern first, then repeat for climate_sim.

**Migration Path** (Phase 2):
```rust
// Phase 1: Local only
struct LocalClimateProvider { climate: Climate }

// Phase 2: Redis + Cache
struct RedisClimateProvider {
    redis: RedisClient,
    cache: Arc<RwLock<ClimateSnapshot>>,
    cache_age_ms: u64,
}

// Same interface for both:
trait ClimateProvider {
    fn get_climate(&self) -> ClimateSnapshot;
}
```

---

## Decision 2: Damage Calculation Split

### The Problem
If wildlife_sim calculates damage AND applies it to players:
- Duplicates armor/buff logic
- Server loses authority over combat
- Inconsistencies arise

### The Solution: Split Responsibility

**Wildlife_sim** (sends):
```json
{
  "damage_source": "tornado_001",
  "damage_base": 20,
  "damage_type": "slashing",
  "position": { "x": 100, "y": 50, "z": 200 }
}
```

**Game Server** (calculates):
```
actual_damage = base * (1 - armor_reduction) * (1 + vulnerability) * buffs
apply_damage(player, actual_damage)
```

### Why This Works

| Aspect | Benefit |
|--------|---------|
| **Authority** | Server stays authoritative for player state |
| **Consistency** | All damage calculations in one place |
| **Flexibility** | Easy to adjust armor values, buffs, etc. |
| **Trust** | No cheating via modified wildlife_sim |

---

## Decision 3: Cascades are Emergent, Not Scripted

### The Pitfall
**Scripted cascade** ("When population < 10, trigger starvation quest"):
- Feels artificial
- Predictable (players know it's coming)
- Limits variety (same quest every time)
- Hard to tune (either too common or too rare)

### The Solution: Probabilistic Conditions

```rust
// Cascade is DETECTED, not TRIGGERED
if season == Winter 
   && herbivore_population < 10 
   && food_available < 5 
   && random() < 0.2 {
    spawn CascadeEvent { type: Starvation, ... }
}
```

### Why This Works

| Scenario | Result |
|----------|--------|
| Winter + low pop + low food + RNG lucky | Cascade triggers |
| Winter + low pop + low food + RNG unlucky | NO cascade (animals survive) |
| Winter + low pop + abundant food | NO cascade (food saves them) |
| Summer + low pop + low food | NO cascade (food grows fast) |

**Emergent behavior**: Same conditions, different outcomes = world feels **alive**, not **scripted**.

---

## Decision 4: Cascades Affect Wildlife, Not Quest System

### The Anti-Pattern
Using cascade events to trigger quests ("Kill all wolves in starving region"):
- Limits what cascades can be
- Couples ecology to quest design
- Game becomes predictable

### The Pattern
Cascades affect wildlife directly, players observe:
```
1. Winter starvation cascade triggered
2. Wolf population desperate (increased aggression)
3. Players encounter more aggressive wolves (danger!)
4. Players can hunt wolves, get good loot
5. Natural consequence: cascade ends faster
6. Players feel they "solved" the problem
```

No quest system needed—gameplay emerges naturally.

### Why This Works

| Aspect | Benefit |
|--------|---------|
| **Organic** | Ecology drives events, not quests |
| **Flexible** | Same cascade plays out differently each time |
| **Impactful** | Player actions ripple through ecosystem |
| **Storytelling** | Emergent narratives ("That winter was brutal!") |

---

## Decision 5: Weather Entities, Not Player Entities

### The Question
Should weather affect players via:
- **Option A**: Direct damage mechanics (weather_sim applies damage directly)
- **Option B**: Damage metadata (weather_sim publishes metadata, game server calculates)

### Decision: Option B (Metadata Only)

**Why**:
1. **Authority**: Server calculates actual damage with all player modifiers
2. **Consistency**: One place for all damage math
3. **Flexibility**: Easy to adjust difficulty without changing weather_sim

**Message Format**:
```json
{
  "effect": "tornado_c1",
  "base_damage": 20,
  "damage_period_seconds": 0.5,
  "damage_type": "slashing",
  "position": { "x": 100, "y": 50, "z": 200 },
  "radius": 25.0
}
```

Server interprets:
- Does this player have wind resistance? Reduce by 50%
- Does this player have armor? Reduce by 30%
- Does this player have a buff? Increase by 20%
- Final: 20 * 0.5 * 0.7 * 1.2 = 8.4 damage

---

## Decision 6: Caching > Querying

### The Problem
If wildlife_sim queries climate state every tick:
```
10 ticks/sec × 500 entities × N zones = massive Redis load
```

### The Solution: Cache with Low TTL

```rust
pub struct ClimateCacheLayer {
    cached_state: Arc<RwLock<ClimateSnapshot>>,
    last_update_ms: u64,
    max_age_ms: 1000,  // 1 second TTL
}

impl ClimateCacheLayer {
    fn get_climate(&self) -> ClimateSnapshot {
        // Return cached if fresh
        if now_ms - self.last_update_ms < 1000 {
            return self.cached_state.read().clone();
        }
        // Query Redis if stale
        let fresh = redis_query()?;
        self.cached_state.write().replace(fresh.clone());
        fresh
    }
}
```

### Why This Works

| Update Type | Frequency | Staleness OK? |
|-------------|-----------|---|
| Season change | Once per ~90 days | YES (hours of staleness acceptable) |
| Temperature | Every few minutes | YES (1 sec staleness fine) |
| Time of day | Continuous | YES (1 sec staleness fine) |
| Is night | Every ~12 hours | YES (100ms staleness acceptable) |

**Result**: Minimal Redis traffic, acceptable staleness.

---

## Decision 7: WebSocket for Danger, Redis for State

### The Pattern

```
REDIS (reliable, batch, per-tick):
  - Regular state updates (position, population)
  - Non-urgent events (births, minor interactions)
  - Allows buffering/batching

WEBSOCKET (real-time, low-latency):
  - Immediate danger (predator stalking player)
  - Time-sensitive alerts (tornado warning)
  - Require <200ms latency
```

### Why This Works

| Medium | Good For | Not Good For |
|--------|----------|-------------|
| **Redis Pub/Sub** | Bulk state, reliability, ordering | Real-time danger, low latency |
| **WebSocket** | Immediate alerts, player notifications | Bulk state, reliability |

---

## Decision 8: Swappable Through Data, Not Code

### The Anti-Pattern
Different planets hardcode different behavior:
```rust
// WRONG: Hardcoded per-planet
match planet_type {
    Earth => { season = calculate_earth_season(...) },
    AlienPlanet => { season = calculate_alien_season(...) },
}
```

### The Pattern
Planets are configuration:
```rust
// RIGHT: Data-driven
let planet_config = load_config("desert_planet.json");
let season = calculate_season_generic(
    day_of_year, 
    planet_config.year_days,
    planet_config.season_boundaries
);
```

### Why This Works

| Aspect | Benefit |
|--------|---------|
| **No recompile** | Change planet, don't rebuild |
| **Easy testing** | Create test configs, run offline |
| **Content creation** | Non-programmers can design worlds |
| **Modular** | Different devs can work on different planets |

---

## Decision 9: Loose Coupling via Redis

### The Anti-Pattern
Services call each other directly:
```
wildlife_sim → http://climate_sim:8080/get_climate
weather_sim → http://wildlife_sim:8080/notify_collision
game_server → http://wildlife_sim:8080/get_population
```

Problems:
- Hard to scale (must know each service URL)
- Hard to test (need all services running)
- Tight coupling (breaking changes cascade)

### The Pattern
Services publish to Redis, others subscribe:
```
climate_sim → PUBLISH climate:tick:zone_42
weather_sim → PUBLISH weather:spawned:zone_42
wildlife_sim ← SUBSCRIBE climate:tick:zone_42, weather:spawned:zone_42
game_server ← SUBSCRIBE wildlife:events:zone_42

# To scale: just add more instances, they auto-discover via Redis
```

### Why This Works

| Aspect | Benefit |
|--------|---------|
| **Decoupled** | Services don't know about each other |
| **Scalable** | Add more zones = add more Redis channels |
| **Resilient** | One service down ≠ system down |
| **Testable** | Mock Redis for unit tests |

---

## Decision 10: Entity Lifespan Managed Locally

### The Pattern
Wildlife_sim owns entity lifecycle:
- Spawn (when population pressure allows)
- Update (every tick)
- Death (health reaches 0)
- Despawn (publish event, remove from memory)

Game server does NOT:
- Spawn wildlife (wildlife_sim owns that)
- Direct wildlife movements (wildlife_sim owns that)
- Decide if wildlife should breed (wildlife_sim owns that)

Game server DOES:
- Receive spawn/death/move events
- Visualize entities
- Handle player attacks
- Apply damage with modifiers

### Why This Works

| Responsibility | Owner | Reason |
|---|---|---|
| Behavior logic | Wildlife_sim | Has all context (population, needs, climate) |
| Visualization | Game server | Knows client capabilities |
| Combat/damage | Game server | Authoritative for player safety |
| Persistence | Database | Both services query for history |

---

## Summary: Design Philosophy

### "Create a world that doesn't care about you, but provides anyway."

This drives every architectural decision:

1. **Harsh** (cascades can wipe regions) → Emergent, not scripted
2. **Fair** (resources exist elsewhere) → Multiple biomes, recovery mechanisms
3. **Responsive** (world reacts to players) → Split responsibility for authority
4. **Alive** (not mechanical) → Probabilistic cascades, randomness tuning
5. **Modular** (enable variety) → Data-driven config, swappable services
6. **Scalable** (support many players) → Loose coupling, caching, async

---

## Future-Proofing

### Extensibility Points

1. **Climate**: Can add orbital mechanics, weather generation
2. **Weather**: Can add biome-specific hazards, contagious effects
3. **Wildlife**: Can add pack behavior, territory systems, learning
4. **Cascade**: Can add diseases, famine chains, ecosystem collapse
5. **Player**: Can add taming, breeding, domestication

### Scaling Checkpoints

- 100 players × 500 wildlife: Current design supports
- 1000 players × 5000 wildlife: Needs spatial partitioning
- 10000 players × 50000 wildlife: Needs sharding per zone
- 100000 players: Needs distributed Redis, multiple instances

---

## Decisions Deferred (Future Phases)

- **Climate extraction**: Phase 2 (after weather_sim pattern proven)
- **Distributed coordination**: Phase 3 (after single-zone stable)
- **Advanced genetics**: Phase 5 (foundation later)
- **Visualization framework**: Not in wildlife_sim scope
- **Content pipeline**: Later (after core loops stable)

---

**These decisions enable a living world. Revisit as needed, but the foundation is solid.**
