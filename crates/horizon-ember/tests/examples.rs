//! Loads the example mods as the game does and drives their bus.
use horizon_ember::{Action, Ember, EventData, Player, World};
use std::path::PathBuf;
use std::process::Command;

struct Fixture {
    sneaking: bool,
    creative: bool,
}
impl World for Fixture {
    fn block(&self, [x, y, z]: [i32; 3]) -> Option<String> {
        // A 3-block coal vein along x at y=10, z=100.
        let vein = y == 10 && z == 100 && (100..=102).contains(&x);
        Some(
            if vein {
                "minecraft:coal_ore"
            } else {
                "minecraft:stone"
            }
            .into(),
        )
    }
    fn player(&self, name: &str) -> Option<Player> {
        (name == "Steve").then_some(Player {
            pos: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            health: 20.0,
            food: 20,
            creative: self.creative,
            sneaking: self.sneaking,
        })
    }
    fn dimension(&self) -> String {
        "minecraft:overworld".into()
    }
}

/// Builds the example mods and returns their libraries.
fn examples() -> Vec<PathBuf> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ember-examples");
    let status = Command::new(cargo)
        .args([
            "build",
            "-q",
            "-p",
            "ember-protect-spawn",
            "-p",
            "ember-vein-miner",
        ])
        .arg("--target-dir")
        .arg(&target)
        .status()
        .expect("cargo");
    assert!(status.success());
    let dir = target.join("debug");
    ["ember_protect_spawn", "ember_vein_miner"]
        .iter()
        .map(|n| {
            dir.join(format!(
                "{}{n}{}",
                std::env::consts::DLL_PREFIX,
                std::env::consts::DLL_SUFFIX
            ))
        })
        .collect()
}

#[test]
fn bus_priorities_cancel_and_commands() {
    let paths = examples();
    let mut ember = Ember::load(&paths, |name| name == "give");
    let ids: Vec<_> = ember
        .mods()
        .map(|m| (m.id.clone(), m.problem.clone()))
        .collect();
    assert_eq!(
        ids,
        [("protect-spawn".into(), None), ("vein-miner".into(), None)]
    );
    let names: Vec<_> = ember.commands().map(|c| c.name).collect();
    assert_eq!(names, ["spawnprotect", "veinminer"]);

    // Inside spawn, survival: Protect Spawn (High) cancels, Vein Miner
    // (Monitor, not receiving cancelled) never runs.
    let world = Fixture {
        sneaking: true,
        creative: false,
    };
    let near = EventData::block_break("Steve", [3, 10, 3], "minecraft:coal_ore", "");
    assert!(ember.fire(&world, near.clone()).cancelled);
    assert_eq!(
        ember.drain(),
        [Action::Tell {
            from: "protect-spawn".into(),
            player: Some("Steve".into()),
            text: "Spawn is protected.".into()
        }]
    );

    // Creative passes.
    let creative = Fixture {
        sneaking: false,
        creative: true,
    };
    assert!(!ember.fire(&creative, near).cancelled);
    assert!(ember.drain().is_empty());

    // Outside spawn, sneaking: the vein's other two blocks are destroyed.
    let far = EventData::block_break("Steve", [100, 10, 100], "minecraft:coal_ore", "");
    assert!(!ember.fire(&world, far).cancelled);
    let lines: Vec<_> = ember
        .drain()
        .into_iter()
        .filter_map(|a| match a {
            Action::Command { line, .. } => Some(line),
            _ => None,
        })
        .collect();
    assert_eq!(
        lines,
        [
            "setblock 101 10 100 minecraft:air destroy",
            "setblock 102 10 100 minecraft:air destroy"
        ]
    );

    // Commands reach their owner only; unknown ones aren't taken.
    assert!(ember.command(&world, "Steve", "spawnprotect off", 0));
    assert!(!ember.command(&world, "Steve", "give @s stone", 0));
    let near = EventData::block_break("Steve", [3, 10, 3], "minecraft:stone", "");
    ember.drain();
    assert!(!ember.fire(&world, near).cancelled);
}

#[test]
fn missing_and_foreign_libraries_are_reported() {
    let ember = Ember::load(&[PathBuf::from("/nonexistent/mod.so")], |_| false);
    assert!(ember.is_empty());
    let m = ember.mods().next().unwrap();
    assert!(m.problem.as_deref().unwrap().starts_with("can't load"));
}

/// The ABI probe built for `abi` (its own target dir: the ABI is baked in).
fn probe(abi: u32) -> PathBuf {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let target =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../target/ember-probe-{abi}"));
    let status = Command::new(cargo)
        .args(["build", "-q", "-p", "ember-abi-probe"])
        .arg("--target-dir")
        .arg(&target)
        .env("EMBER_PROBE_ABI", abi.to_string())
        .status()
        .expect("cargo");
    assert!(status.success());
    target.join("debug").join(format!(
        "{}ember_abi_probe{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ))
}

#[test]
fn older_abi_loads_and_newer_is_refused() {
    let old = Ember::load(&[probe(1)], |_| false);
    let m = old.mods().next().unwrap();
    assert_eq!(m.problem, None, "an ABI 1 mod loads on this game");
    assert_eq!(m.id, "abi-probe");
    assert!(!old.is_empty());

    let new = Ember::load(&[probe(99)], |_| false);
    assert!(new.is_empty());
    let m = new.mods().next().unwrap();
    let why = m.problem.as_deref().unwrap();
    assert!(
        why.contains("built for Ember ABI 99") && why.contains("update the game"),
        "{why}"
    );
}
