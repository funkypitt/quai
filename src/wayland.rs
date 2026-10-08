//! Wayland plumbing: protocol handlers and the event loop.

use anyhow::{Context, Result};
use calloop::{
    EventLoop,
    channel::{Event as ChannelEvent, channel},
    timer::{TimeoutAction, Timer},
};
use cosmic_client_toolkit::{
    cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1,
    sctk::{
        self,
        activation::{ActivationHandler, ActivationState},
        compositor::{CompositorHandler, CompositorState},
        output::{OutputHandler, OutputState},
        reexports::calloop_wayland_source::WaylandSource,
        registry::{ProvidesRegistryState, RegistryState},
        seat::{
            Capability, SeatHandler, SeatState,
            pointer::{CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec},
            touch::TouchHandler,
        },
        shell::{
            WaylandSurface,
            wlr_layer::{LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
            xdg::{
                XdgShell,
                popup::{Popup, PopupConfigure, PopupHandler},
                window::{Window, WindowConfigure, WindowHandler},
            },
        },
        shm::{Shm, ShmHandler, slot::SlotPool},
    },
    toplevel_info::{ToplevelInfoHandler, ToplevelInfoState},
    toplevel_management::{ToplevelManagerHandler, ToplevelManagerState},
};
use std::time::Duration;
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_surface, wl_touch},
};
use wayland_protocols::{
    ext::{
        background_effect::v1::client::{
            ext_background_effect_manager_v1::{self, ExtBackgroundEffectManagerV1},
            ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
        },
        foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    },
    wp::{
        fractional_scale::v1::client::{
            wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
            wp_fractional_scale_v1::{self, WpFractionalScaleV1},
        },
        viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
    },
};

use crate::{
    config::{Config, config_dir},
    dock::{BTN_LEFT, Dock, LaunchData, Notice, PopupKind},
    wallpaper,
    render::{COL_W, COLUMNS, PITCH},
};


