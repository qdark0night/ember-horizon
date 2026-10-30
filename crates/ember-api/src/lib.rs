//! Ember: native Horizon mods loaded at runtime, as Forge loads jars.
//!
//! A mod is a `cdylib` that depends on this crate, implements [`EmberMod`]
//! and names its type in [`ember_mod!`]. The game loads the library from its
//! `mods` folder; nothing of the game is rebuilt or forked.
//! Mods and game talk only through the `#[repr(C)]` types of [`abi`], so a
//! mod built with another Rust compiler keeps loading. [`abi::Host`] only
//! grows at its end, so a mod built for ABI N loads in every game with ABI N
//! or later: new games never break old mods.
//!
//! ```ignore
//! use ember_api::{Ctx, Event, EventKind, EmberMod, Priority, Registrar};
//!
//! struct Hello;
//! impl EmberMod for Hello {
//!     fn init(reg: &mut Registrar) -> Result<Self, String> {
//!         reg.on(EventKind::BlockBreak, Priority::Normal);
//!         reg.command("hello", "/hello: says hello");
//!         Ok(Hello)
//!     }
//!     fn on_event(&mut self, ctx: &mut Ctx, event: &mut Event) {
//!         match event.kind() {
//!             EventKind::Command => ctx.tell(event.player(), "Hello from Ember!"),
//!             EventKind::BlockBreak if event.block().contains("diamond") => event.cancel(),
//!             _ => {}
//!         }
//!     }
//! }
//! ember_api::ember_mod!(Hello);
//! ```
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// The ABI's version: the game loads mods built for its own or an older one.
/// 2 added [`Ctx::sit`] and [`Ctx::free_seat`].
pub const ABI_VERSION: u32 = 2;
/// The symbol the game calls to start a mod.
pub const ENTRY_SYMBOL: &[u8] = b"ember_mod_entry\0";
/// The `u32` static saying which ABI a mod was built for, read first.
pub const ABI_SYMBOL: &[u8] = b"EMBER_ABI\0";

/// What crosses the library boundary. Mod authors use the safe wrappers.
pub mod abi {
    use std::ffi::c_void;

    /// A borrowed UTF-8 string: valid for the call it is passed to.
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct Str {
        pub ptr: *const u8,
        pub len: usize,
    }
    impl Str {
        pub const EMPTY: Str = Str {
            ptr: std::ptr::null(),
            len: 0,
        };
        pub fn new(s: &str) -> Str {
            Str {
                ptr: s.as_ptr(),
                len: s.len(),
            }
        }
        /// # Safety
        /// `ptr` must point at `len` readable bytes for `'a`.
        pub unsafe fn as_str<'a>(self) -> &'a str {
            if self.ptr.is_null() || self.len == 0 {
                return "";
            }
            let bytes = unsafe { std::slice::from_raw_parts(self.ptr, self.len) };
            std::str::from_utf8(bytes).unwrap_or("")
        }
    }

    /// An event as the bus passes it to each listener in turn.
    #[repr(C)]
    #[derive(Debug)]
    pub struct RawEvent {
        pub kind: u32,
        pub cancelable: bool,
        pub cancelled: bool,
        pub pos: [i32; 3],
        pub tick: u64,
        /// The player who acted (empty for none).
        pub player: Str,
        /// The block's state (`minecraft:oak_log[axis=y]`).
        pub block: Str,
        /// The held item's id.
        pub item: Str,
        /// A chat message, a command's name, a custom event's channel.
        pub text: Str,
        /// A command's arguments, a custom event's payload.
        pub args: Str,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct RawPlayer {
        pub present: bool,
        pub pos: [f64; 3],
        pub yaw: f32,
        pub pitch: f32,
        pub health: f32,
        pub food: i32,
        pub creative: bool,
        pub sneaking: bool,
    }

