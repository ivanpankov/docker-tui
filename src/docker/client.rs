//! Thin abstraction over [`bollard::Docker`]: list operations and future extension points
//! for start/stop/remove without leaking Bollard types to the UI.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local, TimeZone, Utc};

use bollard::Docker;
use bollard::container::{
    BlkioStats, InspectContainerOptions, KillContainerOptions, ListContainersOptions, LogOutput,
    LogsOptions, RemoveContainerOptions, StartContainerOptions, Stats, StatsOptions,
    StopContainerOptions,
};
use futures_util::StreamExt;
use bollard::image::{ListImagesOptions, RemoveImageOptions};
use bollard::models::{
    HistoryResponseItem, ImageConfig, ImageInspect, Ipam, MountPoint, MountPointTypeEnum, Network,
    NetworkSettings, PortMap, SystemDataUsageResponse, Volume,
};
use bollard::network::{InspectNetworkOptions, ListNetworksOptions};
use bollard::service::ListServicesOptions;
use bollard::volume::{ListVolumesOptions, RemoveVolumeOptions};

use super::error::DockerError;
use super::types::{
    ContainerDetailMountRow, ContainerDetailNetworkRow, ContainerInspectSnapshot,
    ContainerStatsSnapshot, ContainerRow,
    DashboardSnapshot, ImageDetailSnapshot, ImageLayerRow, ImageRow, NetworkAttachedContainerRow,
    NetworkRow, VolumeContainerMountRow, VolumeRow,
};

/// Swarm stack namespace or Docker Compose project name (same keys as the Volumes tab).
fn insert_stack_name_from_labels(labels: &HashMap<String, String>, names: &mut HashSet<String>) {
    if let Some(s) = labels
        .get("com.docker.stack.namespace")
        .or_else(|| labels.get("com.docker.compose.project"))
        .filter(|s| !s.is_empty())
    {
        names.insert(s.clone());
    }
}

