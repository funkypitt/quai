//! The dock's state and behaviour. The Wayland plumbing is in `wayland.rs`.

use calloop::{
    LoopHandle,
    timer::{TimeoutAction, Timer},
};
use cosmic_client_toolkit::{
    cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1::{
        State as ToplevelState, ZcosmicToplevelHandleV1,
    },
    sctk::{
        activation::{ActivationState, RequestDataExt},
        compositor::{CompositorState, Region},
        output::OutputState,
        registry::RegistryState,
        seat::{SeatState, pointer::ThemedPointer},
        shell::{
            WaylandSurface,
            wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerSurface},
            xdg::{
                XdgPositioner, XdgShell,
                popup::{Popup, PopupConfigure},
            },
        },
        shm::{
            Shm,
            slot::{Buffer, SlotPool},
        },
    },
    toplevel_info::{ToplevelInfo, ToplevelInfoState},
    toplevel_management::ToplevelManagerState,
};
use std::{
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};
use wayland_client::{
    QueueHandle,
    protocol::{wl_output::WlOutput, wl_seat::WlSeat, wl_shm, wl_surface::WlSurface, wl_touch::WlTouch},
};
use wayland_protocols::{
    ext::{
        background_effect::v1::client::{
            ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1,
            ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
        },
        foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    },
    wp::{
        fractional_scale::v1::client::{
            wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
            wp_fractional_scale_v1::WpFractionalScaleV1,
        },
        viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
    },
    xdg::shell::client::xdg_positioner::{Anchor as PopAnchor, ConstraintAdjustment, Gravity},
};

use crate::{
    apps::{App, AppDb, Rgb},
    config::{Backlight, ClickActive, Config},
    i18n::{Msg, close_all, tr},
    launch,
    model::{self, Action, Button, LEFT, Tile, TileId, Win, WinId},
    wallpaper,
    render::{
        self, BOTTOM_PAD, COL_W, COLUMNS, DOCK_W, HEADER_H, MenuLayout, MenuRow, PITCH, Scene,
        TILE, TOP_PAD, TileDraw, tile_x,
    },
};

const DRAG_THRESHOLD: f32 = 8.0;
/// A finger held still this long opens the menu, as the right button does.
const LONG_PRESS_MS: u64 = 600;
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(12);
pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;

/// An invisible surface filling the area left free by every panel: its
/// size tells whether the dock has been placed under them or across them.
pub struct Probe {
    pub layer: LayerSurface,
    pub viewport: Option<WpViewport>,
    pub buffer: Option<Buffer>,
    pub usable_height: Option<u32>,
}

struct Shell {
    layer: LayerSurface,
    viewport: Option<WpViewport>,
    fractional: Option<WpFractionalScaleV1>,
    effect: Option<ExtBackgroundEffectSurfaceV1>,
}

pub struct Bar {
    pub output: WlOutput,
    pub layer: LayerSurface,
    pub viewport: Option<WpViewport>,
    pub fractional: Option<WpFractionalScaleV1>,
    pub effect: Option<ExtBackgroundEffectSurfaceV1>,
    pub probe: Option<Probe>,
    /// Free height for which the dock was last put back in place.
    pub replaced_for: Option<u32>,
    /// Whether the dock has taken its final width (see `settle`).
    pub settled: bool,
    pub scale: f32,
    pub height: f32,
    pub configured: bool,
    pub frame_pending: bool,
    pub dirty: bool,
    pub scroll: [f32; COLUMNS],
    pub buffer: Option<Buffer>,
}

/// A tile on screen, with the state of its animations.
pub struct Sprite {
    pub tile: Tile,
    pub app: Rc<App>,
    pub col: usize,
    /// Position in the column's content, before scrolling.
    pub x: f32,
    pub y: f32,
    pub target: (f32, f32),
    pub alpha: f32,
    pub hover: f32,
    pub pressed: f32,
    pub present: bool,
    pub fixed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MenuAction {
    Activate(WinId),
    Launch(String),
    LaunchAction(String, String),
    Pin(String),
    Unpin(String),
    Close(Vec<WinId>),
    ToggleGroup,
    ToggleBacklight,
    ToggleMinimize,
    ToggleButtons,
    OpenSettings,
}

pub enum PopupKind {
    Tooltip(String),
    Menu { rows: Vec<MenuRow<MenuAction>>, layout: MenuLayout, hover: Option<usize> },
}

pub struct PopupState {
    pub popup: Popup,
    pub kind: PopupKind,
    pub owner: Option<TileId>,
    pub viewport: Option<WpViewport>,
    pub scale: f32,
    pub size: (f32, f32),
    /// Centre of the tile pointed at, in the bar's coordinates.
    pub anchor_cy: f32,
    pub arrow_y: f32,
    pub configured: bool,
    pub dirty: bool,
    pub buffer: Option<Buffer>,
}

#[derive(Clone)]
pub struct Hit {
    pub tile: Tile,
    /// Top-left corner of the tile on the bar.
    pub x: f32,
    pub y: f32,
}

pub struct Press {
    pub hit: Option<Hit>,
    pub bar: usize,
    pub button: u32,
    pub origin: (f32, f32),
}

pub struct Drag {
    pub key: String,
    pub id: TileId,
    pub app: Rc<App>,
    pub windows: usize,
    pub grab: (f32, f32),
    pub bar: usize,
    /// The pinned list as it would be if the tile were dropped now.
    pub pinned: Vec<String>,
}

/// A finger on the dock. Touch goes through the pointer's path — down is enter and
/// press, motion is motion, up is release and leave — so taps, drags and the menu
/// behave as with a mouse. One finger drives the dock; the others are ignored.
pub struct TouchState {
    pub id: i32,
    /// The bar touched; `None` when the finger is on the menu.
    pub bar: Option<usize>,
    pub pos: (f32, f32),
    pub origin: (f32, f32),
    pub moved: bool,
    /// The menu opened under this finger (held still), or the finger is on the menu.
    pub menu: bool,
    pub generation: u64,
}

#[derive(Default)]
pub struct PointerState {
    pub bar: Option<usize>,
    pub pos: (f32, f32),
    pub hover: Option<TileId>,
    pub press: Option<Press>,
    pub serial: u32,
}

/// What travels with an activation token request.
pub struct LaunchData {
    pub key: String,
    pub action: Option<String>,
    pub seat_and_serial: Option<(WlSeat, u32)>,
    pub surface: Option<WlSurface>,
}

impl RequestDataExt for LaunchData {
    fn app_id(&self) -> Option<&str> {
        Some("quai")
    }
    fn seat_and_serial(&self) -> Option<(&WlSeat, u32)> {
        self.seat_and_serial.as_ref().map(|(s, n)| (s, *n))
    }
    fn surface(&self) -> Option<&WlSurface> {
        self.surface.as_ref()
    }
}

/// What reaches the dock from outside the display: files that changed,
/// and the tint worked out from the wallpaper.
pub enum Notice {
    Config,
    Apps,
    Wallpaper,
    Tint(Rgb),
}

pub struct Tracked {
    pub win: Win,
    pub foreign: ExtForeignToplevelHandleV1,
    pub cosmic: Option<ZcosmicToplevelHandleV1>,
}

pub struct Dock {
    pub qh: QueueHandle<Dock>,
    pub loop_handle: LoopHandle<'static, Dock>,
    pub registry_state: RegistryState,
    pub seat_state: SeatState,
    pub output_state: OutputState,
    pub compositor: CompositorState,
    pub shm: Shm,
    pub layer_shell: LayerShell,
    pub xdg_shell: XdgShell,
    pub activation: Option<ActivationState>,
    pub toplevel_info: ToplevelInfoState,
    pub toplevel_manager: Option<ToplevelManagerState>,
    pub viewporter: Option<WpViewporter>,
    pub fractional: Option<WpFractionalScaleManagerV1>,
    pub blur_manager: Option<ExtBackgroundEffectManagerV1>,
    pub blur_supported: bool,
    pub pool: SlotPool,

