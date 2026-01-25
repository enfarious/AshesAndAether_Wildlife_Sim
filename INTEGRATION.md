# Wildlife Sim - Integration with Game Server

Quick reference for game server developers integrating wildlife_sim into Ashes & Aether.

---

## Redis Channels Overview

### Publishing (Wildlife_Sim → Game Server)

#### `wildlife:events:{zone_id}`
**Frequency**: Every simulation tick (100ms, 10 Hz)  
**Format**: JSON array of events

```json
{
  "tick": 12345,
  "timestamp_ms": 1705958400123,
  "zone_id": "zone_42",
  "events": [
    {
      "type": "spawn",
      "entity_id": "rabbit_001",
      "species": "rabbit",
      "position": { "x": 100.5, "y": 50.0, "z": 200.3 },
      "health": 100,
      "level": 1
    },
    {
      "type": "move",
      "entity_id": "fox_001",
      "position": { "x": 150.2, "y": 50.0, "z": 205.1 },
      "velocity": { "x": 3.2, "y": 0.0, "z": 1.5 }
    },
    {
      "type": "attack",
      "attacker_id": "fox_001",
      "defender_id": "rabbit_042",
      "damage": 25
    },
    {
      "type": "death",
      "entity_id": "rabbit_042",
      "cause": "predation",
      "killer_id": "fox_001"
    },
    {
      "type": "birth",
      "entity_id": "rabbit_089",
      "parent_ids": ["rabbit_015", "rabbit_073"],
      "species": "rabbit"
    }
  ]
}
```

**Server handling**:
- Spawn events: Create visual entity, register position
- Move events: Update position (can interpolate between ticks)
- Attack events: Show animation, can trigger sound/effects
- Death events: Remove entity, show corpse decay, drop loot
- Birth events: Add to world population counters

---

#### `wildlife:damage:{zone_id}`
**Frequency**: When wildlife takes environmental damage  
**Format**: Per-damage event

```json
{
  "entity_id": "fox_001",
  "damage_source": "weather",
  "weather_id": "tornado_001",
  "damage_base": 20,
  "damage_type": "slashing",
  "position": { "x": 150.2, "y": 50.0, "z": 205.1 },
  "timestamp_ms": 1705958400200
}
```

**Server handling**:
- Log to wildlife history
- Optional: Show damage indicators
- Note: This is for server records, actual damage was already applied by wildlife_sim

---

#### `cascade:events:{zone_id}`
**Frequency**: When cascade detected (variable)  
**Format**: Cascade event metadata

```json
{
  "cascade_type": "starvation",
  "severity": 0.8,
  "affected_species": ["rabbit"],
  "duration_ticks": 50,
  "description": "Starvation event in zone_42: rabbits critically low on food",
  "triggers": [
    "herbivore_population_below_10",
    "available_food_below_5_patches",
    "season_is_winter"
  ],
  "effects": {
    "hunger_decay_multiplier": 2.0,
    "reproduction_urge_multiplier": 0.0,
    "predator_desperation_level": 0.8
  },
  "timestamp_ms": 1705958400456
}
```

**Server handling**:
- Publish to chat: "A starvation event is sweeping the grasslands!"
- Increase difficulty in zone (more predators, more desperate)
- Quest hooks: "Help the ecosystem recover"
- Reward: Players can profit from hunting desperate predators

---

#### `wildlife:danger:{zone_id}:{entity_id}`
**Frequency**: Real-time when danger imminent (via WebSocket)  
**Format**: Immediate threat alert

```json
{
  "alert_type": "predator_stalking",
  "predator_id": "fox_001",
  "predator_species": "fox",
  "threat_level": "critical",
  "predator_position": { "x": 145.0, "y": 50.0, "z": 200.0 },
  "target_id": "player_123",
  "distance_meters": 15.5,
  "attack_incoming_in_seconds": 1.5,
  "timestamp_ms": 1705958400500
}
```

