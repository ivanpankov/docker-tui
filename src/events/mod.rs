//! Application-wide event types. The UI thread consumes `AppEvent` from the background
//! Docker worker and the terminal input thread.

use crossterm::event::Event;

/// User-facing navigation (Portainer-style sections).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum MainView {
    #[default]
    Dashboard,
    Containers,
    Images,
    Networks,
    Volumes,
}

/// List vs full-screen detail (per tab). Detail content and actions are wired later.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum AppScreen {
    #[default]
    List,
    Detail,
}

/// Sub-views inside container detail (`1`–`4` when screen is [`AppScreen::Detail`] on Containers).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum ContainerDetailTab {
    #[default]
    Details,
    Logs,
    Inspect,
    Stats,
}

/// High-level events merged into the main async loop.
#[derive(Debug)]
pub enum AppEvent {
    /// Raw terminal input (keys, resize, paste).
    Terminal(Event),
    /// Results and signals from the Docker worker.
    Docker(DockerEvent),
    /// Periodic tick for refresh animations or polling fallbacks.
    Tick,
}

/// Background Docker operations report through these variants.
#[derive(Debug)]
pub enum DockerEvent {
    DashboardLoaded(crate::docker::types::DashboardSnapshot),
    ContainersLoaded(Vec<crate::docker::types::ContainerRow>),
    ImagesLoaded(Vec<crate::docker::types::ImageRow>),
    NetworksLoaded(Vec<crate::docker::types::NetworkRow>),
    VolumesLoaded(Vec<crate::docker::types::VolumeRow>),
    VolumeMountsLoaded {
        volume_name: String,
        rows: Vec<crate::docker::types::VolumeContainerMountRow>,
    },
    NetworkContainersLoaded {
        network_id: String,
        rows: Vec<crate::docker::types::NetworkAttachedContainerRow>,
    },
    ImageInspectLoaded {
        image_ref: String,
        detail: crate::docker::types::ImageDetailSnapshot,
    },
    /// Full snapshot from container inspect (start time, mounts, networks) plus formatted JSON.
    ContainerInspectLoaded {
        container_id: String,
        detail: crate::docker::types::ContainerInspectSnapshot,
        inspect_json: String,
    },
    /// Raw stdout/stderr log text for the open container (tail-limited on the client).
    ContainerLogsLoaded {
        container_id: String,
        text: String,
    },
    /// One-shot resource stats for the Stats sub-tab.
    ContainerStatsLoaded {
        container_id: String,
        stats: crate::docker::types::ContainerStatsSnapshot,
    },
    /// Volume was deleted successfully; sent before [`DockerEvent::VolumesLoaded`] so the UI closes detail first.
    VolumeRemoved {
        name: String,
    },
    /// Image was deleted successfully; sent before [`DockerEvent::ImagesLoaded`] so the UI closes detail first.
    ImageRemoved {
        image_ref: String,
    },
    /// Start/stop/etc. succeeded; sent immediately before [`DockerEvent::ContainersLoaded`] for that refresh only.
    ContainerLifecycleFinished,
    OperationFailed {
        context: String,
        message: String,
    },
}
