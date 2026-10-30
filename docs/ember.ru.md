# Ember — загрузчик нативных модов для Horizon (бета 0.1.0)

[English](ember.md) · [Полная документация](ember.ru.md) · [README Horizon](../README.horizon.md)

Ember — загрузчик модов в духе Forge для [Horizon](https://github.com/horizon-rust/horizon), Minecraft Java Edition 26.3, переписанного на Rust. Мод — это Rust-крейт, собранный против маленького крейта `ember-api` в динамическую библиотеку (`.so` / `.dll` / `.dylib`). Кладёшь её в папку `mods/` игры, и мод грузится при старте. Игру пересобирать не нужно, исходники Horizon не патчатся, моды разных авторов работают вместе на одной шине событий.

Зачем: в Horizon вырезали кодовые WASM-моды. Для кода остались только `.hnmod`, а это патчи исходников, привязанные к одному коммиту игры. Мод под Ember собирается один раз под стабильный C ABI и продолжает работать на новых сборках игры.

> **Внимание:** моды Ember — доверенный нативный код. Они работают в процессе игры с её правами, песочницы нет. Ставь только моды, которым доверяешь.

## Возможности

- **Шина событий:** `Tick`, `WorldLoad`/`WorldUnload`, `BlockBreak`, `BlockPlace`, `UseItem`, `Chat`, `Command`, `Custom`. Приоритеты идут от `Highest` до `Lowest`, потом `Monitor`. События можно отменять.
- **Доступ к миру:** читать блоки, игроков и измерение; ставить блоки, выполнять команды от имени сервера, писать сообщения.
- **Свои `/команды`.** Имена ванильных команд заняты, их не перехватить.
- **Контент:** блоки и предметы — это данные в папке рядом с библиотекой (модели, текстуры, лут, рецепты, переводы).
- **Креатив:** предметы модов появляются во вкладке «Функциональные блоки» и в поиске.
- **Стабильный ABI** (`EMBER_ABI` = 2). Таблица функций хоста только дописывается в конец, поэтому мод под ABI N грузится в любой игре с ABI ≥ N. Мод под более новый ABI игра не загрузит и понятно напишет, почему. Паника в моде останавливает только этот мод.
- **Сиденья (бета):** `ctx.sit` / `ctx.free_seat`, см. «Известные проблемы».

## Установка (игрокам)

1. Собери Horizon из этого репозитория: `cargo build --release -p horizon-client`.
2. Положи библиотеку мода и его папку с контентом, если она есть, в `mods/` рядом с `options.txt`:
```
mods/
├── libember_furniture.so
└── furniture/            # assets/ + data/
```
3. Запусти игру. В логе будет `Ember: loaded <id> <версия>` для каждого мода или причина, почему мод пропущен.

## Как написать мод

```toml
[package]
name = "my-mod"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
ember-api = { git = "<URL этого репозитория>" }
```

```rust
use ember_api::{Ctx, EmberMod, Event, EventKind, Priority, Registrar};

struct Hello;

impl EmberMod for Hello {
    fn init(reg: &mut Registrar) -> Result<Self, String> {
        reg.on(EventKind::BlockBreak, Priority::Normal)
            .command("hello", "/hello: поздороваться");
        Ok(Hello)
    }
    fn on_event(&mut self, ctx: &mut Ctx, event: &mut Event) {
        match event.kind() {
            Some(EventKind::BlockBreak) if event.block().starts_with("minecraft:diamond") => {
                event.cancel(); // блок останется
            }
            Some(EventKind::Command) => {
                let p = event.player().to_owned();
                ctx.tell(&p, "Привет от Ember!");
            }
            _ => {}
        }
    }
}

ember_api::ember_mod!(Hello, id = "hello", name = "Hello");
```

Собери через `cargo build --release` и скопируй `target/release/libmy_mod.so` в `mods/`.

## Примеры (`mods/ember/`)

| Мод | Что показывает |
|---|---|
| `protect-spawn` | отмена ломания и постановки, команда `/spawnprotect` |
| `vein-miner` | приоритет `Monitor`, ломание блоков из кода |
| `lamp` | свой блок с текстурой, лутом и рецептом (собран под ABI 1, грузится на ABI 2) |
| `furniture` | 12 блоков мебели, посадка через `ctx.sit` |
| `abi-probe` | тестовый мод для проверки совместимости ABI |

## Структура

- `crates/ember-api`: единственный крейт, от которого зависят моды (C ABI, `EmberMod`, `Ctx`, `Event`, `ember_mod!`).
- `crates/horizon-ember`: сам загрузчик (поиск модов, проверка ABI, шина событий, действия). Тесты: `cargo test -p horizon-ember`.
- `crates/horizon-client/src/game/ember.rs`: хуки в игре.
- `docs/ember.md` / `docs/ember.ru.md`: полная документация.

## Известные проблемы (бета)

- **Сиденья:** игрок сидит на невидимой беззвучной летучей мыши. Сердечки «маунта» скрыты, но посадка пока работает нестабильно. Нормальная поза сидя без вспомогательной сущности запланирована.
- Моды грузятся только в той игре, которая симулирует мир: одиночная игра, LAN-хост, выделенный сервер. Клиент на чужом сервере моды не запускает.
- API для HUD, интерфейса и рендера пока нет.
- Мод нужно собирать отдельно под каждую ОС.

Лицензия: та же, что у Horizon.