    /// The game's side: `ctx` is passed back to every function.
    #[repr(C)]
    pub struct Host {
        pub abi: u32,
        pub ctx: *mut c_void,
        pub log: unsafe extern "C" fn(*mut c_void, u32, Str),
        /// During `init` only: kind, priority, receive cancelled events.
        pub subscribe: unsafe extern "C" fn(*mut c_void, u32, i32, bool),
        /// During `init` only: name, usage.
        pub register_command: unsafe extern "C" fn(*mut c_void, Str, Str),
        /// Player (empty: everyone), text.
        pub tell: unsafe extern "C" fn(*mut c_void, Str, Str),
        pub run_command: unsafe extern "C" fn(*mut c_void, Str),
        pub set_block: unsafe extern "C" fn(*mut c_void, i32, i32, i32, Str),
        /// The state at a position, valid until the next call to the host.
        pub block: unsafe extern "C" fn(*mut c_void, i32, i32, i32) -> Str,
        pub player: unsafe extern "C" fn(*mut c_void, Str) -> RawPlayer,
        /// Channel, payload: a [`crate::EventKind::Custom`] event for every mod.
        pub post: unsafe extern "C" fn(*mut c_void, Str, Str),
        /// The current dimension's id, valid until the next call to the host.
        pub dimension: unsafe extern "C" fn(*mut c_void) -> Str,
        // ABI 2.
        /// Player, seat position, yaw: seats the player there after the event.
        pub sit: unsafe extern "C" fn(*mut c_void, Str, f64, f64, f64, f32),
        /// Removes the seats of the block at a position (its riders stand up).
        pub free_seat: unsafe extern "C" fn(*mut c_void, i32, i32, i32),
    }

    /// What a mod's entry returns.
    #[repr(C)]
    pub struct RawMod {
        pub abi: u32,
        pub id: Str,
        pub name: Str,
        pub version: Str,
        /// Null when `init` failed; `error` says why.
        pub instance: *mut c_void,
        pub error: Str,
        /// 0: handled, 1: the mod panicked.
        pub on_event: unsafe extern "C" fn(*mut c_void, *const Host, *mut RawEvent) -> u32,
        pub drop: unsafe extern "C" fn(*mut c_void),
    }

    pub type Entry = unsafe extern "C" fn(*const Host) -> RawMod;
}
use abi::{Host, RawEvent, RawMod, Str};

/// The events of the bus.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EventKind {
    /// Each game tick of a world this game simulates (20 a second).
    Tick = 1,
    /// A world was opened.
    WorldLoad = 2,
    /// The world is closing.
    WorldUnload = 3,
    /// A player is breaking a block: cancel to keep it.
    BlockBreak = 4,
    /// A player placed a block: cancel to undo it.
    BlockPlace = 5,
    /// A player uses (right-clicks) the held item: cancel to stop it.
    UseItem = 6,
    /// A player sends a chat message: cancel to drop it.
    Chat = 7,
    /// A player ran one of this mod's commands (only its owner hears it).
    Command = 8,
    /// Another mod's [`Ctx::post`].
    Custom = 9,
}
impl EventKind {
    pub const ALL: [EventKind; 9] = [
        EventKind::Tick,
        EventKind::WorldLoad,
        EventKind::WorldUnload,
        EventKind::BlockBreak,
        EventKind::BlockPlace,
        EventKind::UseItem,
        EventKind::Chat,
        EventKind::Command,
        EventKind::Custom,
    ];
    pub fn from_u32(v: u32) -> Option<EventKind> {
        Self::ALL.into_iter().find(|k| *k as u32 == v)
    }
    /// Whether a listener may cancel it.
    pub fn cancelable(self) -> bool {
        matches!(
            self,
            EventKind::BlockBreak | EventKind::BlockPlace | EventKind::UseItem | EventKind::Chat
        )
    }
}

/// When a listener hears an event, as Forge's `EventPriority`: `Highest`
/// first, `Monitor` last (it should only watch the outcome).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Highest = 0,
    High = 1,
    Normal = 2,
    Low = 3,
    Lowest = 4,
    Monitor = 5,
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
}

/// A player's state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Player {
    pub pos: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub health: f32,
    pub food: i32,
    pub creative: bool,
    pub sneaking: bool,
}

/// What a mod asks for while it starts.
pub struct Registrar<'a> {
    host: &'a Host,
}
impl Registrar<'_> {
    /// Listen to `kind`, skipping events an earlier listener cancelled.
    pub fn on(&mut self, kind: EventKind, priority: Priority) -> &mut Self {
        unsafe { (self.host.subscribe)(self.host.ctx, kind as u32, priority as i32, false) };
        self
    }
    /// Listen to `kind`, cancelled events included (Forge's `receiveCanceled`).
    pub fn on_cancelled(&mut self, kind: EventKind, priority: Priority) -> &mut Self {
        unsafe { (self.host.subscribe)(self.host.ctx, kind as u32, priority as i32, true) };
        self
    }
    /// Own `/name`: typed by any player, it reaches only this mod as
    /// [`EventKind::Command`]. Vanilla's and earlier mods' names are refused.
    pub fn command(&mut self, name: &str, usage: &str) -> &mut Self {
        unsafe { (self.host.register_command)(self.host.ctx, Str::new(name), Str::new(usage)) };
        self
    }
    pub fn log(&mut self, level: Level, message: &str) {
        unsafe { (self.host.log)(self.host.ctx, level as u32, Str::new(message)) };
    }
}

