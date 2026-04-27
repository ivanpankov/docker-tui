//! Stable view models for the UI, decoupled from Bollard's wire types.

/// Aggregated `/info` + `/version` data for the dashboard (all display strings).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DashboardSnapshot {
    pub environment: String,
    pub endpoint_url: String,
    pub containers_total: String,
    pub volumes_total: String,
    pub networks_total: String,
    pub stacks: String,
    pub images_total: String,
    pub hostname: String,
    pub operating_system: String,
    pub kernel_version: String,
    pub cpu_total: String,
    pub memory_total: String,
    pub engine_version: String,
    pub docker_root: String,
    pub storage_driver: String,
    pub logging_driver: String,
    pub volume_plugins: String,
    pub network_plugins: String,
}

/// One row in the containers table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerRow {
    /// Short ID (12 hex chars) for compact labels and Docker API prefix matching.
    pub id: String,
    /// Full container ID from the engine (typically 64 hex chars).
    pub full_id: String,
    pub names: String,
    /// Swarm stack or Compose project from Docker labels, or "—".
    pub stack: String,
    pub image: String,
    pub state: String,
    pub status: String,
    /// True when Docker reports state `running` (case-insensitive).
    pub running: bool,
    /// Creation time from the list API (`Created` unix timestamp), local time.
    pub created: String,
}

/// One row in the images table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageRow {
    pub id: String,
    pub tag: String,
    pub size: String,
    pub created: String,
}

/// Image inspect snapshot for the Images detail panel (from `GET /images/{id}/json`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageDetailSnapshot {
    /// Comma-separated repo tags (`RepoTags` from inspect), or `<none>` when untagged.
    pub tags: String,
    pub id: String,
    pub size: String,
    pub created: String,
    /// e.g. `Docker 24.0.2 on linux, amd64` (engine version + OS + architecture from inspect).
    pub build: String,
    /// Serialized `key=value` pairs from image config labels (sorted by key), or `—`.
    pub labels: String,
    /// Effective `CMD` from image config (`Config.Cmd`).
    pub cmd: String,
    /// Effective `ENTRYPOINT` (`Config.Entrypoint`).
    pub entrypoint: String,
    /// `ENV` lines as returned by Docker (`Config.Env`, `KEY=value` each).
    pub env: Vec<String>,
    /// `WORKDIR` (`Config.WorkingDir`).
    pub working_dir: String,
    /// `USER` (`Config.User`).
    pub user: String,
    /// `EXPOSE` ports (sorted, comma-separated).
    pub expose: String,
    /// `VOLUME` paths (sorted, comma-separated).
    pub volume: String,
    /// `SHELL` (`Config.Shell`).
    pub shell: String,
    /// `ONBUILD` instructions joined with ` | `.
    pub on_build: String,
    /// Layers from `GET /images/{name}/history`, base-first, `order` ascending from 1.
    pub layers: Vec<ImageLayerRow>,
}

/// One row in the Image layers table (image history).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageLayerRow {
    pub order: usize,
    pub size: String,
    /// `CreatedBy` from history (may be truncated in the UI).
    pub layer: String,
}

/// One row in the networks table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkRow {
    /// Full Docker network ID (truncate for compact labels).
    pub id: String,
    pub name: String,
    pub driver: String,
    pub scope: String,
    /// `Yes` / `No` / `—` from Docker `attachable`.
    pub attachable: String,
    /// `Yes` / `No` / `—` from Docker `internal`.
    pub internal: String,
    /// Swarm stack or Compose project from Docker labels, or "—".
    pub stack: String,
    pub ipv4_subnet: String,
    pub ipv4_gateway: String,
    pub ipv6_subnet: String,
    pub ipv6_gateway: String,
    /// Built-in Docker / Swarm networks (`bridge`, `host`, `ingress`, etc.).
    pub is_system: bool,
}

/// One row in the volumes table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VolumeRow {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    /// Swarm stack or Compose project from Docker labels, or "—".
    pub stack: String,
    pub created: String,
    /// Serialized `key=value` pairs from Docker volume labels (sorted by key), or "—".
    pub labels: String,
    /// True when `RefCount == 0` (not `-1` unknown). Ref count usually comes from `GET /system/df`, not `GET /volumes`. Used to dim list rows; on volume detail, live mount list overrides when loaded.
    pub unused: bool,
}

/// One mount from container inspect (`Mounts`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerDetailMountRow {
    pub mount_type: String,
    pub source: String,
    pub destination: String,
    /// `Yes` / `No` — writable when `RW` is true.
    pub rw: String,
}

/// One network endpoint from container inspect (`NetworkSettings.Networks`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerDetailNetworkRow {
    pub network_name: String,
    pub network_id: String,
    pub ipv4: String,
    pub ipv6: String,
    pub mac_address: String,
}

/// One-shot `docker stats` snapshot for the Stats sub-tab (`lines` are preformatted).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerStatsSnapshot {
    pub lines: Vec<String>,
}

/// Data from a single `docker inspect` for the container detail view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerInspectSnapshot {
    pub started_at: String,
    /// Comma-separated host bindings, e.g. `0.0.0.0:8080->80/tcp`, or `—`.
    pub published_ports: String,
    pub mounts: Vec<ContainerDetailMountRow>,
    pub networks: Vec<ContainerDetailNetworkRow>,
}

/// One row: a container that mounts a given volume (volume detail panel).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VolumeContainerMountRow {
    pub container_name: String,
    pub mounted_at: String,
    /// `Yes` / `No` / `—`: whether the mount is read-only (from Docker `RW`, with `Mode` fallback).
    pub read_only: String,
}

/// One row: a container attached to a network (network detail panel).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkAttachedContainerRow {
    pub container_name: String,
    pub ipv4_address: String,
    pub ipv6_address: String,
    pub mac_address: String,
}
