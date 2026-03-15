use crate::dwl_ipc::zdwl_ipc_manager_v2::{Event as ManagerEvent, ZdwlIpcManagerV2};
use crate::dwl_ipc::zdwl_ipc_output_v2::{Event as OutputEvent, ZdwlIpcOutputV2};
use crate::event::HeadInfo;
use crate::event::{ToplevelInfo, WaylandEvent, WaylandRequest, WorkspaceInfo, WorkspacePending};
use std::collections::HashMap;
use std::os::unix::io::AsFd;
use std::sync::mpsc::{Receiver, Sender};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_registry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_group_handle_v1::{self, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1},
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::ZwlrOutputConfigurationV1,
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};
use wayland_protocols_wlr::output_power_management::v1::client::{
    zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1,
    zwlr_output_power_v1::{self, ZwlrOutputPowerV1},
};

#[derive(Default)]
struct ToplevelPending {
    title: Option<String>,
    app_id: Option<String>,
    states: Vec<u8>,
}

#[derive(Default)]
struct HeadPending {
    name: Option<String>,
    description: Option<String>,
    width_mm: Option<i32>,
    height_mm: Option<i32>,
    x: Option<i32>,
    y: Option<i32>,
    enabled: Option<bool>,
    current_mode_id: Option<u32>,
    scale: Option<f64>,
    transform: Option<u32>,
}

struct LayerSurfaceState {
    surface: WlSurface,
    layer_surface: ZwlrLayerSurfaceV1,
    configured: bool,
    width: u32,
    height: u32,
    shm_path: Option<String>,
}

struct MplugState {
    manager: Option<ZdwlIpcManagerV2>,
    outputs: Vec<WlOutput>,
    ipc_outputs: Vec<ZdwlIpcOutputV2>,
    event_tx: Sender<WaylandEvent>,
    toplevel_manager: Option<ZwlrForeignToplevelManagerV1>,
    toplevels: HashMap<u32, ToplevelInfo>,
    toplevel_pending: HashMap<u32, ToplevelPending>,
    seat: Option<WlSeat>,
    idle_notifier: Option<ExtIdleNotifierV1>,
    idle_notification: Option<ExtIdleNotificationV1>,
    output_power_manager: Option<ZwlrOutputPowerManagerV1>,
    output_powers: Vec<ZwlrOutputPowerV1>,
    workspace_manager: Option<ExtWorkspaceManagerV1>,
    workspaces: HashMap<u32, WorkspaceInfo>,
    workspace_pending: HashMap<u32, WorkspacePending>,
    output_manager: Option<ZwlrOutputManagerV1>,
    heads: HashMap<u32, ZwlrOutputHeadV1>,
    head_names: HashMap<String, u32>,
    head_states: HashMap<u32, HeadInfo>,
    head_pending: HashMap<u32, HeadPending>,
    modes: HashMap<u32, (i32, i32, i32)>,
    manager_serial: u32,
    compositor: Option<WlCompositor>,
    wl_shm: Option<WlShm>,
    layer_shell: Option<ZwlrLayerShellV1>,
    layer_surfaces: HashMap<u32, LayerSurfaceState>,
    toplevel_handles: HashMap<u32, ZwlrForeignToplevelHandleV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for MplugState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &wayland_client::Connection,
        qh: &QueueHandle<MplugState>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == "zdwl_ipc_manager_v2" {
                let manager: ZdwlIpcManagerV2 = registry.bind(name, 1, qh, ());
                for output in &state.outputs {
                    let ipc_out = manager.get_output(output, qh, ());
                    state.ipc_outputs.push(ipc_out);
                }
                state.manager = Some(manager);
            } else if interface == "wl_output" {
                let output: WlOutput = registry.bind(name, std::cmp::min(version, 4), qh, ());
                if let Some(manager) = &state.manager {
                    let ipc_out = manager.get_output(&output, qh, ());
                    state.ipc_outputs.push(ipc_out);
                }
                if let Some(mgr) = &state.output_power_manager {
                    let op = mgr.get_output_power(&output, qh, ());
                    state.output_powers.push(op);
                }
                state.outputs.push(output);
            } else if interface == "zwlr_foreign_toplevel_manager_v1" {
                let mgr: ZwlrForeignToplevelManagerV1 = registry.bind(name, 3, qh, ());
                state.toplevel_manager = Some(mgr);
            } else if interface == "wl_seat" {
                let seat: WlSeat = registry.bind(name, std::cmp::min(version, 7), qh, ());
                if let Some(notifier) = &state.idle_notifier {
                    let notification = notifier.get_idle_notification(300_000, &seat, qh, ());
                    state.idle_notification = Some(notification);
                }
                state.seat = Some(seat);
            } else if interface == "ext_idle_notifier_v1" {
                let notifier: ExtIdleNotifierV1 = registry.bind(name, 1, qh, ());
                if let Some(seat) = &state.seat {
                    let notification = notifier.get_idle_notification(300_000, seat, qh, ());
                    state.idle_notification = Some(notification);
                }
                state.idle_notifier = Some(notifier);
            } else if interface == "zwlr_output_power_manager_v1" {
                let mgr: ZwlrOutputPowerManagerV1 = registry.bind(name, 1, qh, ());
                for output in &state.outputs {
                    let op = mgr.get_output_power(output, qh, ());
                    state.output_powers.push(op);
                }
                state.output_power_manager = Some(mgr);
            } else if interface == "ext_workspace_manager_v1" {
                let mgr: ExtWorkspaceManagerV1 = registry.bind(name, 1, qh, ());
                state.workspace_manager = Some(mgr);
            } else if interface == "zwlr_output_manager_v1" {
                let mgr: ZwlrOutputManagerV1 =
                    registry.bind(name, std::cmp::min(version, 4), qh, ());
                state.output_manager = Some(mgr);
            } else if interface == "wl_compositor" {
                let comp: WlCompositor = registry.bind(name, std::cmp::min(version, 5), qh, ());
                state.compositor = Some(comp);
            } else if interface == "wl_shm" {
                let shm: WlShm = registry.bind(name, 1, qh, ());
                state.wl_shm = Some(shm);
            } else if interface == "zwlr_layer_shell_v1" {
                let ls: ZwlrLayerShellV1 = registry.bind(name, std::cmp::min(version, 4), qh, ());
                state.layer_shell = Some(ls);
            }
        }
    }
}

