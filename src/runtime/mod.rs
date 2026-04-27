//! Background work: Docker polling on an interval, user-triggered refresh, and a blocking
//! terminal reader that forwards Crossterm events into the async `AppEvent` stream.

use std::thread;
use std::time::Duration;

use crossterm::event;
use tokio::sync::mpsc;

use crate::docker::DockerClient;
use crate::events::{AppEvent, DockerEvent};

/// Lifecycle actions from the containers list (start/stop/pause/etc.).
#[derive(Debug, Clone, Copy)]
pub enum ContainerLifecycleAction {
    Start,
    Stop,
    Pause,
    Unpause,
    Kill,
    Remove,
}

/// Commands sent from the UI loop to the Docker worker.
#[derive(Debug)]
pub enum WorkerCommand {
    Refresh,
    /// Inspect all containers and list mounts of this named volume.
    FetchVolumeMounts {
        volume_name: String,
    },
    /// Inspect network and list attached container endpoints.
    FetchNetworkContainers {
        network_id: String,
    },
    /// Inspect image by ID or tag (UI passes the short image ID from the list row).
    FetchImageInspect {
        image_ref: String,
    },
    /// Inspect container for start time (UI passes the short ID from the list row).
    FetchContainerInspect {
        container_id: String,
    },
    /// Fetch recent container logs (stdout+stderr).
    FetchContainerLogs {
        container_id: String,
    },
    /// One-shot `docker stats` for the open container.
    FetchContainerStats {
        container_id: String,
    },
    /// DELETE volume by name (fails if still referenced).
    RemoveVolume {
        name: String,
    },
    /// DELETE image by ID or reference (fails if used by a running container, etc.).
    RemoveImage {
        image_ref: String,
    },
    /// Start, stop, pause, kill, or remove a container (full engine ID from the list row).
    ContainerLifecycle {
        container_id: String,
        action: ContainerLifecycleAction,
    },
}

const POLL_TICK: Duration = Duration::from_millis(250);

/// Spawns a thread that blocks on Crossterm and forwards input to `tx`.
pub fn spawn_input_forwarder(tx: tokio::sync::mpsc::UnboundedSender<AppEvent>) {
    thread::spawn(move || {
        loop {
            if event::poll(POLL_TICK).unwrap_or(false)
                && let Ok(ev) = event::read()
            {
                let _ = tx.send(AppEvent::Terminal(ev));
            }
        }
    });
}