/// The game, during one event.
pub struct Ctx<'a> {
    host: &'a Host,
}
impl Ctx<'_> {
    pub fn log(&mut self, level: Level, message: &str) {
        unsafe { (self.host.log)(self.host.ctx, level as u32, Str::new(message)) };
    }
    pub fn info(&mut self, message: &str) {
        self.log(Level::Info, message);
    }
    /// A chat line for one player, by name.
    pub fn tell(&mut self, player: &str, text: &str) {
        if !player.is_empty() {
            unsafe { (self.host.tell)(self.host.ctx, Str::new(player), Str::new(text)) };
        }
    }
    /// A chat line for every player.
    pub fn broadcast(&mut self, text: &str) {
        unsafe { (self.host.tell)(self.host.ctx, Str::EMPTY, Str::new(text)) };
    }
    /// A vanilla command (no slash), run with operator rights after the
    /// event, as a server's console runs it.
    pub fn run_command(&mut self, line: &str) {
        unsafe { (self.host.run_command)(self.host.ctx, Str::new(line)) };
    }
    /// Seats `player` at `pos` (the rider's seat, in blocks) facing `yaw`,
    /// as on a chair: no cheats needed, Shift stands up and the seat goes.
    pub fn sit(&mut self, player: &str, pos: [f64; 3], yaw: f32) {
        let [x, y, z] = pos;
        unsafe { (self.host.sit)(self.host.ctx, Str::new(player), x, y, z, yaw) };
    }
    /// Removes the seats inside the block at `pos`: whoever sits there stands up.
    pub fn free_seat(&mut self, pos: [i32; 3]) {
        unsafe { (self.host.free_seat)(self.host.ctx, pos[0], pos[1], pos[2]) };
    }
    /// Sets a block after the event, as `/setblock` does.
    pub fn set_block(&mut self, pos: [i32; 3], state: &str) {
        unsafe { (self.host.set_block)(self.host.ctx, pos[0], pos[1], pos[2], Str::new(state)) };
    }
    /// The block's state, or `None` where no chunk is loaded.
    pub fn block(&mut self, pos: [i32; 3]) -> Option<String> {
        let s = unsafe { (self.host.block)(self.host.ctx, pos[0], pos[1], pos[2]).as_str() };
        (!s.is_empty()).then(|| s.to_owned())
    }
    pub fn player(&mut self, name: &str) -> Option<Player> {
        let p = unsafe { (self.host.player)(self.host.ctx, Str::new(name)) };
        p.present.then_some(Player {
            pos: p.pos,
            yaw: p.yaw,
            pitch: p.pitch,
            health: p.health,
            food: p.food,
            creative: p.creative,
            sneaking: p.sneaking,
        })
    }
    /// Posts a custom event to every mod listening to [`EventKind::Custom`]
    /// (this one included), after the current event.
    pub fn post(&mut self, channel: &str, payload: &str) {
        unsafe { (self.host.post)(self.host.ctx, Str::new(channel), Str::new(payload)) };
    }
    pub fn dimension(&mut self) -> String {
        unsafe { (self.host.dimension)(self.host.ctx).as_str().to_owned() }
    }
}

