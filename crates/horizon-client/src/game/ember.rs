//! Ember (docs/ember.md): native mods from the `mods` folder next to
//! options.txt. The game that simulates the world (in-frame single player,
//! the server thread, a LAN host, the dedicated server) loads them when its
//! first world opens and keeps them for the process, as Forge does. Players'
//! breaking, placing, using, chat and commands reach their event bus, which
//! may cancel the first four; what the mods ask for is done at the tick's
//! end. With no Ember mod, each hook here is one check.
use super::Game;
use horizon_client::{command::vanilla, lan::data::component_nbt, text::Text};
use horizon_ember::{Action, Ember, EventData, EventKind, Player, World};
use horizon_protocol::packets::play::{self, Clientbound as C};
use winit::keyboard::KeyCode;

#[derive(Default)]
pub(super) struct EmberState {
    ember: Ember,
    loaded: bool,
    open: bool,
    /// The seats players sit on, or are about to.
    seats: Vec<Seat>,
    /// The block of the last `UseItem`: the one a seat is for.
    used: Option<[i32; 3]>,
}

/// A player's seat: a summoned mob joins the world after the tick, so they
/// mount it a tick or more later; it goes when they get off or its block
/// changes.
struct Seat {
    player: String,
    block: Option<([i32; 3], String)>,
    yaw: f32,
    age: u16,
    seated: bool,
    mob: Option<u32>,
}

fn block_id(state: &str) -> &str {
    state.split('[').next().unwrap_or(state)
}

/// The world as a mod sees it during a call; `extra` is a player the game
/// can't see then (a guest whose command arrives outside their turn).
struct View<'a, 'g> {
    game: &'a Game<'g>,
    extra: Option<(String, Player)>,
}
impl World for View<'_, '_> {
    fn block(&self, [x, y, z]: [i32; 3]) -> Option<String> {
        Some(self.game.state_string(self.game.world.block(x, y, z)))
    }
    fn player(&self, name: &str) -> Option<Player> {
        match &self.extra {
            Some((n, p)) if n == name => Some(*p),
            _ => self.game.ember_player(name),
        }
    }
    fn dimension(&self) -> String {
        self.game.dim.id().to_owned()
    }
}

fn vanilla_command(name: &str) -> bool {
    let d = vanilla::game_dispatcher();
    d.nodes[0]
        .children
        .iter()
        .any(|&c| d.nodes[c].name() == name)
}

pub(super) fn guest_player(g: &horizon_client::lan::guest::Guest) -> Player {
    Player {
        pos: g.position,
        yaw: g.yaw,
        pitch: g.pitch,
        health: 20.0,
        food: 20,
        creative: g.game_mode == 1,
        sneaking: g.sneaking,
    }
}

/// The tag of the mobs that are Ember seats.
pub(super) const SEAT_TAG: &str = "ember_seat";

