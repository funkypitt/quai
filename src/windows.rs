//! `quai --windows`: lists the open windows as the compositor describes
//! them, with the application each one is matched to.

use anyhow::{Context, Result};
use cosmic_client_toolkit::{
    cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1::State,
    sctk::{
        self,
        output::{OutputHandler, OutputState},
        registry::{ProvidesRegistryState, RegistryState},
    },
    toplevel_info::{ToplevelInfoHandler, ToplevelInfoState},
};
use wayland_client::{Connection, QueueHandle, globals::registry_queue_init, protocol::wl_output};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;

use crate::apps::AppDb;

struct Lister {
    registry: RegistryState,
    output: OutputState,
    toplevels: ToplevelInfoState,
}

pub fn run() -> Result<()> {
    let conn = Connection::connect_to_env().context("no Wayland session")?;
    let (globals, mut queue) = registry_queue_init::<Lister>(&conn)?;
    let qh = queue.handle();
    let registry = RegistryState::new(&globals);
    let toplevels = ToplevelInfoState::try_new(&registry, &qh)
        .context("this compositor does not list its windows")?;
    let mut lister = Lister { output: OutputState::new(&globals, &qh), toplevels, registry };
    // The compositor describes windows at its own pace, not on request:
    // listen for a moment rather than ask and leave.
    let end = std::time::Instant::now() + std::time::Duration::from_millis(700);
    while std::time::Instant::now() < end {
        queue.flush()?;
        if let Some(guard) = queue.prepare_read() {
            let fd = guard.connection_fd();
            let mut fds = [rustix::event::PollFd::new(&fd, rustix::event::PollFlags::IN)];
            let timeout = rustix::event::Timespec { tv_sec: 0, tv_nsec: 100_000_000 };
            if rustix::event::poll(&mut fds, Some(&timeout)).is_ok_and(|n| n > 0) {
                let _ = guard.read();
            }
        }
        queue.dispatch_pending(&mut lister)?;
    }

    let mut db = AppDb::new("");
    for info in lister.toplevels.toplevels() {
        let key = db.resolve(&info.app_id, &info.title);
        let app = db.app(&key);
        let mut flags: Vec<&str> = Vec::new();
        for (state, name) in [
            (State::Activated, "active"),
            (State::Maximized, "maximized"),
            (State::Minimized, "minimized"),
            (State::Fullscreen, "fullscreen"),
            (State::Sticky, "sticky"),
        ] {
            if info.state.contains(&state) {
                flags.push(name);
            }
        }
        let place = info
            .geometry
            .values()
            .next()
            .map(|g| format!("{}x{} at {},{}", g.width, g.height, g.x, g.y))
            .unwrap_or_default();
        let title: String = info.title.chars().take(40).collect();
        println!(
            "{:<34} → {:<34} {:<22} [{}] {}",
            info.app_id,
            if app.entry.is_some() { key } else { format!("({key})") },
            place,
            flags.join(", "),
            title
        );
    }
    Ok(())
}

impl ToplevelInfoHandler for Lister {
    fn toplevel_info_state(&mut self) -> &mut ToplevelInfoState {
        &mut self.toplevels
    }
    fn new_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &ExtForeignToplevelHandleV1) {}
    fn update_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &ExtForeignToplevelHandleV1) {}
    fn toplevel_closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &ExtForeignToplevelHandleV1) {}
}

impl OutputHandler for Lister {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ProvidesRegistryState for Lister {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    sctk::registry_handlers![OutputState];
}

sctk::delegate_output!(Lister);
sctk::delegate_registry!(Lister);
cosmic_client_toolkit::delegate_toplevel_info!(Lister);
