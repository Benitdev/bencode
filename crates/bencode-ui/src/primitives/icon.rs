use std::collections::HashMap;

use gpui::{
    App, AssetSource, Global, Hsla, IntoElement, Radians, RenderOnce, SharedString, Transformation,
    Window, prelude::*, size, svg,
};

use crate::theme::{ActiveTheme, IconSize};

macro_rules! icons {
    ($($variant:ident => $file:literal,)*) => {
        /// Lucide icons bundled with Ely.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum IconName {
            $($variant,)*
        }

        impl IconName {
            pub const ALL: &[IconName] = &[$(IconName::$variant,)*];

            pub fn path(self) -> &'static str {
                match self {
                    $(IconName::$variant => concat!("icons/", $file, ".svg"),)*
                }
            }

            /// Lucide name, such as `chevron-down`.
            pub fn name(self) -> &'static str {
                match self {
                    $(IconName::$variant => $file,)*
                }
            }
        }
    };
}

icons! {
    AppWindow => "app-window",
    Archive => "archive",
    ArrowDown => "arrow-down",
    ArrowLeft => "arrow-left",
    ArrowRight => "arrow-right",
    ArrowUp => "arrow-up",
    Ban => "ban",
    Bot => "bot",
    Bug => "bug",
    CalendarDays => "calendar-days",
    Calendar => "calendar",
    Camera => "camera",
    CaseSensitive => "case-sensitive",
    ChartColumn => "chart-column",
    CheckCheck => "check-check",
    Check => "check",
    ChevronDown => "chevron-down",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    ChevronUp => "chevron-up",
    ChevronsUpDown => "chevrons-up-down",
    CircleAlert => "circle-alert",
    CircleCheck => "circle-check",
    CircleDashed => "circle-dashed",
    CircleDot => "circle-dot",
    CircleStop => "circle-stop",
    CircleX => "circle-x",
    Circle => "circle",
    Clipboard => "clipboard",
    Clock => "clock",
    CloudUpload => "cloud-upload",
    Columns2 => "columns-2",
    Compass => "compass",
    Copy => "copy",
    CornerDownRight => "corner-down-right",
    Cpu => "cpu",
    Crosshair => "crosshair",
    Download => "download",
    Ellipsis => "ellipsis",
    ExternalLink => "external-link",
    EyeOff => "eye-off",
    Eye => "eye",
    FileArchive => "file-archive",
    FileAudio => "file-audio",
    FileCode => "file-code",
    FileCog => "file-cog",
    FileImage => "file-image",
    FileJson => "file-json",
    FilePlus => "file-plus",
    FileSpreadsheet => "file-spreadsheet",
    FileTerminal => "file-terminal",
    FileText => "file-text",
    File => "file",
    FileType => "file-type",
    FileVideoCamera => "file-video-camera",
    FolderOpen => "folder-open",
    FolderPlus => "folder-plus",
    Folder => "folder",
    Gauge => "gauge",
    GitBranch => "git-branch",
    GitCommitHorizontal => "git-commit-horizontal",
    GitMerge => "git-merge",
    GitPullRequest => "git-pull-request",
    GitPullRequestClosed => "git-pull-request-closed",
    GitPullRequestDraft => "git-pull-request-draft",
    Globe => "globe",
    GripVertical => "grip-vertical",
    History => "history",
    Image => "image",
    ImageOff => "image-off",
    Inbox => "inbox",
    Info => "info",
    Key => "key",
    Keyboard => "keyboard",
    Languages => "languages",
    LayoutDashboard => "layout-dashboard",
    Lightbulb => "lightbulb",
    Link => "link",
    ListChecks => "list-checks",
    ListOrdered => "list-ordered",
    ListTodo => "list-todo",
    List => "list",
    LoaderCircle => "loader-circle",
    Lock => "lock",
    MessageSquare => "message-square",
    Minus => "minus",
    Monitor => "monitor",
    Moon => "moon",
    NotebookPen => "notebook-pen",
    Palette => "palette",
    PanelBottom => "panel-bottom",
    PanelLeft => "panel-left",
    PanelRight => "panel-right",
    Pause => "pause",
    PenLine => "pen-line",
    Pencil => "pencil",
    PictureInPicture2 => "picture-in-picture-2",
    Pipette => "pipette",
    PinOff => "pin-off",
    Pin => "pin",
    Play => "play",
    Plus => "plus",
    Power => "power",
    RefreshCw => "refresh-cw",
    Regex => "regex",
    Reply => "reply",
    RotateCcw => "rotate-ccw",
    RotateCw => "rotate-cw",
    Rows2 => "rows-2",
    Save => "save",
    Scissors => "scissors",
    Search => "search",
    SearchX => "search-x",
    Settings => "settings",
    Share2 => "share-2",
    Shield => "shield",
    SlidersHorizontal => "sliders-horizontal",
    SmilePlus => "smile-plus",
    Sparkles => "sparkles",
    SquareCheck => "square-check",
    Square => "square",
    SquareFunction => "square-function",
    SquarePen => "square-pen",
    Star => "star",
    StickyNote => "sticky-note",
    Sun => "sun",
    Terminal => "terminal",
    Trash2 => "trash-2",
    TriangleAlert => "triangle-alert",
    Type => "type",
    Undo2 => "undo-2",
    Upload => "upload",
    User => "user",
    WandSparkles => "wand-sparkles",
    WholeWord => "whole-word",
    WifiOff => "wifi-off",
    Wrench => "wrench",
    X => "x",
    Zap => "zap",
    ZoomIn => "zoom-in",
    ZoomOut => "zoom-out",
}