    pub seat: Option<WlSeat>,
    pub themed_pointer: Option<ThemedPointer>,
    pub pointer: PointerState,
    pub touch_dev: Option<WlTouch>,
    pub touch: Option<TouchState>,
    pub touch_generation: u64,

    pub bars: Vec<Bar>,
    pub popup: Option<PopupState>,

    pub config: Config,
    pub db: AppDb,
    pub tracked: Vec<Tracked>,
    pub columns: [Vec<Tile>; COLUMNS],
    pub buttons: [Option<Tile>; COLUMNS],
    pub sprites: Vec<Sprite>,
    pub drag: Option<Drag>,
    pub launching: HashMap<String, Instant>,

    next_win: WinId,
    seq: u64,
    focus_count: u64,
    last_tick: Instant,
    was_moving: bool,
    hover_generation: u64,
    pub reload_pending: bool,
    pub watcher: Option<notify::RecommendedWatcher>,
    pub notices: Option<calloop::channel::Sender<Notice>>,
    pub tint: Rgb,
    pub exit: bool,
}

#[allow(clippy::too_many_arguments)]
impl Dock {
    pub fn new(
        qh: QueueHandle<Dock>,
        loop_handle: LoopHandle<'static, Dock>,
        registry_state: RegistryState,
        seat_state: SeatState,
        output_state: OutputState,
        compositor: CompositorState,
        shm: Shm,
        layer_shell: LayerShell,
        xdg_shell: XdgShell,
        activation: Option<ActivationState>,
        toplevel_info: ToplevelInfoState,
        toplevel_manager: Option<ToplevelManagerState>,
        viewporter: Option<WpViewporter>,
        fractional: Option<WpFractionalScaleManagerV1>,
        pool: SlotPool,
        config: Config,
    ) -> Self {
        let mut db = AppDb::new(&config.icon_theme);
        let mut config = config;
        canonicalize_pinned(&mut config, &mut db);
        let mut dock = Self {
            qh,
            loop_handle,
            registry_state,
            seat_state,
            output_state,
            compositor,
            shm,
            layer_shell,
            xdg_shell,
            activation,
            toplevel_info,
            toplevel_manager,
            viewporter,
            fractional,
            blur_manager: None,
            blur_supported: false,
            pool,
            seat: None,
            themed_pointer: None,
            pointer: PointerState::default(),
            touch_dev: None,
            touch: None,
            touch_generation: 0,
            bars: Vec::new(),
            popup: None,
            config,
            db,
            tracked: Vec::new(),
            columns: [Vec::new(), Vec::new()],
            buttons: [None, None],
            sprites: Vec::new(),
            drag: None,
            launching: HashMap::new(),
            next_win: 1,
            seq: 0,
            focus_count: 0,
            last_tick: Instant::now(),
            was_moving: false,
            hover_generation: 0,
            reload_pending: false,
            watcher: None,
            notices: None,
            tint: wallpaper::NEUTRAL,
            exit: false,
        };
        dock.refresh();
        // Tiles present at start are simply there; only later ones fade in.
        for s in &mut dock.sprites {
            s.alpha = 1.0;
        }
        dock
    }

    // ------------------------------------------------------------ bars

    pub fn wants_output(&self, output: &WlOutput) -> bool {
        if self.config.output.is_empty() || self.config.output.eq_ignore_ascii_case("all") {
            return true;
        }
        self.output_state
            .info(output)
            .and_then(|i| i.name)
            .is_some_and(|n| n.eq_ignore_ascii_case(&self.config.output))
    }

    pub fn add_bar(&mut self, output: WlOutput) {
        if self.bars.iter().any(|b| b.output == output) || !self.wants_output(&output) {
            return;
        }
        let Shell { layer, viewport, fractional, effect } = self.make_shell(&output);
        let probe = self.make_probe(&output);
        self.bars.push(Bar {
            output,
            layer,
            viewport,
            fractional,
            effect,
            probe,
            replaced_for: None,
            settled: false,
            scale: 1.0,
            height: 0.0,
            configured: false,
            frame_pending: false,
            dirty: true,
            scroll: [0.0; COLUMNS],
            buffer: None,
        });
        self.update_tint();
    }

    /// Sets the dock's colour as the settings say. Reading a wallpaper
    /// takes a moment: it is done aside, and the colour comes as a notice.
    pub fn update_tint(&mut self) {
        let choice = self.config.tint.trim().to_ascii_lowercase();
        if let Some(c) = wallpaper::parse_hex(&choice) {
            self.set_tint(c);
        } else if choice == "wallpaper" {
            let outputs: Vec<String> = self
                .bars
                .iter()
                .filter_map(|b| self.output_state.info(&b.output).and_then(|i| i.name))
                .collect();
            let Some(tx) = self.notices.clone() else { return };
            std::thread::spawn(move || {
                let tint = wallpaper::current(&outputs).and_then(|s| wallpaper::tint_of(&s));
                let _ = tx.send(Notice::Tint(tint.unwrap_or(wallpaper::NEUTRAL)));
            });
        } else {
            self.set_tint(wallpaper::NEUTRAL);
        }
    }

    pub fn set_tint(&mut self, tint: Rgb) {
        if tint != self.tint {
            self.tint = tint;
            self.mark_dirty();
        }
    }

    fn make_shell(&self, output: &WlOutput) -> Shell {
        let surface = self.compositor.create_surface(&self.qh);
        let (viewport, fractional) = match (&self.viewporter, &self.fractional) {
            (Some(vp), Some(fr)) => (
                Some(vp.get_viewport(&surface, &self.qh, ())),
                Some(fr.get_fractional_scale(&surface, &self.qh, surface.clone())),
            ),
            _ => (None, None),
        };
        let effect = self
            .blur_manager
            .as_ref()
            .filter(|_| std::env::var_os("QUAI_NO_EFFECT").is_none())
            .map(|m| m.get_background_effect(&surface, &self.qh, ()));
        let layer = self.layer_shell.create_layer_surface(
            &self.qh,
            surface,
            Layer::Top,
            Some("quai"),
            Some(output),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT);
        // One pixel short of its width: see `settle`.
        layer.set_size(DOCK_W as u32 - 1, 0);
        layer.set_exclusive_zone(DOCK_W as i32);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        Shell { layer, viewport, fractional, effect }
    }

