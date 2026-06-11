use crate::config;
use crate::event::{WaylandEvent, WaylandRequest};
use mlua::{Lua, Result as LuaResult};
use std::cell::{Cell, RefCell};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct TimerEntry {
    deadline: Instant,
    id: u64,
    interval: Option<Duration>,
    cb_key: mlua::RegistryKey,
}

impl PartialEq for TimerEntry {
    fn eq(&self, other: &Self) -> bool {
        self.deadline == other.deadline && self.id == other.id
    }
}
impl Eq for TimerEntry {}
impl PartialOrd for TimerEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for TimerEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.deadline
            .cmp(&other.deadline)
            .then(self.id.cmp(&other.id))
    }
}

struct TimerState {
    heap: BinaryHeap<Reverse<TimerEntry>>,
    cancelled: HashSet<u64>,
    next_id: u64,
}

impl TimerState {
    fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            cancelled: HashSet::new(),
            next_id: 0,
        }
    }

    fn add(&mut self, delay_ms: u64, interval_ms: Option<u64>, cb_key: mlua::RegistryKey) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.heap.push(Reverse(TimerEntry {
            deadline: Instant::now() + Duration::from_millis(delay_ms),
            id,
            interval: interval_ms.map(Duration::from_millis),
            cb_key,
        }));
        id
    }

    fn next_timeout(&self) -> Duration {
        match self.heap.peek() {
            Some(Reverse(entry)) => {
                let now = Instant::now();
                if entry.deadline <= now {
                    Duration::ZERO
                } else {
                    entry.deadline - now
                }
            }
            None => Duration::from_secs(3600),
        }
    }

    fn fire_expired(timers: &Rc<RefCell<Self>>, lua: &mlua::Lua) {
        let mut expired = Vec::new();
        let now = Instant::now();
        {
            let mut state = timers.borrow_mut();
            while let Some(Reverse(entry)) = state.heap.peek() {
                if entry.deadline > now {
                    break;
                }
                let Reverse(entry) = state.heap.pop().unwrap();
                if state.cancelled.remove(&entry.id) {
                    let _ = lua.remove_registry_value(entry.cb_key);
                    continue;
                }
                expired.push(entry);
            }
        }

        for entry in expired {
            if let Ok(f) = lua.registry_value::<mlua::Function>(&entry.cb_key) {
                if let Err(e) = f.call::<(), ()>(()) {
                    crate::log_error!("lua", "timer callback error: {e}");
                }
            }
            let mut state = timers.borrow_mut();
            if state.cancelled.remove(&entry.id) {
                let _ = lua.remove_registry_value(entry.cb_key);
                continue;
            }
            if let Some(interval) = entry.interval {
                state.heap.push(Reverse(TimerEntry {
                    deadline: entry.deadline + interval,
                    id: entry.id,
                    interval: entry.interval,
                    cb_key: entry.cb_key,
                }));
            } else {
                let _ = lua.remove_registry_value(entry.cb_key);
            }
        }
    }
}

fn json_to_lua<'a>(lua: &'a Lua, value: &serde_json::Value) -> LuaResult<mlua::Value<'a>> {
    match value {
        serde_json::Value::Null => Ok(mlua::Value::Nil),
        serde_json::Value::Bool(b) => Ok(mlua::Value::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(mlua::Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(mlua::Value::Number(f))
            } else {
                Ok(mlua::Value::Nil)
            }
        }
        serde_json::Value::String(s) => Ok(mlua::Value::String(lua.create_string(s)?)),
        serde_json::Value::Array(arr) => {
            let table = lua.create_table()?;
            for (i, v) in arr.iter().enumerate() {
                table.set(i + 1, json_to_lua(lua, v)?)?;
            }
            Ok(mlua::Value::Table(table))
        }
        serde_json::Value::Object(obj) => {
            let table = lua.create_table()?;
            for (k, v) in obj {
                table.set(k.as_str(), json_to_lua(lua, v)?)?;
            }
            Ok(mlua::Value::Table(table))
        }
    }
}