**Server/Client handling**:
- Show predator icon on map
- Play warning sound
- Display distance/bearing on compass
- Allow player to prepare defense/escape

---

#### `climate:tick:{zone_id}`
**Frequency**: Every 1-10 seconds  
**Format**: Climate snapshot

```json
{
  "day_of_year": 80,
  "time_of_day": 14.5,
  "year": 1,
  "current_season": "spring",
  "temperature": 0.35,
  "is_night": false,
  "day_length_hours": 12.5,
  "precipitation_chance": 0.45,
  "timestamp_ms": 1705958400000
}
```

**Server handling**:
- Update UI (in-game clock)
- Update lighting (day/night)
- Send to clients for visual updates
- Used by wildlife_sim for behavior (cached locally)

---

#### `weather:spawned:{zone_id}`
**Frequency**: When new weather event appears  
**Format**: Weather creation event

```json
{
  "weather_id": "tornado_001",
  "type": "tornado",
  "position": { "x": 100.0, "y": 50.0, "z": 200.0 },
  "velocity": { "x": 5.0, "y": 0.0, "z": 3.0 },
  "radius": 25.0,
  "max_radius": 50.0,
  "intensity": 8,
  "damage": {
    "base": 20,
    "type": "slashing",
    "period_seconds": 0.5
  },
  "expires_at_ms": 1705961000000,
  "created_at_ms": 1705958400000
}
```

**Server handling**:
- Create visual effect (tornado model)
- Subscribe to movement updates
- Alert players in zone: "A tornado has appeared!"
- Calculate which entities are in danger zone

---

#### `weather:moved:{zone_id}:{weather_id}`
**Frequency**: Every 100-200ms (high frequency)  
**Format**: Position update with velocity intent

```json
{
  "weather_id": "tornado_001",
  "zone_id": "zone_42",
  "position": { "x": 102.5, "y": 50.0, "z": 203.0 },
  "velocity": { "x": 5.2, "y": 0.0, "z": 3.1 },
  "acceleration": { "x": 0.1, "y": 0.0, "z": -0.05 },
  "radius": 26.0,
  "timestamp_ms": 1705958400200
}
```

**Server handling**:
- **Don't** apply damage here (weather_sim already did)
- Update visual position
- Allow client-side interpolation between updates
- Use velocity for predictive rendering
- Recalculate damage zone for collision detection

---

#### `weather:despawned:{zone_id}:{weather_id}`
**Frequency**: Once per weather event  
**Format**: Expiry notification

```json
{
  "weather_id": "tornado_001",
  "zone_id": "zone_42",
  "reason": "natural_expiry",
  "duration_seconds": 300,
  "entities_affected": 12,
  "total_damage_dealt": 240
}
```

**Server handling**:
- Remove visual effect
- Log statistics
- Clear danger alerts
- Optional: Announce "The tornado has passed!"

---

### Subscribing (Game Server → Wildlife_Sim)

#### `zone:players:update:{zone_id}`
**Publish**: Game server, every player movement tick  
**Frequency**: 10-30 Hz (per player)  
**Format**: Player position array

```json
{
  "zone_id": "zone_42",
  "timestamp_ms": 1705958400123,
  "players": [
    {
      "player_id": "player_123",
      "position": { "x": 150.0, "y": 50.0, "z": 200.0 },
      "velocity": { "x": 2.0, "y": 0.0, "z": 1.5 },
      "health": 85,
      "status": "healthy"
    },
    {
      "player_id": "player_456",
      "position": { "x": 200.0, "y": 50.0, "z": 250.0 },
      "velocity": { "x": -1.5, "y": 0.0, "z": 0.0 },
      "health": 120,
      "status": "healthy"
    }
  ]
}
```

**Wildlife_sim handling**:
- Cache player positions locally
- Check for predator-player proximity
- Calculate threat levels
- Can trigger "predator_stalking_player" alerts

---

#### `zone:commands:{zone_id}`
**Publish**: Game server, when player performs action  
**Frequency**: Per action  
**Format**: Command event