impl Game<'_> {
    /// The player acting: the guest whose turn it is, else the window's.
    pub(super) fn ember_actor(&self) -> String {
        self.lan_acting_name
            .clone()
            .unwrap_or_else(|| self.options.player_name())
    }
    fn ember_player(&self, name: &str) -> Option<Player> {
        if let Some(p) = &self.player
            && name == self.ember_actor()
        {
            return Some(Player {
                pos: [p.position.x, p.position.y, p.position.z],
                yaw: p.yaw,
                pitch: self.pitch,
                health: p.health.health(),
                food: p.health.food.food,
                creative: self.creative,
                sneaking: self.keys.contains(&KeyCode::ShiftLeft),
            });
        }
        let host = self.lan.as_ref()?;
        let g = host
            .hub
            .guests
            .iter()
            .find(|g| g.placed && g.name == name)?;
        Some(guest_player(g))
    }
    fn ember_running(&self, kind: EventKind) -> bool {
        self.ember.open && self.ember.ember.listens(kind)
    }
    /// Fires `event` on the bus: whether a mod cancelled it.
    pub(super) fn ember_fire(&mut self, mut event: EventData) -> bool {
        if !event.kind.is_some_and(|k| self.ember_running(k)) {
            return false;
        }
        event.tick = self.ticks;
        let mut ember = std::mem::take(&mut self.ember.ember);
        let view = View {
            game: self,
            extra: None,
        };
        let fired = ember.fire(&view, event);
        self.ember.ember = ember;
        fired.cancelled
    }
    pub(super) fn ember_break(&mut self, pos: [i32; 3]) -> bool {
        if !self.ember_running(EventKind::BlockBreak) {
            return false;
        }
        let block = self.state_string(self.world.block(pos[0], pos[1], pos[2]));
        let item = self.held().map(|s| s.item.to_string()).unwrap_or_default();
        let actor = self.ember_actor();
        self.ember_fire(EventData::block_break(&actor, pos, &block, &item))
    }
    pub(super) fn ember_place(
        &mut self,
        pos: [i32; 3],
        state: horizon_world::registry::BlockStateId,
        item: &str,
    ) -> bool {
        if !self.ember_running(EventKind::BlockPlace) {
            return false;
        }
        let block = self.state_string(state);
        let actor = self.ember_actor();
        self.ember_fire(EventData::block_place(&actor, pos, &block, item))
    }
    pub(super) fn ember_use(&mut self) -> bool {
        if !self.ember_running(EventKind::UseItem) {
            return false;
        }
        let item = self.held().map(|s| s.item.to_string()).unwrap_or_default();
        let actor = self.ember_actor();
        let target = self.pick().map(|h| {
            let state = self.world.block(h.pos[0], h.pos[1], h.pos[2]);
            (h.pos, self.state_string(state))
        });
        self.ember.used = target.as_ref().map(|(pos, _)| *pos);
        let target = target.as_ref().map(|(pos, block)| (*pos, block.as_str()));
        self.ember_fire(EventData::use_item(&actor, &item, target))
    }
    /// A command for an Ember mod, from `player` (`extra` when the game
    /// can't see them now): whether one took it.
    pub(super) fn ember_command(
        &mut self,
        player: &str,
        extra: Option<Player>,
        line: &str,
    ) -> bool {
        if !self.ember.open {
            return false;
        }
        let mut ember = std::mem::take(&mut self.ember.ember);
        let view = View {
            game: self,
            extra: extra.map(|p| (player.to_owned(), p)),
        };
        let taken = ember.command(&view, player, line, self.ticks);
        self.ember.ember = ember;
        taken
    }
    pub(super) fn local_ember_command(&mut self, line: &str) -> bool {
        let name = self.options.player_name();
        self.ember_command(&name, None, line)
    }

    /// The tick's end: open on the first world, the tick event, then what
    /// the mods asked for.
    pub(super) fn tick_ember(&mut self) {
        self.tick_seats();
        if !self.ember.open {
            // Only the game that simulates runs them, never a server's client.
            if self.remote.is_some() || self.ember.ember.is_empty() && self.ember.loaded {
                return;
            }
            if !self.ember.loaded {
                self.ember.loaded = true;
                let dir = Self::options_path().with_file_name(horizon_ember::DIR);
                self.ember.ember = Ember::load_dir(&dir, vanilla_command);
                for m in self.ember.ember.mods() {
                    match &m.problem {
                        None => log::info!("Ember: loaded {} {} ({})", m.id, m.version, m.name),
                        Some(p) => log::warn!("Ember: {}: {p}", m.path.display()),
                    }
                }
            }
            if self.ember.ember.is_empty() {
                return;
            }
            self.ember.open = true;
            self.ember_fire(EventData::world_load());
        }
        if self.ember_running(EventKind::Tick) {
            self.ember_fire(EventData::tick(self.ticks));
        }
        self.apply_ember_actions();
    }
    pub(super) fn close_ember(&mut self) {
        if self.ember.open {
            self.ember_fire(EventData::world_unload());
            self.apply_ember_actions();
            self.ember.open = false;
        }
    }
    fn apply_ember_actions(&mut self) {
        for action in self.ember.ember.drain() {
            match action {
                Action::Tell { player, text, .. } => self.ember_tell(player.as_deref(), &text),
                Action::Command { line, .. } => {
                    self.ember_run(&line);
                }
                Action::Sit {
                    player, pos, yaw, ..
                } => self.ember_sit(&player, pos, yaw),
                Action::FreeSeat { pos, .. } => {
                    let mut seats = std::mem::take(&mut self.ember.seats);
                    seats.retain(|seat| {
                        let here = seat.block.as_ref().is_some_and(|(b, _)| *b == pos);
                        if here {
                            self.ember_stand(seat);
                        }
                        !here
                    });
                    self.ember.seats.append(&mut seats);
                }
                Action::SetBlock { pos, state, .. } => {
                    self.ember_run(&format!(
                        "setblock {} {} {} {state}",
                        pos[0], pos[1], pos[2]
                    ));
                }
            }
        }
    }
    /// A command as the server (`CommandSourceStack` of the server, its
    /// output dropped).
    fn ember_run(&mut self, line: &str) -> bool {
        let quiet = super::commands::execute::Source {
            silent: false,
            ..self.server_source()
        };
        let outer = self.command_source.replace(quiet);
        let capture = self.chat_capture.replace(Vec::new());
        let ok = self.as_console(|g| g.run_command(line));
        let told = std::mem::replace(&mut self.chat_capture, capture).unwrap_or_default();
        if !ok {
            let why: Vec<String> = told.iter().map(|t| t.plain()).collect();
            log::info!("Ember: `{line}` failed: {}", why.join("; "));
        }
        self.command_source = outer;
        ok
    }
    /// `ember_run` with `player` in the game's player slot: a LAN guest
    /// (the window's player in single player too) is only `Target::Player`
    /// during their turn, and `ride` takes nobody else.
    fn ember_run_as(&mut self, player: &str, line: &str) -> bool {
        let conn = self.lan.as_ref().and_then(|h| {
            let g = h.hub.guests.iter().find(|g| g.placed && g.name == player)?;
            Some(g.conn)
        });
        if let Some(conn) = conn
            && self.lan_acting != Some(conn)
            && let Some(mut host) = self.lan.take()
        {
            let ok = self.as_guest(&mut host, conn, |g| g.ember_run(line));
            self.lan = Some(host);
            return ok.unwrap_or(false);
        }
        self.ember_run(line)
    }
    /// Whether `id` is an Ember seat: the HUD shows no vehicle for it.
    pub(super) fn is_ember_seat(&self, id: u32) -> bool {
        self.ember.seats.iter().any(|s| s.mob == Some(id))
    }
    /// Seat upkeep: mount the new ones, and stand up whoever got off (Shift,
    /// death, teleport) or whose seat's block changed.
    fn tick_seats(&mut self) {
        if self.ember.open && self.ember.ember.is_empty() || self.ember.seats.is_empty() {
            return;
        }
        let me = self.options.player_name();
        let mut seats = std::mem::take(&mut self.ember.seats);
        seats.retain_mut(|seat| {
            seat.age = seat.age.saturating_add(1);
            let p = seat.player.clone();
            let block_gone = seat.block.as_ref().is_some_and(|(b, id)| {
                let now = self.state_string(self.world.block(b[0], b[1], b[2]));
                block_id(&now) != id
            });
            let on = if p == me {
                self.riding.is_some_and(|r| seat.mob.is_none_or(|m| m == r))
            } else {
                self.ember_run_as(
                    &p,
                    &format!("execute as {p} on vehicle if entity @s[tag={SEAT_TAG}_{p}]"),
                )
            };
            if on && !seat.seated {
                seat.seated = true;
                seat.mob = if p == me { self.riding } else { None };
            }
            if block_gone || seat.seated && !on {
                if on {
                    self.ember_run_as(&p, &format!("ride {p} dismount"));
                }
                self.ember_stand(seat);
                return false;
            }
            if !seat.seated {
                if seat.age > 10 {
                    self.ember_run(&format!("kill @e[type=bat,tag={SEAT_TAG}_{p}]"));
                    return false;
                }
                if seat.age % 2 == 1 {
                    self.ember_run_as(
                        &p,
                        &format!("ride {p} mount @e[type=bat,tag={SEAT_TAG}_{p},limit=1]"),
                    );
                }
            }
            true
        });
        seats.append(&mut self.ember.seats);
        self.ember.seats = seats;
    }
    /// Off the seat: its mob goes, and the player stands in front of the
    /// block, or on it when that is blocked.
    fn ember_stand(&mut self, seat: &Seat) {
        let p = &seat.player;
        self.ember_run(&format!("kill @e[type=bat,tag={SEAT_TAG}_{p}]"));
        let Some(([x, y, z], _)) = seat.block else {
            return;
        };
        let yaw = seat.yaw.to_radians();
        let (fx, fz) = (
            x + (-yaw.sin()).round() as i32,
            z + yaw.cos().round() as i32,
        );
        let free = |g: &Self, y: i32| g.world.block(fx, y, fz) == g.world.block(fx, 400, fz);
        let to = if free(self, y) && free(self, y + 1) {
            [fx as f64 + 0.5, y as f64, fz as f64 + 0.5]
        } else {
            [x as f64 + 0.5, y as f64 + 1.0, z as f64 + 0.5]
        };
        self.ember_run_as(p, &format!("tp {p} {} {} {}", to[0], to[1], to[2]));
    }
    /// An Ember seat: an invisible, still, silent bat the player rides; the
    /// HUD shows no hearts for it (`draw_mount_hud`).
    fn ember_sit(&mut self, player: &str, [x, y, z]: [f64; 3], yaw: f32) {
        if player.is_empty() || player.contains(' ') {
            return;
        }
        self.ember_run_as(player, &format!("ride {player} dismount"));
        self.ember_run(&format!("kill @e[type=bat,tag={SEAT_TAG}_{player}]"));
        let block = self.ember.used.take().map(|b| {
            let state = self.state_string(self.world.block(b[0], b[1], b[2]));
            (b, block_id(&state).to_owned())
        });
        self.ember_run_as(player, &format!("rotate {player} {yaw} 0"));
        self.ember_run(&format!(
            "summon bat {x} {y} {z} {{NoAI:1b,NoGravity:1b,Silent:1b,Invulnerable:1b,PersistenceRequired:1b,Tags:[{SEAT_TAG},{SEAT_TAG}_{player}],active_effects:[{{id:invisibility,duration:-1,show_particles:0b}}]}}"
        ));
        self.ember.seats.retain(|s| s.player != player);
        self.ember.seats.push(Seat {
            player: player.to_owned(),
            block,
            yaw,
            age: 0,
            seated: false,
            mob: None,
        });
    }
    fn ember_tell(&mut self, to: Option<&str>, text: &str) {
        let line = Text::literal(text.to_owned());
        let local = self.options.execution_role.is_client()
            && self.remote.is_none()
            && self.player.is_some();
        if local && to.is_none_or(|n| n == self.options.player_name()) {
            self.system_message(line.clone());
        }
        if let Some(host) = self.lan.as_mut() {
            let content = component_nbt(&line.component());
            let conns: Vec<u32> = host
                .hub
                .guests
                .iter()
                .filter(|g| g.placed && to.is_none_or(|n| n == g.name))
                .map(|g| g.conn)
                .collect();
            for conn in conns {
                host.hub.send_to(
                    conn,
                    &C::SystemChat(play::SystemChat {
                        content: content.clone(),
                        overlay: false,
                    }),
                );
            }
        }
        if to.is_none() {
            log::info!("[Ember] {text}");
        }
    }
}