#[derive(Debug, PartialEq)]
enum DispatchOutcome {
    Error(String),
    Success(bool),
}

fn parse_dispatch_response(response: &str) -> DispatchOutcome {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(response) {
        if let Some(err) = val.get("error").and_then(|v| v.as_str()) {
            return DispatchOutcome::Error(err.to_string());
        }
        if let Some(success) = val.get("success").and_then(|v| v.as_bool()) {
            return DispatchOutcome::Success(success);
        }
    }
    DispatchOutcome::Success(true)
}

fn dispatch_ipc_helper<'a>(
    lua_ctx: &'a Lua,
    cmd: &str,
) -> LuaResult<(mlua::Value<'a>, mlua::Value<'a>)> {
    match crate::mango_ipc::send_command(cmd) {
        Ok(response) => match parse_dispatch_response(&response) {
            DispatchOutcome::Error(err) => Ok((
                mlua::Value::Nil,
                mlua::Value::String(lua_ctx.create_string(&err)?),
            )),
            DispatchOutcome::Success(ok) => Ok((mlua::Value::Boolean(ok), mlua::Value::Nil)),
        },
        Err(e) => Ok((
            mlua::Value::Nil,
            mlua::Value::String(lua_ctx.create_string(&e.to_string())?),
        )),
    }
}

