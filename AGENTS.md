# AGENTS.md

Context file for AI coding agents. Read this before touching the repo.

---

## 1. What this project is

**CTMLauncher** — a modular **terminal (TUI) Minecraft launcher**, inspired by Prism Launcher.

Idea:

- Bring the full Prism-like workflow (instances, modloaders, Modrinth, Microsoft auth, logs) into a pure terminal UI.
- Domain logic lives in a headless engine crate; the UI is a thin, mouse-friendly ratatui shell on top.
- Visual identity: modern, minimal, **borderless** dark cards — no box-drawing chrome, one vibrant green accent.

The product is still **0.1.0 / pre-release**. Features are being iterated with the owner (Russian-speaking) in tight UI loops; design feedback is highly visual (screenshots) and very specific about pixel/column-level layout.

**Audience / product notes:**

- Desktop Linux primary target (paths via XDG `directories`), but code should stay portable.
- Users are technical Minecraft players (Java memory, GC, loaders, logs).
- Owner prefers Russian for conversation; UI itself is localized En/Uk/Ru.

---

## 2. Workspace layout

```
CTMLauncher/                  Cargo workspace (resolver = "2")
├── Cargo.toml                workspace + shared deps
├── Cargo.lock                committed (ships a binary)
├── README.md                 stub
├── AGENTS.md                 this file
└── crates/
    ├── mc_core/              pure domain engine — NO UI, NO ratatui
    │   └── src/
    │       ├── lib.rs
    │       ├── auth/         Microsoft device-code, offline, account store
    │       ├── instance/     instance.json, managers, loaders, groups
    │       ├── install/      vanilla, fabric, quilt, forge, neoforge
    │       ├── launch/       args, assets, java discovery, process, JVM
    │       ├── modrinth/     Modrinth HTTP client, search, versions
    │       ├── modpack/      .mrpack import
    │       ├── import/       detect external launches (Prism, MultiMC…)
    │       ├── logs/         log buffer, crash analysis
    │       ├── skins/        skin upload/preview
    │       ├── img.rs        PNG/WebP helpers
    │       ├── util.rs       Paths, downloads, hashing, JSON IO, Progress
    │       ├── version.rs    version manifest / parsing
    │       └── error.rs      CoreError / Result
    └── mc_tui/               ratatui frontend → binary `ctmlauncher`
        └── src/
            ├── main.rs       terminal setup, panic hook, image picker
            ├── app/          App state, render, input, actions, events
            ├── views/        one module per screen + shared settings chrome
            ├── widgets/      button, popup, progress primitives
            ├── forms.rs      modal forms, overlays, pickers
            ├── wizard.rs     "New Build" create wizard
            ├── engine.rs     EngineEvent channel (async → UI)
            ├── i18n.rs       En / Uk / Ru tables
            ├── theme.rs      Theme palette + style helpers
            ├── settings.rs   LauncherSettings (settings.json)
            ├── md.rs         tiny markdown renderer (logs/help?)
            └── images.rs     protocol image loading (kitty/sixel/…)
```

**Rule:** anything that needs to know about Minecraft/auth/fs goes in `mc_core`. Anything that draws or handles terminal events stays in `mc_tui`. `mc_core` must never depend on ratatui/crossterm.

Rough size: ~27k LOC, ~65 `.rs` files.

---

## 3. Stack & commands

| Item | Value |
|------|--------|
| Edition | Rust 2021, `rust-version = 1.75` |
| Async | tokio (full) |
| HTTP | reqwest + rustls (no native-tls) |
| TUI | ratatui 0.29, crossterm 0.28 (event-stream, mouse) |
| Images | ratatui-image 9, image 0.25 (webp) |
| Store | serde/serde_json; accounts via JSON; no SQL in use |
| License | MIT |

Commands (run from workspace root):

```bash
cargo check -q -p mc_tui                 # compile UI
cargo check -q -p mc_core                # compile engine
cargo test  -q -p mc_core -p mc_tui      # unit + TestBackend render tests
cargo clippy -q -p mc_tui                # lint (pre-existing warns exist)
cargo run   -p mc_tui                    # launch TUI (needs a real terminal)
cargo build -q --release -p mc_tui       # release binary: target/release/ctmlauncher
```

**Current test baseline (as of last session):** `mc_core` 74 + `mc_tui` 43 = **117 passing**. Keep them green.

