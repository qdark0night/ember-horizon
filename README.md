# Ember — native mod loader for Horizon (beta 0.1.0)

[Русская версия](README.ru.md) · [Full docs](docs/ember.md) · [Horizon README](README.horizon.md)

Ember is a Forge-style loader for [Horizon](https://github.com/horizon-rust/horizon), the Rust Minecraft Java Edition 26.3. A mod is a Rust crate built against one small API crate, `ember-api`, into a dynamic library (`.so` / `.dll` / `.dylib`). Drop it into the game's `mods/` folder and it loads at start. You don't rebuild the game, you don't patch Horizon's source, and mods from different authors share one event bus.

Why it exists: Horizon removed code WASM mods. What remains for code is `.hnmod`, a source patch tied to a single Horizon commit. An Ember mod is built once against a stable C ABI and keeps working on later game builds.

> **Warning:** Ember mods are trusted native code. They run inside the game process with its rights and have no sandbox. Install only mods you trust.

## What you get

- **Event bus:** `Tick`, `WorldLoad`/`WorldUnload`, `BlockBreak`, `BlockPlace`, `UseItem`, `Chat`, `Command`, `Custom`. Priorities run `Highest`..`Lowest`, then `Monitor`, and events can be cancelled.
- **World access:** read blocks, players and the dimension; set blocks, run commands as the server, send messages.
- **Commands:** register your own `/commands`. Vanilla command names are protected.
- **Content:** blocks and items ship as data, in a folder next to the library (models, textures, loot, recipes, lang).
- **Creative inventory:** mod items appear in the Functional Blocks tab and in search.
- **Stable ABI** (`EMBER_ABI` = 2). The host function table only grows at its end, so a mod built for ABI N loads on any game with ABI ≥ N. A mod built for a newer ABI than the game's is refused with a clear message. A panic in a mod stops only that mod.
- **Seats (beta):** `ctx.sit` / `ctx.free_seat`, see Known issues.

## Install (players)

1. Build Horizon from this repository: `cargo build --release -p horizon-client`.
2. Put the mod's library, plus its content folder if it has one, into `mods/`, next to `options.txt`:
```
mods/
├── libember_furniture.so
└── furniture/            # assets/ + data/
```
3. Start the game. The log lists each mod as `Ember: loaded <id> <version>`, or says why it was skipped.

## Write a mod

```toml
[package]
name = "my-mod"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
ember-api = { git = "<this repository URL>" }
```

```rust
use ember_api::{Ctx, EmberMod, Event, EventKind, Priority, Registrar};

struct Hello;

impl EmberMod for Hello {
    fn init(reg: &mut Registrar) -> Result<Self, String> {
        reg.on(EventKind::BlockBreak, Priority::Normal)
            .command("hello", "/hello: say hello");
        Ok(Hello)
    }
    fn on_event(&mut self, ctx: &mut Ctx, event: &mut Event) {
        match event.kind() {
            Some(EventKind::BlockBreak) if event.block().starts_with("minecraft:diamond") => {
                event.cancel(); // the block stays
            }
            Some(EventKind::Command) => {
                let p = event.player().to_owned();
                ctx.tell(&p, "Hello from Ember!");
            }
            _ => {}
        }
    }
}

ember_api::ember_mod!(Hello, id = "hello", name = "Hello");
```

Run `cargo build --release` and copy `target/release/libmy_mod.so` into `mods/`.

## Examples (`mods/ember/`)

| Mod | Shows |
|---|---|
| `protect-spawn` | cancelling break/place, a command (`/spawnprotect`) |
| `vein-miner` | `Monitor` priority, breaking blocks from code |
| `lamp` | its own block with texture, loot and recipe (built for ABI 1, still loads on ABI 2) |
| `furniture` | 12 furniture blocks, seats through `ctx.sit` |
| `abi-probe` | test fixture for ABI compatibility |

## Repository layout

- `crates/ember-api`: the only crate mods depend on (C ABI, `EmberMod`, `Ctx`, `Event`, `ember_mod!`).
- `crates/horizon-ember`: the loader (discovery, ABI checks, event bus, actions). Tests: `cargo test -p horizon-ember`.
- `crates/horizon-client/src/game/ember.rs`: the game hooks.
- `docs/ember.md` / `docs/ember.ru.md`: full documentation.

## Known issues (beta)

- **Seats:** a seated player rides an invisible, silent bat. Vehicle hearts are hidden, but riding a seat isn't fully stable yet. A natural sit pose without a helper entity is planned.
- Mods load only in the game that runs the world (single player, LAN host, dedicated server). A client joining a server runs none.
- Ember has no HUD, GUI or rendering API yet.
- Each OS needs its own build of a mod.

License: same as Horizon.