```json
{
  "command_type": "plant_harvest",
  "player_id": "player_123",
  "plant_id": "apple_tree_001",
  "position": { "x": 100.0, "y": 50.0, "z": 200.0 },
  "timestamp_ms": 1705958400456
}
```

Or for attacks:

```json
{
  "command_type": "attack",
  "player_id": "player_123",
  "target_id": "wolf_001",
  "damage_dealt": 45,
  "timestamp_ms": 1705958400456
}
```

**Wildlife_sim handling**:
- Plant harvest: mark plant as harvested, reset growth
- Attack: reduce health, trigger death if <= 0
- Can trigger escape/retaliation behaviors

---

## HTTP API Endpoints

Wildlife_sim exposes HTTP for one-time queries (no polling):

### GET `/api/zones/{zone_id}/population`

```json
{
  "zone_id": "zone_42",
  "timestamp_ms": 1705958400123,
  "populations": {
    "rabbit": 42,
    "fox": 3,
    "wolf": 0,
    "deer": 12
  },
  "total": 57
}
```

Use case: Display world statistics to players

---

### GET `/api/zones/{zone_id}/climate`

```json
{
  "day_of_year": 80,
  "time_of_day": 14.5,
  "season": "spring",
  "temperature": 0.35,
  "is_night": false,
  "day_length_hours": 12.5
}
```

Use case: Verify climate state, display in-game clock

---

### GET `/api/zones/{zone_id}/wildlife/{entity_id}`

```json
{
  "entity_id": "fox_001",
  "species": "fox",
  "position": { "x": 150.2, "y": 50.0, "z": 205.1 },
  "health": 78,
  "level": 2,
  "experience": 250,
  "age_stage": "adult",
  "behavior_state": "hunting"
}
```

Use case: Inspect individual animal for debugging

---

### GET `/api/zones/{zone_id}/plants/{plant_id}`

```json
{
  "plant_id": "apple_tree_001",
  "species": "apple_tree",
  "position": { "x": 100.5, "y": 50.0, "z": 200.0 },
  "growth_stage": "mature",
  "harvested": false,
  "next_harvest_time_ms": 1705961000000
}
```

Use case: Check plant readiness for harvesting

---

## Event Flow Examples

### Example 1: Player Hunts Animal

```
1. Game server: Player clicks "Attack fox_001"
2. Game server → Redis: zone:commands:zone_42 
   { command_type: "attack", target_id: "fox_001", damage: 45 }
3. Wildlife_sim ← Redis: Receives attack command
4. Wildlife_sim: Updates fox_001 health: 78 → 33
5. Wildlife_sim → Redis: wildlife:events:zone_42
   { type: "attack", attacker_id: "player_123", defender_id: "fox_001", damage: 45 }
6. Game server ← Redis: Receives attack event
7. Game server: Reduces fox health bar, shows damage numbers
8. Wildlife_sim: Fox is now injured, sets behavior to "Fleeing"
9. Wildlife_sim → Redis: wildlife:events:zone_42
   { type: "move", entity_id: "fox_001", position: {...}, velocity: {...} }
10. Game server ← Redis: Fox is running away
11. Game server: Updates fox position on client, plays run animation
```

**Result**: Player sees fox health decrease, then fox flees. Immersive!

---

### Example 2: Tornado Strikes Zone