/// One event, as a listener sees it.
pub struct Event<'a> {
    raw: &'a mut RawEvent,
}
impl Event<'_> {
    pub fn kind(&self) -> Option<EventKind> {
        EventKind::from_u32(self.raw.kind)
    }
    pub fn pos(&self) -> [i32; 3] {
        self.raw.pos
    }
    pub fn tick(&self) -> u64 {
        self.raw.tick
    }
    pub fn player(&self) -> &str {
        unsafe { self.raw.player.as_str() }
    }
    pub fn block(&self) -> &str {
        unsafe { self.raw.block.as_str() }
    }
    pub fn item(&self) -> &str {
        unsafe { self.raw.item.as_str() }
    }
    /// A chat message.
    pub fn message(&self) -> &str {
        unsafe { self.raw.text.as_str() }
    }
    /// A command's name.
    pub fn command(&self) -> &str {
        unsafe { self.raw.text.as_str() }
    }
    /// A command's arguments, as typed.
    pub fn args(&self) -> &str {
        unsafe { self.raw.args.as_str() }
    }
    /// A custom event's channel.
    pub fn channel(&self) -> &str {
        unsafe { self.raw.text.as_str() }
    }
    /// A custom event's payload.
    pub fn payload(&self) -> &str {
        unsafe { self.raw.args.as_str() }
    }
    pub fn is_cancelable(&self) -> bool {
        self.raw.cancelable
    }
    pub fn is_cancelled(&self) -> bool {
        self.raw.cancelled
    }
    /// Cancels a cancelable event; later listeners may undo it.
    pub fn set_cancelled(&mut self, cancelled: bool) {
        if self.raw.cancelable {
            self.raw.cancelled = cancelled;
        }
    }
    pub fn cancel(&mut self) {
        self.set_cancelled(true);
    }
}

/// An Ember mod.
pub trait EmberMod: Sized + 'static {
    /// Once, when the game loads the mod: subscribe and register commands.
    fn init(reg: &mut Registrar) -> Result<Self, String>;
    /// Each event the mod subscribed to, and its own commands.
    fn on_event(&mut self, ctx: &mut Ctx, event: &mut Event);
}

/// Makes a mod's entry points. `id` defaults to the crate's name.
#[macro_export]
macro_rules! ember_mod {
    ($ty:ty) => {
        $crate::ember_mod!(
            $ty,
            id = env!("CARGO_PKG_NAME"),
            name = env!("CARGO_PKG_NAME")
        );
    };
    ($ty:ty, id = $id:expr, name = $name:expr) => {
        #[unsafe(no_mangle)]
        pub static EMBER_ABI: u32 = $crate::ABI_VERSION;
        /// # Safety
        /// Called by the game's loader with its host.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn ember_mod_entry(
            host: *const $crate::abi::Host,
        ) -> $crate::abi::RawMod {
            unsafe { $crate::__entry::<$ty>(host, $id, $name, env!("CARGO_PKG_VERSION")) }
        }
    };
}

#[doc(hidden)]
/// # Safety
/// `host` must be the loader's host, alive for the call.
pub unsafe fn __entry<M: EmberMod>(
    host: *const Host,
    id: &'static str,
    name: &'static str,
    version: &'static str,
) -> RawMod {
    let mut out = RawMod {
        abi: ABI_VERSION,
        id: Str::new(id),
        name: Str::new(name),
        version: Str::new(version),
        instance: std::ptr::null_mut(),
        error: Str::EMPTY,
        on_event: on_event::<M>,
        drop: drop_mod::<M>,
    };
    let host = unsafe { &*host };
    if host.abi < ABI_VERSION {
        out.error = Str::new("needs a newer game (a later Ember ABI)");
        return out;
    }
    let started = catch_unwind(AssertUnwindSafe(|| M::init(&mut Registrar { host })));
    let error = match started {
        Ok(Ok(m)) => {
            out.instance = Box::into_raw(Box::new(m)).cast::<c_void>();
            return out;
        }
        Ok(Err(e)) => e,
        Err(_) => "init panicked".to_owned(),
    };
    // Once per failed mod: leaked so the loader can read it.
    out.error = Str::new(Box::leak(error.into_boxed_str()));
    out
}

unsafe extern "C" fn on_event<M: EmberMod>(
    instance: *mut c_void,
    host: *const Host,
    event: *mut RawEvent,
) -> u32 {
    let run = catch_unwind(AssertUnwindSafe(|| {
        let me = unsafe { &mut *instance.cast::<M>() };
        let mut ctx = Ctx {
            host: unsafe { &*host },
        };
        let mut event = Event {
            raw: unsafe { &mut *event },
        };
        me.on_event(&mut ctx, &mut event);
    }));
    u32::from(run.is_err())
}

unsafe extern "C" fn drop_mod<M: EmberMod>(instance: *mut c_void) {
    if !instance.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            drop(unsafe { Box::from_raw(instance.cast::<M>()) })
        }));
    }
}
