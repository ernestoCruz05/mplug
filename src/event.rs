// src/event.rs

#[derive(Debug, Clone)]
pub struct HeadInfo {
    pub name: String,
    pub description: String,
    pub width_mm: i32,
    pub height_mm: i32,
    pub x: i32,
    pub y: i32,
    pub enabled: bool,
    pub width_px: i32,
    pub height_px: i32,
    pub refresh: i32,
    pub scale: f64,
    pub transform: u32,
}

#[derive(Debug, Clone)]
pub struct WorkspaceInfo {
    pub name: String,
    pub active: bool,
    pub urgent: bool,
    pub hidden: bool,
}

#[derive(Default)]
pub struct WorkspacePending {
    pub name: Option<String>,
    pub state: u32,
}

#[derive(Debug, Clone)]
pub struct ToplevelInfo {
    pub title: String,
    pub app_id: String,
    pub activated: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
}

#[derive(Debug, Clone)]
pub enum WaylandEvent {
    TagsAmount(u32),
    LayoutName(String),
    OutputTag {
        tag: u32,
        state: u32, // 0 = None, 1 = Active, 2 = Urgent
        clients: u32,
        focused: u32, // 0 = False, >0 = True
    },
    OutputLayout(u32),
    OutputTitle(String),
    OutputAppid(String),
    OutputLayoutSymbol(String),
    OutputActive(u32),
    OutputFrame,
    OutputToggleVisibility,
    ToplevelUpdated {
        id: u32,
        info: ToplevelInfo,
    },
    ToplevelClosed {
        id: u32,
    },
    Idled,
    IdleResumed,
    OutputPowerMode {
        on: bool,
    },
    WorkspaceUpdated {
        id: u32,
        info: WorkspaceInfo,
    },
    WorkspaceClosed {
        id: u32,
    },
    OutputHeadUpdated {
        id: u32,
        info: HeadInfo,
    },
    OutputHeadRemoved {
        id: u32,
    },
    LayerSurfaceConfigured {
        id: u32,
        width: u32,
        height: u32,
    },
    LayerSurfaceClosed {
        id: u32,
    },
    ProcessExited {
        id: u64,
        exit_code: Option<i32>,
    },
    ProcessStdout {
        id: u64,
        line: String,
    },
    UserCommand(String),
}

#[derive(Debug, Clone)]
pub enum WaylandRequest {
    SetLayout(u32),
    SetTags(u32),
    ToggleVisibility,
    SetOutputPower {
        on: bool,
    },
    SetOutputMode {
        head_name: String,
        width: i32,
        height: i32,
        refresh: i32,
    },
    SetOutputPosition {
        head_name: String,
        x: i32,
        y: i32,
    },
    SetOutputScale {
        head_name: String,
        scale: f64,
    },
    SetOutputEnabled {
        head_name: String,
        enabled: bool,
    },
    CreateLayerSurface {
        id: u32,
        width: u32,
        height: u32,
        anchor: u32, // bitfield: 1=top, 2=bottom, 4=left, 8=right
        layer: u32,  // 0=background, 1=bottom, 2=top, 3=overlay
        exclusive_zone: i32,
    },
    DestroyLayerSurface {
        id: u32,
    },
    FillLayerSurface {
        id: u32,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
    },
    CloseToplevel {
        id: u32,
    },
    SetToplevelMinimized {
        id: u32,
        minimized: bool,
    },
    ActivateToplevel {
        id: u32,
    },
    SetToplevelTags {
        id: u32,
        tagmask: u32,
    },
    SetClientTags {
        and_tags: u32,
        xor_tags: u32,
    },
}
