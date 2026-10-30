//! An Ember mod with its own block. The Ember Lamp itself (block, item,
//! model, texture, loot, recipe, names) is data in `content/ember_lamp/`,
//! installed beside the library as `mods/ember_lamp/`; this code gives the
//! block its behaviour: `/lamp [count]` hands out lamps, and the mod keeps
//! count of the lamps burning in the world.
use ember_api::{Ctx, EmberMod, Event, EventKind, Priority, Registrar};
use std::collections::HashSet;

const LAMP: &str = "ember_lamp:ember_lamp";

struct Lamp {
    burning: HashSet<[i32; 3]>,
}

impl EmberMod for Lamp {
    fn init(reg: &mut Registrar) -> Result<Self, String> {
        reg.on(EventKind::BlockPlace, Priority::Monitor)
            .on(EventKind::BlockBreak, Priority::Monitor)
            .command("lamp", "/lamp [count]: get Ember Lamps");
        Ok(Lamp {
            burning: HashSet::new(),
        })
    }

    fn on_event(&mut self, ctx: &mut Ctx, event: &mut Event) {
        let player = event.player().to_owned();
        match event.kind() {
            Some(EventKind::BlockPlace) if is_lamp(event.block()) => {
                self.burning.insert(event.pos());
                let n = self.burning.len();
                ctx.tell(&player, &format!("Ember Lamp lit ({n} burning)."));
            }
            Some(EventKind::BlockBreak) if is_lamp(event.block()) => {
                self.burning.remove(&event.pos());
                let n = self.burning.len();
                ctx.tell(&player, &format!("Ember Lamp went out ({n} burning)."));
            }
            Some(EventKind::Command) => match event.args() {
                "" => give(ctx, &player, 4),
                arg => match arg.parse::<u32>() {
                    Ok(n @ 1..=64) => give(ctx, &player, n),
                    _ => ctx.tell(&player, "Usage: /lamp [count 1-64]"),
                },
            },
            _ => {}
        }
    }
}

fn is_lamp(state: &str) -> bool {
    state.split('[').next() == Some(LAMP)
}

fn give(ctx: &mut Ctx, player: &str, n: u32) {
    ctx.run_command(&format!("give {player} {LAMP} {n}"));
    ctx.tell(player, &format!("Here are {n} Ember Lamps."));
}

ember_api::ember_mod!(Lamp, id = "ember-lamp", name = "Ember Lamp");