pub fn run(config: Config) -> Result<()> {
    let conn = Connection::connect_to_env().context("no Wayland session")?;
    let (globals, event_queue) = registry_queue_init::<Dock>(&conn)?;
    let qh = event_queue.handle();
    let mut event_loop: EventLoop<Dock> = EventLoop::try_new()?;
    let loop_handle = event_loop.handle();

    let registry_state = RegistryState::new(&globals);
    let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor is missing")?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .context("this compositor has no layer shell, the dock cannot be placed")?;
    let xdg_shell = XdgShell::bind(&globals, &qh).context("xdg_wm_base is missing")?;
    let shm = Shm::bind(&globals, &qh).context("wl_shm is missing")?;
    let toplevel_info = ToplevelInfoState::try_new(&registry_state, &qh)
        .context("this compositor does not list its windows (ext_foreign_toplevel_list_v1)")?;
    let toplevel_manager = ToplevelManagerState::try_new(&registry_state, &qh);
    if toplevel_manager.is_none() {
        log::warn!("no window management protocol: the dock can launch, not switch");
    }
    let pool = SlotPool::new(512 * 1024, &shm)?;

    let mut dock = Dock::new(
        qh.clone(),
        loop_handle.clone(),
        registry_state,
        SeatState::new(&globals, &qh),
        OutputState::new(&globals, &qh),
        compositor,
        shm,
        layer_shell,
        xdg_shell,
        ActivationState::bind(&globals, &qh).ok(),
        toplevel_info,
        toplevel_manager,
        globals.bind::<WpViewporter, _, _>(&qh, 1..=1, ()).ok(),
        globals.bind::<WpFractionalScaleManagerV1, _, _>(&qh, 1..=1, ()).ok(),
        pool,
        config,
    );
    dock.blur_manager = globals.bind::<ExtBackgroundEffectManagerV1, _, _>(&qh, 1..=1, ()).ok();

    WaylandSource::new(conn, event_queue)
        .insert(loop_handle.clone())
        .map_err(|e| anyhow::anyhow!("cannot watch the Wayland socket: {e}"))?;

    // Settings and desktop files are followed as they change.
    let (tx, rx) = channel::<Notice>();
    dock.notices = Some(tx.clone());
    let cfg_dir = config_dir();
    let cfg_dir_for_events = cfg_dir.clone();
    let wallpaper_dir = wallpaper::state_dir();
    let wallpaper_dir_for_events = wallpaper_dir.clone();
    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(ev) = res else { return };
        if matches!(ev.kind, notify::EventKind::Access(_)) {
            return;
        }
        let about = |dir: &std::path::Path| ev.paths.iter().any(|p| p.starts_with(dir));
        let _ = tx.send(if about(&cfg_dir_for_events) {
            Notice::Config
        } else if about(&wallpaper_dir_for_events) {
            Notice::Wallpaper
        } else {
            Notice::Apps
        });
    });
    match watcher {
        Ok(mut w) => {
            use notify::{RecursiveMode::NonRecursive, Watcher};
            let _ = std::fs::create_dir_all(&cfg_dir);
            let _ = w.watch(&cfg_dir, NonRecursive);
            if wallpaper_dir.is_dir() {
                let _ = w.watch(&wallpaper_dir, NonRecursive);
            }
            for dir in dock.db.application_dirs() {
                if dir.is_dir() {
                    let _ = w.watch(&dir, NonRecursive);
                }
            }
            dock.watcher = Some(w);
        }
        Err(e) => log::warn!("changes to files will not be followed: {e}"),
    }
    loop_handle
        .insert_source(rx, |event, _, dock| match event {
            ChannelEvent::Msg(Notice::Config) => dock.reload_config(),
            ChannelEvent::Msg(Notice::Wallpaper) => dock.update_tint(),
            ChannelEvent::Msg(Notice::Tint(tint)) => dock.set_tint(tint),
            ChannelEvent::Msg(Notice::Apps) => {
                // Package managers write many files at once: wait for the end.
                if !dock.reload_pending {
                    dock.reload_pending = true;
                    let timer = Timer::from_duration(Duration::from_millis(1500));
                    let _ = dock.loop_handle.insert_source(timer, |_, _, dock| {
                        dock.reload_pending = false;
                        dock.reload_apps();
                        TimeoutAction::Drop
                    });
                }
            }
            ChannelEvent::Closed => {}
        })
        .map_err(|e| anyhow::anyhow!("cannot follow file changes: {e}"))?;

    while !dock.exit {
        dock.flush();
        event_loop.dispatch(None, &mut dock)?;
    }
    Ok(())
}

// -------------------------------------------------------------- outputs

impl OutputHandler for Dock {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.add_bar(output);
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        // The output's name may only be known now.
        self.sync_bars();
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if let Some(i) = self.bars.iter().position(|b| b.output == output) {
            self.remove_bar(i);
        }
    }
}

impl CompositorHandler for Dock {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, factor: i32) {
        // Without fractional scaling, the whole factor is all there is.
        if self.fractional.is_none() || self.viewporter.is_none() {
            self.set_scale(surface, factor as f32);
        }
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _: u32) {
        if let Some(i) = self.bar_of(surface) {
            self.bars[i].frame_pending = false;
        }
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl LayerShellHandler for Dock {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(i) = self.bar_of(layer.wl_surface()) {
            self.remove_bar(i);
        }
    }
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, configure: LayerSurfaceConfigure, _: u32) {
        if let Some(i) = self.probe_of(layer.wl_surface()) {
            self.probe_configured(i, configure.new_size.0, configure.new_size.1);
            return;
        }
        let Some(i) = self.bar_of(layer.wl_surface()) else { return };
        let height = configure.new_size.1 as f32;
        let bar = &mut self.bars[i];
        if height > 0.0 && (bar.height - height).abs() > 0.5 {
            bar.height = height;
            bar.buffer = None;
        }
        bar.configured = bar.height > 0.0;
        bar.dirty = true;
        self.apply_blur(i);
        self.refresh();
    }
}

impl PopupHandler for Dock {
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, popup: &Popup, config: PopupConfigure) {
        if self.popup.as_ref().is_some_and(|p| &p.popup == popup) {
            self.popup_configured(&config);
        }
    }
    fn done(&mut self, _: &Connection, _: &QueueHandle<Self>, popup: &Popup) {
        if self.popup.as_ref().is_some_and(|p| &p.popup == popup) {
            self.close_popup();
        }
    }
}