#[derive(IntoElement)]
pub struct Icon {
    path: SharedString,
    bundled: bool,
    size: IconSize,
    color: Option<Hsla>,
    hover: Option<(SharedString, Hsla)>,
    rotation: Option<Radians>,
    scale: f32,
}

impl Icon {
    pub fn new(name: IconName) -> Self {
        Self {
            bundled: true,
            ..Self::from_path(name.path())
        }
    }

    /// Draws an SVG supplied by the application's asset source; a missing one logs and shows `broken`.
    pub fn from_path(path: impl Into<SharedString>) -> Self {
        Self {
            path: path.into(),
            bundled: false,
            size: IconSize::Md,
            color: None,
            hover: None,
            rotation: None,
            scale: 1.0,
        }
    }

    pub fn size(mut self, size: IconSize) -> Self {
        self.size = size;
        self
    }

    /// Defaults to `colors.fg`.
    pub fn color(mut self, color: impl Into<Hsla>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Color while the pointer is over `group`.
    pub fn group_hover_color(
        mut self,
        group: impl Into<SharedString>,
        color: impl Into<Hsla>,
    ) -> Self {
        self.hover = Some((group.into(), color.into()));
        self
    }

    pub fn rotate(mut self, angle: impl Into<Radians>) -> Self {
        self.rotation = Some(angle.into());
        self
    }

    /// Draws the glyph larger or smaller inside the same box.
    pub fn scale(mut self, factor: f32) -> Self {
        self.scale = factor;
        self
    }
}

impl From<IconName> for Icon {
    fn from(name: IconName) -> Self {
        Icon::new(name)
    }
}

/// App paths already looked up, and whether the source held them.
#[derive(Default)]
struct FoundIcons(HashMap<SharedString, bool>);

impl Global for FoundIcons {}

fn found(source: &dyn AssetSource, path: &str) -> bool {
    match source.load(path) {
        Ok(Some(_)) => true,
        Ok(None) => {
            log::error!("ely: icon {path} is not in the app's asset source");
            false
        }
        Err(error) => {
            log::error!("ely: icon {path} failed to load: {error:#}");
            false
        }
    }
}

impl Icon {
    /// What a missing app icon shows: a broken picture in danger.
    fn broken(self, cx: &App) -> Self {
        Self {
            path: IconName::ImageOff.path().into(),
            bundled: true,
            color: Some(cx.theme().colors.danger),
            hover: None,
            ..self
        }
    }
}

impl RenderOnce for Icon {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let source = cx.asset_source().clone();
        let missing = !self.bundled
            && !*cx
                .default_global::<FoundIcons>()
                .0
                .entry(self.path.clone())
                .or_insert_with(|| found(source.as_ref(), &self.path));
        let icon = if missing { self.broken(cx) } else { self };
        let theme = cx.theme();
        let color = icon.color.unwrap_or(theme.colors.fg);
        svg()
            .path(icon.path)
            .size(theme.icon_size(icon.size))
            .flex_none()
            .text_color(color)
            .when_some(icon.hover, |svg, (group, hover)| {
                svg.group_hover(group, |style| style.text_color(hover))
            })
            .when(icon.rotation.is_some() || icon.scale != 1.0, |svg| {
                svg.with_transformation(
                    Transformation::rotate(icon.rotation.unwrap_or(Radians(0.0)))
                        .with_scaling(size(icon.scale, icon.scale)),
                )
            })
    }
}