    fn make_probe(&self, output: &WlOutput) -> Option<Probe> {
        if std::env::var_os("QUAI_NO_PROBE").is_some() {
            return None;
        }
        let surface = self.compositor.create_surface(&self.qh);
        let region = Region::new(&self.compositor).ok()?;
        surface.set_input_region(Some(region.wl_region()));
        let viewport = self.viewporter.as_ref().map(|v| v.get_viewport(&surface, &self.qh, ()));
        let layer = self.layer_shell.create_layer_surface(
            &self.qh,
            surface,
            Layer::Background,
            Some("quai-probe"),
            Some(output),
        );
        layer.set_anchor(Anchor::all());
        layer.set_size(0, 0);
        layer.set_exclusive_zone(0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        Some(Probe { layer, viewport, buffer: None, usable_height: None })
    }

    fn drop_shell(viewport: Option<WpViewport>, fractional: Option<WpFractionalScaleV1>, effect: Option<ExtBackgroundEffectSurfaceV1>) {
        // These must go before the surface they belong to.
        if let Some(e) = effect {
            e.destroy();
        }
        if let Some(f) = fractional {
            f.destroy();
        }
        if let Some(v) = viewport {
            v.destroy();
        }
    }

    pub fn remove_bar(&mut self, index: usize) {
        if index >= self.bars.len() {
            return;
        }
        self.close_popup();
        self.pointer = PointerState::default();
        self.drag = None;
        let mut bar = self.bars.remove(index);
        Self::drop_shell(bar.viewport.take(), bar.fractional.take(), bar.effect.take());
        if let Some(v) = bar.probe.as_mut().and_then(|p| p.viewport.take()) {
            v.destroy();
        }
    }

    pub fn probe_of(&self, surface: &WlSurface) -> Option<usize> {
        self.bars.iter().position(|b| b.probe.as_ref().is_some_and(|p| p.layer.wl_surface() == surface))
    }

    /// The compositor tells the probe how much room the panels leave.
    pub fn probe_configured(&mut self, index: usize, width: u32, height: u32) {
        let Some(probe) = self.bars[index].probe.as_mut() else { return };
        probe.usable_height = (height > 0).then_some(height);
        if probe.buffer.is_none() {
            probe.buffer = self
                .pool
                .create_buffer(1, 1, 4, wl_shm::Format::Argb8888)
                .ok()
                .map(|(buffer, canvas)| {
                    canvas.fill(0);
                    buffer
                });
        }
        let surface = probe.layer.wl_surface();
        if let Some(b) = &probe.buffer {
            let _ = b.attach_to(surface);
        }
        if let Some(v) = &probe.viewport
            && width > 0
            && height > 0
        {
            v.set_destination(width as i32, height as i32);
        }
        probe.layer.commit();

        // Panels settle in a burst at login: look once they have.
        let timer = Timer::from_duration(Duration::from_millis(400));
        let _ = self.loop_handle.insert_source(timer, |_, _, dock| {
            dock.check_placement();
            TimeoutAction::Drop
        });
    }

    /// The compositor serves panels in their order of arrival. A panel that
    /// came after the dock is squeezed beside it, and the dock crosses the
    /// panel's place; the dock then steps out and back in, behind the panel.
    pub fn check_placement(&mut self) {
        for i in 0..self.bars.len() {
            let bar = &self.bars[i];
            let Some(free) = bar.probe.as_ref().and_then(|p| p.usable_height) else { continue };
            if !bar.configured || bar.height <= free as f32 + 0.5 || bar.replaced_for == Some(free) {
                continue;
            }
            log::info!("dock is {} px high for {free} px of free height: taking place again", bar.height);
            self.close_popup();
            self.pointer = PointerState::default();
            self.drag = None;
            let output = self.bars[i].output.clone();
            let shell = self.make_shell(&output);
            let bar = &mut self.bars[i];
            Self::drop_shell(bar.viewport.take(), bar.fractional.take(), bar.effect.take());
            bar.layer = shell.layer;
            bar.viewport = shell.viewport;
            bar.fractional = shell.fractional;
            bar.effect = shell.effect;
            bar.buffer = None;
            bar.configured = false;
            bar.frame_pending = false;
            bar.dirty = true;
            bar.settled = false;
            bar.replaced_for = Some(free);
        }
    }

    /// Asks for the blur behind the dock, over its whole surface.
    pub fn apply_blur(&self, index: usize) {
        let bar = &self.bars[index];
        let Some(effect) = &bar.effect else { return };
        if !self.blur_supported || !self.config.blur || bar.height <= 0.0 {
            effect.set_blur_region(None);
            return;
        }
        if let Ok(region) = Region::new(&self.compositor) {
            region.add(0, 0, DOCK_W as i32, bar.height as i32);
            effect.set_blur_region(Some(region.wl_region()));
        }
    }

    /// Outputs come and go, and the setting may change.
    pub fn sync_bars(&mut self) {
        let outputs: Vec<WlOutput> = self.output_state.outputs().collect();
        let mut i = 0;
        while i < self.bars.len() {
            let out = self.bars[i].output.clone();
            if !outputs.contains(&out) || !self.wants_output(&out) {
                self.remove_bar(i);
            } else {
                i += 1;
            }
        }
        for o in outputs {
            self.add_bar(o);
        }
    }

    pub fn bar_of(&self, surface: &WlSurface) -> Option<usize> {
        self.bars.iter().position(|b| b.layer.wl_surface() == surface)
    }

    pub fn set_scale(&mut self, surface: &WlSurface, scale: f32) {
        let scale = scale.clamp(0.5, 8.0);
        if let Some(i) = self.bar_of(surface) {
            if (self.bars[i].scale - scale).abs() > 0.001 {
                self.bars[i].scale = scale;
                self.bars[i].buffer = None;
                self.bars[i].dirty = true;
            }
        } else if let Some(p) = self.popup.as_mut().filter(|p| p.popup.wl_surface() == surface)
            && (p.scale - scale).abs() > 0.001
        {
            p.scale = scale;
            p.buffer = None;
            p.dirty = true;
        }
    }

    pub fn mark_dirty(&mut self) {
        for b in &mut self.bars {
            b.dirty = true;
        }
    }

    fn top(&self) -> f32 {
        if self.config.buttons { HEADER_H } else { TOP_PAD }
    }

    fn max_scroll(&self, bar: usize, col: usize) -> f32 {
        let n = self.columns[col].len() as f32;
        let content = if n > 0.0 { n * PITCH - render::GAP + BOTTOM_PAD } else { 0.0 };
        (content - (self.bars[bar].height - self.top())).max(0.0)
    }

    fn clamp_scrolls(&mut self) {
        for b in 0..self.bars.len() {
            for c in 0..COLUMNS {
                let max = self.max_scroll(b, c);
                let s = &mut self.bars[b].scroll[c];
                *s = s.clamp(0.0, max);
            }
        }
    }

    pub fn scroll_by(&mut self, bar: usize, col: usize, delta: f32) {
        if bar >= self.bars.len() || col >= COLUMNS {
            return;
        }
        let max = self.max_scroll(bar, col);
        let s = &mut self.bars[bar].scroll[col];
        let new = (*s + delta).clamp(0.0, max);
        if (new - *s).abs() > 0.01 {
            *s = new;
            self.bars[bar].dirty = true;
            self.close_tooltip();
        }
    }

    // --------------------------------------------------------- windows

    pub fn wins(&self) -> Vec<Win> {
        self.tracked.iter().map(|t| t.win.clone()).collect()
    }

    pub fn upsert_toplevel(&mut self, info: &ToplevelInfo) {
        let active = info.state.contains(&ToplevelState::Activated);
        let minimized = info.state.contains(&ToplevelState::Minimized);
        if let Some(t) = self.tracked.iter_mut().find(|t| t.foreign == info.foreign_toplevel) {
            if active && !t.win.active {
                self.focus_count += 1;
                t.win.last_active = self.focus_count;
            }
            if t.win.app_id != info.app_id || (t.win.key.starts_with("title:") && t.win.title != info.title) {
                t.win.key = self.db.resolve(&info.app_id, &info.title);
            }
            t.win.app_id = info.app_id.clone();
            t.win.title = info.title.clone();
            t.win.active = active;
            t.win.minimized = minimized;
            t.cosmic = info.cosmic_toplevel.clone();
        } else {
            self.seq += 1;
            if active {
                self.focus_count += 1;
            }
            let key = self.db.resolve(&info.app_id, &info.title);
            self.launching.remove(&key);
            let win = Win {
                id: self.next_win,
                app_id: info.app_id.clone(),
                title: info.title.clone(),
                key,
                active,
                minimized,
                seq: self.seq,
                last_active: if active { self.focus_count } else { 0 },
            };
            self.next_win += 1;
            self.tracked.push(Tracked {
                win,
                foreign: info.foreign_toplevel.clone(),
                cosmic: info.cosmic_toplevel.clone(),
            });
        }
        self.refresh();
    }

    pub fn remove_toplevel(&mut self, handle: &ExtForeignToplevelHandleV1) {
        self.tracked.retain(|t| &t.foreign != handle);
        self.refresh();
    }

    // ---------------------------------------------------------- layout

    fn effective_pinned(&self) -> &[String] {
        self.drag.as_ref().map_or(&self.config.pinned, |d| &d.pinned)
    }

    /// Rebuilds the columns and brings the sprites in line with them.
    pub fn refresh(&mut self) {
        let wins = self.wins();
        self.columns = model::build(self.effective_pinned(), &wins, self.config.group_windows);
        self.buttons = if self.config.buttons {
            [Button::Applications, Button::Workspaces].map(|b| {
                Some(Tile {
                    id: TileId::Button(b),
                    key: button_key(b).to_string(),
                    wins: Vec::new(),
                    active: false,
                    pinned: false,
                })
            })
        } else {
            [None, None]
        };

        for s in &mut self.sprites {
            s.present = false;
        }
        for col in 0..COLUMNS {
            if let Some(t) = self.buttons[col].clone() {
                let TileId::Button(b) = t.id else { continue };
                let (desktop, name) = button_entry(b);
                let app = self.db.builtin(&t.key, desktop, name);
                self.place(t, app, col, (tile_x(col), TOP_PAD), true);
            }
            for (i, t) in self.columns[col].clone().into_iter().enumerate() {
                let app = self.db.app(&t.key);
                self.place(t, app, col, (tile_x(col), i as f32 * PITCH), false);
            }
        }
        self.clamp_scrolls();
        self.rehover();
        self.mark_dirty();
        if log::log_enabled!(log::Level::Debug) {
            let show = |tiles: &[Tile]| {
                tiles
                    .iter()
                    .map(|t| format!("{}×{}{}", t.key, t.wins.len(), if t.active { "*" } else { "" }))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            log::debug!("left [{}] right [{}]", show(&self.columns[0]), show(&self.columns[1]));
        }
    }

    fn place(&mut self, tile: Tile, app: Rc<App>, col: usize, target: (f32, f32), fixed: bool) {
        if let Some(s) = self.sprites.iter_mut().find(|s| s.tile.id == tile.id) {
            s.tile = tile;
            s.app = app;
            s.col = col;
            s.target = target;
            s.present = true;
            s.fixed = fixed;
        } else {
            self.sprites.push(Sprite {
                tile,
                app,
                col,
                x: target.0,
                y: target.1,
                target,
                alpha: 0.0,
                hover: 0.0,
                pressed: 0.0,
                present: true,
                fixed,
            });
        }
    }

    /// Advances the animations; true while something still moves.
    pub fn tick(&mut self) -> bool {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f32();
        self.last_tick = now;
        // Out of a rest, start with one ordinary frame instead of a leap;
        // once moving, follow the clock even when frames come slowly.
        let dt = if self.was_moving { dt.min(0.25) } else { 1.0 / 60.0 };
        let ease = |rate: f32| 1.0 - (-dt * rate).exp();
        let mut moving = false;

        let live: Vec<String> = self.tracked.iter().map(|t| t.win.key.clone()).collect();
        self.launching.retain(|k, since| !live.contains(k) && since.elapsed() < LAUNCH_TIMEOUT);
        moving |= !self.launching.is_empty();

        let hover = self.pointer.hover.clone();
        let pressed = self
            .pointer
            .press
            .as_ref()
            .filter(|p| p.button == BTN_LEFT)
            .and_then(|p| p.hit.as_ref().map(|h| h.tile.id.clone()));
        let popup_owner = self.popup.as_ref().and_then(|p| match p.kind {
            PopupKind::Menu { .. } => p.owner.clone(),
            PopupKind::Tooltip(_) => None,
        });
        for s in &mut self.sprites {
            let mut approach = |v: &mut f32, to: f32, rate: f32| {
                if (*v - to).abs() < 0.004 {
                    *v = to;
                } else {
                    *v += (to - *v) * ease(rate);
                    moving = true;
                }
            };
            let is_hover = Some(&s.tile.id) == hover.as_ref() || Some(&s.tile.id) == popup_owner.as_ref();
            approach(&mut s.alpha, if s.present { 1.0 } else { 0.0 }, 14.0);
            approach(&mut s.hover, if is_hover { 1.0 } else { 0.0 }, 20.0);
            approach(&mut s.pressed, if Some(&s.tile.id) == pressed.as_ref() { 1.0 } else { 0.0 }, 30.0);
            for (v, to) in [(&mut s.x, s.target.0), (&mut s.y, s.target.1)] {
                if (*v - to).abs() < 0.3 {
                    *v = to;
                } else {
                    *v += (to - *v) * ease(16.0);
                    moving = true;
                }
            }
        }
        self.sprites.retain(|s| s.present || s.alpha > 0.02);
        if moving {
            self.mark_dirty();
        }
        self.was_moving = moving;
        moving
    }

    fn pulse(&self, key: &str) -> f32 {
        self.launching.get(key).map_or(0.0, |since| {
            let t = since.elapsed().as_secs_f32();
            0.5 - 0.5 * (t * std::f32::consts::TAU / 1.1).cos()
        })
    }

    fn lit(&self, tile: &Tile) -> bool {
        self.config.backlight == Backlight::Always || !tile.wins.is_empty()
    }

    pub fn scene(&self, bar: usize) -> Scene {
        let b = &self.bars[bar];
        let top = self.top();
        let hidden = |s: &Sprite| {
            self.drag.as_ref().is_some_and(|d| {
                s.tile.key == d.key && (s.tile.id == d.id || matches!(s.tile.id, TileId::App(_)))
            })
        };
        let mut tiles: Vec<TileDraw> = self
            .sprites
            .iter()
            .filter(|s| !hidden(s))
            .map(|s| {
                let y = if s.fixed { s.y } else { top + s.y - b.scroll[s.col] };
                TileDraw {
                    app: s.app.clone(),
                    x: s.x,
                    y,
                    col: s.col,
                    lit: self.lit(&s.tile),
                    windows: s.tile.wins.len(),
                    active: s.tile.active,
                    hover: s.hover,
                    pressed: s.pressed,
                    pulse: if matches!(s.tile.id, TileId::App(_)) { self.pulse(&s.tile.key) } else { 0.0 },
                    alpha: s.alpha,
                    zoom: 0.72 + 0.28 * s.alpha,
                    fixed: s.fixed,
                    lifted: false,
                }
            })
            .filter(|t| t.fixed || (t.y + TILE + 14.0 > top && t.y - 14.0 < b.height))
            .collect();

        if let Some(d) = self.drag.as_ref().filter(|d| d.bar == bar) {
            let (px, py) = self.pointer.pos;
            let stays = d.pinned.contains(&d.key) || d.windows > 0;
            tiles.push(TileDraw {
                app: d.app.clone(),
                x: px - d.grab.0,
                y: py - d.grab.1,
                // Arrows are drawn against a column; a lifted tile shows none.
                col: 0,
                lit: self.config.backlight == Backlight::Always || d.windows > 0,
                windows: 0,
                active: false,
                hover: 1.0,
                pressed: 0.0,
                pulse: 0.0,
                alpha: if stays { 0.92 } else { 0.45 },
                zoom: 1.06,
                fixed: true,
                lifted: true,
            });
        }

        let mut overflow = [(false, false); COLUMNS];
        for (c, o) in overflow.iter_mut().enumerate() {
            let max = self.max_scroll(bar, c);
            *o = (b.scroll[c] > 0.5, b.scroll[c] < max - 0.5);
        }
        Scene {
            height: b.height,
            scale: b.scale,
            opacity: self.config.opacity,
            tint: self.tint,
            header: self.config.buttons,
            tiles,
            overflow,
            drop_mark: None,
        }
    }

    pub fn hit(&self, bar: usize, x: f32, y: f32) -> Option<Hit> {
        let b = self.bars.get(bar)?;
        if !(0.0..DOCK_W).contains(&x) || y < 0.0 || y >= b.height {
            return None;
        }
        let col = ((x / COL_W) as usize).min(COLUMNS - 1);
        let tx = tile_x(col);
        if self.config.buttons && y < HEADER_H {
            let t = self.buttons[col].clone()?;
            return (TOP_PAD..TOP_PAD + TILE).contains(&y).then_some(Hit { tile: t, x: tx, y: TOP_PAD });
        }
        let top = self.top();
        let cy = y - top + b.scroll[col];
        if cy < 0.0 {
            return None;
        }
        let i = (cy / PITCH) as usize;
        let tile = self.columns[col].get(i)?.clone();
        (cy - i as f32 * PITCH <= TILE).then_some(Hit {
            tile,
            x: tx,
            y: top + i as f32 * PITCH - b.scroll[col],
        })
    }

    // --------------------------------------------------------- pointer

    /// Recomputes what is under the pointer after the layout changed.
    fn rehover(&mut self) {
        let Some(bar) = self.pointer.bar else { return };
        let (x, y) = self.pointer.pos;
        self.set_hover(bar, x, y);
    }

    fn set_hover(&mut self, bar: usize, x: f32, y: f32) {
        let hit = if self.drag.is_some() { None } else { self.hit(bar, x, y) };
        let id = hit.as_ref().map(|h| h.tile.id.clone());
        if id == self.pointer.hover {
            return;
        }
        self.pointer.hover = id.clone();
        self.hover_generation += 1;
        self.mark_dirty();

        let menu_open = matches!(self.popup.as_ref().map(|p| &p.kind), Some(PopupKind::Menu { .. }));
        if menu_open || self.pointer.press.is_some() {
            return;
        }
        let tooltip_open = self.popup.is_some();
        self.close_tooltip();
        let Some(hit) = hit else { return };
        if tooltip_open {
            // Going from tile to tile, the next tooltip comes at once.
            self.show_tooltip(bar, &hit);
        } else {
            let generation = self.hover_generation;
            let delay = Duration::from_millis(self.config.tooltip_delay_ms);
            let _ = self.loop_handle.insert_source(Timer::from_duration(delay), move |_, _, dock| {
                if dock.hover_generation == generation
                    && dock.popup.is_none()
                    && dock.pointer.press.is_none()
                    && let Some(bar) = dock.pointer.bar
                    && let Some(hit) = dock.hit(bar, dock.pointer.pos.0, dock.pointer.pos.1)
                {
                    dock.show_tooltip(bar, &hit);
                }
                TimeoutAction::Drop
            });
        }
    }

    pub fn pointer_enter(&mut self, bar: usize, x: f32, y: f32) {
        self.pointer.bar = Some(bar);
        self.pointer.pos = (x, y);
        self.set_hover(bar, x, y);
    }

    pub fn pointer_leave(&mut self) {
        // While a button is held, the surface keeps the pointer: a drag
        // that leaves the dock goes on.
        if self.pointer.press.is_some() {
            return;
        }
        self.pointer.bar = None;
        if self.pointer.hover.take().is_some() {
            self.hover_generation += 1;
            self.mark_dirty();
        }
        self.close_tooltip();
    }

    pub fn pointer_motion(&mut self, bar: usize, x: f32, y: f32) {
        self.pointer.bar = Some(bar);
        self.pointer.pos = (x, y);
        if self.drag.is_some() {
            self.update_drag();
            return;
        }
        let start = self.pointer.press.as_ref().and_then(|p| {
            let far = (x - p.origin.0).hypot(y - p.origin.1) > DRAG_THRESHOLD;
            let hit = p.hit.as_ref()?;
            (p.button == BTN_LEFT && far && !matches!(hit.tile.id, TileId::Button(_)))
                .then(|| (hit.clone(), p.origin, p.bar))
        });
        if let Some((hit, origin, bar)) = start {
            self.close_popup();
            self.drag = Some(Drag {
                app: self.db.app(&hit.tile.key),
                key: hit.tile.key.clone(),
                id: hit.tile.id.clone(),
                windows: self.tracked.iter().filter(|t| t.win.key == hit.tile.key).count(),
                grab: (origin.0 - hit.x, origin.1 - hit.y),
                bar,
                pinned: self.config.pinned.clone(),
            });
            self.pointer.hover = None;
            self.update_drag();
            return;
        }
        if self.pointer.press.is_none() {
            self.set_hover(bar, x, y);
        }
    }

    fn update_drag(&mut self) {
        let Some(d) = self.drag.as_ref() else { return };
        let (px, py) = self.pointer.pos;
        let mut pinned: Vec<String> = self.config.pinned.iter().filter(|k| **k != d.key).cloned().collect();
        // Over the left column the tile is pinned where it hovers; anywhere
        // else it is let go.
        let centre_x = px - d.grab.0 + TILE / 2.0;
        if centre_x < COL_W && px > -40.0 {
            let scroll = self.bars.get(d.bar).map_or(0.0, |b| b.scroll[LEFT]);
            let tile_top = py - d.grab.1 - self.top() + scroll;
            let index = ((tile_top + PITCH / 2.0) / PITCH).floor().max(0.0) as usize;
            pinned.insert(index.min(pinned.len()), d.key.clone());
        }
        let changed = pinned != d.pinned;
        if let Some(d) = self.drag.as_mut() {
            d.pinned = pinned;
        }
        if changed {
            self.refresh();
        }
        self.mark_dirty();
    }

    pub fn pointer_press(&mut self, bar: usize, x: f32, y: f32, button: u32, serial: u32) {
        self.pointer.bar = Some(bar);
        self.pointer.pos = (x, y);
        self.pointer.serial = serial;
        self.close_popup();
        let hit = self.hit(bar, x, y);
        if button == BTN_RIGHT {
            self.open_menu(bar, hit, y, serial);
            return;
        }
        self.pointer.press = Some(Press { hit, bar, button, origin: (x, y) });
        self.mark_dirty();
    }

    pub fn pointer_release(&mut self, bar: usize, x: f32, y: f32, button: u32, serial: u32) {
        self.pointer.pos = (x, y);
        let Some(press) = self.pointer.press.take() else { return };
        if press.button != button {
            self.pointer.press = Some(press);
            return;
        }
        self.mark_dirty();
        if let Some(d) = self.drag.take() {
            self.drop_tile(d);
        } else if let (Some(was), Some(now)) = (press.hit, self.hit(bar, x, y))
            && was.tile.id == now.tile.id
        {
            let surface = self.bars.get(bar).map(|b| b.layer.wl_surface().clone());
            match button {
                BTN_LEFT => {
                    let action = model::click(&now.tile, &self.wins(), self.config.click_active);
                    self.perform(action, serial, surface);
                }
                BTN_MIDDLE if !matches!(now.tile.id, TileId::Button(_)) => {
                    self.perform(Action::Launch(now.tile.key), serial, surface);
                }
                _ => {}
            }
        }
        // The pointer may have ended up outside the dock during a drag.
        let inside = self.bars.get(bar).is_some_and(|b| (0.0..DOCK_W).contains(&x) && (0.0..b.height).contains(&y));
        if inside {
            self.set_hover(bar, x, y);
        } else {
            self.pointer_leave();
        }
    }

    // ---- touch: routed through the pointer path ---------------------------

    pub fn touch_down(&mut self, id: i32, surface: &WlSurface, x: f32, y: f32, serial: u32) {
        if self.touch.is_some() {
            return;
        }
        self.touch_generation += 1;
        let generation = self.touch_generation;
        let state = |bar, menu| TouchState { id, bar, pos: (x, y), origin: (x, y), moved: false, menu, generation };
        if self.popup.as_ref().is_some_and(|p| p.popup.wl_surface() == surface) {
            if matches!(self.popup.as_ref().map(|p| &p.kind), Some(PopupKind::Menu { .. })) {
                self.menu_motion(x, y);
                self.touch = Some(state(None, true));
            }
            return;
        }
        let Some(bar) = self.bar_of(surface) else { return };
        self.pointer_enter(bar, x, y);
        self.pointer_press(bar, x, y, BTN_LEFT, serial);
        self.touch = Some(state(Some(bar), false));
        log::debug!("touch {id} down on bar {bar} at {x:.0},{y:.0}");
        // A finger held still opens the menu, as the right button does.
        let _ = self.loop_handle.insert_source(Timer::from_duration(Duration::from_millis(LONG_PRESS_MS)), move |_, _, dock| {
            let held = dock.touch.as_ref().is_some_and(|t| t.generation == generation && !t.moved && !t.menu) && dock.drag.is_none();
            if held {
                if let Some(t) = dock.touch.as_mut() {
                    t.menu = true;
                }
                // lifting the finger must not count as a click
                dock.pointer.press = None;
                let hit = dock.hit(bar, x, y);
                dock.open_menu(bar, hit, y, serial);
                dock.mark_dirty();
                log::debug!("touch held: menu");
            }
            TimeoutAction::Drop
        });
    }

    pub fn touch_motion(&mut self, id: i32, x: f32, y: f32) {
        let Some(t) = self.touch.as_mut() else { return };
        if t.id != id {
            return;
        }
        t.pos = (x, y);
        if (x - t.origin.0).hypot(y - t.origin.1) > DRAG_THRESHOLD {
            t.moved = true;
        }
        match (t.bar, t.menu) {
            (None, _) => self.menu_motion(x, y),
            // the menu opened under the held finger: it waits for the next tap
            (Some(_), true) => {}
            (Some(bar), false) => self.pointer_motion(bar, x, y),
        }
    }

    pub fn touch_up(&mut self, id: i32, serial: u32) {
        let Some(t) = self.touch.take() else { return };
        if t.id != id {
            self.touch = Some(t);
            return;
        }
        let (x, y) = t.pos;
        log::debug!("touch {id} up at {x:.0},{y:.0}");
        match (t.bar, t.menu) {
            (None, _) => self.menu_click(x, y, serial),
            (Some(_), true) => {}
            (Some(bar), false) => {
                self.pointer_release(bar, x, y, BTN_LEFT, serial);
                // no hover lingers once the finger is gone
                self.pointer_leave();
            }
        }
    }

    pub fn touch_cancel(&mut self) {
        if self.touch.take().is_none() {
            return;
        }
        self.pointer.press = None;
        self.drag = None;
        self.pointer_leave();
        self.mark_dirty();
    }

    fn drop_tile(&mut self, d: Drag) {
        let (px, py) = self.pointer.pos;
        log::debug!("drop {} at {px:.0},{py:.0}: pinned {:?}", d.key, d.pinned);
        if d.pinned != self.config.pinned {
            self.config.pinned = d.pinned;
            self.save_config();
        }
        self.refresh();
        // The tile settles into place from where it was let go.
        let scroll = self.bars.get(d.bar).map(|b| b.scroll).unwrap_or_default();
        let top = self.top();
        for s in self.sprites.iter_mut().filter(|s| s.present && s.tile.key == d.key && !s.fixed) {
            s.x = px - d.grab.0;
            s.y = py - d.grab.1 - top + scroll[s.col];
            s.alpha = 1.0;
        }
    }

    // --------------------------------------------------------- actions

    fn tracked(&self, id: WinId) -> Option<&Tracked> {
        self.tracked.iter().find(|t| t.win.id == id)
    }

    pub fn perform(&mut self, action: Action, serial: u32, surface: Option<WlSurface>) {
        log::debug!("click: {action:?}");
        match action {
            Action::None => {}
            Action::Activate(id) => self.activate(id),
            Action::Minimize(id) => {
                if let (Some(m), Some(c)) =
                    (&self.toplevel_manager, self.tracked(id).and_then(|t| t.cosmic.as_ref()))
                {
                    m.manager.set_minimized(c);
                }
            }
            Action::Launch(key) => self.launch(key, None, serial, surface),
            Action::Press(b) => self.launch(button_key(b).to_string(), None, serial, surface),
        }
    }

    fn activate(&mut self, id: WinId) {
        let (Some(m), Some(seat)) = (&self.toplevel_manager, &self.seat) else { return };
        let Some(t) = self.tracked(id) else { return };
        let Some(c) = t.cosmic.as_ref() else { return };
        if t.win.minimized {
            m.manager.unset_minimized(c);
        }
        m.manager.activate(c, seat);
    }

    fn close(&mut self, ids: &[WinId]) {
        let Some(m) = &self.toplevel_manager else { return };
        for c in ids.iter().filter_map(|id| self.tracked(*id)).filter_map(|t| t.cosmic.as_ref()) {
            m.manager.close(c);
        }
    }

    fn launch(&mut self, key: String, action: Option<String>, serial: u32, surface: Option<WlSurface>) {
        if !key.starts_with("button:") {
            if !self.db.app(&key).can_launch() {
                return;
            }
            if action.is_none() {
                self.launching.insert(key.clone(), Instant::now());
                self.mark_dirty();
            }
        }
        let data = LaunchData {
            key,
            action,
            seat_and_serial: self.seat.clone().map(|s| (s, serial)),
            surface,
        };
        match &self.activation {
            Some(a) => a.request_token_with_data(&self.qh, data),
            None => self.spawn(&data, None),
        }
    }

    /// Starts the application once its activation token has arrived.
    pub fn spawn(&mut self, data: &LaunchData, token: Option<&str>) {
        log::debug!("launch {} {:?}, token: {}", data.key, data.action, token.is_some());
        let app = if let Some(b) = button_from_key(&data.key) {
            let (desktop, name) = button_entry(b);
            self.db.builtin(&data.key, desktop, name)
        } else {
            self.db.app(&data.key)
        };
        let started = match &app.entry {
            Some(e) => launch::launch(e, data.action.as_deref(), token),
            None => match button_from_key(&data.key) {
                Some(b) => launch::run(&[button_command(b).to_string()], button_command(b), None, token),
                None => false,
            },
        };
        if !started {
            self.launching.remove(&data.key);
            self.mark_dirty();
        }
    }

    pub fn save_config(&mut self) {
        if let Err(e) = self.config.save() {
            log::error!("cannot save the settings: {e}");
        }
    }

    /// Applies settings edited by hand in the file.
    pub fn reload_config(&mut self) {
        let Some(mut new) = Config::reload() else { return };
        canonicalize_pinned(&mut new, &mut self.db);
        if new == self.config {
            return;
        }
        let theme_changed = new.icon_theme != self.config.icon_theme;
        self.config = new;
        if theme_changed {
            self.db.set_theme(&self.config.icon_theme);
            self.sprites.clear();
            render::clear_sprites();
        }
        self.sync_bars();
        self.update_tint();
        for i in 0..self.bars.len() {
            self.apply_blur(i);
        }
        self.refresh();
    }

    /// Desktop files were added or removed.
    pub fn reload_apps(&mut self) {
        self.db.reload();
        render::clear_sprites();
        for t in &mut self.tracked {
            t.win.key = self.db.resolve(&t.win.app_id, &t.win.title);
        }
        self.refresh();
        for s in &mut self.sprites {
            s.alpha = 1.0;
        }
    }

    pub fn run_menu_action(&mut self, action: MenuAction, serial: u32) {
        log::debug!("menu: {action:?}");
        let surface = self.bars.first().map(|b| b.layer.wl_surface().clone());
        match action {
            MenuAction::Activate(id) => self.activate(id),
            MenuAction::Launch(key) => self.launch(key, None, serial, surface),
            MenuAction::LaunchAction(key, a) => self.launch(key, Some(a), serial, surface),
            MenuAction::Close(ids) => self.close(&ids),
            MenuAction::Pin(key) => {
                let n = self.config.pinned.len();
                model::pin_at(&mut self.config.pinned, &key, n);
                self.save_config();
            }
            MenuAction::Unpin(key) => {
                model::unpin(&mut self.config.pinned, &key);
                self.save_config();
            }
            MenuAction::ToggleGroup => {
                self.config.group_windows = !self.config.group_windows;
                self.save_config();
            }
            MenuAction::ToggleBacklight => {
                self.config.backlight = match self.config.backlight {
                    Backlight::Always => Backlight::Running,
                    Backlight::Running => Backlight::Always,
                };
                self.save_config();
            }
            MenuAction::ToggleMinimize => {
                self.config.click_active = match self.config.click_active {
                    ClickActive::Minimize => ClickActive::Cycle,
                    ClickActive::Cycle => ClickActive::Minimize,
                };
                self.save_config();
            }
            MenuAction::ToggleButtons => {
                self.config.buttons = !self.config.buttons;
                self.save_config();
            }
            MenuAction::OpenSettings => {
                let path = crate::config::config_path().to_string_lossy().into_owned();
                launch::run(&["xdg-open".into(), path], "quai-settings", None, None);
            }
        }
        self.refresh();
    }

    // ---------------------------------------------------------- popups

    pub fn close_popup(&mut self) {
        if let Some(p) = self.popup.take() {
            if let Some(v) = p.viewport {
                v.destroy();
            }
            self.mark_dirty();
        }
    }

    pub fn close_tooltip(&mut self) {
        if matches!(self.popup.as_ref().map(|p| &p.kind), Some(PopupKind::Tooltip(_))) {
            self.close_popup();
        }
    }

    fn tooltip_text(&self, tile: &Tile) -> String {
        match &tile.id {
            TileId::Button(Button::Applications) => tr(Msg::Applications).to_string(),
            TileId::Button(Button::Workspaces) => tr(Msg::Workspaces).to_string(),
            TileId::Window(id) => self
                .tracked(*id)
                .map(|t| t.win.title.trim().to_string())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| self.name_of(&tile.key)),
            TileId::App(_) => self.name_of(&tile.key),
        }
    }

    fn name_of(&self, key: &str) -> String {
        self.sprites
            .iter()
            .find(|s| s.tile.key == key)
            .map(|s| s.app.name.clone())
            .unwrap_or_else(|| key.to_string())
    }

    fn show_tooltip(&mut self, bar: usize, hit: &Hit) {
        let text = self.tooltip_text(&hit.tile);
        if text.is_empty() {
            return;
        }
        log::debug!("tooltip: {text}");
        let size = render::tooltip_size(&text);
        self.open_popup(bar, PopupKind::Tooltip(text), Some(hit.tile.id.clone()), size, hit.y + TILE / 2.0, None);
    }

    fn window_label(&self, id: WinId) -> String {
        let title = self.tracked(id).map(|t| t.win.title.trim().to_string()).unwrap_or_default();
        let title = if title.is_empty() { tr(Msg::Untitled).to_string() } else { title };
        // Long titles are cut in the middle of nothing: keep the start.
        if title.chars().count() > 46 {
            format!("{}…", title.chars().take(45).collect::<String>().trim_end())
        } else {
            title
        }
    }

    fn menu_rows(&mut self, hit: Option<&Hit>) -> Vec<MenuRow<MenuAction>> {
        let entry = |label: String, action: MenuAction| MenuRow::Entry { label, action, check: None };
        let tile = hit.map(|h| &h.tile).filter(|t| !matches!(t.id, TileId::Button(_)));
        let Some(tile) = tile else {
            let check = |msg: Msg, on: bool, action: MenuAction| MenuRow::Entry {
                label: tr(msg).to_string(),
                action,
                check: Some(on),
            };
            return vec![
                MenuRow::Header(tr(Msg::Dock).to_string()),
                MenuRow::Separator,
                check(Msg::GroupWindows, self.config.group_windows, MenuAction::ToggleGroup),
                check(Msg::LightAll, self.config.backlight == Backlight::Always, MenuAction::ToggleBacklight),
                check(Msg::MinimizeOnClick, self.config.click_active == ClickActive::Minimize, MenuAction::ToggleMinimize),
                check(Msg::Buttons, self.config.buttons, MenuAction::ToggleButtons),
                MenuRow::Separator,
                entry(tr(Msg::OpenSettings).to_string(), MenuAction::OpenSettings),
            ];
        };

        let app = self.db.app(&tile.key);
        let key = tile.key.clone();
        let mut rows = vec![MenuRow::Header(app.name.clone())];
        if tile.wins.len() > 1 {
            rows.push(MenuRow::Separator);
            for id in &tile.wins {
                rows.push(MenuRow::Entry {
                    label: self.window_label(*id),
                    action: MenuAction::Activate(*id),
                    check: Some(self.tracked(*id).is_some_and(|t| t.win.active)),
                });
            }
        }
        let mut launchers = Vec::new();
        for a in &app.actions {
            launchers.push(entry(a.name.clone(), MenuAction::LaunchAction(key.clone(), a.id.clone())));
        }
        let has_new_window = app.actions.iter().any(|a| a.id.to_ascii_lowercase().contains("new-window") || a.id.eq_ignore_ascii_case("NewWindow"));
        if app.can_launch() && !tile.wins.is_empty() && !has_new_window {
            launchers.insert(0, entry(tr(Msg::NewWindow).to_string(), MenuAction::Launch(key.clone())));
        }
        if !launchers.is_empty() {
            rows.push(MenuRow::Separator);
            rows.append(&mut launchers);
        }
        rows.push(MenuRow::Separator);
        if self.config.pinned.contains(&key) {
            rows.push(entry(tr(Msg::Unpin).to_string(), MenuAction::Unpin(key.clone())));
        } else if app.can_launch() {
            rows.push(entry(tr(Msg::Pin).to_string(), MenuAction::Pin(key.clone())));
        }
        match tile.wins.len() {
            0 => {}
            1 => rows.push(entry(tr(Msg::Close).to_string(), MenuAction::Close(tile.wins.clone()))),
            n => rows.push(entry(close_all(n), MenuAction::Close(tile.wins.clone()))),
        }
        if matches!(rows.last(), Some(MenuRow::Separator)) {
            rows.pop();
        }
        rows
    }

    fn open_menu(&mut self, bar: usize, hit: Option<Hit>, y: f32, serial: u32) {
        let rows = self.menu_rows(hit.as_ref());
        let layout = render::layout_menu(&rows);
        let size = (layout.width, layout.height);
        let cy = hit.as_ref().map_or(y, |h| h.y + TILE / 2.0);
        let owner = hit.map(|h| h.tile.id);
        self.open_popup(bar, PopupKind::Menu { rows, layout, hover: None }, owner, size, cy, Some(serial));
    }

    fn open_popup(
        &mut self,
        bar: usize,
        kind: PopupKind,
        owner: Option<TileId>,
        size: (f32, f32),
        anchor_cy: f32,
        grab: Option<u32>,
    ) {
        self.close_popup();
        let Some(b) = self.bars.get(bar) else { return };
        let Ok(positioner) = XdgPositioner::new(&self.xdg_shell) else { return };
        let (w, h) = (size.0.ceil() as i32, size.1.ceil() as i32);
        // The anchor is the tile's row, across the whole dock: the popup
        // opens beside the dock, its arrow level with the tile.
        let y0 = (anchor_cy - TILE / 2.0).clamp(0.0, (b.height - 2.0).max(0.0));
        let y1 = (anchor_cy + TILE / 2.0).clamp(y0 + 1.0, b.height.max(y0 + 1.0));
        positioner.set_size(w.max(1), h.max(1));
        positioner.set_anchor_rect(0, y0 as i32, DOCK_W as i32, ((y1 - y0) as i32).max(1));
        positioner.set_anchor(PopAnchor::Right);
        positioner.set_gravity(Gravity::Right);
        positioner.set_offset(3, 0);
        positioner.set_constraint_adjustment(ConstraintAdjustment::SlideY);

        let surface = self.compositor.create_surface(&self.qh);
        let viewport = self.viewporter.as_ref().map(|v| v.get_viewport(&surface, &self.qh, ()));
        if grab.is_none()
            && let Ok(region) = Region::new(&self.compositor)
        {
            // A tooltip must never take the pointer from the dock.
            surface.set_input_region(Some(region.wl_region()));
        }
        let Ok(popup) = Popup::from_surface(None, &positioner, &self.qh, surface, &self.xdg_shell) else {
            return;
        };
        b.layer.get_popup(popup.xdg_popup());
        if let (Some(serial), Some(seat)) = (grab, &self.seat) {
            popup.xdg_popup().grab(seat, serial);
        }
        popup.wl_surface().commit();
        self.popup = Some(PopupState {
            popup,
            kind,
            owner,
            viewport,
            scale: b.scale,
            size: (w as f32, h as f32),
            anchor_cy,
            arrow_y: h as f32 / 2.0,
            configured: false,
            dirty: true,
            buffer: None,
        });
        self.mark_dirty();
    }

    pub fn popup_configured(&mut self, configure: &PopupConfigure) {
        log::debug!("popup at {:?}, {} x {}", configure.position, configure.width, configure.height);
        if let Some(p) = self.popup.as_mut() {
            p.arrow_y = p.anchor_cy - configure.position.1 as f32;
            p.configured = true;
            p.dirty = true;
        }
    }

    pub fn menu_motion(&mut self, x: f32, y: f32) {
        if let Some(p) = self.popup.as_mut()
            && let PopupKind::Menu { rows, layout, hover } = &mut p.kind
        {
            let row = layout.row_at(x, y).filter(|i| matches!(rows[*i], MenuRow::Entry { .. }));
            if row != *hover {
                *hover = row;
                p.dirty = true;
            }
        }
    }

    pub fn menu_leave(&mut self) {
        self.menu_motion(-1.0, -1.0);
    }

    pub fn menu_click(&mut self, x: f32, y: f32, serial: u32) {
        let action = self.popup.as_ref().and_then(|p| match &p.kind {
            PopupKind::Menu { rows, layout, .. } => match rows.get(layout.row_at(x, y)?)? {
                MenuRow::Entry { action, .. } => Some(action.clone()),
                _ => None,
            },
            PopupKind::Tooltip(_) => None,
        });
        if let Some(a) = action {
            self.close_popup();
            self.run_menu_action(a, serial);
        }
    }

    // --------------------------------------------------------- drawing

    /// Draws whatever changed; called after each batch of events.
    pub fn flush(&mut self) {
        let ready = |b: &Bar| b.configured && b.dirty && !b.frame_pending;
        if self.bars.iter().any(ready) {
            let moving = self.tick();
            for i in 0..self.bars.len() {
                if ready(&self.bars[i]) {
                    self.draw_bar(i, moving);
                }
            }
        }
        if self.popup.as_ref().is_some_and(|p| p.configured && p.dirty) {
            self.draw_popup();
        }
    }

    fn draw_bar(&mut self, i: usize, animating: bool) {
        let scene = self.scene(i);
        let Some(pix) = render::render_dock(&scene) else { return };
        let (w, h) = (pix.width() as i32, pix.height() as i32);
        let bar = &mut self.bars[i];
        let reusable = bar
            .buffer
            .as_ref()
            .is_some_and(|b| b.height() == h && b.stride() == w * 4 && b.canvas(&mut self.pool).is_some());
        if !reusable {
            bar.buffer = self.pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888).ok().map(|(b, _)| b);
        }
        let Some(buffer) = bar.buffer.as_ref() else { return };
        let Some(canvas) = buffer.canvas(&mut self.pool) else { return };
        render::copy_to_shm(&pix, canvas);

        let surface = bar.layer.wl_surface();
        if buffer.attach_to(surface).is_err() {
            return;
        }
        surface.damage_buffer(0, 0, w, h);
        match &bar.viewport {
            Some(v) => v.set_destination(DOCK_W as i32, (bar.height as i32).max(1)),
            None => surface.set_buffer_scale((bar.scale.round() as i32).max(1)),
        }
        surface.frame(&self.qh, surface.clone());
        bar.frame_pending = true;
        bar.dirty = animating;
        bar.layer.commit();
        if !bar.settled {
            Self::settle(bar);
        }
    }