/// The dock opens no window of its own; the toolkit only asks for this
/// handler because window decorations share the xdg shell's plumbing.
impl WindowHandler for Dock {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {}
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window, _: WindowConfigure, _: u32) {}
}

// ----------------------------------------------------------------- seat

impl SeatHandler for Dock {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        match capability {
            Capability::Pointer if self.themed_pointer.is_none() => {
                let cursor_surface = self.compositor.create_surface(qh);
                match self.seat_state.get_pointer_with_theme(qh, &seat, self.shm.wl_shm(), cursor_surface, ThemeSpec::default()) {
                    Ok(p) => self.themed_pointer = Some(p),
                    Err(e) => log::error!("no pointer: {e}"),
                }
            }
            // a touchscreen: fingers go the pointer's way (dock.rs, touch_*)
            Capability::Touch if self.touch_dev.is_none() => match self.seat_state.get_touch(qh, &seat) {
                Ok(t) => {
                    self.touch_dev = Some(t);
                    log::info!("touch input available");
                }
                Err(e) => log::error!("no touch: {e}"),
            },
            _ => {}
        }
        if self.seat.is_none() {
            self.seat = Some(seat);
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if self.seat.as_ref() != Some(&seat) {
            return;
        }
        if capability == Capability::Pointer {
            if let Some(p) = self.themed_pointer.take() {
                p.pointer().release();
            }
            self.pointer = Default::default();
            self.drag = None;
        }
        if capability == Capability::Touch {
            if let Some(t) = self.touch_dev.take() {
                t.release();
            }
            self.touch_cancel();
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if self.seat.as_ref() == Some(&seat) {
            self.seat = None;
            self.themed_pointer = None;
        }
    }
}

impl PointerHandler for Dock {
    fn pointer_frame(&mut self, conn: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for event in events {
            let (x, y) = (event.position.0 as f32, event.position.1 as f32);
            if matches!(event.kind, PointerEventKind::Enter { .. })
                && let Some(p) = &self.themed_pointer
            {
                let _ = p.set_cursor(conn, CursorIcon::Default);
            }

            let on_popup = self.popup.as_ref().is_some_and(|p| p.popup.wl_surface() == &event.surface);
            if on_popup {
                let is_menu = matches!(self.popup.as_ref().map(|p| &p.kind), Some(PopupKind::Menu { .. }));
                if !is_menu {
                    continue;
                }
                match event.kind {
                    PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => self.menu_motion(x, y),
                    PointerEventKind::Leave { .. } => self.menu_leave(),
                    PointerEventKind::Release { button: BTN_LEFT, serial, .. } => self.menu_click(x, y, serial),
                    _ => {}
                }
                continue;
            }

            let Some(bar) = self.bar_of(&event.surface) else { continue };
            match event.kind {
                PointerEventKind::Enter { .. } => self.pointer_enter(bar, x, y),
                PointerEventKind::Leave { .. } => self.pointer_leave(),
                PointerEventKind::Motion { .. } => self.pointer_motion(bar, x, y),
                PointerEventKind::Press { button, serial, .. } => self.pointer_press(bar, x, y, button, serial),
                PointerEventKind::Release { button, serial, .. } => self.pointer_release(bar, x, y, button, serial),
                PointerEventKind::Axis { vertical, .. } => {
                    let delta = if vertical.value120 != 0 {
                        vertical.value120 as f32 / 120.0 * PITCH
                    } else if vertical.discrete != 0 {
                        vertical.discrete as f32 * PITCH
                    } else {
                        vertical.absolute as f32
                    };
                    let col = ((x / COL_W) as usize).min(COLUMNS - 1);
                    self.scroll_by(bar, col, delta);
                }
            }
        }
    }
}

impl TouchHandler for Dock {
    fn down(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch, serial: u32, _: u32, surface: wl_surface::WlSurface, id: i32, position: (f64, f64)) {
        self.touch_down(id, &surface, position.0 as f32, position.1 as f32, serial);
    }
    fn up(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch, serial: u32, _: u32, id: i32) {
        self.touch_up(id, serial);
    }
    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch, _: u32, id: i32, position: (f64, f64)) {
        self.touch_motion(id, position.0 as f32, position.1 as f32);
    }
    fn shape(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch, _: i32, _: f64, _: f64) {}
    fn orientation(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch, _: i32, _: f64) {}
    fn cancel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch) {
        self.touch_cancel();
    }
}