impl Dispatch<WlOutput, ()> for MplugState {
    fn event(
        _state: &mut Self,
        _output: &WlOutput,
        _event: <WlOutput as wayland_client::Proxy>::Event,
        _: &(),
        _: &wayland_client::Connection,
        _qh: &QueueHandle<MplugState>,
    ) {
    }
}

impl Dispatch<ZdwlIpcManagerV2, ()> for MplugState {
    fn event(
        _state: &mut Self,
        _manager: &ZdwlIpcManagerV2,
        event: <ZdwlIpcManagerV2 as wayland_client::Proxy>::Event,
        _: &(),
        _: &wayland_client::Connection,
        _qh: &QueueHandle<MplugState>,
    ) {
        match event {
            ManagerEvent::Tags { amount } => {
                let _ = _state.event_tx.send(WaylandEvent::TagsAmount(amount));
            }
            ManagerEvent::Layout { name } => {
                let _ = _state.event_tx.send(WaylandEvent::LayoutName(name));
            }
        }
    }
}

impl Dispatch<ZdwlIpcOutputV2, ()> for MplugState {
    fn event(
        _state: &mut Self,
        _output: &ZdwlIpcOutputV2,
        event: <ZdwlIpcOutputV2 as wayland_client::Proxy>::Event,
        _: &(),
        _: &wayland_client::Connection,
        _qh: &QueueHandle<MplugState>,
    ) {
        match event {
            OutputEvent::Tag {
                tag,
                state,
                clients,
                focused,
            } => {
                let _ = _state.event_tx.send(WaylandEvent::OutputTag {
                    tag,
                    state: match state {
                        wayland_client::WEnum::Value(v) => v as u32,
                        _ => 0,
                    },
                    clients,
                    focused,
                });
            }
            OutputEvent::Layout { layout } => {
                let _ = _state.event_tx.send(WaylandEvent::OutputLayout(layout));
            }
            OutputEvent::Title { title } => {
                let _ = _state.event_tx.send(WaylandEvent::OutputTitle(title));
            }
            OutputEvent::Appid { appid } => {
                let _ = _state.event_tx.send(WaylandEvent::OutputAppid(appid));
            }
            OutputEvent::LayoutSymbol { layout } => {
                let _ = _state
                    .event_tx
                    .send(WaylandEvent::OutputLayoutSymbol(layout));
            }
            OutputEvent::Active { active } => {
                let _ = _state.event_tx.send(WaylandEvent::OutputActive(active));
            }
            OutputEvent::Frame => {
                let _ = _state.event_tx.send(WaylandEvent::OutputFrame);
            }
            OutputEvent::ToggleVisibility => {
                let _ = _state.event_tx.send(WaylandEvent::OutputToggleVisibility);
            }
        }
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for MplugState {
    fn event(
        state: &mut Self,
        _manager: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &wayland_client::Connection,
        _qh: &QueueHandle<MplugState>,
    ) {
        match event {
            zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } => {
                let id = toplevel.id().protocol_id();
                state
                    .toplevel_pending
                    .insert(id, ToplevelPending::default());
                state.toplevel_handles.insert(id, toplevel);
            }
            zwlr_foreign_toplevel_manager_v1::Event::Finished => {
                state.toplevel_manager = None;
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(
        MplugState,
        ZwlrForeignToplevelManagerV1,
        [
            zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE
                => (ZwlrForeignToplevelHandleV1, ())
        ]
    );
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for MplugState {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &wayland_client::Connection,
        _qh: &QueueHandle<MplugState>,
    ) {
        let id = handle.id().protocol_id();
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                if let Some(p) = state.toplevel_pending.get_mut(&id) {
                    p.title = Some(title);
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some(p) = state.toplevel_pending.get_mut(&id) {
                    p.app_id = Some(app_id);
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: raw } => {
                if let Some(p) = state.toplevel_pending.get_mut(&id) {
                    p.states = raw;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Done => {
                if let Some(pending) = state.toplevel_pending.get(&id) {
                    let state_vals: Vec<u32> = pending
                        .states
                        .chunks_exact(4)
                        .map(|b| u32::from_ne_bytes(b.try_into().unwrap()))
                        .collect();
                    let info = ToplevelInfo {
                        title: pending.title.clone().unwrap_or_default(),
                        app_id: pending.app_id.clone().unwrap_or_default(),
                        activated: state_vals.contains(&2),
                        minimized: state_vals.contains(&1),
                        maximized: state_vals.contains(&0),
                        fullscreen: state_vals.contains(&3),
                    };
                    state.toplevels.insert(id, info.clone());
                    let _ = state
                        .event_tx
                        .send(crate::event::WaylandEvent::ToplevelUpdated { id, info });
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.toplevels.remove(&id);
                state.toplevel_pending.remove(&id);
                state.toplevel_handles.remove(&id);
                let _ = state
                    .event_tx
                    .send(crate::event::WaylandEvent::ToplevelClosed { id });
            }
            _ => {}
        }
    }
}

impl Dispatch<WlSeat, ()> for MplugState {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
    }
}

impl Dispatch<ExtIdleNotifierV1, ()> for MplugState {
    fn event(
        _: &mut Self,
        _: &ExtIdleNotifierV1,
        _: <ExtIdleNotifierV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for MplugState {
    fn event(
        state: &mut Self,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => {
                let _ = state.event_tx.send(WaylandEvent::Idled);
            }
            ext_idle_notification_v1::Event::Resumed => {
                let _ = state.event_tx.send(WaylandEvent::IdleResumed);
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputPowerManagerV1, ()> for MplugState {
    fn event(
        _: &mut Self,
        _: &ZwlrOutputPowerManagerV1,
        _: <ZwlrOutputPowerManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
    }
}

impl Dispatch<ZwlrOutputPowerV1, ()> for MplugState {
    fn event(
        state: &mut Self,
        _: &ZwlrOutputPowerV1,
        event: zwlr_output_power_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
        match event {
            zwlr_output_power_v1::Event::Mode { mode } => {
                let on = matches!(
                    mode,
                    wayland_client::WEnum::Value(zwlr_output_power_v1::Mode::On)
                );
                let _ = state.event_tx.send(WaylandEvent::OutputPowerMode { on });
            }
            zwlr_output_power_v1::Event::Failed => {
                eprintln!(
                    "mplug: zwlr_output_power_v1 failed (compositor may not support output power control)"
                );
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceManagerV1, ()> for MplugState {
    fn event(
        state: &mut Self,
        _: &ExtWorkspaceManagerV1,
        event: ext_workspace_manager_v1::Event,
        _: &(),
        _: &Connection,
        _qh: &QueueHandle<MplugState>,
    ) {
        match event {
            ext_workspace_manager_v1::Event::WorkspaceGroup { workspace_group: _ } => {}
            ext_workspace_manager_v1::Event::Workspace { workspace } => {
                let id = workspace.id().protocol_id();
                state.workspace_pending.entry(id).or_default();
            }
            ext_workspace_manager_v1::Event::Done => {
                let ids: Vec<u32> = state.workspace_pending.keys().copied().collect();
                for id in ids {
                    if let Some(pending) = state.workspace_pending.remove(&id) {
                        let info = WorkspaceInfo {
                            name: pending.name.unwrap_or_default(),
                            active: pending.state & 1 != 0,
                            urgent: pending.state & 2 != 0,
                            hidden: pending.state & 4 != 0,
                        };
                        state.workspaces.insert(id, info.clone());
                        let _ = state
                            .event_tx
                            .send(WaylandEvent::WorkspaceUpdated { id, info });
                    }
                }
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(MplugState, ExtWorkspaceManagerV1, [
        0 => (ExtWorkspaceGroupHandleV1, ()),
        1 => (ExtWorkspaceHandleV1, ()),
    ]);
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for MplugState {
    fn event(
        _: &mut Self,
        _: &ExtWorkspaceGroupHandleV1,
        _: ext_workspace_group_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
    }
}

impl Dispatch<ExtWorkspaceHandleV1, ()> for MplugState {
    fn event(
        state: &mut Self,
        handle: &ExtWorkspaceHandleV1,
        event: ext_workspace_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
        let id = handle.id().protocol_id();
        match event {
            ext_workspace_handle_v1::Event::Name { name } => {
                state.workspace_pending.entry(id).or_default().name = Some(name);
            }
            ext_workspace_handle_v1::Event::State { state: bits } => {
                let raw: u32 = match bits {
                    wayland_client::WEnum::Value(v) => u32::from(v),
                    wayland_client::WEnum::Unknown(v) => v,
                };
                state.workspace_pending.entry(id).or_default().state = raw;
            }
            ext_workspace_handle_v1::Event::Removed => {
                state.workspaces.remove(&id);
                let _ = state.event_tx.send(WaylandEvent::WorkspaceClosed { id });
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for MplugState {
    fn event(
        state: &mut Self,
        _: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head } => {
                let id = head.id().protocol_id();
                state.heads.insert(id, head);
                state.head_pending.entry(id).or_default();
            }
            zwlr_output_manager_v1::Event::Done { serial } => {
                state.manager_serial = serial;
                let ids: Vec<u32> = state.head_pending.keys().copied().collect();
                for id in ids {
                    if let Some(pending) = state.head_pending.remove(&id) {
                        let existing = state.head_states.entry(id).or_insert_with(|| HeadInfo {
                            name: String::new(),
                            description: String::new(),
                            width_mm: 0,
                            height_mm: 0,
                            x: 0,
                            y: 0,
                            enabled: false,
                            width_px: 0,
                            height_px: 0,
                            refresh: 0,
                            scale: 1.0,
                            transform: 0,
                        });
                        if let Some(v) = pending.name {
                            existing.name = v;
                        }
                        if let Some(v) = pending.description {
                            existing.description = v;
                        }
                        if let Some(v) = pending.width_mm {
                            existing.width_mm = v;
                        }
                        if let Some(v) = pending.height_mm {
                            existing.height_mm = v;
                        }
                        if let Some(v) = pending.x {
                            existing.x = v;
                        }
                        if let Some(v) = pending.y {
                            existing.y = v;
                        }
                        if let Some(v) = pending.enabled {
                            existing.enabled = v;
                        }
                        if let Some(v) = pending.scale {
                            existing.scale = v;
                        }
                        if let Some(v) = pending.transform {
                            existing.transform = v;
                        }
                        if let Some(mode_id) = pending.current_mode_id {
                            if let Some(&(w, h, r)) = state.modes.get(&mode_id) {
                                existing.width_px = w;
                                existing.height_px = h;
                                existing.refresh = r;
                            }
                        }
                        state.head_names.insert(existing.name.clone(), id);
                        let _ = state.event_tx.send(WaylandEvent::OutputHeadUpdated {
                            id,
                            info: existing.clone(),
                        });
                    }
                }
            }
            zwlr_output_manager_v1::Event::Finished => {}
            _ => {}
        }
    }

    wayland_client::event_created_child!(MplugState, ZwlrOutputManagerV1, [
        0 => (ZwlrOutputHeadV1, ())
    ]);
}

impl Dispatch<ZwlrOutputHeadV1, ()> for MplugState {
    fn event(
        state: &mut Self,
        handle: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
        let id = handle.id().protocol_id();
        match event {
            zwlr_output_head_v1::Event::Name { name } => {
                state.head_pending.entry(id).or_default().name = Some(name);
            }
            zwlr_output_head_v1::Event::Description { description } => {
                state.head_pending.entry(id).or_default().description = Some(description);
            }
            zwlr_output_head_v1::Event::PhysicalSize { width, height } => {
                let p = state.head_pending.entry(id).or_default();
                p.width_mm = Some(width);
                p.height_mm = Some(height);
            }
            zwlr_output_head_v1::Event::Position { x, y } => {
                let p = state.head_pending.entry(id).or_default();
                p.x = Some(x);
                p.y = Some(y);
            }
            zwlr_output_head_v1::Event::Enabled { enabled } => {
                let is_enabled = enabled != 0;
                state.head_pending.entry(id).or_default().enabled = Some(is_enabled);
            }
            zwlr_output_head_v1::Event::CurrentMode { mode } => {
                let mode_id = mode.id().protocol_id();
                state.head_pending.entry(id).or_default().current_mode_id = Some(mode_id);
            }
            zwlr_output_head_v1::Event::Scale { scale } => {
                state.head_pending.entry(id).or_default().scale = Some(scale);
            }
            zwlr_output_head_v1::Event::Transform { transform } => {
                let raw = match transform {
                    wayland_client::WEnum::Value(v) => v as u32,
                    wayland_client::WEnum::Unknown(v) => v,
                };
                state.head_pending.entry(id).or_default().transform = Some(raw);
            }
            zwlr_output_head_v1::Event::Mode { mode: _ } => {}
            zwlr_output_head_v1::Event::Finished => {
                state.heads.remove(&id);
                state.head_pending.remove(&id);
                if let Some(info) = state.head_states.remove(&id) {
                    state.head_names.remove(&info.name);
                }
                let _ = state.event_tx.send(WaylandEvent::OutputHeadRemoved { id });
            }
            _ => {}
        }
    }

    // name(0), description(1), physical_size(2), mode(3), enabled(4), current_mode(5),
    // position(6), transform(7), scale(8), finished(9)
    wayland_client::event_created_child!(MplugState, ZwlrOutputHeadV1, [
        3 => (ZwlrOutputModeV1, ())
    ]);
}

impl Dispatch<ZwlrOutputConfigurationV1, ()> for MplugState {
    fn event(
        _: &mut Self,
        _: &ZwlrOutputConfigurationV1,
        event: wayland_protocols_wlr::output_management::v1::client::zwlr_output_configuration_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
        use wayland_protocols_wlr::output_management::v1::client::zwlr_output_configuration_v1::Event;
        match event {
            Event::Succeeded => {}
            Event::Failed => {
                eprintln!(
                    "mplug: zwlr_output_configuration_v1 apply failed (compositor rejected the configuration)"
                );
            }
            Event::Cancelled => {
                eprintln!(
                    "mplug: zwlr_output_configuration_v1 cancelled (serial outdated — retry)"
                );
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for MplugState {
    fn event(
        _: &mut Self,
        _: &ZwlrOutputConfigurationHeadV1,
        _: wayland_protocols_wlr::output_management::v1::client::zwlr_output_configuration_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
    }
}

impl Dispatch<ZwlrOutputModeV1, ()> for MplugState {
    fn event(
        state: &mut Self,
        handle: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
        let id = handle.id().protocol_id();
        match event {
            zwlr_output_mode_v1::Event::Size { width, height } => {
                let entry = state.modes.entry(id).or_insert((0, 0, 0));
                entry.0 = width;
                entry.1 = height;
            }
            zwlr_output_mode_v1::Event::Refresh { refresh } => {
                state.modes.entry(id).or_insert((0, 0, 0)).2 = refresh;
            }
            zwlr_output_mode_v1::Event::Finished => {
                state.modes.remove(&id);
            }
            _ => {}
        }
    }
}

impl Dispatch<WlCompositor, ()> for MplugState {
    fn event(
        _: &mut Self,
        _: &WlCompositor,
        _: <WlCompositor as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
    }
}

impl Dispatch<WlSurface, ()> for MplugState {
    fn event(
        _: &mut Self,
        _: &WlSurface,
        _: <WlSurface as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
    }
}

impl Dispatch<WlShm, ()> for MplugState {
    fn event(
        _: &mut Self,
        _: &WlShm,
        _: wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
    }
}

impl Dispatch<WlShmPool, ()> for MplugState {
    fn event(
        _: &mut Self,
        _: &WlShmPool,
        _: <WlShmPool as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
    }
}

impl Dispatch<WlBuffer, ()> for MplugState {
    fn event(
        _: &mut Self,
        _: &WlBuffer,
        _: <WlBuffer as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
    }
}

impl Dispatch<ZwlrLayerShellV1, ()> for MplugState {
    fn event(
        _: &mut Self,
        _: &ZwlrLayerShellV1,
        _: <ZwlrLayerShellV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, u32> for MplugState {
    fn event(
        state: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<MplugState>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer_surface.ack_configure(serial);
                if let Some(lss) = state.layer_surfaces.get_mut(id) {
                    if width > 0 {
                        lss.width = width;
                    }
                    if height > 0 {
                        lss.height = height;
                    }
                    lss.configured = true;
                }
                let _ = state.event_tx.send(WaylandEvent::LayerSurfaceConfigured {
                    id: *id,
                    width: state.layer_surfaces.get(id).map(|s| s.width).unwrap_or(0),
                    height: state.layer_surfaces.get(id).map(|s| s.height).unwrap_or(0),
                });
            }
            zwlr_layer_surface_v1::Event::Closed => {
                let _ = state
                    .event_tx
                    .send(WaylandEvent::LayerSurfaceClosed { id: *id });
                if let Some(lss) = state.layer_surfaces.remove(id) {
                    if let Some(path) = &lss.shm_path {
                        let _ = std::fs::remove_file(path);
                    }
                    lss.layer_surface.destroy();
                    lss.surface.destroy();
                }
            }
            _ => {}
        }
    }
}

pub fn run_wayland(tx: Sender<WaylandEvent>, rx: Receiver<WaylandRequest>) {
    let conn = Connection::connect_to_env().expect("Failed to bind to Wayland socket");
    let display = conn.display();
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    let _registry = display.get_registry(&qh, ());

    let mut state = MplugState {
        manager: None,
        outputs: Vec::new(),
        ipc_outputs: Vec::new(),
        event_tx: tx,
        toplevel_manager: None,
        toplevels: HashMap::new(),
        toplevel_pending: HashMap::new(),
        seat: None,
        idle_notifier: None,
        idle_notification: None,
        output_power_manager: None,
        output_powers: Vec::new(),
        workspace_manager: None,
        workspaces: HashMap::new(),
        workspace_pending: HashMap::new(),
        output_manager: None,
        heads: HashMap::new(),
        head_names: HashMap::new(),
        head_states: HashMap::new(),
        head_pending: HashMap::new(),
        modes: HashMap::new(),
        manager_serial: 0,
        compositor: None,
        wl_shm: None,
        layer_shell: None,
        layer_surfaces: HashMap::new(),
        toplevel_handles: HashMap::new(),
    };

    println!("Wayland thread running...");
    loop {
        if let Err(e) = event_queue.blocking_dispatch(&mut state) {
            eprintln!("Wayland dispatch error: {}", e);
            break;
        }

        std::thread::sleep(std::time::Duration::from_millis(15));

        while let Ok(req) = rx.try_recv() {
            println!("Wayland handling request from Lua: {:?}", req);
            match req {
                WaylandRequest::SetLayout(index) => {
                    for out in &state.ipc_outputs {
                        out.set_layout(index);
                    }
                }
                WaylandRequest::SetTags(tagmask) => {
                    for out in &state.ipc_outputs {
                        out.set_tags(tagmask, 0);
                    }
                }
                WaylandRequest::ToggleVisibility => {}
                WaylandRequest::SetOutputPower { on } => {
                    use zwlr_output_power_v1::Mode;
                    let mode = if on { Mode::On } else { Mode::Off };
                    for op in &state.output_powers {
                        op.set_mode(mode);
                    }
                }
                WaylandRequest::SetOutputMode {
                    head_name,
                    width,
                    height,
                    refresh,
                } => {
                    if let (Some(mgr), Some(&head_id)) =
                        (&state.output_manager, state.head_names.get(&head_name))
                    {
                        if let Some(head) = state.heads.get(&head_id) {
                            let config = mgr.create_configuration(state.manager_serial, &qh, ());
                            let config_head = config.enable_head(head, &qh, ());
                            config_head.set_custom_mode(width, height, refresh);
                            config.apply();
                            let _ = event_queue.flush();
                        }
                    }
                }
                WaylandRequest::SetOutputPosition { head_name, x, y } => {
                    if let (Some(mgr), Some(&head_id)) =
                        (&state.output_manager, state.head_names.get(&head_name))
                    {
                        if let Some(head) = state.heads.get(&head_id) {
                            let config = mgr.create_configuration(state.manager_serial, &qh, ());
                            let config_head = config.enable_head(head, &qh, ());
                            config_head.set_position(x, y);
                            config.apply();
                            let _ = event_queue.flush();
                        }
                    }
                }
                WaylandRequest::SetOutputScale { head_name, scale } => {
                    if let (Some(mgr), Some(&head_id)) =
                        (&state.output_manager, state.head_names.get(&head_name))
                    {
                        if let Some(head) = state.heads.get(&head_id) {
                            let config = mgr.create_configuration(state.manager_serial, &qh, ());
                            let config_head = config.enable_head(head, &qh, ());
                            config_head.set_scale(scale);
                            config.apply();
                            let _ = event_queue.flush();
                        }
                    }
                }
                WaylandRequest::SetOutputEnabled { head_name, enabled } => {
                    if let (Some(mgr), Some(&head_id)) =
                        (&state.output_manager, state.head_names.get(&head_name))
                    {
                        if let Some(head) = state.heads.get(&head_id) {
                            let config = mgr.create_configuration(state.manager_serial, &qh, ());
                            if enabled {
                                let _config_head = config.enable_head(head, &qh, ());
                            } else {
                                config.disable_head(head);
                            }
                            config.apply();
                            let _ = event_queue.flush();
                        }
                    }
                }
                WaylandRequest::CreateLayerSurface {
                    id,
                    width,
                    height,
                    anchor,
                    layer,
                    exclusive_zone,
                } => {
                    if let (Some(compositor), Some(shell)) = (&state.compositor, &state.layer_shell)
                    {
                        let surface = compositor.create_surface(&qh, ());
                        let layer_enum = match layer {
                            0 => Layer::Background,
                            1 => Layer::Bottom,
                            3 => Layer::Overlay,
                            _ => Layer::Top,
                        };
                        let anchor_flags = Anchor::from_bits(anchor).unwrap_or(Anchor::empty());
                        let ls = shell.get_layer_surface(
                            &surface,
                            None,
                            layer_enum,
                            "mplug".into(),
                            &qh,
                            id,
                        );
                        ls.set_size(width, height);
                        ls.set_anchor(anchor_flags);
                        ls.set_exclusive_zone(exclusive_zone);
                        ls.set_keyboard_interactivity(KeyboardInteractivity::None);
                        surface.commit();
                        let _ = event_queue.flush();
                        state.layer_surfaces.insert(
                            id,
                            LayerSurfaceState {
                                surface,
                                layer_surface: ls,
                                configured: false,
                                width,
                                height,
                                shm_path: None,
                            },
                        );
                    }
                }
                WaylandRequest::DestroyLayerSurface { id } => {
                    if let Some(lss) = state.layer_surfaces.remove(&id) {
                        if let Some(path) = &lss.shm_path {
                            let _ = std::fs::remove_file(path);
                        }
                        lss.layer_surface.destroy();
                        lss.surface.destroy();
                        let _ = event_queue.flush();
                    }
                }
                WaylandRequest::CloseToplevel { id } => {
                    if let Some(handle) = state.toplevel_handles.get(&id) {
                        handle.close();
                        let _ = event_queue.flush();
                    }
                }
                WaylandRequest::SetToplevelMinimized { id, minimized } => {
                    if let Some(handle) = state.toplevel_handles.get(&id) {
                        if minimized {
                            handle.set_minimized();
                        } else {
                            handle.unset_minimized();
                        }
                        let _ = event_queue.flush();
                    }
                }
                WaylandRequest::ActivateToplevel { id } => {
                    if let (Some(handle), Some(seat)) =
                        (state.toplevel_handles.get(&id), &state.seat)
                    {
                        handle.activate(seat);
                        let _ = event_queue.flush();
                    }
                }
                WaylandRequest::SetToplevelTags { id, tagmask } => {
                    if let (Some(handle), Some(seat)) =
                        (state.toplevel_handles.get(&id), &state.seat)
                    {
                        handle.activate(seat);
                    }
                    for out in &state.ipc_outputs {
                        out.set_client_tags(0, tagmask);
                    }
                    let _ = event_queue.flush();
                }
                WaylandRequest::SetClientTags { and_tags, xor_tags } => {
                    for out in &state.ipc_outputs {
                        out.set_client_tags(and_tags, xor_tags);
                    }
                    let _ = event_queue.flush();
                }
                WaylandRequest::FillLayerSurface { id, r, g, b, a } => {
                    if let Some(lss) = state.layer_surfaces.get_mut(&id) {
                        if !lss.configured {
                            return;
                        }

                        let width = lss.width as i32;
                        let height = lss.height as i32;
                        let stride = width * 4;
                        let size = (stride * height) as usize;

                        let rb = (r.clamp(0.0, 1.0) * 255.0) as u8;
                        let gb = (g.clamp(0.0, 1.0) * 255.0) as u8;
                        let bb = (b.clamp(0.0, 1.0) * 255.0) as u8;
                        let ab = (a.clamp(0.0, 1.0) * 255.0) as u8;
                        let pixel: u32 = ((ab as u32) << 24)
                            | ((rb as u32) << 16)
                            | ((gb as u32) << 8)
                            | (bb as u32);
                        let pixel_bytes = pixel.to_ne_bytes();

                        let mut buf = Vec::with_capacity(size);
                        for _ in 0..(width * height) {
                            buf.extend_from_slice(&pixel_bytes);
                        }

                        let path = format!("/dev/shm/mplug-layer-{}", id);
                        let file = std::fs::OpenOptions::new()
                            .create(true)
                            .read(true)
                            .write(true)
                            .open(&path);
                        match file {
                            Err(e) => {
                                eprintln!("mplug: failed to open shm file {}: {}", path, e);
                            }
                            Ok(mut f) => {
                                if let Err(e) = f.set_len(size as u64) {
                                    eprintln!("mplug: set_len failed: {}", e);
                                    return;
                                }
                                use std::io::Write as IoWrite;
                                if let Err(e) = f.write_all(&buf) {
                                    eprintln!("mplug: write failed: {}", e);
                                    return;
                                }

                                if let Some(shm) = &state.wl_shm {
                                    let pool = shm.create_pool(f.as_fd(), size as i32, &qh, ());
                                    let buffer = pool.create_buffer(
                                        0,
                                        width,
                                        height,
                                        stride,
                                        wl_shm::Format::Argb8888,
                                        &qh,
                                        (),
                                    );
                                    lss.surface.attach(Some(&buffer), 0, 0);
                                    lss.surface.damage_buffer(0, 0, width, height);
                                    lss.surface.commit();
                                    let _ = event_queue.flush();
                                    lss.shm_path = Some(path);
                                }
                            }
                        }
                    }
                }
            }
        }

        let _ = event_queue.flush();
    }
}