```
1. Weather_sim: Detects conditions for tornado spawn
2. Weather_sim → Redis: weather:spawned:zone_42
   { weather_id: "tornado_001", position: {...}, damage: {...} }
3. Game server ← Redis: Tornado appeared
4. Game server: Creates tornado visual, alerts players
5. Wildlife_sim ← Redis: Weather event received
6. Wildlife_sim: All entities check proximity to tornado
7. Fox_001 is within damage radius, takes 20 damage: 98 → 78
8. Rabbit_042 is within radius, takes 20 damage: 50 → 30
9. Wildlife_sim → Redis: wildlife:events:zone_42
   { type: "move", entity_id: "fox_001", ...flee direction... }
   { type: "move", entity_id: "rabbit_042", ...flee direction... }
   { type: "attack", attacker_id: "tornado", defender_id: "...", damage: 20 }
10. Game server ← Redis: Updates positions (entities fleeing)
11. Weather_sim: Tornado continues moving
12. Weather_sim → Redis: weather:moved:zone_42:tornado_001
    { position: {...}, velocity: {...} }
13. Game server ← Redis: Moves tornado visual
14. Game server: Interpolates smooth movement on client

...300 seconds later...

15. Weather_sim: Tornado expires
16. Weather_sim → Redis: weather:despawned:zone_42:tornado_001
17. Game server ← Redis: Tornado gone
18. Game server: Removes visual, ends alert
```

**Result**: Dramatic weather event affects wildlife and players equally. World feels reactive!

---

## Data Format Specifications

### Position Format
```json
{
  "x": 100.5,
  "y": 50.0,
  "z": 200.3
}
```

### Velocity Format
```json
{
  "x": 3.2,
  "y": 0.0,
  "z": 1.5
}
```

### Damage Format
```json
{
  "base": 20,
  "type": "slashing|bludgeoning|fire|cold|poison|water|acid|energy",
  "period_seconds": 0.5
}
```

### Entity Type
```json
{
  "entity_id": "unique_id",
  "type": "wildlife|plant|weather",
  "species": "fox|rabbit|apple_tree|tornado",
  "position": { "x": 100, "y": 50, "z": 200 }
}
```

---

## Error Handling

### Wildlife_sim Offline
If wildlife_sim is down, game server should:
- Continue functioning
- Don't spawn new wildlife
- Keep cached wildlife entities as-is
- Log errors, alert ops team

### Redis Offline
Wildlife_sim can run offline (prints to stdout), but won't communicate with server.
To resume:
1. Restart wildlife_sim (same process, or new)
2. It will publish `wildlife:recovery` event with state
3. Game server can resync from database if needed

### Stale Data
If wildlife_sim data > 2 seconds old:
- Warn in logs
- Use last known state
- Don't make critical decisions (damage, spawns)

---

## Performance Notes

### Update Frequency Scaling
- **10 players, 50 wildlife**: ~100 Redis messages/sec (negligible)
- **100 players, 500 wildlife**: ~1000 messages/sec (fine)
- **1000 players, 5000 wildlife**: ~10k messages/sec (monitor)

### Optimization Tips
1. **Filter events**: Server doesn't need every move, just important ones (deaths, attacks, danger)
2. **Batch updates**: Wildlife_sim could send batched every 100ms instead of per-entity
3. **Cache climate**: Server caches climate state, only updates on change
4. **Use SET EX**: Expire old weather from Redis automatically

---

## Questions for Integration

1. **How should loot drops work?** Should wildlife_sim publish loot metadata, or does game server calculate from wildlife stats?

2. **Should crossover damage be calculated here or in server?** (I.e., if player attacks wolf, does wolf health reduce here or on server?)

3. **Do you want real-time predator tracking via WebSocket, or can players discover predators via proximity checks?**

4. **Should climate affect other systems** (NPC circadian rhythm, quest availability, etc.) or just wildlife?

5. **What's the max entity count before optimization is critical?** (Affects spatial partitioning decisions)

---

## Testing Checklist

- [ ] Wildlife_sim starts, connects to Redis
- [ ] Receives player position updates
- [ ] Publishes climate tick (verify season/time correct)
- [ ] Publishes wildlife events (spawn/move/attack/death)
- [ ] Game server receives and processes events
- [ ] Player position appears in wildlife_sim
- [ ] Attack command reduces animal health
- [ ] Cascade event publishes to game server
- [ ] Weather event publishes and damages wildlife
- [ ] Offline mode works (prints to stdout)
- [ ] Redis failover handled gracefully

---

**Ready to integrate! Contact wildlife_sim team with any questions.**
