//! A stand-in for a desktop panel, for testing only: a strip along the top
//! of the screen that reserves its place, as a panel arriving after the dock
//! would. Run with `quai --test-panel SECONDS`.

use anyhow::{Context, Result};
use cosmic_client_toolkit::sctk::{
    self,
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use std::time::{Duration, Instant};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
};

const HEIGHT: u32 = 40;

struct Panel {
    registry: RegistryState,
    output: OutputState,
    shm: Shm,
    pool: SlotPool,
    /// Kept so that the surface lives as long as the panel.
    _layer: LayerSurface,
    closed: bool,
}

pub fn run(seconds: u64) -> Result<()> {
    let conn = Connection::connect_to_env().context("no Wayland session")?;
    let (globals, mut queue) = registry_queue_init::<Panel>(&conn)?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Top, Some("quai-test-panel"), None);
    layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
    layer.set_size(0, HEIGHT);
    layer.set_exclusive_zone(HEIGHT as i32);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.commit();
    let mut panel = Panel {
        registry: RegistryState::new(&globals),
        output: OutputState::new(&globals, &qh),
        pool: SlotPool::new(64 * 1024, &shm)?,
        shm,
        _layer: layer,
        closed: false,
    };
    let end = Instant::now() + Duration::from_secs(seconds);
    while !panel.closed && Instant::now() < end {
        queue.flush()?;
        if let Some(guard) = queue.prepare_read() {
            let fd = guard.connection_fd();
            let mut fds = [rustix::event::PollFd::new(&fd, rustix::event::PollFlags::IN)];
            let timeout = rustix::event::Timespec { tv_sec: 0, tv_nsec: 200_000_000 };
            if rustix::event::poll(&mut fds, Some(&timeout)).is_ok_and(|n| n > 0) {
                let _ = guard.read();
            }
        }
        queue.dispatch_pending(&mut panel)?;
    }
    Ok(())
}

impl LayerShellHandler for Panel {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.closed = true;
    }
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, configure: LayerSurfaceConfigure, _: u32) {
        let (w, h) = (configure.new_size.0.max(1) as i32, configure.new_size.1.max(1) as i32);
        println!("test panel: {w} x {h}");
        let Ok((buffer, canvas)) = self.pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) else { return };
        for px in canvas.chunks_exact_mut(4) {
            px.copy_from_slice(&[40, 40, 200, 230]);
        }
        let surface = layer.wl_surface();
        let _ = buffer.attach_to(surface);
        surface.damage_buffer(0, 0, w, h);
        layer.commit();
    }
}

impl CompositorHandler for Panel {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Panel {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for Panel {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Panel {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    sctk::registry_handlers![OutputState];
}

sctk::delegate_compositor!(Panel);
sctk::delegate_output!(Panel);
sctk::delegate_shm!(Panel);
sctk::delegate_layer!(Panel);
sctk::delegate_registry!(Panel);