fn network_stack_from_labels(labels: &Option<HashMap<String, String>>) -> String {
    labels
        .as_ref()
        .and_then(|l| {
            l.get("com.docker.stack.namespace")
                .or_else(|| l.get("com.docker.compose.project"))
                .cloned()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "—".into())
}

fn opt_bool_yes_no(b: Option<bool>) -> String {
    match b {
        Some(true) => "Yes".into(),
        Some(false) => "No".into(),
        None => "—".into(),
    }
}

/// Splits [`Ipam::config`] entries into IPv4 vs IPv6 (by subnet or gateway containing `:`).
fn ipam_columns(ipam: &Option<Ipam>) -> (String, String, String, String) {
    let mut v4_sub = Vec::new();
    let mut v4_gw = Vec::new();
    let mut v6_sub = Vec::new();
    let mut v6_gw = Vec::new();

    for c in ipam
        .as_ref()
        .and_then(|i| i.config.as_ref())
        .into_iter()
        .flatten()
    {
        let sub = c.subnet.as_deref().unwrap_or("");
        let gw = c.gateway.as_deref().unwrap_or("");
        if sub.is_empty() && gw.is_empty() {
            continue;
        }
        let is_v6 = if !sub.is_empty() {
            sub.contains(':')
        } else {
            gw.contains(':')
        };
        if is_v6 {
            if !sub.is_empty() {
                v6_sub.push(sub.to_string());
            }
            if !gw.is_empty() {
                v6_gw.push(gw.to_string());
            }
        } else {
            if !sub.is_empty() {
                v4_sub.push(sub.to_string());
            }
            if !gw.is_empty() {
                v4_gw.push(gw.to_string());
            }
        }
    }

    let fmt = |v: &[String]| -> String {
        if v.is_empty() {
            "—".into()
        } else {
            v.join(", ")
        }
    };

    (fmt(&v4_sub), fmt(&v4_gw), fmt(&v6_sub), fmt(&v6_gw))
}

/// Docker engine–managed networks (same idea as Portainer “System”): default bridges, Swarm routing mesh, etc.
fn network_is_system(name: &str, swarm_ingress: Option<bool>) -> bool {
    if swarm_ingress == Some(true) {
        return true;
    }
    matches!(
        name,
        "bridge" | "host" | "none" | "ingress" | "docker_gwbridge"
    )
}

fn network_rows_from_list(list: Vec<Network>) -> Vec<NetworkRow> {
    let mut rows: Vec<NetworkRow> = list
        .into_iter()
        .map(|n| {
            let name = n.name.clone().unwrap_or_else(|| "<none>".into());
            let is_system = network_is_system(name.as_str(), n.ingress);
            let (ipv4_subnet, ipv4_gateway, ipv6_subnet, ipv6_gateway) = ipam_columns(&n.ipam);
            NetworkRow {
                id: n.id.unwrap_or_default(),
                name,
                driver: n.driver.unwrap_or_else(|| "unknown".into()),
                scope: n.scope.unwrap_or_else(|| "local".into()),
                attachable: opt_bool_yes_no(n.attachable),
                internal: opt_bool_yes_no(n.internal),
                stack: network_stack_from_labels(&n.labels),
                ipv4_subnet,
                ipv4_gateway,
                ipv6_subnet,
                ipv6_gateway,
                is_system,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        b.is_system
            .cmp(&a.is_system)
            .then_with(|| a.name.cmp(&b.name))
    });
    rows
}

fn opt_addr_display(addr: Option<&str>) -> String {
    let Some(s) = addr else {
        return "—".into();
    };
    let s = s.trim();
    if s.is_empty() {
        return "—".into();
    }
    s.split('/').next().unwrap_or(s).to_string()
}

/// Portainer-style “Build” line: Docker engine version used to build the image (if present) plus OS/arch.
fn shell_vec_display(opt: &Option<Vec<String>>) -> String {
    opt.as_ref()
        .filter(|v| !v.is_empty())
        .map(|v| v.join(" "))
        .unwrap_or_else(|| "—".into())
}

/// Dockerfile-oriented fields from `ImageConfig` (inspect `Config`).
fn dockerfile_fields_from_config(
    cfg: Option<&ImageConfig>,
) -> (
    String,
    String,
    Vec<String>,
    String,
    String,
    String,
    String,
    String,
    String,
) {
    let Some(cfg) = cfg else {
        return (
            "—".into(),
            "—".into(),
            vec![],
            "—".into(),
            "—".into(),
            "—".into(),
            "—".into(),
            "—".into(),
            "—".into(),
        );
    };

    let cmd = shell_vec_display(&cfg.cmd);
    let entrypoint = shell_vec_display(&cfg.entrypoint);
    let env = cfg.env.clone().unwrap_or_default();
    let working_dir = cfg
        .working_dir
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("—")
        .to_string();
    let user = cfg
        .user
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("—")
        .to_string();
    let expose = cfg
        .exposed_ports
        .as_ref()
        .map(|m| {
            let mut keys: Vec<String> = m.keys().cloned().collect();
            keys.sort();
            keys.join(", ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "—".into());
    let volume = cfg
        .volumes
        .as_ref()
        .map(|m| {
            let mut keys: Vec<String> = m.keys().cloned().collect();
            keys.sort();
            keys.join(", ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "—".into());
    let shell = shell_vec_display(&cfg.shell);
    let on_build = cfg
        .on_build
        .as_ref()
        .filter(|v| !v.is_empty())
        .map(|v| v.join(" | "))
        .unwrap_or_else(|| "—".into());

    (
        cmd,
        entrypoint,
        env,
        working_dir,
        user,
        expose,
        volume,
        shell,
        on_build,
    )
}

/// Image history is newest-first; reverse so base layer is order 1, ascending.
fn image_layers_from_history(history: Vec<HistoryResponseItem>) -> Vec<ImageLayerRow> {
    history
        .into_iter()
        .rev()
        .enumerate()
        .map(|(i, h)| ImageLayerRow {
            order: i + 1,
            size: format_bytes(h.size),
            layer: h.created_by,
        })
        .collect()
}

fn image_build_display(ins: &ImageInspect) -> String {
    let os = ins.os.as_deref().unwrap_or("—");
    let arch = ins.architecture.as_deref().unwrap_or("—");
    let arch_part = match ins.variant.as_deref().filter(|s| !s.is_empty()) {
        Some(v) => format!("{arch} ({v})"),
        None => arch.to_string(),
    };
    match ins.docker_version.as_deref().filter(|s| !s.is_empty()) {
        Some(dv) => format!("Docker {dv} on {os}, {arch_part}"),
        None => format!("Docker on {os}, {arch_part}"),
    }
}

fn format_volume_labels_csv(labels: &std::collections::HashMap<String, String>) -> String {
    if labels.is_empty() {
        return "—".into();
    }
    let mut pairs: Vec<(&String, &String)> = labels.iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    pairs
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `RefCount` is usually **not** present on `GET /volumes`; Docker populates it on `GET /system/df`.
fn volume_ref_counts_from_df(df: &SystemDataUsageResponse) -> HashMap<String, i64> {
    df.volumes
        .as_ref()
        .into_iter()
        .flatten()
        .filter_map(|v| {
            let rc = v.usage_data.as_ref().map(|u| u.ref_count)?;
            Some((v.name.clone(), rc))
        })
        .collect()
}

fn volume_rows_from_list(vols: Vec<Volume>, ref_by_name: &HashMap<String, i64>) -> Vec<VolumeRow> {
    let mut rows: Vec<VolumeRow> = vols
        .into_iter()
        .map(|v| {
            let stack = v
                .labels
                .get("com.docker.stack.namespace")
                .or_else(|| v.labels.get("com.docker.compose.project"))
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "—".into());
            let labels = format_volume_labels_csv(&v.labels);
            let created = v
                .created_at
                .as_ref()
                .map(format_volume_created)
                .unwrap_or_else(|| "—".into());
            // Prefer inline UsageData from list; else merge from `docker system df` (see `list_volumes`).
            // RefCount -1 means unknown — do not dim. Only 0 means unused.
            let ref_count = v
                .usage_data
                .as_ref()
                .map(|u| u.ref_count)
                .or_else(|| ref_by_name.get(&v.name).copied());
            let unused = ref_count.is_some_and(|rc| rc == 0);
            VolumeRow {
                name: v.name,
                driver: v.driver,
                mountpoint: v.mountpoint,
                stack,
                created,
                labels,
                unused,
            }
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

fn mount_point_is_named_volume(m: &MountPoint, volume_name: &str) -> bool {
    matches!(
        m.typ,
        Some(MountPointTypeEnum::VOLUME) | Some(MountPointTypeEnum::CLUSTER)
    ) && m.name.as_deref() == Some(volume_name)
}

/// Docker [`MountPoint`] uses `RW`: `true` = writable, `false` = read-only.
/// If `RW` is missing, infer read-only from `Mode` (e.g. `ro`, `z,ro`).
fn mount_point_read_only_display(m: &MountPoint) -> &'static str {
    match m.rw {
        Some(false) => "Yes",
        Some(true) => "No",
        None => {
            if m.mode
                .as_deref()
                .is_some_and(|mode| mode.split(',').any(|p| p.trim() == "ro"))
            {
                "Yes"
            } else {
                "—"
            }
        }
    }
}

/// Owns a [`Docker`] connection and maps responses into [`crate::docker::types`].
#[derive(Clone)]
pub struct DockerClient {
    inner: Docker,
}

impl DockerClient {
    pub fn connect() -> Result<Self, DockerError> {
        let inner = Docker::connect_with_socket_defaults()?;
        Ok(Self { inner })
    }

    pub fn from_inner(inner: Docker) -> Self {
        Self { inner }
    }

    pub async fn list_containers(&self, all: bool) -> Result<Vec<ContainerRow>, DockerError> {
        let options = Some(ListContainersOptions::<String> {
            all,
            ..Default::default()
        });
        let list = self.inner.list_containers(options).await?;
        let mut rows = Vec::new();
        for c in list.into_iter() {
            let raw_id = c.id.unwrap_or_default();
            let id = raw_id.chars().take(12).collect::<String>();
            let full_id = raw_id;
            let names = c
                .names
                .as_ref()
                .map(|n| {
                    n.iter()
                        .map(|s| s.trim_start_matches('/').to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "<none>".into());
            let image = c.image.unwrap_or_else(|| "<none>".into());
            let state = c.state.unwrap_or_else(|| "unknown".into());
            let status = c.status.unwrap_or_default();
            let running = state.eq_ignore_ascii_case("running");
            let stack = network_stack_from_labels(&c.labels);
            let created = c
                .created
                .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
                .map(|dt| {
                    dt.with_timezone(&Local)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string()
                })
                .unwrap_or_else(|| "—".into());
            rows.push(ContainerRow {
                id,
                full_id,
                names,
                stack,
                image,
                state,
                status,
                running,
                created,
            });
        }
        Ok(rows)
    }

    pub async fn start_container(&self, container_id: &str) -> Result<(), DockerError> {
        self.inner
            .start_container(container_id, None::<StartContainerOptions<String>>)
            .await?;
        Ok(())
    }

    pub async fn stop_container(&self, container_id: &str) -> Result<(), DockerError> {
        self.inner
            .stop_container(container_id, Some(StopContainerOptions::default()))
            .await?;
        Ok(())
    }

    pub async fn pause_container(&self, container_id: &str) -> Result<(), DockerError> {
        self.inner.pause_container(container_id).await?;
        Ok(())
    }

    pub async fn unpause_container(&self, container_id: &str) -> Result<(), DockerError> {
        self.inner.unpause_container(container_id).await?;
        Ok(())
    }

    pub async fn kill_container(&self, container_id: &str) -> Result<(), DockerError> {
        self.inner
            .kill_container(
                container_id,
                Some(KillContainerOptions {
                    signal: "SIGKILL",
                }),
            )
            .await?;
        Ok(())
    }

    /// Removes the container; `force` matches Docker’s kill-then-remove when still running.
    pub async fn remove_container(&self, container_id: &str) -> Result<(), DockerError> {
        self.inner
            .remove_container(
                container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await?;
        Ok(())
    }

    /// Full container inspect for the detail view: start time, mounts, network endpoints, and pretty JSON.
    pub async fn inspect_container_detail(
        &self,
        container_id: &str,
    ) -> Result<(ContainerInspectSnapshot, String), DockerError> {
        let ins = self
            .inner
            .inspect_container(container_id, None::<InspectContainerOptions>)
            .await?;
        let inspect_json = serde_json::to_string_pretty(&ins)
            .map_err(|e| DockerError::Json(e.to_string()))?;
        let started_at = format_container_started_at_display(
            ins.state.as_ref().and_then(|s| s.started_at.as_ref()),
        );
        let published_ports = published_ports_display(
            ins.network_settings
                .as_ref()
                .and_then(|n| n.ports.as_ref()),
        );
        let mounts = container_mount_rows_from_inspect(&ins.mounts.unwrap_or_default());
        let networks = container_network_rows_from_inspect(ins.network_settings.as_ref());
        Ok((
            ContainerInspectSnapshot {
                started_at,
                published_ports,
                mounts,
                networks,
            },
            inspect_json,
        ))
    }

    /// Recent container logs (stdout and stderr), newest tail only (see [`LogsOptions::tail`]).
    pub async fn container_logs(&self, container_id: &str) -> Result<String, DockerError> {
        let options = Some(LogsOptions::<String> {
            stdout: true,
            stderr: true,
            follow: false,
            tail: "10000".into(),
            ..Default::default()
        });
        let mut stream = self.inner.logs(container_id, options);
        let mut out = String::new();
        while let Some(item) = stream.next().await {
            let chunk = item?;
            let bytes = match &chunk {
                LogOutput::StdOut { message }
                | LogOutput::StdErr { message }
                | LogOutput::StdIn { message }
                | LogOutput::Console { message } => message,
            };
            out.push_str(&String::from_utf8_lossy(bytes.as_ref()));
        }
        Ok(out)
    }

    /// One-shot resource stats (`stream: false`, `one_shot: true`), formatted for the Stats tab.
    pub async fn container_stats(
        &self,
        container_id: &str,
    ) -> Result<ContainerStatsSnapshot, DockerError> {
        let options = Some(StatsOptions {
            stream: false,
            one_shot: true,
        });
        let mut stream = self.inner.stats(container_id, options);
        while let Some(item) = stream.next().await {
            let stats = item?;
            return Ok(format_container_stats(&stats));
        }
        Err(DockerError::Stats("no stats returned".into()))
    }

    /// Inspect metadata for the Image details panel (ID, size, created, build string, labels).
    pub async fn inspect_image_detail(
        &self,
        image_ref: &str,
    ) -> Result<ImageDetailSnapshot, DockerError> {
        let ins: ImageInspect = self.inner.inspect_image(image_ref).await?;
        let raw_id = ins.id.clone().unwrap_or_default();
        let id = raw_id
            .strip_prefix("sha256:")
            .map(str::to_owned)
            .unwrap_or(raw_id);
        let size = ins.size.map(format_bytes).unwrap_or_else(|| "—".into());
        let created = ins
            .created
            .as_ref()
            .map(|dt| {
                dt.with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
            .unwrap_or_else(|| "—".into());
        let labels_map = ins
            .config
            .as_ref()
            .and_then(|c| c.labels.as_ref())
            .cloned()
            .unwrap_or_default();
        let labels = format_volume_labels_csv(&labels_map);
        let build = image_build_display(&ins);
        let tags = ins
            .repo_tags
            .as_ref()
            .filter(|v| !v.is_empty())
            .map(|v| v.join(", "))
            .unwrap_or_else(|| "<none>".into());
        let (cmd, entrypoint, env, working_dir, user, expose, volume, shell, on_build) =
            dockerfile_fields_from_config(ins.config.as_ref());
        let history = self.inner.image_history(image_ref).await?;
        let layers = image_layers_from_history(history);
        Ok(ImageDetailSnapshot {
            tags,
            id,
            size,
            created,
            build,
            labels,
            cmd,
            entrypoint,
            env,
            working_dir,
            user,
            expose,
            volume,
            shell,
            on_build,
            layers,
        })
    }

    pub async fn list_images(&self) -> Result<Vec<ImageRow>, DockerError> {
        let options = Some(ListImagesOptions::<String> {
            all: true,
            ..Default::default()
        });
        let list = self.inner.list_images(options).await?;
        let mut rows = Vec::new();
        for img in list {
            let id = img.id.chars().take(12).collect::<String>();
            if img.repo_tags.is_empty() {
                rows.push(ImageRow {
                    id: id.clone(),
                    tag: "<none>".into(),
                    size: format_bytes(img.size),
                    created: format_ts(img.created),
                });
            } else {
                for rt in img.repo_tags {
                    rows.push(ImageRow {
                        id: id.clone(),
                        tag: rt.clone(),
                        size: format_bytes(img.size),
                        created: format_ts(img.created),
                    });
                }
            }
        }
        rows.sort_by(|a, b| a.tag.cmp(&b.tag).then_with(|| a.tag.cmp(&b.tag)));
        Ok(rows)
    }

    pub async fn list_networks(&self) -> Result<Vec<NetworkRow>, DockerError> {
        let list = self
            .inner
            .list_networks(None::<ListNetworksOptions<String>>)
            .await?;
        Ok(network_rows_from_list(list))
    }

    /// Endpoints attached to this network (`GET /networks/{id}` inspect).
    pub async fn containers_in_network(
        &self,
        network_id: &str,
    ) -> Result<Vec<NetworkAttachedContainerRow>, DockerError> {
        let net = self
            .inner
            .inspect_network(network_id, None::<InspectNetworkOptions<String>>)
            .await?;

        let mut rows: Vec<NetworkAttachedContainerRow> = net
            .containers
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(_k, c)| {
                let name = c
                    .name
                    .as_deref()
                    .unwrap_or("")
                    .trim_start_matches('/')
                    .to_string();
                if name.is_empty() {
                    return None;
                }
                Some(NetworkAttachedContainerRow {
                    container_name: name,
                    ipv4_address: opt_addr_display(c.ipv4_address.as_deref()),
                    ipv6_address: opt_addr_display(c.ipv6_address.as_deref()),
                    mac_address: c
                        .mac_address
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "—".into()),
                })
            })
            .collect();

        rows.sort_by(|a, b| a.container_name.cmp(&b.container_name));
        Ok(rows)
    }

    pub async fn list_volumes(&self) -> Result<Vec<VolumeRow>, DockerError> {
        let resp = self
            .inner
            .list_volumes(None::<ListVolumesOptions<String>>)
            .await?;
        let vols = resp.volumes.unwrap_or_default();
        let ref_by_name = match self.inner.df().await {
            Ok(df) => volume_ref_counts_from_df(&df),
            Err(_) => HashMap::new(),
        };
        Ok(volume_rows_from_list(vols, &ref_by_name))
    }

    /// Remove a volume (`force: false` so the engine rejects removal while still referenced).
    pub async fn remove_volume(&self, name: &str) -> Result<(), DockerError> {
        self.inner
            .remove_volume(name, Some(RemoveVolumeOptions { force: false }))
            .await?;
        Ok(())
    }

    /// Remove an image by ID or reference (`force: false` so the engine rejects removal when unsafe).
    pub async fn remove_image(&self, image_ref: &str) -> Result<(), DockerError> {
        self.inner
            .remove_image(
                image_ref,
                Some(RemoveImageOptions {
                    force: false,
                    noprune: false,
                }),
                None,
            )
            .await?;
        Ok(())
    }

    /// Containers that mount this volume (named volume or cluster volume), via container inspect.
    pub async fn containers_using_volume(
        &self,
        volume_name: &str,
    ) -> Result<Vec<VolumeContainerMountRow>, DockerError> {
        let list = self
            .inner
            .list_containers(Some(ListContainersOptions::<String> {
                all: true,
                ..Default::default()
            }))
            .await?;

        let mut rows = Vec::new();

        for summary in list {
            let id = summary.id.unwrap_or_default();
            if id.is_empty() {
                continue;
            }

            let inspect = self
                .inner
                .inspect_container(&id, None::<InspectContainerOptions>)
                .await?;

            let container_name = inspect
                .name
                .as_deref()
                .unwrap_or("")
                .trim_start_matches('/')
                .to_string();
            if container_name.is_empty() {
                continue;
            }

            let Some(mounts) = inspect.mounts else {
                continue;
            };

            for m in mounts {
                if !mount_point_is_named_volume(&m, volume_name) {
                    continue;
                }
                let read_only = mount_point_read_only_display(&m).to_string();
                let mounted_at = m.destination.unwrap_or_else(|| "—".into());

                rows.push(VolumeContainerMountRow {
                    container_name: container_name.clone(),
                    mounted_at,
                    read_only,
                });
            }
        }

        rows.sort_by(|a, b| {
            a.container_name
                .cmp(&b.container_name)
                .then_with(|| a.mounted_at.cmp(&b.mounted_at))
        });

        Ok(rows)
    }

    pub async fn load_dashboard(&self) -> Result<DashboardSnapshot, DockerError> {
        let (info, version) = tokio::try_join!(self.inner.info(), self.inner.version())?;
        let (vol_resp, net_list) = tokio::try_join!(
            self.inner.list_volumes(None::<ListVolumesOptions<String>>),
            self.inner
                .list_networks(None::<ListNetworksOptions<String>>)
        )?;

        let volumes_raw = vol_resp.volumes.unwrap_or_default();
        let mut stack_names = HashSet::new();
        for v in &volumes_raw {
            insert_stack_name_from_labels(&v.labels, &mut stack_names);
        }
        for n in &net_list {
            if let Some(ref labels) = n.labels {
                insert_stack_name_from_labels(labels, &mut stack_names);
            }
        }

        if let Ok(services) = self
            .inner
            .list_services(None::<ListServicesOptions<String>>)
            .await
        {
            for s in services {
                if let Some(spec) = s.spec {
                    if let Some(lbl) = spec.labels {
                        insert_stack_name_from_labels(&lbl, &mut stack_names);
                    }
                }
            }
        }

        if let Ok(containers) = self
            .inner
            .list_containers(Some(ListContainersOptions::<String> {
                all: true,
                ..Default::default()
            }))
            .await
        {
            for c in containers {
                if let Some(ref labels) = c.labels {
                    insert_stack_name_from_labels(labels, &mut stack_names);
                }
            }
        }

        let stacks = stack_names.len().to_string();
        let ref_by_name = match self.inner.df().await {
            Ok(df) => volume_ref_counts_from_df(&df),
            Err(_) => HashMap::new(),
        };
        let vols = volume_rows_from_list(volumes_raw, &ref_by_name);
        let nets = network_rows_from_list(net_list);

        let environment = {
            let os = info.os_type.as_deref().unwrap_or("?");
            let arch = info.architecture.as_deref().unwrap_or("?");
            format!("{os} ({arch})")
        };

        let operating_system = {
            let mut s = info.operating_system.clone().unwrap_or_default();
            if let Some(ver) = info.os_version.clone().filter(|v| !v.is_empty()) {
                if !s.is_empty() {
                    s.push(' ');
                }
                s.push_str(&ver);
            }
            if s.is_empty() { "—".into() } else { s }
        };

        let (volume_plugins, network_plugins) = match &info.plugins {
            Some(p) => (
                p.volume
                    .as_ref()
                    .map(|v| v.join(", "))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "—".into()),
                p.network
                    .as_ref()
                    .map(|v| v.join(", "))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "—".into()),
            ),
            None => ("—".into(), "—".into()),
        };

        Ok(DashboardSnapshot {
            environment,
            endpoint_url: docker_endpoint_display(),
            containers_total: info
                .containers
                .map(|n| n.to_string())
                .unwrap_or_else(|| "—".into()),
            volumes_total: vols.len().to_string(),
            networks_total: nets.len().to_string(),
            stacks,
            images_total: info
                .images
                .map(|n| n.to_string())
                .unwrap_or_else(|| "—".into()),
            hostname: info.name.clone().unwrap_or_else(|| "—".into()),
            operating_system,
            kernel_version: info.kernel_version.clone().unwrap_or_else(|| "—".into()),
            cpu_total: info
                .ncpu
                .map(|n| n.to_string())
                .unwrap_or_else(|| "—".into()),
            memory_total: info
                .mem_total
                .map(format_mem_bytes)
                .unwrap_or_else(|| "—".into()),
            engine_version: version
                .version
                .clone()
                .or(info.server_version.clone())
                .unwrap_or_else(|| "—".into()),
            docker_root: info.docker_root_dir.clone().unwrap_or_else(|| "—".into()),
            storage_driver: info.driver.clone().unwrap_or_else(|| "—".into()),
            logging_driver: info.logging_driver.clone().unwrap_or_else(|| "—".into()),
            volume_plugins,
            network_plugins,
        })
    }
}

fn docker_endpoint_display() -> String {
    std::env::var("DOCKER_HOST").unwrap_or_else(|_| {
        if cfg!(windows) {
            "npipe:////./pipe/docker_engine".into()
        } else {
            "unix:///var/run/docker.sock".into()
        }
    })
}
fn format_bytes(n: i64) -> String {
    const KB: i64 = 1024;
    const MB: i64 = KB * 1024;
    const GB: i64 = MB * 1024;
    if n >= GB {
        format!("{:.2} GB", n as f64 / GB as f64)
    } else if n >= MB {
        format!("{:.2} MB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.1} KB", n as f64 / KB as f64)
    } else {
        format!("{n} B")
    }
}

fn format_mem_bytes(n: i64) -> String {
    const KIB: i64 = 1024;
    const MIB: i64 = KIB * 1024;
    const GIB: i64 = MIB * 1024;
    if n >= GIB {
        format!("{:.2} GiB", n as f64 / GIB as f64)
    } else if n >= MIB {
        format!("{:.2} MiB", n as f64 / MIB as f64)
    } else if n >= KIB {
        format!("{:.1} KiB", n as f64 / KIB as f64)
    } else {
        format!("{n} B")
    }
}

fn format_u64_bytes(n: u64) -> String {
    format_mem_bytes(n.min(i64::MAX as u64) as i64)
}

fn stats_aggregate_net_rx_tx(stats: &Stats) -> (u64, u64) {
    if let Some(ref map) = stats.networks {
        let mut rx = 0u64;
        let mut tx = 0u64;
        for n in map.values() {
            rx = rx.saturating_add(n.rx_bytes);
            tx = tx.saturating_add(n.tx_bytes);
        }
        (rx, tx)
    } else if let Some(ref n) = stats.network {
        (n.rx_bytes, n.tx_bytes)
    } else {
        (0, 0)
    }
}

fn stats_blkio_read_write(blk: &BlkioStats) -> (u64, u64) {
    let mut read_b = 0u64;
    let mut write_b = 0u64;
    if let Some(ref entries) = blk.io_service_bytes_recursive {
        for e in entries {
            if e.op.eq_ignore_ascii_case("read") {
                read_b = read_b.saturating_add(e.value);
            } else if e.op.eq_ignore_ascii_case("write") {
                write_b = write_b.saturating_add(e.value);
            }
        }
    }
    (read_b, write_b)
}

fn stats_cpu_percent(s: &Stats) -> f64 {
    let cpu_delta = s
        .cpu_stats
        .cpu_usage
        .total_usage
        .saturating_sub(s.precpu_stats.cpu_usage.total_usage);
    let system_delta = match (
        s.cpu_stats.system_cpu_usage,
        s.precpu_stats.system_cpu_usage,
    ) {
        (Some(a), Some(b)) => a.saturating_sub(b),
        _ => return 0.0,
    };
    if system_delta == 0 {
        return 0.0;
    }
    let ncpu = s
        .cpu_stats
        .cpu_usage
        .percpu_usage
        .as_ref()
        .filter(|v| !v.is_empty())
        .map(|v| v.len() as u64)
        .or(s.cpu_stats.online_cpus)
        .unwrap_or(1)
        .max(1);
    (cpu_delta as f64 / system_delta as f64) * ncpu as f64 * 100.0
}

fn format_container_stats(stats: &Stats) -> ContainerStatsSnapshot {
    let cpu = stats_cpu_percent(stats);
    let usage = stats.memory_stats.usage.unwrap_or(0);
    let limit = stats.memory_stats.limit.unwrap_or(0);
    let mem_line = if limit > 0 {
        let pct = (usage as f64 / limit as f64) * 100.0;
        format!(
            "{} / {} ({:.2}%)",
            format_u64_bytes(usage),
            format_u64_bytes(limit),
            pct
        )
    } else {
        format!("{} / —", format_u64_bytes(usage))
    };

    let (rx, tx) = stats_aggregate_net_rx_tx(stats);
    let net_line = format!(
        "{} in / {} out",
        format_u64_bytes(rx),
        format_u64_bytes(tx)
    );

    let (br, bw) = stats_blkio_read_write(&stats.blkio_stats);
    let blk_line = format!(
        "{} read / {} write",
        format_u64_bytes(br),
        format_u64_bytes(bw)
    );

    let pids_cur = stats
        .pids_stats
        .current
        .map(|n| n.to_string())
        .unwrap_or_else(|| "—".into());
    let pids_lim = stats
        .pids_stats
        .limit
        .filter(|&n| n > 0)
        .map(|n| n.to_string())
        .unwrap_or_else(|| "—".into());
    let pids_line = format!("{pids_cur} / {pids_lim}");

    let lines = vec![
        "Resource usage (one-shot)".to_string(),
        String::new(),
        format!("CPU %:          {cpu:.2}%"),
        format!("Memory:         {mem_line}"),
        format!("Network I/O:    {net_line}"),
        format!("Block I/O:      {blk_line}"),
        format!("PIDs:           {pids_line}"),
        String::new(),
        "CPU and memory % are computed from the last daemon sample (same idea as docker stats)."
            .to_string(),
    ];

    ContainerStatsSnapshot { lines }
}

fn format_volume_created(d: &DateTime<Utc>) -> String {
    d.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn format_ts(ts: i64) -> String {
    if ts <= 0 {
        return "—".into();
    }
    let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return ts.to_string();
    };
    let now_secs = now.as_secs() as i64;
    let age = now_secs.saturating_sub(ts);
    if age < 60 {
        format!("{age}s ago")
    } else if age < 3600 {
        format!("{}m ago", age / 60)
    } else if age < 86400 {
        format!("{}h ago", age / 3600)
    } else {
        format!("{}d ago", age / 86400)
    }
}

/// Host port bindings from inspect `NetworkSettings.Ports` (`ip:host->container/proto`, comma-separated).
fn published_ports_display(ports: Option<&PortMap>) -> String {
    let Some(map) = ports else {
        return "—".into();
    };
    if map.is_empty() {
        return "—".into();
    }
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    let mut parts: Vec<String> = Vec::new();
    for key in keys {
        let bindings = map.get(key.as_str()).and_then(|x| x.as_ref());
        match bindings {
            Some(bs) if !bs.is_empty() => {
                for b in bs {
                    let ip = b
                        .host_ip
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .unwrap_or("0.0.0.0");
                    let Some(hp) = b.host_port.as_deref().filter(|s| !s.is_empty()) else {
                        continue;
                    };
                    parts.push(format!("{ip}:{hp}->{key}"));
                }
            }
            _ => {
                parts.push(key.clone());
            }
        }
    }
    if parts.is_empty() {
        "—".into()
    } else {
        parts.join(", ")
    }
}

fn mount_type_display(t: Option<&MountPointTypeEnum>) -> String {
    match t {
        None | Some(MountPointTypeEnum::EMPTY) => "—".into(),
        Some(x) => format!("{x}"),
    }
}

fn container_mount_rows_from_inspect(mounts: &[MountPoint]) -> Vec<ContainerDetailMountRow> {
    let mut rows: Vec<ContainerDetailMountRow> = mounts
        .iter()
        .map(|m| {
            let mount_type = mount_type_display(m.typ.as_ref());
            let source = m
                .name
                .clone()
                .or_else(|| m.source.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "—".into());
            let destination = m
                .destination
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "—".into());
            let rw = m
                .rw
                .map(|w| if w { "Yes".into() } else { "No".into() })
                .unwrap_or_else(|| "—".into());
            ContainerDetailMountRow {
                mount_type,
                source,
                destination,
                rw,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        a.mount_type
            .cmp(&b.mount_type)
            .then_with(|| a.destination.cmp(&b.destination))
            .then_with(|| a.source.cmp(&b.source))
    });
    rows
}

fn container_network_rows_from_inspect(ns: Option<&NetworkSettings>) -> Vec<ContainerDetailNetworkRow> {
    let Some(ns) = ns else {
        return vec![];
    };
    let Some(map) = ns.networks.as_ref() else {
        return vec![];
    };
    let mut rows: Vec<ContainerDetailNetworkRow> = map
        .iter()
        .map(|(name, ep)| {
            let network_id = ep
                .network_id
                .as_deref()
                .map(|id| id.chars().take(12).collect::<String>())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "—".into());
            ContainerDetailNetworkRow {
                network_name: name.clone(),
                network_id,
                ipv4: ep.ip_address.clone().unwrap_or_else(|| "—".into()),
                ipv6: ep.global_ipv6_address.clone().unwrap_or_else(|| "—".into()),
                mac_address: ep.mac_address.clone().unwrap_or_else(|| "—".into()),
            }
        })
        .collect();
    rows.sort_by(|a, b| a.network_name.cmp(&b.network_name));
    rows
}

fn format_container_started_at_display(raw: Option<&String>) -> String {
    let Some(s) = raw else {
        return "—".into();
    };
    let t = s.trim();
    if t.is_empty() || t.starts_with("0001-01-01") {
        return "—".into();
    }
    match DateTime::parse_from_rfc3339(t) {
        Ok(dt) => dt
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
        Err(_) => t.to_string(),
    }
}
