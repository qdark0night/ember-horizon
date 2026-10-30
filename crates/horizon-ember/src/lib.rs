//! Ember: Horizon's native mod loader (docs/ember.md).
//!
//! [`Ember::load`] opens every library in the `mods` folder, checks the
//! ABI it was built for and starts it; [`Ember::fire`] passes an event to
//! the mods that listen to it, in priority order, and says whether one
//! cancelled it. What mods ask of the game (chat lines, commands, blocks) is
//! queued as [`Action`]s for the game to apply after the event, so a mod
//! never runs inside the game's own borrow of the world. A mod that panics
//! is stopped; the others and the game keep running.
//!
//! With no mod in the folder nothing is loaded and every hook is one check.
use ember_api::abi::{Entry, Host, RawEvent, RawPlayer, Str};
pub use ember_api::{ABI_VERSION, EventKind, Level, Player, Priority};
use std::collections::{BTreeMap, VecDeque};
use std::ffi::c_void;
use std::path::{Path, PathBuf};

/// The folder Ember loads mods from, beside the game's options.
pub const DIR: &str = "mods";
/// Custom events handled after one event, so two mods answering each
/// other's posts can't hang the game.
const MAX_POSTED: usize = 256;
/// Actions kept per event; a mod asking for more loses the rest.
const MAX_ACTIONS: usize = 4096;

/// What the game lets mods read during an event.
pub trait World {
    /// The block's state (`minecraft:stone`), `None` where no chunk is loaded.
    fn block(&self, pos: [i32; 3]) -> Option<String>;
    fn player(&self, name: &str) -> Option<Player>;
    fn dimension(&self) -> String;
}

/// A world with nothing in it: what mods see while they start.
pub struct NoWorld;
impl World for NoWorld {
    fn block(&self, _pos: [i32; 3]) -> Option<String> {
        None
    }
    fn player(&self, _name: &str) -> Option<Player> {
        None
    }
    fn dimension(&self) -> String {
        String::new()
    }
}

/// What a mod asked the game to do, in the order it asked.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// A chat line for `player`, or for everyone when `player` is `None`.
    Tell {
        from: String,
        player: Option<String>,
        text: String,
    },
    /// A vanilla command, without its slash.
    Command { from: String, line: String },
    SetBlock {
        from: String,
        pos: [i32; 3],
        state: String,
    },
    /// Seat `player` at `pos`, facing `yaw`.
    Sit {
        from: String,
        player: String,
        pos: [f64; 3],
        yaw: f32,
    },
    /// Remove the seats inside the block at `pos`.
    FreeSeat { from: String, pos: [i32; 3] },
}

/// An event to fire: build it with the constructors, then [`Ember::fire`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EventData {
    pub kind: Option<EventKind>,
    pub pos: [i32; 3],
    pub tick: u64,
    pub player: String,
    pub block: String,
    pub item: String,
    pub text: String,
    pub args: String,
}
impl EventData {
    fn of(kind: EventKind) -> Self {
        EventData {
            kind: Some(kind),
            ..Default::default()
        }
    }
    pub fn tick(tick: u64) -> Self {
        EventData {
            tick,
            ..Self::of(EventKind::Tick)
        }
    }
    pub fn world_load() -> Self {
        Self::of(EventKind::WorldLoad)
    }
    pub fn world_unload() -> Self {
        Self::of(EventKind::WorldUnload)
    }
    pub fn block_break(player: &str, pos: [i32; 3], block: &str, item: &str) -> Self {
        EventData {
            pos,
            player: player.into(),
            block: block.into(),
            item: item.into(),
            ..Self::of(EventKind::BlockBreak)
        }
    }
    pub fn block_place(player: &str, pos: [i32; 3], block: &str, item: &str) -> Self {
        EventData {
            kind: Some(EventKind::BlockPlace),
            ..Self::block_break(player, pos, block, item)
        }
    }
    /// `pos`/`block`: the block clicked, if any.
    pub fn use_item(player: &str, item: &str, target: Option<([i32; 3], &str)>) -> Self {
        let (pos, block) = target.unwrap_or(([0; 3], ""));
        EventData {
            kind: Some(EventKind::UseItem),
            ..Self::block_break(player, pos, block, item)
        }
    }
    pub fn chat(player: &str, message: &str) -> Self {
        EventData {
            player: player.into(),
            text: message.into(),
            ..Self::of(EventKind::Chat)
        }
    }
    fn with_tick(mut self, tick: u64) -> Self {
        self.tick = tick;
        self
    }
}

