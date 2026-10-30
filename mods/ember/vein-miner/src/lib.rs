//! Breaking an ore or a log while sneaking breaks the whole vein, with
//! drops, as vanilla's `/setblock ... destroy` breaks blocks. Listens at
//! `Monitor` priority, so a block another mod saved (Protect Spawn) starts
//! no vein. `/veinminer` turns it on and off. Posts `vein-miner:mined` with
//! the count, for other mods.
use ember_api::{Ctx, EmberMod, Event, EventKind, Priority, Registrar};
use std::collections::{HashSet, VecDeque};

const MAX_BLOCKS: usize = 64;

struct VeinMiner {
    on: bool,
}

fn veins(block: &str) -> bool {
    let name = block.split('[').next().unwrap_or(block);
    name.ends_with("_ore") || name.ends_with("_log") || name == "minecraft:ancient_debris"
}

impl EmberMod for VeinMiner {
    fn init(reg: &mut Registrar) -> Result<Self, String> {
        reg.on(EventKind::BlockBreak, Priority::Monitor)
            .command("veinminer", "/veinminer: turn vein mining on or off");
        Ok(VeinMiner { on: true })
    }

    fn on_event(&mut self, ctx: &mut Ctx, event: &mut Event) {
        match event.kind() {
            Some(EventKind::BlockBreak) => {
                if !self.on || !veins(event.block()) {
                    return;
                }
                let player = event.player().to_owned();
                if !ctx.player(&player).is_some_and(|p| p.sneaking) {
                    return;
                }
                let name = event
                    .block()
                    .split('[')
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                let start = event.pos();
                let mut seen = HashSet::from([start]);
                let mut queue = VecDeque::from([start]);
                let mut mined = 0;
                while let Some([x, y, z]) = queue.pop_front() {
                    for dx in -1..=1 {
                        for dy in -1..=1 {
                            for dz in -1..=1 {
                                let p = [x + dx, y + dy, z + dz];
                                if mined >= MAX_BLOCKS || !seen.insert(p) {
                                    continue;
                                }
                                let same = ctx
                                    .block(p)
                                    .is_some_and(|b| b.split('[').next() == Some(&name));
                                if same {
                                    ctx.run_command(&format!(
                                        "setblock {} {} {} minecraft:air destroy",
                                        p[0], p[1], p[2]
                                    ));
                                    mined += 1;
                                    queue.push_back(p);
                                }
                            }
                        }
                    }
                }
                if mined > 0 {
                    ctx.post("vein-miner:mined", &mined.to_string());
                }
            }
            Some(EventKind::Command) => {
                self.on = !self.on;
                let player = event.player().to_owned();
                let state = if self.on { "on" } else { "off" };
                ctx.tell(&player, &format!("Vein miner {state}."));
            }
            _ => {}
        }
    }
}

ember_api::ember_mod!(VeinMiner, id = "vein-miner", name = "Vein Miner");