    /// COSMIC fits maximized windows to the room left by the panels only
    /// when a panel changes size, not when one arrives: without this they
    /// would stay spread under the dock. So the dock arrives one pixel
    /// short, and takes its width once it is on screen.
    fn settle(bar: &mut Bar) {
        bar.layer.set_size(DOCK_W as u32, 0);
        bar.layer.commit();
        bar.settled = true;
    }

    fn draw_popup(&mut self) {
        let Some(p) = self.popup.as_mut() else { return };
        let s = if p.viewport.is_some() { p.scale } else { p.scale.round().max(1.0) };
        let (w, h) = p.size;
        let pix = match &p.kind {
            PopupKind::Tooltip(text) => render::render_tooltip(text, w, h, p.arrow_y, s),
            PopupKind::Menu { rows, layout, hover } => render::render_menu(rows, layout, *hover, p.arrow_y, s),
        };
        let Some(pix) = pix else { return };
        let (bw, bh) = (pix.width() as i32, pix.height() as i32);
        let reusable = p
            .buffer
            .as_ref()
            .is_some_and(|b| b.height() == bh && b.stride() == bw * 4 && b.canvas(&mut self.pool).is_some());
        if !reusable {
            p.buffer = self.pool.create_buffer(bw, bh, bw * 4, wl_shm::Format::Argb8888).ok().map(|(b, _)| b);
        }
        let Some(buffer) = p.buffer.as_ref() else { return };
        let Some(canvas) = buffer.canvas(&mut self.pool) else { return };
        render::copy_to_shm(&pix, canvas);
        let surface = p.popup.wl_surface();
        if buffer.attach_to(surface).is_err() {
            return;
        }
        surface.damage_buffer(0, 0, bw, bh);
        match &p.viewport {
            Some(v) => v.set_destination(w as i32, h as i32),
            None => surface.set_buffer_scale(s as i32),
        }
        p.popup.xdg_surface().set_window_geometry(0, 0, w as i32, h as i32);
        surface.commit();
        p.dirty = false;
    }
}

fn canonicalize_pinned(config: &mut Config, db: &mut AppDb) {
    let mut out: Vec<String> = Vec::new();
    for id in &config.pinned {
        let key = db.canonical(id);
        if !out.contains(&key) {
            out.push(key);
        }
    }
    config.pinned = out;
}

pub fn button_key(b: Button) -> &'static str {
    match b {
        Button::Applications => "button:applications",
        Button::Workspaces => "button:workspaces",
    }
}

fn button_from_key(key: &str) -> Option<Button> {
    match key {
        "button:applications" => Some(Button::Applications),
        "button:workspaces" => Some(Button::Workspaces),
        _ => None,
    }
}

fn button_entry(b: Button) -> (&'static str, &'static str) {
    match b {
        Button::Applications => ("com.system76.CosmicAppLibrary", tr(Msg::Applications)),
        Button::Workspaces => ("com.system76.CosmicWorkspaces", tr(Msg::Workspaces)),
    }
}

fn button_command(b: Button) -> &'static str {
    match b {
        Button::Applications => "cosmic-app-library",
        Button::Workspaces => "cosmic-workspaces",
    }
}
