//! Furniture on Ember. The blocks (chairs, stool, armchair, bench, tables,
//! nightstand, kitchen counter, bookcase, lamps, a test block) are data in
//! `content/furniture/`, installed beside the library as `mods/furniture/`;
//! this code makes the seats seats: a right click with an empty hand sits
//! you down facing the way the seat faces, Shift stands you up, and
//! breaking a seat stands its rider up. No cheats needed.
use ember_api::{Ctx, EmberMod, Event, EventKind, Priority, Registrar};

const SEATS: [&str; 4] = [
    "furniture:chair",
    "furniture:stool",
    "furniture:armchair",
    "furniture:bench",
];
/// Below the block's floor, so the rider's feet sit on the seat.
const SEAT_Y: f64 = -0.8;

struct Furniture;

fn seat_yaw(state: &str) -> Option<f32> {
    let id = state.split('[').next()?;
    if !SEATS.contains(&id) {
        return None;
    }
    let facing = state
        .split("facing=")
        .nth(1)
        .map(|f| f.trim_end_matches(']'));
    Some(match facing.and_then(|f| f.split(',').next()) {
        Some("north") => 180.0,
        Some("east") => -90.0,
        Some("west") => 90.0,
        _ => 0.0,
    })
}

impl EmberMod for Furniture {
    fn init(reg: &mut Registrar) -> Result<Self, String> {
        reg.on(EventKind::UseItem, Priority::High)
            .on(EventKind::BlockBreak, Priority::Monitor);
        Ok(Furniture)
    }

    fn on_event(&mut self, ctx: &mut Ctx, event: &mut Event) {
        match event.kind() {
            Some(EventKind::UseItem) => {
                let empty = matches!(event.item(), "" | "minecraft:air");
                let Some(yaw) = seat_yaw(event.block()).filter(|_| empty) else {
                    return;
                };
                let [x, y, z] = event.pos();
                let player = event.player().to_owned();
                let seat = [x as f64 + 0.5, y as f64 + SEAT_Y, z as f64 + 0.5];
                ctx.sit(&player, seat, yaw);
                event.cancel();
            }
            Some(EventKind::BlockBreak) if seat_yaw(event.block()).is_some() => {
                ctx.free_seat(event.pos());
            }
            _ => {}
        }
    }
}

ember_api::ember_mod!(Furniture, id = "furniture", name = "Furniture");