/// What came of an event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fired {
    pub cancelled: bool,
}

/// A loaded mod, for the game's list and log.
#[derive(Clone, Debug, PartialEq)]
pub struct ModInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    /// Why it isn't running (failed to load, init failed, panicked).
    pub problem: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub name: String,
    pub usage: String,
    pub owner: String,
}

struct Running {
    /// The ABI the mod was built for: its host says that one.
    abi: u32,
    instance: *mut c_void,
    on_event: unsafe extern "C" fn(*mut c_void, *const Host, *mut RawEvent) -> u32,
    drop: unsafe extern "C" fn(*mut c_void),
}

struct Loaded {
    info: ModInfo,
    run: Option<Running>,
    // Dropped after `run`'s instance (see `Drop`): its code is in here.
    _lib: Option<libloading::Library>,
}
impl Drop for Loaded {
    fn drop(&mut self) {
        if let Some(run) = self.run.take() {
            unsafe { (run.drop)(run.instance) };
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Listener {
    kind: EventKind,
    priority: i32,
    cancelled_too: bool,
    owner: usize,
}

/// The loaded mods and their bus.
#[derive(Default)]
pub struct Ember {
    mods: Vec<Loaded>,
    /// By priority, then load order.
    listeners: Vec<Listener>,
    commands: BTreeMap<String, (usize, String)>,
    actions: Vec<Action>,
}
// The instances are only ever called from the thread that owns the Ember.
unsafe impl Send for Ember {}

/// The libraries in `dir`, sorted by name.
pub fn find(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = read
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .filter(|p| {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or_default();
            ext.eq_ignore_ascii_case(std::env::consts::DLL_EXTENSION)
        })
        .collect();
    out.sort();
    out
}

/// What mods register while they start.
#[derive(Default)]
struct Registration {
    subscriptions: Vec<(u32, i32, bool)>,
    commands: Vec<(String, String)>,
}

/// One call's state behind the host's `ctx` pointer.
struct Call<'a> {
    mod_id: &'a str,
    world: &'a dyn World,
    actions: &'a mut Vec<Action>,
    posted: &'a mut VecDeque<(String, String)>,
    registration: Option<&'a mut Registration>,
    /// What `block` and `dimension` lend until the next call.
    scratch: String,
}

unsafe fn call<'a>(ctx: *mut c_void) -> &'a mut Call<'a> {
    unsafe { &mut *ctx.cast::<Call<'a>>() }
}

impl Call<'_> {
    fn act(&mut self, action: Action) {
        if self.actions.len() < MAX_ACTIONS {
            self.actions.push(action);
        }
    }
}

unsafe extern "C" fn host_log(ctx: *mut c_void, level: u32, message: Str) {
    let c = unsafe { call(ctx) };
    let message = unsafe { message.as_str() };
    let level = match level {
        1 => log::Level::Error,
        2 => log::Level::Warn,
        4 => log::Level::Debug,
        _ => log::Level::Info,
    };
    log::log!(level, "[ember {}] {message}", c.mod_id);
}
unsafe extern "C" fn host_subscribe(ctx: *mut c_void, kind: u32, priority: i32, cancelled: bool) {
    let c = unsafe { call(ctx) };
    if let Some(r) = c.registration.as_deref_mut() {
        r.subscriptions.push((kind, priority, cancelled));
    }
}
unsafe extern "C" fn host_register_command(ctx: *mut c_void, name: Str, usage: Str) {
    let c = unsafe { call(ctx) };
    let (name, usage) = unsafe { (name.as_str(), usage.as_str()) };
    if let Some(r) = c.registration.as_deref_mut() {
        r.commands.push((name.to_owned(), usage.to_owned()));
    }
}
unsafe extern "C" fn host_tell(ctx: *mut c_void, player: Str, text: Str) {
    let c = unsafe { call(ctx) };
    let (player, text) = unsafe { (player.as_str(), text.as_str()) };
    let action = Action::Tell {
        from: c.mod_id.to_owned(),
        player: (!player.is_empty()).then(|| player.to_owned()),
        text: text.to_owned(),
    };
    c.act(action);
}
unsafe extern "C" fn host_run_command(ctx: *mut c_void, line: Str) {
    let c = unsafe { call(ctx) };
    let line = unsafe { line.as_str() };
    let line = line.trim().trim_start_matches('/');
    if !line.is_empty() {
        let action = Action::Command {
            from: c.mod_id.to_owned(),
            line: line.to_owned(),
        };
        c.act(action);
    }
}
unsafe extern "C" fn host_set_block(ctx: *mut c_void, x: i32, y: i32, z: i32, state: Str) {
    let c = unsafe { call(ctx) };
    let state = unsafe { state.as_str() };
    let action = Action::SetBlock {
        from: c.mod_id.to_owned(),
        pos: [x, y, z],
        state: state.to_owned(),
    };
    c.act(action);
}
unsafe extern "C" fn host_block(ctx: *mut c_void, x: i32, y: i32, z: i32) -> Str {
    let c = unsafe { call(ctx) };
    c.scratch = c.world.block([x, y, z]).unwrap_or_default();
    Str::new(&c.scratch)
}
unsafe extern "C" fn host_player(ctx: *mut c_void, name: Str) -> RawPlayer {
    let c = unsafe { call(ctx) };
    let name = unsafe { name.as_str() };
    c.world
        .player(name)
        .map_or_else(RawPlayer::default, |p| RawPlayer {
            present: true,
            pos: p.pos,
            yaw: p.yaw,
            pitch: p.pitch,
            health: p.health,
            food: p.food,
            creative: p.creative,
            sneaking: p.sneaking,
        })
}
unsafe extern "C" fn host_post(ctx: *mut c_void, channel: Str, payload: Str) {
    let c = unsafe { call(ctx) };
    let (channel, payload) = unsafe { (channel.as_str(), payload.as_str()) };
    if c.posted.len() < MAX_POSTED {
        c.posted.push_back((channel.to_owned(), payload.to_owned()));
    }
}
unsafe extern "C" fn host_dimension(ctx: *mut c_void) -> Str {
    let c = unsafe { call(ctx) };
    c.scratch = c.world.dimension();
    Str::new(&c.scratch)
}

unsafe extern "C" fn host_sit(ctx: *mut c_void, player: Str, x: f64, y: f64, z: f64, yaw: f32) {
    let c = unsafe { call(ctx) };
    let player = unsafe { player.as_str() };
    let action = Action::Sit {
        from: c.mod_id.to_owned(),
        player: player.to_owned(),
        pos: [x, y, z],
        yaw,
    };
    c.act(action);
}
unsafe extern "C" fn host_free_seat(ctx: *mut c_void, x: i32, y: i32, z: i32) {
    let c = unsafe { call(ctx) };
    let action = Action::FreeSeat {
        from: c.mod_id.to_owned(),
        pos: [x, y, z],
    };
    c.act(action);
}

/// The host for a mod built for `abi`: an older mod checks it is its own
/// version, and reads only the functions that version has, a prefix of ours.
fn host(ctx: &mut Call, abi: u32) -> Host {
    Host {
        abi,
        ctx: (ctx as *mut Call).cast::<c_void>(),
        log: host_log,
        subscribe: host_subscribe,
        register_command: host_register_command,
        tell: host_tell,
        run_command: host_run_command,
        set_block: host_set_block,
        block: host_block,
        player: host_player,
        post: host_post,
        dimension: host_dimension,
        sit: host_sit,
        free_seat: host_free_seat,
    }
}

/// Strings an event lends its listeners.
struct RawStrings<'a>(&'a EventData);
impl RawStrings<'_> {
    fn event(&self) -> RawEvent {
        let e = self.0;
        let kind = e.kind.unwrap_or(EventKind::Tick);
        RawEvent {
            kind: kind as u32,
            cancelable: kind.cancelable(),
            cancelled: false,
            pos: e.pos,
            tick: e.tick,
            player: Str::new(&e.player),
            block: Str::new(&e.block),
            item: Str::new(&e.item),
            text: Str::new(&e.text),
            args: Str::new(&e.args),
        }
    }
}

