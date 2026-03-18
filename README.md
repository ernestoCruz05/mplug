# mplug

A Lua plugin manager and runtime daemon for MangoWM Wayland compositors.

mplug bridges the `zdwl_ipc` Wayland protocol into an embedded Lua 5.4 environment, allowing users to write plugins that react to compositor events and dispatch window, layout, and output commands — without modifying compositor source code. Plugins are installed from git repositories, validated against a manifest, and hot-reloaded on daemon restart.

---

## Examples

The `examples/` directory contains ready-to-use plugins that demonstrate capabilities that cannot be replicated with a static compositor configuration:

| File | What it does |
|---|---|
| `autotile.lua` | Switches between monocle and tile automatically based on runtime window count per tag |
| `focus-history.lua` | When the focused window closes, focus is returned to the previously focused window |
| `powersave.lua` | On idle: launches a screen locker and blanks the display; restores on activity |
| `output-hotplug.lua` | Configures a monitor automatically the moment it is connected at runtime |
| `urgent-follow.lua` | Switches to a tag with an urgent window when the current tag is idle |

To use an example, copy it to `~/.config/mplug/plugins/`, then run `mplug enable <name>`.

---

## Table of Contents

- [Requirements](#requirements)
- [Installation](#installation)
- [Getting Started](#getting-started)
- [CLI Reference](#cli-reference)
- [Writing Plugins](#writing-plugins)
  - [Single-file plugins](#single-file-plugins)
  - [Directory plugins](#directory-plugins)
  - [Manifest format and validation](#manifest-format-and-validation)
  - [Collections](#collections)
  - [Multi-file plugins and require](#multi-file-plugins-and-require)
- [Plugin API Reference](#plugin-api-reference)
  - [mplug.add_listener](#mplugadd_listener)
  - [mplug.dispatch](#mplugdispatch)
  - [mplug.exec](#mplugexec)
  - [Window management](#window-management)
  - [Output management](#output-management)
  - [Layer shell](#layer-shell)
  - [Timers](#timers)
  - [Process lifecycle](#process-lifecycle)
- [Event Reference](#event-reference)
- [State Snapshot Reference](#state-snapshot-reference)
- [Socket IPC](#socket-ipc)
- [Plugin Discovery and Loading](#plugin-discovery-and-loading)
- [Error Handling](#error-handling)

---

## Requirements

- Rust toolchain (stable, 2024 edition)
- Git (required for `mplug add`, `mplug update`, `mplug outdated`)
- MangoWM or MangoWC Wayland compositor supporting the following protocols (they should come with mangowm):
  - `dwl-ipc-unstable-v2` (`zdwl_ipc_manager_v2`)
  - `ext-foreign-toplevel-list-v1`
  - `ext-idle-notify-v1`
  - `wlr-output-power-management-unstable-v1`
  - `ext-workspace-v1`
  - `wlr-output-management-unstable-v1`
  - `wlr-layer-shell-unstable-v1`

---

## Installation

```
git clone https://github.com/ernestoCruz05/mplug.git
cd mplug
cargo build --release
sudo cp target/release/mplug /usr/local/bin/
```

Alternatively, you can use `cargo install` which handles the binary placement automatically:

```
git clone https://github.com/ernestoCruz05/mplug.git
cd mplug
cargo install --path .
``` 
This installs the `mplug` binary to `~/.cargo/bin/`. Ensure this directory is in your `PATH`:
```
export PATH="HOME/.cargo/bin:HOME/.cargo/bin:HOME/.cargo/bin:PATH"
```
Add this line to your `~/.bashrc`, `~/.zshrc`, or shell config to make it permanent. This method is recommended for NixOS and other systems where writing to `/usr/local/bin` is restricted.


---

## Getting Started

Add the daemon to your compositor's autostart configuration:

```
mplug daemon
```

Install and enable a plugin:

```
mplug add https://github.com/user/my-plugin
mplug enable my-plugin
```

Restart the daemon for the plugin to be loaded.

---

## CLI Reference

### `mplug daemon`

Starts the background event loop. Spawns three threads:

- **Wayland thread**: connects to the compositor, listens for protocol events, dispatches requests.
- **Lua thread**: initializes the Lua VM, loads enabled plugins, broadcasts events to registered listeners.
- **Socket thread**: listens on `/tmp/mplug.sock` for IPC commands from external tools or keybinds.

This command is typically placed in the compositor autostart configuration, not run interactively.

---

### `mplug add <repo>`

Clones a plugin from a git URL into `~/.config/mplug/plugins/`. The target directory name is derived from the last path segment of the URL (`.git` suffix is stripped).

```
mplug add https://github.com/user/my-plugin
mplug add https://github.com/user/my-plugin.git
```

After cloning, mplug reads and validates the `mplug.toml` manifest. If the manifest is absent or invalid, the cloned directory is deleted and an error is printed. The plugin is not automatically enabled; run `mplug enable <name>` to activate it.

---

### `mplug enable <name>`

Adds a plugin to the `enabled_plugins` set in `~/.config/mplug/mplug.toml`. The plugin must already be installed. Changes take effect the next time `mplug daemon` is started.

```
mplug enable my-plugin
```

---

### `mplug disable <name>`

Removes a plugin from the `enabled_plugins` set. The plugin files remain on disk.

```
mplug disable my-plugin
```

---

### `mplug list`

Prints all installed plugins with their enabled or disabled status. Collection members show a `(via <repo>)` annotation so you know which repository they belong to and how to update them. Collection repositories themselves are listed separately with a `collection` label and a reminder of the update command.

---

### `mplug remove <name>`

Removes an installed plugin from disk and disables it. For collection repositories, also removes all member symlinks and removes each member from the enabled set.

```
mplug remove my-plugin
mplug remove my-bundle      # also removes carousel.lua, all-float.lua, etc.
```

---

### `mplug update <name>`

Runs `git pull` inside the plugin's directory. Requires the plugin to have been installed via `mplug add`.

```
mplug update my-plugin
```

---

### `mplug outdated`

For each plugin directory under `~/.config/mplug/plugins/`, runs `git fetch` and then reports how many commits behind the local HEAD is relative to the upstream tracking branch. Plugins that are up to date are shown with a check mark; outdated plugins show the commit count.

---

## Writing Plugins

Plugins are Lua 5.4 scripts. At daemon startup, mplug executes each enabled plugin's entry point. Plugins register event listeners using `mplug.add_listener()`. Every Wayland event from the compositor calls all registered listeners with an event table and a state snapshot table.

### Single-file plugins

A single `.lua` file placed in `~/.config/mplug/plugins/`:

```
~/.config/mplug/plugins/autotile.lua
```

No manifest is needed for single-file plugins. The plugin is referenced by its filename without the `.lua` extension.

```lua
-- ~/.config/mplug/plugins/autotile.lua

mplug.add_listener(function(event, state)
    if event.type == "OutputTag" then
        if event.clients == 1 then
            mplug.dispatch("set_layout 3")  -- monocle when one client
        elseif event.clients > 1 then
            mplug.dispatch("set_layout 0")  -- tile layout otherwise
        end
    end
end)
```

### Directory plugins

A directory plugin lives under `~/.config/mplug/plugins/<name>/` and must contain a `mplug.toml` manifest. The directory name is the plugin name used in `mplug enable` / `mplug disable`.

```
~/.config/mplug/plugins/
  my-plugin/
    mplug.toml
    init.lua
    helpers.lua
```

### Manifest format and validation

The `mplug.toml` manifest is a TOML file at the root of the plugin directory. A standard single-plugin manifest has three fields:

```toml
name        = "my-plugin"
version     = "0.1.0"
entry_point = "init.lua"
```

**Field definitions:**

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | always | Human-readable plugin name. Used in error messages and logging. |
| `version` | string | always | Plugin version. No format is enforced; any non-empty string is accepted. |
| `entry_point` | string | unless collection | Path to the Lua entry point, relative to the plugin directory. |

**Validation rules:**

mplug applies two stages of validation when a plugin is installed via `mplug add` and again when loaded at daemon startup:

1. **TOML parse**: The file must be valid TOML. Any syntax error causes validation to fail.
2. **Non-empty check**: `name` and `version` are trimmed of whitespace and must be non-empty. `entry_point` must be non-empty unless a `[collection]` section is present (see below).

Examples of invalid manifests:

```toml
# Fails: entry_point is missing and no collection section
name    = "my-plugin"
version = "0.1.0"
```

```toml
# Fails: name is whitespace-only (trimmed to empty)
name        = "   "
version     = "0.1.0"
entry_point = "init.lua"
```

When validation fails during `mplug add`, the cloned directory is removed and the error is printed. When validation fails at daemon startup (for example if a manifest was edited after installation), the plugin is skipped and a warning is printed to stderr; other plugins continue to load normally.

### Collections

A collection is a single repository that ships multiple independent plugins. Instead of `entry_point`, the manifest declares a `[collection]` section listing plugin names. Each name must correspond to a `.lua` file of the same name in the repository root.

```toml
name    = "my-bundle"
version = "1.0.0"

[collection]
plugins = ["carousel", "all-float", "autotile"]
```

When `mplug add` clones a collection repository, it creates a symlink in `~/.config/mplug/plugins/` for each member (`carousel.lua`, `all-float.lua`, `autotile.lua` pointing into the cloned directory). Each member then behaves exactly like a standalone single-file plugin: it can be enabled, disabled, and listed independently.

```
mplug add https://github.com/user/my-bundle
# → Added collection: my-bundle
#   → mplug enable carousel
#   → mplug enable all-float
#   → mplug enable autotile

mplug enable carousel
mplug enable autotile
```

`mplug update my-bundle` updates the cloned git repository; all symlinks remain valid because they point into it.

### Multi-file plugins and require

When a directory plugin is loaded, its directory is prepended to Lua's `package.path` using the pattern `<plugin-dir>/?.lua`. This allows the plugin to use `require()` to load sibling files:

```lua
-- init.lua
local helpers = require("helpers")   -- loads helpers.lua from the plugin directory
```

Standard Lua module conventions apply. The entry point is executed first; any `require()` calls inside it resolve relative to the plugin directory.

---

## Plugin API Reference

The global `mplug` table is available to all plugins. It must not be reassigned or modified at the top level.

### mplug.add_listener

```lua
mplug.add_listener(function(event, state) ... end)
```

Registers a function to be called on every Wayland event. Multiple listeners can be registered; they are called in registration order. Both arguments are plain Lua tables.

- `event`: describes the event that just occurred (see [Event Reference](#event-reference))
- `state`: a snapshot of the full compositor state at the time of the event (see [State Snapshot Reference](#state-snapshot-reference))

The listener runs synchronously in the Lua thread. Avoid blocking operations inside listeners.

---

### mplug.dispatch

```lua
mplug.dispatch(command)
```

Sends a command string to the Wayland thread. The following command strings are supported:

**`set_layout <index>`**

Switch the active layout to the layout at the given zero-based index.

```lua
mplug.dispatch("set_layout 0")   -- first layout
mplug.dispatch("set_layout 3")   -- fourth layout (monocle in typical setups)
```

**`set_tags <tagmask>`**

Set the active tag set to the given bitmask. Tag 1 = bit 0, tag 2 = bit 1, and so on.

```lua
mplug.dispatch("set_tags 1")    -- show tag 1 only
mplug.dispatch("set_tags 255")  -- show all 8 tags
```

**`set_client_tags <and_tags> <xor_tags>`**

Modify the tag assignment of the currently focused window using bitmask arithmetic. The new tagmask is computed as `(current AND and_tags) XOR xor_tags`.

```lua
-- Move focused window to tag 1 only:
mplug.dispatch("set_client_tags 0 1")

-- Toggle tag 3 on the focused window:
mplug.dispatch("set_client_tags 4294967295 4")  -- 0xFFFFFFFF AND then XOR bit 2
```

**`set_window_tag <id> <tagmask>`**

Move the window identified by `id` to the given tagmask. The `id` corresponds to the protocol-level toplevel ID reported in `ToplevelUpdated` events and the `state.toplevels` table.

```lua
mplug.dispatch("set_window_tag 5 2")  -- move window 5 to tag 2
```

Unknown or malformed commands are logged to stderr and ignored.

---

### mplug.exec

```lua
local stdout, exit_code = mplug.exec(shell_command)
```

Runs a shell command via `/bin/sh -c` and returns two values: the trimmed stdout as a string, and the exit code as an integer. If the process cannot be spawned, a Lua runtime error is raised.

```lua
local out, code = mplug.exec("date +%H")
local hour = tonumber(out)
```

---

### Window management

**`mplug.focus_window(id)`**

Activates (gives keyboard focus to) the window with the given protocol ID.

```lua
mplug.focus_window(event.id)
```

**`mplug.close_window(id)`**

Requests that the window with the given protocol ID be closed.

```lua
mplug.close_window(event.id)
```

**`mplug.set_window_minimized(id, minimized)`**

Minimizes or restores a window.

```lua
mplug.set_window_minimized(event.id, true)   -- minimize
mplug.set_window_minimized(event.id, false)  -- restore
```

**`mplug.set_window_tag(id, tagmask)`**

Moves a window to the given tag bitmask. Equivalent to `mplug.dispatch("set_window_tag <id> <tagmask>")` but takes numeric arguments directly.

```lua
mplug.set_window_tag(event.id, 1)  -- move to tag 1
```

**`mplug.set_client_tags(and_tags, xor_tags)`**

Modifies the tag assignment of the currently focused window. Equivalent to `mplug.dispatch("set_client_tags ...")` but takes numeric arguments directly.

```lua
mplug.set_client_tags(0xFFFFFFFF, 4)  -- toggle tag 3 on focused window
```

---

### Output management

**`mplug.set_output_power(on)`**

Turns display power on or off.

```lua
mplug.set_output_power(false)  -- blank display
mplug.set_output_power(true)   -- unblank display
```

**`mplug.set_output_mode(head_name, width, height, refresh)`**

Sets the resolution and refresh rate of the named output head. `refresh` is in millihertz (e.g., `60000` for 60 Hz).

```lua
mplug.set_output_mode("HDMI-A-1", 1920, 1080, 60000)
```

**`mplug.set_output_position(head_name, x, y)`**

Sets the position of the named output in the compositor's global coordinate space.

```lua
mplug.set_output_position("HDMI-A-1", 1920, 0)
```

**`mplug.set_output_scale(head_name, scale)`**

Sets the output scale factor as a floating-point number.

```lua
mplug.set_output_scale("eDP-1", 2.0)
```

**`mplug.set_output_enabled(head_name, enabled)`**

Enables or disables the named output head.

```lua
mplug.set_output_enabled("HDMI-A-1", false)
```

The `head_name` string for all output functions corresponds to the `name` field of `OutputHeadUpdated` events and the `name` field in the `state.outputs` table.

---

### Layer shell

`mplug.create_layer_surface(config, callback)` creates a `wlr-layer-shell-unstable-v1` surface and calls `callback` with a surface handle once the compositor has configured it.

**Config table fields:**

| Field | Type | Default | Description |
|---|---|---|---|
| `width` | integer | `200` | Surface width in pixels |
| `height` | integer | `30` | Surface height in pixels |
| `anchor` | string | `""` | Anchor edges, any combination of `"top"`, `"bottom"`, `"left"`, `"right"` |
| `layer` | string | `"top"` | Compositor layer: `"background"`, `"bottom"`, `"top"`, or `"overlay"` |
| `exclusive_zone` | integer | `0` | Exclusive zone in pixels; positive value reserves screen space |

The `anchor` string is parsed for substring matches. To anchor to the top edge, include `"top"` anywhere in the string. To anchor to multiple edges use a space-separated string such as `"top left"`.

**Surface handle methods (available inside the callback):**

| Method | Signature | Description |
|---|---|---|
| `fill` | `surface:fill(r, g, b, a)` | Fills the surface with a solid RGBA color (values 0.0 to 1.0) |
| `destroy` | `surface:destroy()` | Destroys the surface and releases Wayland resources |

The callback is called exactly once, immediately after the compositor sends the `Configure` event. After the callback returns, the handle remains valid until `surface:destroy()` is called or the compositor closes the surface.

```lua
mplug.create_layer_surface({
    width         = 1920,
    height        = 30,
    anchor        = "top left right",
    layer         = "top",
    exclusive_zone = 30,
}, function(surface)
    surface:fill(0.1, 0.1, 0.1, 0.9)  -- dark translucent bar
end)
```

The `LayerSurfaceConfigured` and `LayerSurfaceClosed` events are also delivered to all registered listeners so plugins can react to surface lifecycle changes.

---

### Timers

**`mplug.every(ms, fn)`**

Registers a recurring timer that calls `fn` approximately every `ms` milliseconds. Returns a handle table with a `:cancel()` method and an `id` field.

```lua
local t = mplug.every(5000, function()
    local out, _ = mplug.exec("date +%H:%M")
    print("time:", out)
end)

-- later:
t:cancel()
```

Timers are checked on every event loop tick. The callback is called synchronously in the Lua thread; avoid blocking operations inside timer callbacks. Recurring timers reschedule based on the original deadline to avoid drift.

**`mplug.after(ms, fn)`**

Registers a one-shot timer that calls `fn` once after `ms` milliseconds. Returns a handle table with a `:cancel()` method and an `id` field. The callback is not called if `:cancel()` is called before the timer fires.

```lua
local t = mplug.after(2000, function()
    mplug.set_output_power(false)
end)

-- cancel before it fires:
t:cancel()
```

---

### Process lifecycle

**`mplug.spawn(cmd, opts)`**

Spawns an external process and returns a handle table. `cmd` is the executable path or name. `opts` is an optional table with the following fields:

| Field | Type | Description |
|---|---|---|
| `args` | array of strings | Command-line arguments |
| `on_exit` | function | Called when the process exits: `on_exit(id, exit_code)`. `exit_code` is an integer or `nil` if the process was signalled |
| `on_stdout` | function | Called for each line of stdout output: `on_stdout(id, line)` |

The returned handle table has:

| Field/Method | Description |
|---|---|
| `id` | mplug-internal process ID (`integer`) |
| `pid` | OS process ID (`integer`) |
| `:kill()` | Sends SIGTERM to the process |

Stderr is discarded. Stdout is read line-by-line in a background thread; each line triggers `on_stdout` in the Lua thread. Exit is detected by polling `try_wait` every 100 ms; `on_exit` is called in the Lua thread after the process terminates.

```lua
local proc = mplug.spawn("my-script", {
    args = { "--flag", "value" },
    on_exit = function(id, code)
        print("process", id, "exited with", code)
    end,
    on_stdout = function(id, line)
        print("output:", line)
    end,
})

-- kill it later:
proc:kill()
```

The `ProcessExited` and `ProcessStdout` events are also delivered to all registered listeners (see [Event Reference](#event-reference)).

---

## Event Reference

Every listener receives an `event` table as its first argument. The `event.type` field is always present and identifies the event. Additional fields depend on the event type.

### `OutputTag`

Fired for each tag on each output. Reports the tag's current state from the compositor.

| Field | Type | Description |
|---|---|---|
| `type` | string | `"OutputTag"` |
| `tag` | integer | Tag number (1-based) |
| `state` | integer | `0` = inactive, `1` = active, `2` = urgent |
| `clients` | integer | Number of clients on this tag |
| `focused` | integer | `0` if no client on this tag has focus, non-zero otherwise |

### `TagsAmount`

Reports the total number of tags configured on an output.

| Field | Type | Description |
|---|---|---|
| `type` | string | `"TagsAmount"` |
| `amount` | integer | Total number of tags |

### `LayoutName`

Reports the name of the currently active layout.

| Field | Type | Description |
|---|---|---|
| `type` | string | `"LayoutName"` |
| `name` | string | Layout name string |

### `OutputLayout`

Reports the index of the currently active layout.

| Field | Type | Description |
|---|---|---|
| `type` | string | `"OutputLayout"` |
| `layout` | integer | Zero-based layout index |

### `ToplevelUpdated`

Fired when a window (toplevel) is created or its properties change.

| Field | Type | Description |
|---|---|---|
| `type` | string | `"ToplevelUpdated"` |
| `id` | integer | Protocol-level window ID |
| `title` | string | Window title |
| `app_id` | string | Application ID (e.g., `"foot"`, `"firefox"`) |
| `activated` | boolean | Whether this window currently has keyboard focus |
| `minimized` | boolean | Whether the window is minimized |
| `maximized` | boolean | Whether the window is maximized |
| `fullscreen` | boolean | Whether the window is fullscreen |

### `ToplevelClosed`

Fired when a window is destroyed.

| Field | Type | Description |
|---|---|---|
| `type` | string | `"ToplevelClosed"` |
| `id` | integer | Protocol-level window ID of the closed window |

### `WorkspaceUpdated`

Fired when a workspace is created or its state changes.

| Field | Type | Description |
|---|---|---|
| `type` | string | `"WorkspaceUpdated"` |
| `id` | integer | Protocol-level workspace ID |
| `name` | string | Workspace name |
| `active` | boolean | Whether this workspace is currently active |
| `urgent` | boolean | Whether this workspace has an urgent client |
| `hidden` | boolean | Whether this workspace is hidden |

### `WorkspaceClosed`

Fired when a workspace is removed.

| Field | Type | Description |
|---|---|---|
| `type` | string | `"WorkspaceClosed"` |
| `id` | integer | Protocol-level workspace ID |

### `OutputHeadUpdated`

Fired when an output head is added or its properties change (resolution, position, scale, etc.).

| Field | Type | Description |
|---|---|---|
| `type` | string | `"OutputHeadUpdated"` |
| `id` | integer | Protocol-level head ID |
| `name` | string | Connector name (e.g., `"HDMI-A-1"`, `"eDP-1"`) |
| `enabled` | boolean | Whether the output is currently active |
| `width_px` | integer | Horizontal resolution in pixels |
| `height_px` | integer | Vertical resolution in pixels |
| `refresh` | integer | Refresh rate in millihertz (e.g., `60000` = 60 Hz) |
| `scale` | number | Output scale factor |
| `x` | integer | Horizontal position in global compositor space |
| `y` | integer | Vertical position in global compositor space |

### `OutputHeadRemoved`

Fired when an output head is disconnected or removed.

| Field | Type | Description |
|---|---|---|
| `type` | string | `"OutputHeadRemoved"` |
| `id` | integer | Protocol-level head ID |

### `Idled`

Fired when the idle timer expires and the session enters idle state.

| Field | Type | Description |
|---|---|---|
| `type` | string | `"Idled"` |

### `IdleResumed`

Fired when user activity ends the idle state.

| Field | Type | Description |
|---|---|---|
| `type` | string | `"IdleResumed"` |

### `OutputPowerMode`

Fired when the display power state changes.

| Field | Type | Description |
|---|---|---|
| `type` | string | `"OutputPowerMode"` |
| `on` | boolean | `true` if the display was turned on, `false` if turned off |

### `LayerSurfaceConfigured`

Fired when the compositor has configured a layer shell surface created by `mplug.create_layer_surface()`.

| Field | Type | Description |
|---|---|---|
| `type` | string | `"LayerSurfaceConfigured"` |
| `id` | integer | Surface ID (as assigned by mplug internally) |
| `width` | integer | Configured width in pixels |
| `height` | integer | Configured height in pixels |

### `LayerSurfaceClosed`

Fired when the compositor has closed a layer shell surface.

| Field | Type | Description |
|---|---|---|
| `type` | string | `"LayerSurfaceClosed"` |
| `id` | integer | Surface ID |

### `ProcessExited`

Fired when a process spawned by `mplug.spawn()` exits.

| Field | Type | Description |
|---|---|---|
| `type` | string | `"ProcessExited"` |
| `id` | integer | mplug-internal process ID |
| `exit_code` | integer or nil | Exit code, or `nil` if the process was killed by a signal |

### `ProcessStdout`

Fired once per line of stdout output from a process spawned by `mplug.spawn()`.

| Field | Type | Description |
|---|---|---|
| `type` | string | `"ProcessStdout"` |
| `id` | integer | mplug-internal process ID |
| `line` | string | One line of output (newline stripped) |

### `UserCommand`

Fired when an external process sends a `trigger` command over the Unix socket. This is the primary mechanism for reacting to compositor keybinds from within a plugin.

| Field | Type | Description |
|---|---|---|
| `type` | string | `"UserCommand"` |
| `name` | string | The string sent after `trigger` in the socket command |

```
echo "trigger toggle_scratchpad" | socat - UNIX-CONNECT:/tmp/mplug.sock
```

```lua
mplug.add_listener(function(event, state)
    if event.type == "UserCommand" and event.name == "toggle_scratchpad" then
        -- handle scratchpad toggle
    end
end)
```

### `Generic`

A catch-all type for any Wayland event that does not map to one of the above types. No additional fields are guaranteed.

---

## State Snapshot Reference

The second argument passed to every listener is a snapshot of accumulated compositor state, rebuilt on every event.

### Top-level fields

| Field | Type | Description |
|---|---|---|
| `tag_count` | integer | Total number of tags configured |
| `layout_name` | string | Name of the active layout |
| `layout_index` | integer | Zero-based index of the active layout |
| `layout_symbol` | string | Active layout symbol string from the compositor |
| `idle` | boolean | Whether the session is currently idle |
| `output_power_on` | boolean | Whether the display power is on |

### `state.active_tags`

An array of tag numbers that are currently active (state = 1).

```lua
for _, tag in ipairs(state.active_tags) do
    print("active tag:", tag)
end
```

### `state.tags`

A table keyed by tag number. Each value is a table with:

| Field | Type | Description |
|---|---|---|
| `state` | integer | `0` = inactive, `1` = active, `2` = urgent |
| `clients` | integer | Number of clients on the tag |
| `focused` | integer | Non-zero if a client on this tag has focus |

```lua
local tag3 = state.tags[3]
if tag3 and tag3.clients > 0 then
    -- tag 3 has windows
end
```

### `state.toplevels`

A table keyed by protocol window ID. Each value is a table with:

| Field | Type | Description |
|---|---|---|
| `title` | string | Window title |
| `app_id` | string | Application ID |
| `activated` | boolean | Whether this window has keyboard focus |
| `minimized` | boolean | Whether the window is minimized |
| `maximized` | boolean | Whether the window is maximized |
| `fullscreen` | boolean | Whether the window is fullscreen |

```lua
for id, win in pairs(state.toplevels) do
    if win.app_id == "foot" then
        mplug.focus_window(id)
    end
end
```

### `state.focused_window`

A table with `title` and `app_id` for the currently focused window, or `nil` if no window has focus.

```lua
if state.focused_window then
    print("focused:", state.focused_window.app_id)
end
```

### `state.workspaces`

An array of workspace tables, each with:

| Field | Type | Description |
|---|---|---|
| `id` | integer | Protocol-level workspace ID |
| `name` | string | Workspace name |
| `active` | boolean | Whether this workspace is active |
| `urgent` | boolean | Whether this workspace has an urgent client |
| `hidden` | boolean | Whether this workspace is hidden |

### `state.outputs`

An array of output head tables, each with:

| Field | Type | Description |
|---|---|---|
| `id` | integer | Protocol-level head ID |
| `name` | string | Connector name (e.g., `"eDP-1"`) |
| `description` | string | Human-readable description from the compositor |
| `x` | integer | Horizontal position in global space |
| `y` | integer | Vertical position in global space |
| `enabled` | boolean | Whether the output is active |
| `width_px` | integer | Horizontal resolution |
| `height_px` | integer | Vertical resolution |
| `refresh` | integer | Refresh rate in millihertz |
| `scale` | number | Scale factor |
| `transform` | integer | Raw transform value from the compositor |
| `width_mm` | integer | Physical width in millimeters |
| `height_mm` | integer | Physical height in millimeters |

---

## Socket IPC

The daemon listens on the Unix domain socket `/tmp/mplug.sock`. Commands are newline-delimited text strings. Each line is parsed as a whitespace-separated sequence of tokens.

To send a command from a compositor keybind or external script:

```
echo "command args" | socat - UNIX-CONNECT:/tmp/mplug.sock
```

### Socket commands

**`trigger <name>`**

Sends a `UserCommand` event to the Lua thread with the given name. This is the primary bridge between compositor keybinds and plugin logic.

```
echo "trigger toggle_scratchpad" | socat - UNIX-CONNECT:/tmp/mplug.sock
```

**`set_tags <tagmask>`**

Sends a `SetTags` request directly to the Wayland thread, bypassing Lua.

**`set_layout <index>`**

Sends a `SetLayout` request directly to the Wayland thread.

**`focus_window <id>`**

Activates the window with the given protocol ID.

**`close_window <id>`**

Requests that the window with the given protocol ID be closed.

**`set_window_tag <id> <tagmask>`**

Moves the window with the given ID to the given tag bitmask.

**`set_client_tags <and_tags> <xor_tags>`**

Modifies the focused window's tags using the bitmask formula `(current AND and_tags) XOR xor_tags`.

**`set_window_minimized <id> <true|false>`**

Minimizes or restores the window with the given ID. The boolean argument is parsed as `true` or `1` for minimized, anything else for restored.

Unknown commands are logged to stderr and ignored.

---

## Plugin Discovery and Loading

At daemon startup, the Lua thread performs the following steps for each plugin name in `enabled_plugins`:

1. Check for a single-file plugin at `~/.config/mplug/plugins/<name>.lua`. If found, load it directly.
2. If no file exists, check for a directory at `~/.config/mplug/plugins/<name>/`.
   - If found, add `<plugin-dir>/?.lua` to Lua's `package.path` to enable `require()`.
   - Load and validate `<plugin-dir>/mplug.toml`. If the manifest is missing or invalid, print a warning to stderr and skip this plugin.
   - Resolve the entry point as `<plugin-dir>/<entry_point>`.
3. If neither a file nor directory exists, print a warning and skip.
4. Read the entry point file and execute it in the shared Lua VM.
   - If reading fails, the plugin is silently skipped.
   - If the Lua script raises an error at load time, the error is printed to stderr and the plugin is skipped; other plugins continue loading.

All enabled plugins share a single Lua VM and the same global `mplug` table. Listeners registered by all plugins are stored in `mplug.__listeners` and are called for every event regardless of which plugin registered them.

After all plugins are loaded, mplug also checks for a legacy `init.lua` in the current working directory and executes it if present. This behavior exists for compatibility and is not recommended for new plugins.

---

## Error Handling

mplug is designed to be resilient to plugin errors:

- A plugin that fails to load does not prevent other plugins from loading.
- A listener that raises a runtime error has the error printed to stderr; subsequent listeners and events continue to be processed.
- An invalid manifest during `mplug add` results in a clean removal of the cloned directory.
- An invalid config file at `~/.config/mplug/mplug.toml` is ignored and an empty default config is used.
- Errors from Wayland requests (e.g., dispatching an unknown command) are logged to stderr and dropped.

mplug does not restart crashed plugins automatically. If a plugin needs to maintain persistent state across multiple events, it should use Lua upvalues or module-level variables:

```lua
local window_count = 0

mplug.add_listener(function(event, state)
    window_count = 0
    for _ in pairs(state.toplevels) do
        window_count = window_count + 1
    end
end)
```