**Clippy:** `-D warnings` fails on **pre-existing** issues in `mc_core` (`img.rs` manual_div_ceil / redundant_slicing, `modrinth/client.rs` too_many_arguments) and a few `mc_tui` ones (`actions.rs` manual_ok_err / needless_borrows). Do not "fix the world" unless asked; just ensure **your** files are clean:

```bash
cargo clippy -q -p mc_tui 2>&1 | rg "views/settings|views/instance_settings|settings_ui" 
```

---

## 4. Design language (non-negotiable)

### 4.1 Visual system

- **Borderless flat cards.** Surfaces are filled rects (`theme.panel` #181818) on a darker page (`theme.bg` #0D0D0D). No `│─┌┐` boxes.
- **One accent:** green `#50FA7B` / bright `#00FF87`. Used for selection, keybinding chips, focused `▎` bar.
- **Selection bg:** soft dark green `#142B1E` (`selection_bg`). Hover: `#1E1E1E` (`hover_bg`).
- **Error:** `#FF5555`. Warning: `#F1FA8C`.
- Sidebar is a right-hand card (~30 cols) with chunky 3-row nav buttons; content is left.
- Header (1 row) + body (min) + status footer (1 row) + optional progress (1 row).
- Images: try kitty/sixel/iTerm2 via ratatui-image; fall back to half-blocks.

Style helpers live on `Theme` (`theme.rs`): `card()`, `card_dim()`, `accent()`, `row_selected()`, `row_hover()`, etc. Prefer these over ad-hoc `Style::default()`.

### 4.2 Interaction model

- **Full mouse support** alongside keyboard. Every clickable region pushes a `Hitbox { rect, action: HitAction }` during render; clicks dispatch through `app/input.rs` → `dispatch_hit`.
- Hitbox **order matters**: more specific rects (slider track) must be pushed **before** generic row fallbacks.
- Global keys: `q` quit (with confirm if `confirm_quit`), `Tab`/`BackTab` focus toggle, `1..9` jump nav, `?` help, `F2` accounts, `F3` launcher settings, `F4` modpacks.
- While typing in an inline field (`App::is_typing()`), single-letter shortcuts are suppressed.

### 4.3 Settings pages (active design work)

Two screens share chrome in `views/settings_ui.rs`:

| Nav | View file | Fields |
|-----|-----------|--------|
| `Nav::Launcher` | `views/settings.rs` | 9 fields, 2 sections (Java, Interface) |
| `Nav::Jvm` (instance) | `views/instance_settings.rs` | 7 fields, 3 sections (Java, Display, Arguments) |

**Layout contract (owner-enforced):**

- Outer gutters **exactly `SIDE_PAD = 2` chars on both sides** → `panel_w = body.width - SIDE_PAD * 2`.
- Section panels are **solid filled blocks** (`fill_rect` with `theme.panel` before header/rows) so no black page-bg holes appear in seams.
- Between sections: `PANEL_GAP = 1` row of page bg.
- Section headers: **plain title only** — no `▸`/`▼`, no active/hover highlight (looks like a broken accordion otherwise).
- Rows stack tightly (no blank spacer rows). Content inside panel uses `inner = panel ± 1` horizontal pad.
- Inline editing only — **no modal dialogs for settings**. Enter edits, Esc cancels, Enter commits.
- Dropdowns open under the field as extra rows inside the same panel (height accounted in `field_h`).
- Selected slider gains **+1 row** for scale labels under the track.

**RAM sliders** (`settings_ui.rs`):

- Grid: `RAM_MIN=512`, `RAM_STEP=1024`, `RAM_MAX=32768`.
- Effective max = `system_ram_mb().clamp(MIN+STEP, MAX)` — never offer more than physical RAM (Linux `/proc/meminfo`, fallback 8192 MB).
- Min/max ranges stay consistent (`min_ram_range` / `max_ram_range`) so min < max always.
- Shared geometry: `value_to_pos(value, min, max, w)` used by handle, ticks **and** scale labels so `●` always sits on `|` for grid values. Old `tick_positions` removed.
- Scale labels are **whole GB** (`512`, `1024`, … `15360`) via `scale_values` / `format_scale_mb` — no decimals.
- Row columns: `[LABEL_W=22][LABEL_GAP=2][track][LABEL_GAP=2][VALUE_W=10][INPUT_W=8]`; helpers `slider_track_rect` / `slider_value_rect` / `slider_input_rect`.
- `label_cell(label)` returns **exactly** `LABEL_W+LABEL_GAP` chars (truncate with `…` **inside**). Callers must **not** truncate again (double-truncate bug happened once).
- `draw_slider` / `draw_slider_scale` take an explicit `bg: Color` so the track never punches a black hole through a selected/hovered row.
- **Warning threshold: ~60% of system RAM** (not 50% — half is the common working point and must stay green). `ram_warn_threshold()` = `system_ram * 60/100` snapped down to grid; `is_ram_over_half(mb)` compares against it. When exceeded: filled/handle/value/input turn `error` red **and** a toast fires (`toast.ram_over_half`, key `replacen("{}"…)` ×3 → selected, total, threshold).
- Toast wiring: `warn_ram_over_half` in `app/actions.rs`, called from `settings_set_slider`, `settings_adjust`, `instance_settings_adjust`, `commit_launcher_setting`, `commit_instance_setting`.
- Backgrounds: selected row → `selection_bg`, hover → `hover_bg`, idle → `panel`. Paint a full-row `fill_rect` **first**, then widgets on top (sliders paint cell-by-cell).

**Do not** reintroduce: modal settings dialogs, accordion collapse on section headers, asymmetric right margin, scale labels with decimals, warning at 50%.

---

## 5. Architecture patterns

### 5.1 App state machine

- Single `App` struct (`app/mod.rs`) holds nav, focus, lists, overlays, hitboxes, settings, toasts, progress.
- Submodules:
  - `render.rs` — frame composition: header / body match on `Nav` / footer / overlay.
  - `input.rs` — keyboard + mouse → higher-level actions; overlay keys first.
  - `actions.rs` — side-effecting operations (save, install, launch, auth, settings commit). Async work spawns via engine channel.
  - `events.rs` — apply `EngineEvent`s back onto `App`.
- `Nav` enum drives both sidebar menu and which `render_*` runs. `Nav::is_build_scoped()` gates instance-dependent pages (empty state when no instance selected).

### 5.2 Async engine channel

- Background tasks emit `EngineEvent` (`engine.rs`) over tokio mpsc.
- UI loop drains → `App::apply_event` (or equivalent in `events.rs`).
- Event kinds: Status, Progress/ProgressDone, Message overlay, Started(process), Accounts*, SearchResults, Browse*, Wizard*, InstalledMods, Crash, DeviceCode, Authenticated, JavaRuntimes, InstancesChanged, …

Never block the draw loop on network/FS.

### 5.3 Overlays & forms

- `forms.rs`: `Overlay` enum (message, confirm, text prompt, form, picker, device-code…), `Form`/`FormField`/`FieldKind`, `FormAction`.
- `wizard.rs`: multi-tab create wizard (Clean / Import mrpack / Modrinth / External), fully mouse-driven.
- Settings deliberately **bypass** the Form modal path for day-to-day editing.

### 5.4 Persistence

- Launcher settings: `settings.json` via `LauncherSettings::{load,save}` under XDG config (`Paths::discover` / `rooted_at` for tests).
- Instances: `instance.json` per instance dir (`InstanceManager`).
- Accounts: `AccountStore` JSON (never log tokens).
- `Paths::{config_dir,data_dir,cache_dir}` — always go through `Paths`, hardcoding `~/.minecraft` is wrong.

### 5.5 Auth

- Microsoft **device code** flow implemented stepwise in `auth/microsoft.rs` (OAuth → Xbox → XSTS → Minecraft).
- `DEFAULT_CLIENT_ID` is Prism Launcher's public client id (`c36a9fb6-…`). Override via `MicrosoftAuth::with_client_id`. The owner has **not** confirmed a custom Azure app — do not invent a new client id.
- Offline auth in `auth/offline.rs`.

---

## 6. i18n

- File: `crates/mc_tui/src/i18n.rs` (~1565 lines), three flat match tables: `en`, `uk`, `ru`.
- `Lang::{En,Uk,Ru}` stored in `settings.language` (serde snake_case).
- Keys: dotted stable ids (`nav.instances`, `btn.save`, `toast.ram_over_half`, `settings.section.java`).
- Lookup: `tr(lang,key) → Cow`; missing Uk/Ru falls back to En; unknown key → key itself.
- App helpers: `app.tr(key)`, `app.trs(key)`, `app.lang()`.
- **When adding a user-visible string:** add the key to **all three** tables (En required; Uk/Ru required for parity). Placeholders use `{}` positional, filled with `replacen`.
- Endonyms in the language picker are never translated (`native_label`).

---

## 7. Code conventions

- **No comments unless asked** in newly written code; existing doc-comments on public items are fine to keep/extend.
- Prefer editing existing files over creating new modules unless the feature is a new screen.
- Follow neighboring style: `pub(crate)` for view helpers, `impl App` blocks inside view files for render/key handlers.
- Renderer functions take `(&mut self, frame: &mut Frame, area: Rect)` and register hitboxes on `self.hitboxes`.
- Clear `hitboxes` / ephemeral collections (`settings_sliders`) at the start of each relevant render.
- Integer geometry in cells (u16). Use `saturating_sub` for rects — never underflow.
- Truncate UI strings with `crate::views::truncate` / `truncate_str` / `label_cell` — not ad-hoc slices (Unicode width matters; `unicode-width` is a dep).
- Errors: `mc_core::error::{CoreError,Result}` in engine; `anyhow` at the binary edge; UI shows toasts/status, does not panic.
- Serde enums: `#[serde(rename_all=...)]` explicit; defaults via `#[serde(default = "...")]` so old JSON loads.
- Tests: inline `#[cfg(test)] mod tests`. UI render tests use `ratatui::backend::TestBackend` + `buffer_text()` helper in `app/mod.rs`. Temp dirs via `Paths::rooted_at(temp_dir()+nanos)`. **EN is the default language in tests** — assert English strings or i18n keys accordingly.
- Do not commit secrets, tokens, or `.env`. `Cargo.lock` **is** committed.

---

## 8. Key files cheat-sheet

| Task | Where |
|------|--------|
| Global layout / which page renders | `mc_tui/src/app/render.rs` |
| Keys, mouse, hit dispatch | `mc_tui/src/app/input.rs` |
| Side effects (save/install/launch/settings) | `mc_tui/src/app/actions.rs` |
| Engine events → state | `mc_tui/src/app/events.rs` |
| App struct, Nav, HitAction, ButtonId | `mc_tui/src/app/mod.rs` |
| Launcher settings screen | `mc_tui/src/views/settings.rs` |
| Instance JVM settings screen | `mc_tui/src/views/instance_settings.rs` |
| Shared settings chrome / RAM sliders | `mc_tui/src/views/settings_ui.rs` |
| Cards, buttons_row, row helpers | `mc_tui/src/views/mod.rs` |
| Instance tiles / groups | `mc_tui/src/views/tiles.rs` |
| Modrinth browse | `mc_tui/src/views/browse.rs` |
| Colors / styles | `mc_tui/src/theme.rs` |
| Translations | `mc_tui/src/i18n.rs` |
| settings.json schema | `mc_tui/src/settings.rs` |
| Overlays / modal forms | `mc_tui/src/forms.rs` |
| Create wizard | `mc_tui/src/wizard.rs` |
| Instance model | `mc_core/src/instance/mod.rs` |
| Launch pipeline | `mc_core/src/launch/` |
| Paths / downloads / JSON | `mc_core/src/util.rs` |
| Microsoft auth | `mc_core/src/auth/microsoft.rs` |

---

## 9. Working agreement with the owner

- **Language:** reply in **Russian** unless asked otherwise. Code identifiers/comments in English.
- UI changes are reviewed via **screenshots**; expect follow-ups about exact padding, alignment, colors, and "holes" in backgrounds.
- Prefer small, verifiable UI diffs over big refactors. Always re-run `cargo test -q -p mc_tui -p mc_core` after visual changes (render tests catch panics/layout regressions).
- When unsure about a product/UX call (thresholds, wording, keybinds), ask — the owner has strong opinions (e.g. RAM warn at 60% not 50%; no fake accordion headers).
- Deferred / known open items:
  - English async toasts (only some paths are fully async-friendly).
  - Java runtime list is not scrollable when long.
  - `DEFAULT_CLIENT_ID` not confirmed as final.
  - README is still a stub.
- Do not commit unless explicitly asked. Do not amend/force-push. Stage only intended files.

---

## 10. Quick self-check before finishing a task

1. `cargo test -q -p mc_core -p mc_tui` — all green.
2. `cargo clippy -q -p mc_tui` — no **new** warnings in files you touched.
3. If i18n strings changed: En + Uk + Ru present, `{}` count matches format args.
4. If settings layout changed: gutters still 2+2, panels still solid, no black seams, scale/track/handle still share `value_to_pos`.
5. If mouse targets changed: hitboxes pushed specific-before-generic; no dead zones.
6. Report back in Russian, briefly: what changed, test status, anything deferred.