impl Ember {
    /// Loads and starts the mods at `paths`; `reserved` names the commands
    /// they may not take (vanilla's).
    pub fn load(paths: &[PathBuf], reserved: impl Fn(&str) -> bool) -> Ember {
        let mut ember = Ember::default();
        for path in paths {
            ember.load_one(path, &reserved);
        }
        ember
    }

    /// Loads every library in `dir` ([`find`]).
    pub fn load_dir(dir: &Path, reserved: impl Fn(&str) -> bool) -> Ember {
        Self::load(&find(dir), reserved)
    }

    fn load_one(&mut self, path: &Path, reserved: &impl Fn(&str) -> bool) {
        let file = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut info = ModInfo {
            id: file.clone(),
            name: file,
            version: String::new(),
            path: path.to_owned(),
            problem: None,
        };
        // Loading runs the library's initialisers: native code, trusted by
        // the player who put it in the folder.
        let lib = match unsafe { libloading::Library::new(path) } {
            Ok(lib) => lib,
            Err(e) => return self.failed(info, None, format!("can't load: {e}")),
        };
        let abi = unsafe { lib.get::<*const u32>(ember_api::ABI_SYMBOL) }
            .ok()
            .map(|s| unsafe { **s });
        match abi {
            None => return self.failed(info, Some(lib), "not an Ember mod (no EMBER_ABI)".into()),
            Some(v) if v > ABI_VERSION => {
                let why = format!(
                    "built for Ember ABI {v}, this game has {ABI_VERSION}: update the game"
                );
                return self.failed(info, Some(lib), why);
            }
            Some(_) => {}
        }
        let abi = abi.unwrap_or(ABI_VERSION);
        let entry: Entry = match unsafe { lib.get::<Entry>(ember_api::ENTRY_SYMBOL) } {
            Ok(entry) => *entry,
            Err(e) => return self.failed(info, Some(lib), format!("no entry: {e}")),
        };
        let mut registration = Registration::default();
        let mut actions = Vec::new();
        let mut posted = VecDeque::new();
        let raw = {
            let mut c = Call {
                mod_id: &info.id,
                world: &NoWorld,
                actions: &mut actions,
                posted: &mut posted,
                registration: Some(&mut registration),
                scratch: String::new(),
            };
            let host = host(&mut c, abi);
            unsafe { entry(&host) }
        };
        unsafe {
            info.id = raw.id.as_str().to_owned();
            info.name = raw.name.as_str().to_owned();
            info.version = raw.version.as_str().to_owned();
        }
        if raw.abi > ABI_VERSION {
            return self.failed(info, Some(lib), "entry returned another ABI".into());
        }
        if raw.instance.is_null() {
            let why = unsafe { raw.error.as_str() };
            return self.failed(info, Some(lib), format!("init failed: {why}"));
        }
        if self
            .mods
            .iter()
            .any(|m| m.info.id == info.id && m.run.is_some())
        {
            unsafe { (raw.drop)(raw.instance) };
            let why = format!("another mod has id {}", info.id);
            return self.failed(info, Some(lib), why);
        }
        let owner = self.mods.len();
        for (kind, priority, cancelled_too) in registration.subscriptions {
            match EventKind::from_u32(kind) {
                Some(EventKind::Command) | None => {}
                Some(kind) => self.listeners.push(Listener {
                    kind,
                    priority,
                    cancelled_too,
                    owner,
                }),
            }
        }
        // Stable: equal priorities keep load order.
        self.listeners.sort_by_key(|l| l.priority);
        for (name, usage) in registration.commands {
            let name = name.trim().trim_start_matches('/').to_owned();
            let valid = !name.is_empty() && !name.contains(char::is_whitespace);
            if !valid {
                log::warn!("[ember {}] command {name:?} refused: not a name", info.id);
            } else if reserved(&name) {
                log::warn!("[ember {}] /{name} refused: a vanilla command", info.id);
            } else if let Some((other, _)) = self.commands.get(&name) {
                let other = &self.mods[*other].info.id;
                log::warn!("[ember {}] /{name} refused: {other} owns it", info.id);
            } else {
                self.commands.insert(name, (owner, usage));
            }
        }
        log::info!(
            "Ember: loaded {} {} from {}",
            info.id,
            info.version,
            path.display()
        );
        self.mods.push(Loaded {
            info,
            run: Some(Running {
                abi,
                instance: raw.instance,
                on_event: raw.on_event,
                drop: raw.drop,
            }),
            _lib: Some(lib),
        });
        self.actions.extend(actions);
    }