/// Tokio task: periodic refresh and explicit [`WorkerCommand::Refresh`].
pub fn spawn_docker_worker(
    client: DockerClient,
    app_tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
    mut cmd_rx: mpsc::Receiver<WorkerCommand>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        async fn push_lists(
            client: &DockerClient,
            app_tx: &tokio::sync::mpsc::UnboundedSender<AppEvent>,
        ) {
            match client.load_dashboard().await {
                Ok(snapshot) => {
                    let _ = app_tx.send(AppEvent::Docker(DockerEvent::DashboardLoaded(snapshot)));
                }
                Err(e) => {
                    let _ = app_tx.send(AppEvent::Docker(DockerEvent::OperationFailed {
                        context: "dashboard".into(),
                        message: e.to_string(),
                    }));
                }
            }
            match client.list_containers(true).await {
                Ok(rows) => {
                    let _ = app_tx.send(AppEvent::Docker(DockerEvent::ContainersLoaded(rows)));
                }
                Err(e) => {
                    let _ = app_tx.send(AppEvent::Docker(DockerEvent::OperationFailed {
                        context: "list containers".into(),
                        message: e.to_string(),
                    }));
                }
            }
            match client.list_images().await {
                Ok(rows) => {
                    let _ = app_tx.send(AppEvent::Docker(DockerEvent::ImagesLoaded(rows)));
                }
                Err(e) => {
                    let _ = app_tx.send(AppEvent::Docker(DockerEvent::OperationFailed {
                        context: "list images".into(),
                        message: e.to_string(),
                    }));
                }
            }
            match client.list_networks().await {
                Ok(rows) => {
                    let _ = app_tx.send(AppEvent::Docker(DockerEvent::NetworksLoaded(rows)));
                }
                Err(e) => {
                    let _ = app_tx.send(AppEvent::Docker(DockerEvent::OperationFailed {
                        context: "list networks".into(),
                        message: e.to_string(),
                    }));
                }
            }
            match client.list_volumes().await {
                Ok(rows) => {
                    let _ = app_tx.send(AppEvent::Docker(DockerEvent::VolumesLoaded(rows)));
                }
                Err(e) => {
                    let _ = app_tx.send(AppEvent::Docker(DockerEvent::OperationFailed {
                        context: "list volumes".into(),
                        message: e.to_string(),
                    }));
                }
            }
        }

        push_lists(&client, &app_tx).await;

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    push_lists(&client, &app_tx).await;
                }
                Some(cmd) = cmd_rx.recv() => {
                    match cmd {
                        WorkerCommand::Refresh => {
                            push_lists(&client, &app_tx).await;
                        }
                        WorkerCommand::FetchVolumeMounts { volume_name } => {
                            match client.containers_using_volume(&volume_name).await {
                                Ok(rows) => {
                                    let _ = app_tx.send(AppEvent::Docker(
                                        DockerEvent::VolumeMountsLoaded {
                                            volume_name,
                                            rows,
                                        },
                                    ));
                                }
                                Err(e) => {
                                    let _ = app_tx.send(AppEvent::Docker(
                                        DockerEvent::OperationFailed {
                                            context: "volume mounts".into(),
                                            message: e.to_string(),
                                        },
                                    ));
                                }
                            }
                        }
                        WorkerCommand::FetchImageInspect { image_ref } => {
                            match client.inspect_image_detail(&image_ref).await {
                                Ok(detail) => {
                                    let _ = app_tx.send(AppEvent::Docker(DockerEvent::ImageInspectLoaded {
                                        image_ref,
                                        detail,
                                    }));
                                }
                                Err(e) => {
                                    let _ = app_tx.send(AppEvent::Docker(
                                        DockerEvent::OperationFailed {
                                            context: "inspect image".into(),
                                            message: e.to_string(),
                                        },
                                    ));
                                }
                            }
                        }
                        WorkerCommand::FetchContainerInspect { container_id } => {
                            match client.inspect_container_detail(&container_id).await {
                                Ok((detail, inspect_json)) => {
                                    let _ = app_tx.send(AppEvent::Docker(
                                        DockerEvent::ContainerInspectLoaded {
                                            container_id,
                                            detail,
                                            inspect_json,
                                        },
                                    ));
                                }
                                Err(e) => {
                                    let _ = app_tx.send(AppEvent::Docker(
                                        DockerEvent::OperationFailed {
                                            context: "inspect container".into(),
                                            message: e.to_string(),
                                        },
                                    ));
                                }
                            }
                        }
                        WorkerCommand::FetchContainerLogs { container_id } => {
                            match client.container_logs(&container_id).await {
                                Ok(text) => {
                                    let _ = app_tx.send(AppEvent::Docker(
                                        DockerEvent::ContainerLogsLoaded {
                                            container_id,
                                            text,
                                        },
                                    ));
                                }
                                Err(e) => {
                                    let _ = app_tx.send(AppEvent::Docker(
                                        DockerEvent::OperationFailed {
                                            context: "container logs".into(),
                                            message: e.to_string(),
                                        },
                                    ));
                                }
                            }
                        }
                        WorkerCommand::FetchContainerStats { container_id } => {
                            match client.container_stats(&container_id).await {
                                Ok(stats) => {
                                    let _ = app_tx.send(AppEvent::Docker(
                                        DockerEvent::ContainerStatsLoaded {
                                            container_id,
                                            stats,
                                        },
                                    ));
                                }
                                Err(e) => {
                                    let _ = app_tx.send(AppEvent::Docker(
                                        DockerEvent::OperationFailed {
                                            context: "container stats".into(),
                                            message: e.to_string(),
                                        },
                                    ));
                                }
                            }
                        }
                        WorkerCommand::FetchNetworkContainers { network_id } => {
                            match client.containers_in_network(&network_id).await {
                                Ok(rows) => {
                                    let _ = app_tx.send(AppEvent::Docker(
                                        DockerEvent::NetworkContainersLoaded {
                                            network_id,
                                            rows,
                                        },
                                    ));
                                }
                                Err(e) => {
                                    let _ = app_tx.send(AppEvent::Docker(
                                        DockerEvent::OperationFailed {
                                            context: "network containers".into(),
                                            message: e.to_string(),
                                        },
                                    ));
                                }
                            }
                        }
                        WorkerCommand::RemoveVolume { name } => {
                            match client.remove_volume(&name).await {
                                Ok(()) => match client.list_volumes().await {
                                    Ok(rows) => {
                                        // Close detail before VolumesLoaded so we do not re-fetch mounts
                                        // while still on the detail screen for the deleted name.
                                        let _ = app_tx.send(AppEvent::Docker(
                                            DockerEvent::VolumeRemoved { name },
                                        ));
                                        let _ = app_tx.send(AppEvent::Docker(
                                            DockerEvent::VolumesLoaded(rows),
                                        ));
                                    }
                                    Err(e) => {
                                        let _ = app_tx.send(AppEvent::Docker(
                                            DockerEvent::OperationFailed {
                                                context: "list volumes after remove".into(),
                                                message: e.to_string(),
                                            },
                                        ));
                                    }
                                },
                                Err(e) => {
                                    let mut message = e.to_string();
                                    let lower = message.to_lowercase();
                                    if lower.contains("in use") || lower.contains("is being used") {
                                        message = format!(
                                            "Cannot remove: volume is still in use by a container. {message}"
                                        );
                                    }
                                    let _ = app_tx.send(AppEvent::Docker(
                                        DockerEvent::OperationFailed {
                                            context: "remove volume".into(),
                                            message,
                                        },
                                    ));
                                }
                            }
                        }
                        WorkerCommand::RemoveImage { image_ref } => {
                            match client.remove_image(&image_ref).await {
                                Ok(()) => match client.list_images().await {
                                    Ok(rows) => {
                                        let _ = app_tx.send(AppEvent::Docker(
                                            DockerEvent::ImageRemoved {
                                                image_ref: image_ref.clone(),
                                            },
                                        ));
                                        let _ = app_tx.send(AppEvent::Docker(
                                            DockerEvent::ImagesLoaded(rows),
                                        ));
                                    }
                                    Err(e) => {
                                        let _ = app_tx.send(AppEvent::Docker(
                                            DockerEvent::OperationFailed {
                                                context: "list images after remove".into(),
                                                message: e.to_string(),
                                            },
                                        ));
                                    }
                                },
                                Err(e) => {
                                    let _ = app_tx.send(AppEvent::Docker(
                                        DockerEvent::OperationFailed {
                                            context: "remove image".into(),
                                            message: e.to_string(),
                                        },
                                    ));
                                }
                            }
                        }
                        WorkerCommand::ContainerLifecycle {
                            container_id,
                            action,
                        } => {
                            let res = match action {
                                ContainerLifecycleAction::Start => {
                                    client.start_container(&container_id).await
                                }
                                ContainerLifecycleAction::Stop => {
                                    client.stop_container(&container_id).await
                                }
                                ContainerLifecycleAction::Pause => {
                                    client.pause_container(&container_id).await
                                }
                                ContainerLifecycleAction::Unpause => {
                                    client.unpause_container(&container_id).await
                                }
                                ContainerLifecycleAction::Kill => {
                                    client.kill_container(&container_id).await
                                }
                                ContainerLifecycleAction::Remove => {
                                    client.remove_container(&container_id).await
                                }
                            };
                            match res {
                                Ok(()) => match client.list_containers(true).await {
                                    Ok(rows) => {
                                        let _ = app_tx.send(AppEvent::Docker(
                                            DockerEvent::ContainerLifecycleFinished,
                                        ));
                                        let _ = app_tx.send(AppEvent::Docker(
                                            DockerEvent::ContainersLoaded(rows),
                                        ));
                                    }
                                    Err(e) => {
                                        let _ = app_tx.send(AppEvent::Docker(
                                            DockerEvent::OperationFailed {
                                                context: "list containers".into(),
                                                message: e.to_string(),
                                            },
                                        ));
                                    }
                                },
                                Err(e) => {
                                    let context = match action {
                                        ContainerLifecycleAction::Start => "start container",
                                        ContainerLifecycleAction::Stop => "stop container",
                                        ContainerLifecycleAction::Pause => "pause container",
                                        ContainerLifecycleAction::Unpause => "unpause container",
                                        ContainerLifecycleAction::Kill => "kill container",
                                        ContainerLifecycleAction::Remove => "remove container",
                                    };
                                    let _ = app_tx.send(AppEvent::Docker(
                                        DockerEvent::OperationFailed {
                                            context: context.into(),
                                            message: e.to_string(),
                                        },
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    })
}