pub fn run_lua(
    rx: Receiver<WaylandEvent>,
    tx: Sender<WaylandRequest>,
    event_tx: Sender<WaylandEvent>,
) -> LuaResult<()> {
    let lua = Lua::new();
    let mplug_table = lua.create_table()?;

    let timers = Rc::new(RefCell::new(TimerState::new()));

    let proc_next_id = Rc::new(Cell::new(0u64));

    let proc_callbacks: Rc<
        RefCell<
            std::collections::HashMap<u64, (Option<mlua::RegistryKey>, Option<mlua::RegistryKey>)>,
        >,
    > = Rc::new(RefCell::new(std::collections::HashMap::new()));

    let watch_next_id = Rc::new(Cell::new(0u64));
    let watch_callbacks: Rc<RefCell<std::collections::HashMap<u64, mlua::RegistryKey>>> =
        Rc::new(RefCell::new(std::collections::HashMap::new()));

    let t_every = Rc::clone(&timers);
    let every_fn = lua.create_function(move |lua_ctx, (ms, cb): (u64, mlua::Function)| {
        let cb_key = lua_ctx.create_registry_value(cb)?;
        let id = t_every.borrow_mut().add(ms, Some(ms), cb_key);

        let handle = lua_ctx.create_table()?;
        handle.set("id", id)?;

        let t_cancel = Rc::clone(&t_every);
        let cancel_fn = lua_ctx.create_function(move |_, ()| {
            t_cancel.borrow_mut().cancelled.insert(id);
            Ok(())
        })?;
        handle.set("cancel", cancel_fn)?;

        Ok(handle)
    })?;
    mplug_table.set("every", every_fn)?;

    let t_after = Rc::clone(&timers);
    let after_fn = lua.create_function(move |lua_ctx, (ms, cb): (u64, mlua::Function)| {
        let cb_key = lua_ctx.create_registry_value(cb)?;
        let id = t_after.borrow_mut().add(ms, None, cb_key);

        let handle = lua_ctx.create_table()?;
        handle.set("id", id)?;

        let t_cancel = Rc::clone(&t_after);
        let cancel_fn = lua_ctx.create_function(move |_, ()| {
            t_cancel.borrow_mut().cancelled.insert(id);
            Ok(())
        })?;
        handle.set("cancel", cancel_fn)?;

        Ok(handle)
    })?;
    mplug_table.set("after", after_fn)?;

    let log_fn = lua.create_function(|lua_ctx, msg: String| {
        let component = lua_ctx
            .inspect_stack(1)
            .and_then(|debug| {
                debug
                    .source()
                    .source
                    .map(|s| PathBuf::from(s.as_ref()).file_stem().map(|n| n.to_string_lossy().into_owned()))
            })
            .flatten()
            .unwrap_or_else(|| "lua".to_string());
        crate::log_info!(&component, "{}", msg);
        Ok(())
    })?;
    mplug_table.set("log", log_fn)?;

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
            _ => crate::log_error!("lua", "unknown Wayland dispatch command from Lua: {}", command),
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

    let ipc_get_fn = lua.create_function(|lua_ctx, command: String| {
        if command == "watch" || command.starts_with("watch ") {
            return Ok((
                mlua::Value::Nil,
                mlua::Value::String(lua_ctx.create_string(
                    "ipc_get is for one-shot get/dispatch; use mplug.watch(topic, callback) for streaming watches",
                )?),
            ));
        }
        let full_cmd = if command.starts_with("get ") || command.starts_with("dispatch ") {
            command
        } else {
            format!("get {}", command)
        };
        match crate::mango_ipc::send_command(&full_cmd) {
            Ok(response) => {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&response) {
                    match json_to_lua(lua_ctx, &val) {
                        Ok(lua_val) => Ok((lua_val, mlua::Value::Nil)),
                        Err(e) => Ok((mlua::Value::Nil, mlua::Value::String(lua_ctx.create_string(&e.to_string())?))),
                    }
                } else {
                    Ok((mlua::Value::String(lua_ctx.create_string(&response)?), mlua::Value::Nil))
                }
            }
            Err(e) => {
                Ok((mlua::Value::Nil, mlua::Value::String(lua_ctx.create_string(&e.to_string())?)))
            }
        }
    })?;
    mplug_table.set("ipc_get", ipc_get_fn)?;

    let ipc_dispatch_fn = lua.create_function(|lua_ctx, command: String| {
        let full_cmd = if command.starts_with("dispatch ") {
            command
        } else {
            format!("dispatch {}", command)
        };
        dispatch_ipc_helper(lua_ctx, &full_cmd)
    })?;
    mplug_table.set("ipc_dispatch", ipc_dispatch_fn)?;

    for (name, cmd) in [
        ("focus_last", "dispatch focuslast"),
        ("zoom", "dispatch zoom"),
        ("toggle_floating", "dispatch togglefloating"),
        ("toggle_fullscreen", "dispatch togglefullscreen"),
        ("center_window", "dispatch centerwin"),
        ("toggle_gaps", "dispatch togglegaps"),
        ("reload_config", "dispatch reload_config"),
        ("toggle_scratchpad", "dispatch toggle_scratchpad"),
    ] {
        let f = lua.create_function(move |lua_ctx, ()| dispatch_ipc_helper(lua_ctx, cmd))?;
        mplug_table.set(name, f)?;
    }

    let focus_dir_fn = lua.create_function(|lua_ctx, direction: String| {
        let cmd = format!("dispatch focusdir,{}", direction);
        dispatch_ipc_helper(lua_ctx, &cmd)
    })?;
    mplug_table.set("focus_dir", focus_dir_fn)?;

    let exchange_client_fn = lua.create_function(|lua_ctx, direction: String| {
        let cmd = format!("dispatch exchange_client,{}", direction);
        dispatch_ipc_helper(lua_ctx, &cmd)
    })?;
    mplug_table.set("exchange_client", exchange_client_fn)?;

    let inc_nmaster_fn = lua.create_function(|lua_ctx, n: i32| {
        let cmd = format!("dispatch incnmaster,{}", n);
        dispatch_ipc_helper(lua_ctx, &cmd)
    })?;
    mplug_table.set("inc_nmaster", inc_nmaster_fn)?;

    let set_mfact_fn = lua.create_function(|lua_ctx, fact: f32| {
        let cmd = format!("dispatch setmfact,{}", fact);
        dispatch_ipc_helper(lua_ctx, &cmd)
    })?;
    mplug_table.set("set_mfact", set_mfact_fn)?;

    let inc_gaps_fn = lua.create_function(|lua_ctx, n: i32| {
        let cmd = format!("dispatch incgaps,{}", n);
        dispatch_ipc_helper(lua_ctx, &cmd)
    })?;
    mplug_table.set("inc_gaps", inc_gaps_fn)?;

    let toggle_overview_fn = lua.create_function(|lua_ctx, n: i32| {
        let cmd = format!("dispatch toggleoverview,{}", n);
        dispatch_ipc_helper(lua_ctx, &cmd)
    })?;
    mplug_table.set("toggle_overview", toggle_overview_fn)?;

    let toggle_named_scratchpad_fn =
        lua.create_function(|lua_ctx, (name, cmd_str, rule): (String, String, String)| {
            let cmd = format!(
                "dispatch toggle_named_scratchpad,{},{},{}",
                name, cmd_str, rule
            );
            dispatch_ipc_helper(lua_ctx, &cmd)
        })?;
    mplug_table.set("toggle_named_scratchpad", toggle_named_scratchpad_fn)?;

    let proc_event_tx_spawn = event_tx.clone();
    let proc_id_rc = Rc::clone(&proc_next_id);
    let proc_cbs_spawn = Rc::clone(&proc_callbacks);
    let spawn_fn =
        lua.create_function(move |lua_ctx, (cmd, opts): (String, Option<mlua::Table>)| {
            let id = proc_id_rc.get();
            proc_id_rc.set(id + 1);

            let (args_vec, on_exit_key, on_stdout_key) = if let Some(opts) = opts {
                let args_raw: Vec<String> = opts
                    .get::<_, Option<mlua::Table>>("args")
                    .ok()
                    .flatten()
                    .map(|t| t.sequence_values::<String>().flatten().collect())
                    .unwrap_or_default();
                let on_exit_key = opts
                    .get::<_, Option<mlua::Function>>("on_exit")
                    .ok()
                    .flatten()
                    .map(|f| lua_ctx.create_registry_value(f))
                    .transpose()?;
                let on_stdout_key = opts
                    .get::<_, Option<mlua::Function>>("on_stdout")
                    .ok()
                    .flatten()
                    .map(|f| lua_ctx.create_registry_value(f))
                    .transpose()?;
                (args_raw, on_exit_key, on_stdout_key)
            } else {
                (vec![], None, None)
            };

            proc_cbs_spawn
                .borrow_mut()
                .insert(id, (on_exit_key, on_stdout_key));

            let child = Command::new(&cmd)
                .args(&args_vec)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| mlua::Error::RuntimeError(format!("mplug.spawn failed: {e}")))?;

            let pid = child.id();
            let child_arc = Arc::new(Mutex::new(child));

            let stdout = child_arc.lock().unwrap().stdout.take();
            if let Some(stdout) = stdout {
                let tx_out = proc_event_tx_spawn.clone();
                std::thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines().flatten() {
                        let _ = tx_out.send(crate::event::WaylandEvent::ProcessStdout { id, line });
                    }
                });
            }

            let tx_exit = proc_event_tx_spawn.clone();
            let child_arc_exit = Arc::clone(&child_arc);
            std::thread::spawn(move || {
                loop {
                    let status = child_arc_exit.lock().unwrap().try_wait();
                    match status {
                        Ok(Some(s)) => {
                            let _ = tx_exit.send(crate::event::WaylandEvent::ProcessExited {
                                id,
                                exit_code: s.code(),
                            });
                            break;
                        }
                        Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                        Err(_) => break,
                    }
                }
            });

            let handle = lua_ctx.create_table()?;
            handle.set("id", id)?;
            handle.set("pid", pid)?;

            let child_arc_kill = Arc::clone(&child_arc);
            let kill_fn = lua_ctx.create_function(move |_, ()| {
                let child_pid = child_arc_kill.lock().unwrap().id();
                let nix_pid = nix::unistd::Pid::from_raw(child_pid as i32);
                let _ = nix::sys::signal::kill(nix_pid, nix::sys::signal::Signal::SIGTERM);
                Ok(())
            })?;
            handle.set("kill", kill_fn)?;

            let pid_fn = lua_ctx.create_function(move |_, ()| Ok(pid))?;
            handle.set("pid_fn", pid_fn)?;

            Ok(handle)
        })?;
    mplug_table.set("spawn", spawn_fn)?;

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

    let watch_cbs_reg = Rc::clone(&watch_callbacks);
    let watch_next = Rc::clone(&watch_next_id);
    let watch_tx = event_tx.clone();
    let watch_fn = lua.create_function(move |lua_ctx, (topic, cb): (String, mlua::Function)| {
        let id = watch_next.get();
        watch_next.set(id + 1);
        let cb_key = lua_ctx.create_registry_value(cb)?;
        watch_cbs_reg.borrow_mut().insert(id, cb_key);
        crate::mango_ipc::start_callback_watch(topic, id, watch_tx.clone());
        Ok(())
    })?;
    mplug_table.set("watch", watch_fn)?;

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

    let cfg = config::load_config();
    let plugins_dir = config::get_config_dir().join("plugins");
    mplug_table.set("plugin_dir", plugins_dir.to_string_lossy().to_string())?;

    lua.globals().set("mplug", &mplug_table)?;

    let mut loaded: HashSet<PathBuf> = HashSet::new();
    for plugin_name in &cfg.enabled_plugins {
        let plugin_dir_path = plugins_dir.join(plugin_name);
        if plugin_dir_path.is_dir() {
            if let Some(path_str) = plugin_dir_path.to_str() {
                let package_table: mlua::Table = lua.globals().get("package")?;
                let cur_path: String = package_table.get("path")?;
                package_table.set("path", format!("{};{}/?.lua", cur_path, path_str))?;
            }
        }

        let targets = config::resolve_load_targets(&plugins_dir, plugin_name);
        if targets.is_empty() {
            crate::log_error!(
                "lua",
                "Enabled plugin '{}' could not be loaded (not found, or no entry_point/collection in mplug.toml) in {:?}",
                plugin_name,
                plugins_dir
            );
            continue;
        }

        for (member, target) in targets {
            let canon = fs::canonicalize(&target).unwrap_or_else(|_| target.clone());
            if !loaded.insert(canon) {
                continue;
            }
            match fs::read_to_string(&target) {
                Ok(script) => {
                    crate::log_info!("lua", "Loading plugin: {}", member);
                    let chunk_name = format!("@{}", target.display());
                    if let Err(e) = lua.load(&script).set_name(chunk_name).exec() {
                        crate::log_error!(&member, "load error: {}", e);
                    }
                }
                Err(e) => crate::log_error!(&member, "could not be read: {}", e),
            }
        }
    }

    let init_script = PathBuf::from("init.lua");
    if init_script.exists() {
        if let Ok(script) = fs::read_to_string(&init_script) {
            let _ = lua.load(&script).exec();
        }
    }

    crate::log_info!("lua", "Lua Event Engine running...");

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

    let mut keymode = String::new();
    let mut keyboard_layout = String::new();
    let mut ipc_monitors: Option<mlua::Value> = None;
    let mut ipc_clients: Option<mlua::Value> = None;
    let mut ipc_tags: Option<mlua::Value> = None;

    loop {
        let timeout = timers.borrow().next_timeout();
        let event = match rx.recv_timeout(timeout) {
            Ok(event) => event,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                TimerState::fire_expired(&timers, &lua);
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };

        if crate::logging::debug_enabled() {
            let mut desc = format!("{:?}", event);
            if desc.len() > 200 {
                let cut = (0..=200).rev().find(|&i| desc.is_char_boundary(i)).unwrap_or(0);
                desc.truncate(cut);
                desc.push('…');
            }
            crate::log_debug!("events", "dispatch {}", desc);
        }

        if let WaylandEvent::WatchUpdate { id, value } = &event {
            let cb = watch_callbacks
                .borrow()
                .get(id)
                .and_then(|key| lua.registry_value::<mlua::Function>(key).ok());
            if let Some(f) = cb {
                match json_to_lua(&lua, value) {
                    Ok(v) => {
                        if let Err(e) = f.call::<mlua::Value, ()>(v) {
                            crate::log_error!("lua", "watch callback error: {e}");
                        }
                    }
                    Err(e) => crate::log_error!("lua", "watch json convert error: {e}"),
                }
            }
            TimerState::fire_expired(&timers, &lua);
            continue;
        }

        let lua_event_table = lua.create_table()?;

        match &event {
            WaylandEvent::OutputTag {
                tag,
                state,
                clients,
                focused,
            } => {
                if let Err(e) = lua_event_table.set("type", "OutputTag") {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("tag", *tag) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("state", *state) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("clients", *clients) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("focused", *focused) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
            }
            WaylandEvent::TagsAmount(amount) => {
                if let Err(e) = lua_event_table.set("type", "TagsAmount") {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("amount", *amount) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
            }
            WaylandEvent::LayoutName(name) => {
                if let Err(e) = lua_event_table.set("type", "LayoutName") {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("name", name.clone()) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
            }
            WaylandEvent::OutputLayout(layout) => {
                if let Err(e) = lua_event_table.set("type", "OutputLayout") {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("layout", *layout) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
            }
            WaylandEvent::ToplevelUpdated { id, info } => {
                if let Err(e) = lua_event_table.set("type", "ToplevelUpdated") {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("id", *id) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("title", info.title.clone()) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("app_id", info.app_id.clone()) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("activated", info.activated) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("minimized", info.minimized) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("maximized", info.maximized) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("fullscreen", info.fullscreen) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
            }
            WaylandEvent::ToplevelClosed { id } => {
                if let Err(e) = lua_event_table.set("type", "ToplevelClosed") {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("id", *id) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
            }
            WaylandEvent::WorkspaceUpdated { id, info } => {
                if let Err(e) = lua_event_table.set("type", "WorkspaceUpdated") {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("id", *id) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("name", info.name.clone()) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("active", info.active) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("urgent", info.urgent) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("hidden", info.hidden) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
            }
            WaylandEvent::WorkspaceClosed { id } => {
                if let Err(e) = lua_event_table.set("type", "WorkspaceClosed") {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("id", *id) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
            }
            WaylandEvent::OutputHeadUpdated { id, info } => {
                if let Err(e) = lua_event_table.set("type", "OutputHeadUpdated") {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("id", *id) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("name", info.name.clone()) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("enabled", info.enabled) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("width_px", info.width_px) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("height_px", info.height_px) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("refresh", info.refresh) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("scale", info.scale) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("x", info.x) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("y", info.y) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
            }
            WaylandEvent::OutputHeadRemoved { id } => {
                if let Err(e) = lua_event_table.set("type", "OutputHeadRemoved") {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("id", *id) {
                    crate::log_error!("lua", "failed to set event field: {e}");
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
            WaylandEvent::ProcessExited { id, exit_code } => {
                let _ = lua_event_table.set("type", "ProcessExited");
                let _ = lua_event_table.set("id", *id);
                if let Some(code) = exit_code {
                    let _ = lua_event_table.set("exit_code", *code);
                }
            }
            WaylandEvent::ProcessStdout { id, line } => {
                let _ = lua_event_table.set("type", "ProcessStdout");
                let _ = lua_event_table.set("id", *id);
                let _ = lua_event_table.set("line", line.clone());
            }
            WaylandEvent::UserCommand(name) => {
                let _ = lua_event_table.set("type", "UserCommand");
                let _ = lua_event_table.set("name", name.clone());
            }
            WaylandEvent::Idled => {
                if let Err(e) = lua_event_table.set("type", "Idled") {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
            }
            WaylandEvent::IdleResumed => {
                if let Err(e) = lua_event_table.set("type", "IdleResumed") {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
            }
            WaylandEvent::OutputPowerMode { on } => {
                if let Err(e) = lua_event_table.set("type", "OutputPowerMode") {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
                if let Err(e) = lua_event_table.set("on", *on) {
                    crate::log_error!("lua", "failed to set event field: {e}");
                }
            }
            WaylandEvent::IpcKeyMode(mode) => {
                let _ = lua_event_table.set("type", "IpcKeyMode");
                let _ = lua_event_table.set("keymode", mode.clone());
            }
            WaylandEvent::IpcKeyboardLayout(layout) => {
                let _ = lua_event_table.set("type", "IpcKeyboardLayout");
                let _ = lua_event_table.set("layout", layout.clone());
            }
            WaylandEvent::IpcMonitors(val) => {
                let _ = lua_event_table.set("type", "IpcMonitors");
                if let Ok(lua_val) = json_to_lua(&lua, val) {
                    let _ = lua_event_table.set("data", lua_val);
                }
            }
            WaylandEvent::IpcClients(val) => {
                let _ = lua_event_table.set("type", "IpcClients");
                if let Ok(lua_val) = json_to_lua(&lua, val) {
                    let _ = lua_event_table.set("data", lua_val);
                }
            }
            WaylandEvent::IpcTags(val) => {
                let _ = lua_event_table.set("type", "IpcTags");
                if let Ok(lua_val) = json_to_lua(&lua, val) {
                    let _ = lua_event_table.set("data", lua_val);
                }
            }
            _ => {
                if let Err(e) = lua_event_table.set("type", "Generic") {
                    crate::log_error!("lua", "failed to set event field: {e}");
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
                        Err(e) => crate::log_error!("lua", "failed to create surface table: {e}"),
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
                                Err(e) => crate::log_error!("lua", "failed to create fill fn: {e}"),
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
                                Err(e) => crate::log_error!("lua", "failed to create destroy fn: {e}"),
                            }

                            if let Err(e) = cb.call::<mlua::Table, ()>(surface_tbl) {
                                crate::log_error!("lua", "layer surface callback error: {e}");
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
            WaylandEvent::ProcessExited { id, exit_code } => {
                let (f, key) = {
                    let mut cbs = proc_callbacks.borrow_mut();
                    if let Some((on_exit_key, _)) = cbs.remove(id) {
                        if let Some(key) = on_exit_key {
                            let f = lua.registry_value::<mlua::Function>(&key).ok();
                            (f, Some(key))
                        } else {
                            (None, None)
                        }
                    } else {
                        (None, None)
                    }
                };

                if let Some(f) = f {
                    let code_val: mlua::Value = match exit_code {
                        Some(c) => mlua::Value::Integer(*c as i64),
                        None => mlua::Value::Nil,
                    };
                    if let Err(e) = f.call::<mlua::Value, ()>(code_val) {
                        crate::log_error!("lua", "process on_exit error: {e}");
                    }
                }

                if let Some(key) = key {
                    let _ = lua.remove_registry_value(key);
                }
            }
            WaylandEvent::ProcessStdout { id, line } => {
                let f = {
                    let cbs = proc_callbacks.borrow();
                    cbs.get(id)
                        .and_then(|(_, stdout_key)| stdout_key.as_ref())
                        .and_then(|key| lua.registry_value::<mlua::Function>(key).ok())
                };
                if let Some(f) = f {
                    if let Err(e) = f.call::<String, ()>(line.clone()) {
                        crate::log_error!("lua", "process on_stdout error: {e}");
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
            WaylandEvent::IpcKeyMode(mode) => {
                keymode = mode.clone();
            }
            WaylandEvent::IpcKeyboardLayout(layout) => {
                keyboard_layout = layout.clone();
            }
            WaylandEvent::IpcMonitors(val) => {
                ipc_monitors = json_to_lua(&lua, val).ok();
            }
            WaylandEvent::IpcClients(val) => {
                ipc_clients = json_to_lua(&lua, val).ok();
            }
            WaylandEvent::IpcTags(val) => {
                ipc_tags = json_to_lua(&lua, val).ok();
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
        state_table.set("keymode", keymode.clone())?;
        state_table.set("keyboard_layout", keyboard_layout.clone())?;
        for (key, cached) in [
            ("ipc_monitors", &ipc_monitors),
            ("ipc_clients", &ipc_clients),
            ("ipc_tags", &ipc_tags),
        ] {
            if let Some(v) = cached {
                state_table.set(key, v.clone())?;
            }
        }

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
                    crate::log_error!("lua", "plugin listener error: {e}");
                }
            }
        }

        TimerState::fire_expired(&timers, &lua);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noop_cb_key(lua: &Lua) -> mlua::RegistryKey {
        let f = lua.create_function(|_, ()| Ok(())).unwrap();
        lua.create_registry_value(f).unwrap()
    }

    #[test]
    fn cancelled_timer_id_is_reaped_from_cancelled_set() {
        let lua = Lua::new();
        let timers = Rc::new(RefCell::new(TimerState::new()));
        let id = timers.borrow_mut().add(0, None, noop_cb_key(&lua));
        timers.borrow_mut().cancelled.insert(id);

        TimerState::fire_expired(&timers, &lua);

        let state = timers.borrow();
        assert!(state.heap.is_empty());
        assert!(
            state.cancelled.is_empty(),
            "reaping a cancelled timer must also drop its id from the cancelled set"
        );
    }

    #[test]
    fn interval_timer_cancelled_during_callback_is_reaped() {
        let lua = Lua::new();
        let timers = Rc::new(RefCell::new(TimerState::new()));
        let t = Rc::clone(&timers);
        let f = lua
            .create_function(move |_, ()| {
                t.borrow_mut().cancelled.insert(0);
                Ok(())
            })
            .unwrap();
        let key = lua.create_registry_value(f).unwrap();
        let id = timers.borrow_mut().add(0, Some(60_000), key);
        assert_eq!(id, 0);

        TimerState::fire_expired(&timers, &lua);

        let state = timers.borrow();
        assert!(
            state.heap.is_empty(),
            "an interval timer cancelled from its own callback must not reschedule"
        );
        assert!(
            state.cancelled.is_empty(),
            "reaping a cancelled timer must also drop its id from the cancelled set"
        );
    }

    #[test]
    fn dispatch_response_json_error_field_is_error() {
        assert_eq!(
            parse_dispatch_response(r#"{"error":"unknown dispatcher"}"#),
            DispatchOutcome::Error("unknown dispatcher".to_string())
        );
    }

    #[test]
    fn dispatch_response_success_bool_passes_through() {
        assert_eq!(
            parse_dispatch_response(r#"{"success":false}"#),
            DispatchOutcome::Success(false)
        );
        assert_eq!(
            parse_dispatch_response(r#"{"success":true}"#),
            DispatchOutcome::Success(true)
        );
    }

    #[test]
    fn dispatch_response_other_json_is_success() {
        assert_eq!(
            parse_dispatch_response(r#"{"clients":[]}"#),
            DispatchOutcome::Success(true)
        );
    }

    #[test]
    fn dispatch_response_non_json_is_success() {
        assert_eq!(
            parse_dispatch_response("ok"),
            DispatchOutcome::Success(true)
        );
    }
}
