//! Keeps the blocks around the world's origin from being broken or placed
//! by players outside creative mode. `/spawnprotect <radius>|off`.
use ember_api::{Ctx, EmberMod, Event, EventKind, Priority, Registrar};

struct ProtectSpawn {
    radius: Option<i32>,
}

impl EmberMod for ProtectSpawn {
    fn init(reg: &mut Registrar) -> Result<Self, String> {
        reg.on(EventKind::BlockBreak, Priority::High)
            .on(EventKind::BlockPlace, Priority::High)
            .command("spawnprotect", "/spawnprotect <radius>|off");
        Ok(ProtectSpawn { radius: Some(16) })
    }

    fn on_event(&mut self, ctx: &mut Ctx, event: &mut Event) {
        match event.kind() {
            Some(EventKind::BlockBreak | EventKind::BlockPlace) => {
                let Some(radius) = self.radius else { return };
                let [x, _, z] = event.pos();
                if x.abs() > radius || z.abs() > radius {
                    return;
                }
                let creative = ctx.player(event.player()).is_some_and(|p| p.creative);
                if !creative {
                    event.cancel();
                    let player = event.player().to_owned();
                    ctx.tell(&player, "Spawn is protected.");
                }
            }
            Some(EventKind::Command) => {
                let player = event.player().to_owned();
                let reply = match event.args() {
                    "off" => {
                        self.radius = None;
                        "Spawn protection off.".to_owned()
                    }
                    "" => match self.radius {
                        Some(r) => format!("Spawn protected within {r} blocks."),
                        None => "Spawn protection is off.".to_owned(),
                    },
                    arg => match arg.parse::<i32>() {
                        Ok(r) if (0..=1024).contains(&r) => {
                            self.radius = Some(r);
                            format!("Spawn protected within {r} blocks.")
                        }
                        _ => "Usage: /spawnprotect <radius 0-1024>|off".to_owned(),
                    },
                };
                ctx.tell(&player, &reply);
            }
            _ => {}
        }
    }
}

ember_api::ember_mod!(ProtectSpawn, id = "protect-spawn", name = "Protect Spawn");