impl ShmHandler for Dock {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Dock {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    sctk::registry_handlers![OutputState, SeatState];
}

// -------------------------------------------------------------- windows

impl ToplevelInfoHandler for Dock {
    fn toplevel_info_state(&mut self) -> &mut ToplevelInfoState {
        &mut self.toplevel_info
    }
    fn new_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, toplevel: &ExtForeignToplevelHandleV1) {
        if let Some(info) = self.toplevel_info.info(toplevel).cloned() {
            self.upsert_toplevel(&info);
        }
    }
    fn update_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, toplevel: &ExtForeignToplevelHandleV1) {
        if let Some(info) = self.toplevel_info.info(toplevel).cloned() {
            self.upsert_toplevel(&info);
        }
    }
    fn toplevel_closed(&mut self, _: &Connection, _: &QueueHandle<Self>, toplevel: &ExtForeignToplevelHandleV1) {
        self.remove_toplevel(toplevel);
    }
}

impl ToplevelManagerHandler for Dock {
    fn toplevel_manager_state(&mut self) -> &mut ToplevelManagerState {
        self.toplevel_manager.as_mut().expect("bound before any of its events")
    }
    fn capabilities(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: Vec<WEnum<zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1>>,
    ) {
    }
}

impl ActivationHandler for Dock {
    type RequestData = LaunchData;
    fn new_token(&mut self, token: String, data: &LaunchData) {
        self.spawn(data, Some(&token));
    }
}

// ------------------------------------------------------------- scaling

impl Dispatch<WpViewporter, ()> for Dock {
    fn event(_: &mut Self, _: &WpViewporter, _: <WpViewporter as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpViewport, ()> for Dock {
    fn event(_: &mut Self, _: &WpViewport, _: <WpViewport as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpFractionalScaleManagerV1, ()> for Dock {
    fn event(_: &mut Self, _: &WpFractionalScaleManagerV1, _: <WpFractionalScaleManagerV1 as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpFractionalScaleV1, wl_surface::WlSurface> for Dock {
    fn event(dock: &mut Self, _: &WpFractionalScaleV1, event: wp_fractional_scale_v1::Event, surface: &wl_surface::WlSurface, _: &Connection, _: &QueueHandle<Self>) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            dock.set_scale(surface, scale as f32 / 120.0);
        }
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for Dock {
    fn event(dock: &mut Self, _: &ExtBackgroundEffectManagerV1, event: ext_background_effect_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let ext_background_effect_manager_v1::Event::Capabilities { flags } = event {
            let blur = matches!(flags, WEnum::Value(f) if f.contains(ext_background_effect_manager_v1::Capability::Blur));
            if blur != dock.blur_supported {
                dock.blur_supported = blur;
                for i in 0..dock.bars.len() {
                    dock.apply_blur(i);
                    dock.bars[i].dirty = true;
                }
            }
        }
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, ()> for Dock {
    fn event(_: &mut Self, _: &ExtBackgroundEffectSurfaceV1, _: <ExtBackgroundEffectSurfaceV1 as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

sctk::delegate_compositor!(Dock);
sctk::delegate_output!(Dock);
sctk::delegate_shm!(Dock);
sctk::delegate_seat!(Dock);
sctk::delegate_pointer!(Dock);
sctk::delegate_touch!(Dock);
sctk::delegate_layer!(Dock);
sctk::delegate_xdg_shell!(Dock);
sctk::delegate_xdg_popup!(Dock);
sctk::delegate_registry!(Dock);
sctk::delegate_activation!(Dock, LaunchData);
cosmic_client_toolkit::delegate_toplevel_info!(Dock);
cosmic_client_toolkit::delegate_toplevel_manager!(Dock);