    fn failed(&mut self, mut info: ModInfo, lib: Option<libloading::Library>, why: String) {
        log::warn!("Ember: {} not loaded: {why}", info.path.display());
        info.problem = Some(why);
        self.mods.push(Loaded {
            info,
            run: None,
            _lib: lib,
        });
    }

    /// No mod running: the game skips every hook.
    pub fn is_empty(&self) -> bool {
        !self.mods.iter().any(|m| m.run.is_some())
    }

    /// Whether a running mod listens to `kind`.
    pub fn listens(&self, kind: EventKind) -> bool {
        self.listeners
            .iter()
            .any(|l| l.kind == kind && self.mods[l.owner].run.is_some())
    }

    pub fn mods(&self) -> impl Iterator<Item = &ModInfo> {
        self.mods.iter().map(|m| &m.info)
    }

    /// The commands running mods own.
    pub fn commands(&self) -> impl Iterator<Item = Command> + '_ {
        self.commands
            .iter()
            .filter(|(_, (owner, _))| self.mods[*owner].run.is_some())
            .map(|(name, (owner, usage))| Command {
                name: name.clone(),
                usage: usage.clone(),
                owner: self.mods[*owner].info.id.clone(),
            })
    }

    /// Fires `event` through the bus: each listener in priority order,
    /// those that don't take cancelled events skipped once one cancelled it.
    /// Custom events the mods post meanwhile follow it.
    pub fn fire(&mut self, world: &dyn World, event: EventData) -> Fired {
        let Some(kind) = event.kind else {
            return Fired::default();
        };
        if !self.listens(kind) {
            return Fired::default();
        }
        let tick = event.tick;
        let mut posted = VecDeque::new();
        let fired = self.dispatch(world, &event, None, &mut posted);
        self.run_posted(world, tick, posted);
        fired
    }

    /// `/line` typed by `player`: `true` when a mod owns the command (its
    /// owner heard it as [`EventKind::Command`]).
    pub fn command(&mut self, world: &dyn World, player: &str, line: &str, tick: u64) -> bool {
        let line = line.trim_start_matches('/');
        let (name, args) = line.split_once(' ').unwrap_or((line, ""));
        let Some(&(owner, _)) = self.commands.get(name) else {
            return false;
        };
        if self.mods[owner].run.is_none() {
            return false;
        }
        let event = EventData {
            player: player.into(),
            text: name.into(),
            args: args.trim().into(),
            ..EventData::of(EventKind::Command)
        }
        .with_tick(tick);
        let mut posted = VecDeque::new();
        self.dispatch(world, &event, Some(owner), &mut posted);
        self.run_posted(world, tick, posted);
        true
    }

    fn run_posted(&mut self, world: &dyn World, tick: u64, mut posted: VecDeque<(String, String)>) {
        let mut handled = 0;
        while let Some((channel, payload)) = posted.pop_front() {
            handled += 1;
            if handled > MAX_POSTED {
                log::warn!("Ember: more than {MAX_POSTED} custom events at once: the rest dropped");
                break;
            }
            let event = EventData {
                text: channel,
                args: payload,
                ..EventData::of(EventKind::Custom)
            }
            .with_tick(tick);
            self.dispatch(world, &event, None, &mut posted);
        }
    }

    fn dispatch(
        &mut self,
        world: &dyn World,
        event: &EventData,
        only: Option<usize>,
        posted: &mut VecDeque<(String, String)>,
    ) -> Fired {
        let strings = RawStrings(event);
        let mut raw = strings.event();
        let kind = event.kind.unwrap_or(EventKind::Tick);
        let targets: Vec<(usize, bool)> = match only {
            Some(owner) => vec![(owner, true)],
            None => self
                .listeners
                .iter()
                .filter(|l| l.kind == kind)
                .map(|l| (l.owner, l.cancelled_too))
                .collect(),
        };
        for (owner, cancelled_too) in targets {
            if raw.cancelled && !cancelled_too {
                continue;
            }
            let m = &mut self.mods[owner];
            let Some(run) = &m.run else { continue };
            let mut c = Call {
                mod_id: &m.info.id,
                world,
                actions: &mut self.actions,
                posted,
                registration: None,
                scratch: String::new(),
            };
            let host = host(&mut c, run.abi);
            let status = unsafe { (run.on_event)(run.instance, &host, &mut raw) };
            // A mod can't make an event cancelable.
            raw.cancelable = kind.cancelable();
            raw.cancelled &= raw.cancelable;
            if status != 0 {
                log::error!("Ember: {} panicked on {kind:?}: stopped", m.info.id);
                m.info.problem = Some(format!("panicked on {kind:?}"));
                if let Some(run) = m.run.take() {
                    unsafe { (run.drop)(run.instance) };
                }
            }
        }
        Fired {
            cancelled: raw.cancelled,
        }
    }

    /// What the mods asked for since the last call, in order.
    pub fn drain(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }
}
