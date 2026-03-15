use crate::config;
use crate::event::{WaylandEvent, WaylandRequest};
use crate::manifest::load_manifest;
use mlua::{Lua, Result as LuaResult};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

pub fn run_lua(rx: Receiver<WaylandEvent>, tx: Sender<WaylandRequest>) -> LuaResult<()> {
    let lua = Lua::new();
    let mplug_table = lua.create_table()?;

    let listeners_table = lua.create_table()?;
    mplug_table.set("__listeners", listeners_table.clone())?;

    let add_listener = lua.create_function(move |lua_ctx: &Lua, func: mlua::Function| {
        let listeners: mlua::Table = lua_ctx
            .globals()
            .get::<_, mlua::Table>("mplug")?
            .get("__listeners")?;
        let len = listeners.len()?;
        listeners.set(len + 1, func)?;
        Ok(())
    })?;
    mplug_table.set("add_listener", add_listener)?;

    let tx_dispatch = tx.clone();
    let dispatch = lua.create_function(move |_, command: String| {
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(());
        }

        match parts[0] {
            "set_layout" => {
                if parts.len() > 1 {
                    if let Ok(index) = parts[1].parse::<u32>() {
                        let _ = tx_dispatch.send(WaylandRequest::SetLayout(index));
                    }
                }
            }
            "set_tags" => {
                if parts.len() > 1 {
                    if let Ok(tagmask) = parts[1].parse::<u32>() {
                        let _ = tx_dispatch.send(WaylandRequest::SetTags(tagmask));
                    }
                }
            }
            "set_window_tag" => {
                if parts.len() > 2 {
                    if let (Ok(id), Ok(tagmask)) =
                        (parts[1].parse::<u32>(), parts[2].parse::<u32>())
                    {
                        let _ = tx_dispatch.send(WaylandRequest::SetToplevelTags { id, tagmask });
                    }
                }
            }
            "set_client_tags" => {
                if parts.len() > 2 {
                    if let (Ok(and_tags), Ok(xor_tags)) =
                        (parts[1].parse::<u32>(), parts[2].parse::<u32>())
                    {
                        let _ =
                            tx_dispatch.send(WaylandRequest::SetClientTags { and_tags, xor_tags });
                    }
                }
            }
            _ => eprintln!(
                "mplug: unknown Wayland dispatch command from Lua: {}",
                command
            ),
        }
        Ok(())
    })?;

    mplug_table.set("dispatch", dispatch)?;

    let tx_power = tx.clone();
    let set_output_power_fn = lua.create_function(move |_, on: bool| {
        let _ = tx_power.send(WaylandRequest::SetOutputPower { on });
        Ok(())
    })?;
    mplug_table.set("set_output_power", set_output_power_fn)?;

    let tx_out_mode = tx.clone();
    let set_output_mode_fn = lua.create_function(
        move |_, (head_name, width, height, refresh): (String, i32, i32, i32)| {
            let _ = tx_out_mode.send(WaylandRequest::SetOutputMode {
                head_name,
                width,
                height,
                refresh,
            });
            Ok(())
        },
    )?;
    mplug_table.set("set_output_mode", set_output_mode_fn)?;

    let tx_out_pos = tx.clone();
    let set_output_position_fn =
        lua.create_function(move |_, (head_name, x, y): (String, i32, i32)| {
            let _ = tx_out_pos.send(WaylandRequest::SetOutputPosition { head_name, x, y });
            Ok(())
        })?;
    mplug_table.set("set_output_position", set_output_position_fn)?;

    let tx_out_scale = tx.clone();
    let set_output_scale_fn =
        lua.create_function(move |_, (head_name, scale): (String, f64)| {
            let _ = tx_out_scale.send(WaylandRequest::SetOutputScale { head_name, scale });
            Ok(())
        })?;
    mplug_table.set("set_output_scale", set_output_scale_fn)?;

    let tx_out_enabled = tx.clone();
    let set_output_enabled_fn =
        lua.create_function(move |_, (head_name, enabled): (String, bool)| {
            let _ = tx_out_enabled.send(WaylandRequest::SetOutputEnabled { head_name, enabled });
            Ok(())
        })?;
    mplug_table.set("set_output_enabled", set_output_enabled_fn)?;

    let tx_close = tx.clone();
    let close_window_fn = lua.create_function(move |_, id: u32| {
        let _ = tx_close.send(WaylandRequest::CloseToplevel { id });
        Ok(())
    })?;
    mplug_table.set("close_window", close_window_fn)?;

    let tx_minimize = tx.clone();
    let set_window_minimized_fn = lua.create_function(move |_, (id, minimized): (u32, bool)| {
        let _ = tx_minimize.send(WaylandRequest::SetToplevelMinimized { id, minimized });
        Ok(())
    })?;
    mplug_table.set("set_window_minimized", set_window_minimized_fn)?;

    let tx_focus = tx.clone();
    let focus_window_fn = lua.create_function(move |_, id: u32| {
        let _ = tx_focus.send(WaylandRequest::ActivateToplevel { id });
        Ok(())
    })?;
    mplug_table.set("focus_window", focus_window_fn)?;

    let tx_wt = tx.clone();
    let set_window_tag_fn = lua.create_function(move |_, (id, tagmask): (u32, u32)| {
        let _ = tx_wt.send(WaylandRequest::SetToplevelTags { id, tagmask });
        Ok(())
    })?;
    mplug_table.set("set_window_tag", set_window_tag_fn)?;

    let tx_ct = tx.clone();
    let set_client_tags_fn = lua.create_function(move |_, (and_tags, xor_tags): (u32, u32)| {
        let _ = tx_ct.send(WaylandRequest::SetClientTags { and_tags, xor_tags });
        Ok(())
    })?;
    mplug_table.set("set_client_tags", set_client_tags_fn)?;

    let exec_fn = lua.create_function(|_, cmd: String| {
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .output()
            .map_err(|e| mlua::Error::RuntimeError(format!("mplug.exec failed: {}", e)))?;
        let stdout = String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string();
        let exit_code = output.status.code().unwrap_or(-1) as i32;
        Ok((stdout, exit_code))
    })?;
    mplug_table.set("exec", exec_fn)?;

    let surface_callbacks_tbl = lua.create_table()?;
    mplug_table.set("__surface_callbacks", surface_callbacks_tbl)?;

    let tx_cls = tx.clone();
    let create_layer_surface_fn = lua.create_function(
        move |lua_ctx, (config, callback): (mlua::Table, mlua::Function)| {
            let mplug_g: mlua::Table = lua_ctx.globals().get("mplug")?;

            let current_id: u32 = mplug_g.get::<_, u32>("__next_surface_id").unwrap_or(0);
            let id = current_id + 1;
            mplug_g.set("__next_surface_id", id)?;

            let width: u32 = config.get::<_, u32>("width").unwrap_or(200);
            let height: u32 = config.get::<_, u32>("height").unwrap_or(30);
            let exclusive_zone: i32 = config.get::<_, i32>("exclusive_zone").unwrap_or(0);

            let anchor_str: String = config.get::<_, String>("anchor").unwrap_or_default();
            let mut anchor: u32 = 0;
            if anchor_str.contains("top") {
                anchor |= 1;
            }
            if anchor_str.contains("bottom") {
                anchor |= 2;
            }
            if anchor_str.contains("left") {
                anchor |= 4;
            }
            if anchor_str.contains("right") {
                anchor |= 8;
            }

            let layer_str: String = config
                .get::<_, String>("layer")
                .unwrap_or_else(|_| "top".to_string());
            let layer: u32 = match layer_str.as_str() {
                "background" => 0,
                "bottom" => 1,
                "overlay" => 3,
                _ => 2, // default: "top"
            };

            let surface_cbs: mlua::Table = mplug_g.get("__surface_callbacks")?;
            surface_cbs.set(id, callback)?;

            let _ = tx_cls.send(WaylandRequest::CreateLayerSurface {
                id,
                width,
                height,
                anchor,
                layer,
                exclusive_zone,
            });
            Ok(())
        },
    )?;
    mplug_table.set("create_layer_surface", create_layer_surface_fn)?;
    mplug_table.set("__next_surface_id", 0u32)?;

    lua.globals().set("mplug", &mplug_table)?;

    let cfg = config::load_config();
    let plugins_dir = config::get_config_dir().join("plugins");

    for plugin_name in cfg.enabled_plugins {
        let plugin_file_path = plugins_dir.join(format!("{}.lua", plugin_name));
        let plugin_dir_path = plugins_dir.join(&plugin_name);

        let active_path = if plugin_file_path.exists() {
            Some(plugin_file_path)
        } else if plugin_dir_path.exists() {
            if let Some(path_str) = plugin_dir_path.to_str() {
                let package_table: mlua::Table = lua.globals().get("package")?;
                let cur_path: String = package_table.get("path")?;
                package_table.set("path", format!("{};{}/?.lua", cur_path, path_str))?;
            }

            match load_manifest(&plugin_dir_path) {
                Ok(manifest) => Some(plugin_dir_path.join(&manifest.entry_point)),
                Err(err) => {
                    eprintln!("Plugin '{}' has no valid mplug.toml: {}", plugin_name, err);
                    None
                }
            }
        } else {
            eprintln!(
                "Enabled plugin '{}' not found in {:?}",
                plugin_name, plugins_dir
            );
            None
        };

        if let Some(target) = active_path {
            if let Ok(script) = fs::read_to_string(&target) {
                println!("Loading plugin: {}", plugin_name);
                if let Err(e) = lua.load(&script).exec() {
                    eprintln!("Plugin Error ({}): {}", plugin_name, e);
                }
            }
        }
    }

    let init_script = PathBuf::from("init.lua");
    if init_script.exists() {
        if let Ok(script) = fs::read_to_string(&init_script) {
            let _ = lua.load(&script).exec();
        }
    }

    println!("Lua Event Engine running...");

    let mut tag_count: u32 = 0;
    let mut layout_name = String::new();
    let mut layout_index: u32 = 0;
    let mut layout_symbol = String::new();
    let mut idle: bool = false;
    let mut output_power_on: bool = true;
    let mut tag_states: std::collections::HashMap<u32, (u32, u32, u32)> =
        std::collections::HashMap::new();
    let mut toplevel_states: std::collections::HashMap<u32, crate::event::ToplevelInfo> =
        std::collections::HashMap::new();
    let mut workspace_states: std::collections::HashMap<u32, crate::event::WorkspaceInfo> =
        std::collections::HashMap::new();
    let mut output_head_states: std::collections::HashMap<u32, crate::event::HeadInfo> =
        std::collections::HashMap::new();

    while let Ok(event) = rx.recv() {
        let lua_event_table = lua.create_table()?;

        match &event {
            WaylandEvent::OutputTag {
                tag,
                state,
                clients,
                focused,
            } => {
                if let Err(e) = lua_event_table.set("type", "OutputTag") {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("tag", *tag) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("state", *state) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("clients", *clients) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("focused", *focused) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
            }
            WaylandEvent::TagsAmount(amount) => {
                if let Err(e) = lua_event_table.set("type", "TagsAmount") {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("amount", *amount) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
            }
            WaylandEvent::LayoutName(name) => {
                if let Err(e) = lua_event_table.set("type", "LayoutName") {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("name", name.clone()) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
            }
            WaylandEvent::OutputLayout(layout) => {
                if let Err(e) = lua_event_table.set("type", "OutputLayout") {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("layout", *layout) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
            }
            WaylandEvent::ToplevelUpdated { id, info } => {
                if let Err(e) = lua_event_table.set("type", "ToplevelUpdated") {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("id", *id) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("title", info.title.clone()) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("app_id", info.app_id.clone()) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("activated", info.activated) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("minimized", info.minimized) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("maximized", info.maximized) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("fullscreen", info.fullscreen) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
            }
            WaylandEvent::ToplevelClosed { id } => {
                if let Err(e) = lua_event_table.set("type", "ToplevelClosed") {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("id", *id) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
            }
            WaylandEvent::WorkspaceUpdated { id, info } => {
                if let Err(e) = lua_event_table.set("type", "WorkspaceUpdated") {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("id", *id) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("name", info.name.clone()) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("active", info.active) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("urgent", info.urgent) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("hidden", info.hidden) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
            }
            WaylandEvent::WorkspaceClosed { id } => {
                if let Err(e) = lua_event_table.set("type", "WorkspaceClosed") {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("id", *id) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
            }
            WaylandEvent::OutputHeadUpdated { id, info } => {
                if let Err(e) = lua_event_table.set("type", "OutputHeadUpdated") {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("id", *id) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("name", info.name.clone()) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("enabled", info.enabled) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("width_px", info.width_px) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("height_px", info.height_px) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("refresh", info.refresh) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("scale", info.scale) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("x", info.x) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("y", info.y) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
            }
            WaylandEvent::OutputHeadRemoved { id } => {
                if let Err(e) = lua_event_table.set("type", "OutputHeadRemoved") {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("id", *id) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
            }
            WaylandEvent::LayerSurfaceConfigured { id, width, height } => {
                let _ = lua_event_table.set("type", "LayerSurfaceConfigured");
                let _ = lua_event_table.set("id", *id);
                let _ = lua_event_table.set("width", *width);
                let _ = lua_event_table.set("height", *height);
            }
            WaylandEvent::LayerSurfaceClosed { id } => {
                let _ = lua_event_table.set("type", "LayerSurfaceClosed");
                let _ = lua_event_table.set("id", *id);
            }
            WaylandEvent::UserCommand(name) => {
                let _ = lua_event_table.set("type", "UserCommand");
                let _ = lua_event_table.set("name", name.clone());
            }
            WaylandEvent::Idled => {
                if let Err(e) = lua_event_table.set("type", "Idled") {
                    eprintln!("mplug: failed to set event field: {e}");
                }
            }
            WaylandEvent::IdleResumed => {
                if let Err(e) = lua_event_table.set("type", "IdleResumed") {
                    eprintln!("mplug: failed to set event field: {e}");
                }
            }
            WaylandEvent::OutputPowerMode { on } => {
                if let Err(e) = lua_event_table.set("type", "OutputPowerMode") {
                    eprintln!("mplug: failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("on", *on) {
                    eprintln!("mplug: failed to set event field: {e}");
                }
            }
            _ => {
                if let Err(e) = lua_event_table.set("type", "Generic") {
                    eprintln!("mplug: failed to set event field: {e}");
                }
            }
        }

        match &event {
            WaylandEvent::TagsAmount(amount) => {
                tag_count = *amount;
            }
            WaylandEvent::LayoutName(name) => {
                layout_name = name.clone();
            }
            WaylandEvent::OutputLayout(layout) => {
                layout_index = *layout;
            }
            WaylandEvent::OutputLayoutSymbol(sym) => {
                layout_symbol = sym.clone();
            }
            WaylandEvent::OutputTag {
                tag,
                state,
                clients,
                focused,
            } => {
                tag_states.insert(*tag, (*state, *clients, *focused));
            }
            WaylandEvent::ToplevelUpdated { id, info } => {
                toplevel_states.insert(*id, info.clone());
            }
            WaylandEvent::ToplevelClosed { id } => {
                toplevel_states.remove(id);
            }
            WaylandEvent::WorkspaceUpdated { id, info } => {
                workspace_states.insert(*id, info.clone());
            }
            WaylandEvent::WorkspaceClosed { id } => {
                workspace_states.remove(id);
            }
            WaylandEvent::OutputHeadUpdated { id, info } => {
                output_head_states.insert(*id, info.clone());
            }
            WaylandEvent::OutputHeadRemoved { id } => {
                output_head_states.remove(id);
            }
            WaylandEvent::LayerSurfaceConfigured {
                id,
                width: _,
                height: _,
            } => {
                let mplug_g: mlua::Table = match lua.globals().get::<_, mlua::Table>("mplug") {
                    Ok(t) => t,
                    Err(_) => return Ok(()),
                };
                let surface_cbs: mlua::Table =
                    match mplug_g.get::<_, mlua::Table>("__surface_callbacks") {
                        Ok(t) => t,
                        Err(_) => return Ok(()),
                    };
                let cb: Option<mlua::Function> = surface_cbs
                    .get::<_, Option<mlua::Function>>(*id)
                    .ok()
                    .flatten();
                if let Some(cb) = cb {
                    let _ = surface_cbs.set(*id, mlua::Value::Nil);

                    match lua.create_table() {
                        Err(e) => eprintln!("mplug: failed to create surface table: {e}"),
                        Ok(surface_tbl) => {
                            let _ = surface_tbl.set("id", *id);

                            let sid = *id;
                            let tx_fill = tx.clone();
                            match lua.create_function(
                                move |_, (r, g, b, a): (f32, f32, f32, f32)| {
                                    let _ = tx_fill.send(WaylandRequest::FillLayerSurface {
                                        id: sid,
                                        r,
                                        g,
                                        b,
                                        a,
                                    });
                                    Ok(())
                                },
                            ) {
                                Ok(f) => {
                                    let _ = surface_tbl.set("fill", f);
                                }
                                Err(e) => eprintln!("mplug: failed to create fill fn: {e}"),
                            }

                            let sid = *id;
                            let tx_destroy = tx.clone();
                            match lua.create_function(move |_, ()| {
                                let _ = tx_destroy
                                    .send(WaylandRequest::DestroyLayerSurface { id: sid });
                                Ok(())
                            }) {
                                Ok(f) => {
                                    let _ = surface_tbl.set("destroy", f);
                                }
                                Err(e) => eprintln!("mplug: failed to create destroy fn: {e}"),
                            }

                            if let Err(e) = cb.call::<mlua::Table, ()>(surface_tbl) {
                                eprintln!("mplug: layer surface callback error: {e}");
                            }
                        }
                    }
                }
            }
            WaylandEvent::LayerSurfaceClosed { id } => {
                if let Ok(mplug_g) = lua.globals().get::<_, mlua::Table>("mplug") {
                    if let Ok(cbs) = mplug_g.get::<_, mlua::Table>("__surface_callbacks") {
                        let _ = cbs.set(*id, mlua::Value::Nil);
                    }
                }
            }
            WaylandEvent::Idled => {
                idle = true;
            }
            WaylandEvent::IdleResumed => {
                idle = false;
            }
            WaylandEvent::OutputPowerMode { on } => {
                output_power_on = *on;
            }
            _ => {}
        }

        let state_table = lua.create_table()?;
        state_table.set("tag_count", tag_count)?;
        state_table.set("layout_name", layout_name.clone())?;
        state_table.set("layout_index", layout_index)?;
        state_table.set("layout_symbol", layout_symbol.clone())?;
        state_table.set("idle", idle)?;
        state_table.set("output_power_on", output_power_on)?;

        let active_tags_tbl = lua.create_table()?;
        let mut active_idx: i64 = 1;
        let tags_tbl = lua.create_table()?;
        for (tag_num, (tag_state, clients, focused)) in &tag_states {
            if *tag_state == 1 {
                active_tags_tbl.set(active_idx, *tag_num)?;
                active_idx += 1;
            }
            let tag_info = lua.create_table()?;
            tag_info.set("state", *tag_state)?;
            tag_info.set("clients", *clients)?;
            tag_info.set("focused", *focused)?;
            tags_tbl.set(*tag_num, tag_info)?;
        }
        state_table.set("active_tags", active_tags_tbl)?;
        state_table.set("tags", tags_tbl)?;

        let toplevels_tbl = lua.create_table()?;
        let mut focused_window_info: Option<&crate::event::ToplevelInfo> = None;
        for (id, info) in &toplevel_states {
            let t = lua.create_table()?;
            t.set("title", info.title.clone())?;
            t.set("app_id", info.app_id.clone())?;
            t.set("activated", info.activated)?;
            t.set("minimized", info.minimized)?;
            t.set("maximized", info.maximized)?;
            t.set("fullscreen", info.fullscreen)?;
            toplevels_tbl.set(*id, t)?;
            if info.activated {
                focused_window_info = Some(info);
            }
        }
        state_table.set("toplevels", toplevels_tbl)?;

        let workspaces_tbl = lua.create_table()?;
        let mut ws_idx: i64 = 1;
        for (id, info) in &workspace_states {
            let t = lua.create_table()?;
            t.set("id", *id)?;
            t.set("name", info.name.clone())?;
            t.set("active", info.active)?;
            t.set("urgent", info.urgent)?;
            t.set("hidden", info.hidden)?;
            workspaces_tbl.set(ws_idx, t)?;
            ws_idx += 1;
        }
        state_table.set("workspaces", workspaces_tbl)?;

        let outputs_tbl = lua.create_table()?;
        let mut out_idx: i64 = 1;
        for (id, info) in &output_head_states {
            let t = lua.create_table()?;
            t.set("id", *id)?;
            t.set("name", info.name.clone())?;
            t.set("description", info.description.clone())?;
            t.set("x", info.x)?;
            t.set("y", info.y)?;
            t.set("enabled", info.enabled)?;
            t.set("width_px", info.width_px)?;
            t.set("height_px", info.height_px)?;
            t.set("refresh", info.refresh)?;
            t.set("scale", info.scale)?;
            t.set("transform", info.transform)?;
            t.set("width_mm", info.width_mm)?;
            t.set("height_mm", info.height_mm)?;
            outputs_tbl.set(out_idx, t)?;
            out_idx += 1;
        }
        state_table.set("outputs", outputs_tbl)?;

        if let Some(fw) = focused_window_info {
            let fw_tbl = lua.create_table()?;
            fw_tbl.set("title", fw.title.clone())?;
            fw_tbl.set("app_id", fw.app_id.clone())?;
            state_table.set("focused_window", fw_tbl)?;
        } else {
            state_table.set("focused_window", mlua::Value::Nil)?;
        }

        let listeners: mlua::Table = lua
            .globals()
            .get::<_, mlua::Table>("mplug")?
            .get("__listeners")?;
        for pair in listeners.pairs::<mlua::Value, mlua::Function>() {
            if let Ok((_, func)) = pair {
                if let Err(e) = func.call::<(mlua::Table, mlua::Table), ()>((
                    lua_event_table.clone(),
                    state_table.clone(),
                )) {
                    eprintln!("mplug: plugin listener error: {e}");
                }
            }
        }
    }

    Ok(())
}
