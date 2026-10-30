# Ember: native mods, Forge-style

Ember is Horizon's loader for **native Rust mods**, in the way Forge is for
Java Edition: a mod is a separate crate built against one small API crate,
[`ember-api`](../crates/ember-api), into a dynamic library. You drop it into
the game's `mods` folder and it loads at start. The game is not rebuilt, no
Horizon source is patched or forked, and mods from different authors run side
by side on one event bus.

How it sits next to the other mod systems:

| | [Mods](mods.md) (WASM) | [Native mods](native-mods.md) (mixins) | Ember |
|---|---|---|---|
| Code | WebAssembly component, sandboxed | Rust source patched into the game | Native `cdylib`, trusted |
| Built against | `horizon-mod-sdk` | One Horizon commit | `ember-api` (ABI version) |
| Rebuild the game | No | Yes | No |
| Where | `horizon-mods/` | Source tree | `mods/` |

**Ember mods are trusted native code.** They run in the game's process with
its rights and are not sandboxed. Install only mods you trust, as with Forge.

## Writing a mod

```toml
# Cargo.toml
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
        reg.on(EventKind::Chat, Priority::Normal)
            .command("hello", "/hello: say hello");
        Ok(Hello)
    }

    fn on_event(&mut self, ctx: &mut Ctx, event: &mut Event) {
        match event.kind() {
            Some(EventKind::Chat) if event.message().contains("bad word") => event.cancel(),
            Some(EventKind::Command) => {
                let player = event.player().to_owned();
                ctx.tell(&player, "Hello from Ember!");
            }
            _ => {}
        }
    }
}

ember_api::ember_mod!(Hello, id = "hello", name = "Hello");
```

`cargo build --release` gives `target/release/libmy_mod.so` (`my_mod.dll` on
Windows, `libmy_mod.dylib` on macOS). Copy it into `mods/` beside
`options.txt` in the game folder.

## Events

| Event | When | Cancel |
|---|---|---|
| `Tick` | The end of every game tick (20 a second) | |
| `WorldLoad`, `WorldUnload` | A world opens or closes | |
| `BlockBreak` | A player breaks a block (`pos`, `block`, `item`) | The block stays |
| `BlockPlace` | A player placed a block (`pos`, `block`, `item`) | The block is taken back |
| `UseItem` | A player right-clicks with `item` | Nothing is used |
| `Chat` | A player says `message` | Not shown (the window's own player; a LAN guest's message is already out) |
| `Command` | A player runs one of the mod's commands (`command`, `args`) | |
| `Custom` | Another mod's `ctx.post(channel, payload)` | |

Listeners run by priority, `Highest`, `High`, `Normal`, `Low`, `Lowest`, then
`Monitor`, and in load order within one. A listener doesn't see an event
cancelled before it unless it subscribes with `Registrar::on_cancelled`. As in
Forge, `Monitor` is for watching the outcome, not for changing it.

## What a mod can do

`Ctx` reads the world (`block`, `player`, `dimension`) at once. What changes
it, `tell`, `broadcast`, `run_command` and `set_block`, is done at the end of
the tick, in order, as the server. Commands run with the server's rights and
their output is dropped.

A command name belongs to the first mod that registers it. Vanilla's names
are refused.

## Blocks and items

New blocks and items are data, not code: the same declarations a
[content mod](mods.md) uses (`data/<id>/horizon/block/*.json` copying a vanilla
template, with models, textures, loot, recipes and names). An Ember mod ships
them as a folder in `mods/` named after its namespace, beside the library:

```
mods/
├── libember_lamp.so
└── ember_lamp/
    ├── assets/ember_lamp/{blockstates,models,items,textures,lang}/...
    └── data/ember_lamp/{horizon/block,loot_table,recipe}/...
```

A folder in `mods/` with a `data/<folder>/` is read at start, before the
resources load, under the rules (and limits) of a `registries` mod with that
id. The library then sees the blocks by name, `ember_lamp:ember_lamp`, in
its events and through `Ctx`.

## Seats (beta)

`ctx.sit(player, [x, y, z], yaw)` (ABI 2) seats a player: an invisible, still,
silent bat the player rides, summoned and mounted as the server (no cheats
needed), with no vehicle hearts on the HUD. Shift stands the player up in
front of the seat, and the seat goes when its block changes.
`ctx.free_seat(pos)` frees the seats of the block at `pos`. **Beta:** mounting
is not reliable yet, and a natural sit pose without a helper entity is
planned. The [furniture example](../mods/ember/furniture) sits a player down
on a right click with an empty hand.

## Creative inventory

Items from content mods (`assets/<namespace>/items/`), Ember's and `.hmod`s
alike, are listed at the end of the Functional Blocks tab and in search.

## Safety

- The loader checks `EMBER_ABI` before it calls anything. The ABI only grows
  (new host functions go at the end of `Host`), so a mod built for ABI N loads
  in every game with ABI N or later. A mod built for a newer ABI than the
  game's is reported and skipped, not loaded.
- A panic in a mod's `init` or listener is caught at the boundary. That mod
  is stopped and the reason logged, and the game and the other mods go on.
  Undefined behaviour in `unsafe` code isn't caught.
- Strings cross the ABI as pointer and length pairs, valid only during the
  call.
- Mods are loaded once per process, when the first world opens, by the game
  that simulates it: single player's server thread, a LAN host or the
  dedicated server. A client joining a server runs none.

## Examples

- [`mods/ember/protect-spawn`](../mods/ember/protect-spawn): cancels breaking
  and placing within 16 blocks of the world origin for players outside
  creative (`/spawnprotect <radius>|off`).
- [`mods/ember/vein-miner`](../mods/ember/vein-miner): breaking an ore or a log
  while sneaking breaks the whole vein, up to 64 blocks, with drops
  (`/veinminer`). It listens at `Monitor`, so a block Protect Spawn keeps
  starts no vein.

- [`mods/ember/furniture`](../mods/ember/furniture): chairs, a stool, an
  armchair and a bench you sit on (`ctx.sit`), tables, a nightstand, a kitchen
  counter, a bookcase and lamps, all data in `content/furniture/`.
- [`mods/ember/lamp`](../mods/ember/lamp): its own block, the Ember Lamp (a
  light-15 glass-sounding block from the `glowstone` template, with its
  texture, loot and a copper and coal recipe in `content/ember_lamp/`).
  `/lamp [count]` hands out lamps, and placing or breaking one tells how many
  burn.

```sh
cargo build --release -p ember-protect-spawn -p ember-vein-miner -p ember-lamp
cp target/release/libember_*.so <game folder>/mods/
cp -r mods/ember/lamp/content/ember_lamp <game folder>/mods/
```

`cargo test -p horizon-ember` builds both and drives them through the loader.
