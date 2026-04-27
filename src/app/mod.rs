mod state;

pub use state::AppState;

use std::io::stdout;

use anyhow::{Context, Result};
use crossterm::{
    event::{Event, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, widgets::TableState};
use tokio::sync::mpsc;

use crate::docker::types::{ContainerInspectSnapshot, ContainerStatsSnapshot};
use crate::docker::DockerClient;
use crate::events::{AppEvent, AppScreen, ContainerDetailTab, DockerEvent, MainView};
use crate::runtime::{
    ContainerLifecycleAction, WorkerCommand, spawn_docker_worker, spawn_input_forwarder,
};
use crate::ui;

/// Entry point: terminal setup, channels, background tasks, and the main event loop.
pub async fn run() -> Result<()> {
    let docker = DockerClient::connect().context("connect to Docker socket")?;

    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen).context("enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("create terminal")?;

    let (app_tx, mut app_rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    let (worker_cmd_tx, worker_cmd_rx) = mpsc::channel::<WorkerCommand>(8);

    spawn_input_forwarder(app_tx.clone());
    let _worker = spawn_docker_worker(docker, app_tx.clone(), worker_cmd_rx);

    let mut state = AppState::new();
    state.status_line =
        "1-5: tab · Tab: cycle · ↑↓ j/k · PgUp/Dn · Enter: details · r: refresh · q: quit"
            .to_string();

    let mut tick = tokio::time::interval(std::time::Duration::from_millis(500));

    let result: Result<()> = loop {
        terminal
            .draw(|f| ui::draw(f, &mut state))
            .context("draw frame")?;

        tokio::select! {
            Some(event) = app_rx.recv() => {
                match handle_app_event(
                    event,
                    &mut state,
                    &worker_cmd_tx,
                ) {
                    LoopCtl::Continue => {}
                    LoopCtl::Quit => break Ok(()),
                }
            }
            _ = tick.tick() => {}
        }
    };

    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();

    result
}

enum LoopCtl {
    Continue,
    Quit,
}

fn selected_container_id(state: &AppState) -> Option<String> {
    let i = state.table_state.selected()?;
    state.containers.get(i).map(|c| c.id.clone())
}

fn selected_container_full_id(state: &AppState) -> Option<String> {
    let i = state.table_state.selected()?;
    state.containers.get(i).map(|c| c.full_id.clone())
}

fn try_container_lifecycle(
    state: &mut AppState,
    worker_cmd_tx: &mpsc::Sender<WorkerCommand>,
    action: ContainerLifecycleAction,
) {
    if state.screen != AppScreen::List || state.view != MainView::Containers {
        return;
    }
    if state.container_action_pending.is_some() {
        return;
    }
    let Some(id) = selected_container_full_id(state) else {
        return;
    };
    if worker_cmd_tx
        .try_send(WorkerCommand::ContainerLifecycle {
            container_id: id.clone(),
            action,
        })
        .is_ok()
    {
        state.container_action_pending = Some(id);
    }
}

fn request_container_logs(
    state: &mut AppState,
    worker_cmd_tx: &mpsc::Sender<WorkerCommand>,
    force: bool,
) {
    if state.screen != AppScreen::Detail || state.view != MainView::Containers {
        return;
    }
    let Some(id) = selected_container_id(state) else {
        return;
    };
    if !force {
        if state.container_logs_for_id.as_deref() == Some(id.as_str())
            && state.container_logs.is_some()
        {
            return;
        }
    }
    state.container_logs = None;
    state.container_logs_for_id = None;
    let _ = worker_cmd_tx.try_send(WorkerCommand::FetchContainerLogs { container_id: id });
}

fn request_container_stats(
    state: &mut AppState,
    worker_cmd_tx: &mpsc::Sender<WorkerCommand>,
    force: bool,
) {
    if state.screen != AppScreen::Detail || state.view != MainView::Containers {
        return;
    }
    let Some(id) = selected_container_id(state) else {
        return;
    };
    if !force {
        if state.container_stats_for_id.as_deref() == Some(id.as_str())
            && state.container_stats.is_some()
        {
            return;
        }
    }
    state.container_stats = None;
    state.container_stats_for_id = None;
    let _ = worker_cmd_tx.try_send(WorkerCommand::FetchContainerStats { container_id: id });
}

fn handle_image_delete_confirm_key(
    key: KeyEvent,
    state: &mut AppState,
    worker_cmd_tx: &mpsc::Sender<WorkerCommand>,
) -> LoopCtl {
    match key.code {
        KeyCode::Esc => {
            state.image_delete_confirm = false;
        }
        KeyCode::Enter => {
            if let Some(id) = state.image_detail_ref.clone() {
                if worker_cmd_tx
                    .try_send(WorkerCommand::RemoveImage {
                        image_ref: id.clone(),
                    })
                    .is_ok()
                {
                    state.image_remove_pending = Some(id);
                    state.image_delete_confirm = false;
                }
            }
        }
        KeyCode::Char(c) => match c {
            'n' | 'N' | 'q' | 'Q' => {
                state.image_delete_confirm = false;
            }
            'y' | 'Y' => {
                if let Some(id) = state.image_detail_ref.clone() {
                    if worker_cmd_tx
                        .try_send(WorkerCommand::RemoveImage {
                            image_ref: id.clone(),
                        })
                        .is_ok()
                    {
                        state.image_remove_pending = Some(id);
                        state.image_delete_confirm = false;
                    }
                }
            }
            _ => {}
        },
        _ => {}
    }
    LoopCtl::Continue
}

fn handle_app_event(
    event: AppEvent,
    state: &mut AppState,
    worker_cmd_tx: &mpsc::Sender<WorkerCommand>,
) -> LoopCtl {
    match event {
        AppEvent::Terminal(ev) => handle_terminal(ev, state, worker_cmd_tx),
        AppEvent::Docker(ev) => {
            match ev {
                DockerEvent::DashboardLoaded(snapshot) => {
                    state.dashboard = Some(snapshot);
                }
                DockerEvent::ContainersLoaded(rows) => {
                    state.containers = rows;
                    state.clamp_selection();
                    if state.screen == AppScreen::Detail && state.view == MainView::Containers {
                        if let Some(i) = state.table_state.selected() {
                            if let Some(c) = state.containers.get(i) {
                                let same = state.container_inspect_id.as_deref()
                                    == Some(c.id.as_str());
                                if !same {
                                    state.container_inspect = None;
                                    state.container_inspect_json = None;
                                    state.container_inspect_json_scroll = 0;
                                    state.container_logs = None;
                                    state.container_logs_for_id = None;
                                    state.container_logs_scroll = 0;
                                    state.container_stats = None;
                                    state.container_stats_for_id = None;
                                    state.container_stats_scroll = 0;
                                }
                                state.container_inspect_id = Some(c.id.clone());
                                let _ = worker_cmd_tx.try_send(WorkerCommand::FetchContainerInspect {
                                    container_id: c.id.clone(),
                                });
                            }
                        }
                    }
                }
                DockerEvent::ContainerInspectLoaded {
                    container_id,
                    detail,
                    inspect_json,
                } => {
                    if state.screen == AppScreen::Detail && state.view == MainView::Containers {
                        if state.container_inspect_id.as_deref() == Some(container_id.as_str()) {
                            state.container_inspect = Some(detail);
                            state.container_inspect_json = Some(inspect_json);
                            state.container_inspect_json_scroll = 0;
                        }
                    }
                }
                DockerEvent::ContainerLogsLoaded { container_id, text } => {
                    if state.screen == AppScreen::Detail && state.view == MainView::Containers {
                        if state.container_inspect_id.as_deref() == Some(container_id.as_str()) {
                            state.container_logs = Some(text);
                            state.container_logs_for_id = Some(container_id);
                            state.container_logs_scroll = usize::MAX;
                        }
                    }
                }
                DockerEvent::ContainerStatsLoaded { container_id, stats } => {
                    if state.screen == AppScreen::Detail && state.view == MainView::Containers {
                        if state.container_inspect_id.as_deref() == Some(container_id.as_str()) {
                            state.container_stats = Some(stats);
                            state.container_stats_for_id = Some(container_id);
                            state.container_stats_scroll = 0;
                        }
                    }
                }
                DockerEvent::ImagesLoaded(rows) => {
                    state.images = rows;
                    state.clamp_selection();
                    if state.screen == AppScreen::Detail && state.view == MainView::Images {
                        if let Some(i) = state.table_state.selected() {
                            if let Some(im) = state.images.get(i) {
                                let same =
                                    state.image_detail_ref.as_deref() == Some(im.id.as_str());
                                if !same {
                                    state.image_detail = None;
                                }
                                state.image_detail_ref = Some(im.id.clone());
                                let _ = worker_cmd_tx.try_send(WorkerCommand::FetchImageInspect {
                                    image_ref: im.id.clone(),
                                });
                            }
                        }
                    }
                }
                DockerEvent::ImageInspectLoaded { image_ref, detail } => {
                    if state.screen == AppScreen::Detail && state.view == MainView::Images {
                        if state.image_detail_ref.as_deref() == Some(image_ref.as_str()) {
                            state.image_detail = Some(detail);
                        }
                    }
                }
                DockerEvent::NetworksLoaded(rows) => {
                    state.networks = rows;
                    state.clamp_selection();
                    if state.screen == AppScreen::Detail && state.view == MainView::Networks {
                        if let Some(i) = state.table_state.selected() {
                            if let Some(net) = state.networks.get(i) {
                                if net.is_system {
                                    state.network_containers = None;
                                    state.network_containers_network_id = None;
                                    state.network_containers_table_state = TableState::default();
                                } else {
                                    let same = state.network_containers_network_id.as_deref()
                                        == Some(net.id.as_str());
                                    if !same {
                                        state.network_containers = None;
                                        state.network_containers_network_id = None;
                                        state.network_containers_table_state =
                                            TableState::default();
                                    }
                                    let _ = worker_cmd_tx.try_send(
                                        WorkerCommand::FetchNetworkContainers {
                                            network_id: net.id.clone(),
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
                DockerEvent::NetworkContainersLoaded { network_id, rows } => {
                    if state.screen == AppScreen::Detail && state.view == MainView::Networks {
                        if let Some(i) = state.table_state.selected() {
                            if state.networks.get(i).map(|n| n.id.as_str())
                                == Some(network_id.as_str())
                            {
                                state.network_containers = Some(rows);
                                state.network_containers_network_id = Some(network_id);
                                state.network_containers_table_state = TableState::default();
                            }
                        }
                    }
                }
                DockerEvent::VolumesLoaded(rows) => {
                    state.volumes = rows;
                    state.clamp_selection();
                    if state.screen == AppScreen::Detail && state.view == MainView::Volumes {
                        if let Some(i) = state.table_state.selected() {
                            if let Some(name) = state.volumes.get(i).map(|v| v.name.clone()) {
                                let same_volume = state.volume_mount_users_volume.as_deref()
                                    == Some(name.as_str());
                                if !same_volume {
                                    state.volume_mount_users = None;
                                    state.volume_mount_users_volume = None;
                                    state.volume_mount_users_table_state = TableState::default();
                                }
                                let _ = worker_cmd_tx.try_send(WorkerCommand::FetchVolumeMounts {
                                    volume_name: name,
                                });
                            }
                        }
                    }
                }
                DockerEvent::VolumeMountsLoaded { volume_name, rows } => {
                    if state.screen == AppScreen::Detail && state.view == MainView::Volumes {
                        if let Some(i) = state.table_state.selected() {
                            if state.volumes.get(i).map(|v| v.name.as_str())
                                == Some(volume_name.as_str())
                            {
                                state.volume_mount_users = Some(rows);
                                state.volume_mount_users_volume = Some(volume_name);
                                state.volume_mount_users_table_state = TableState::default();
                            }
                        }
                    }
                }
                DockerEvent::VolumeRemoved { .. } => {
                    state.volume_remove_pending = None;
                    if state.screen == AppScreen::Detail && state.view == MainView::Volumes {
                        state.close_detail();
                    }
                }
                DockerEvent::ImageRemoved { .. } => {
                    state.image_remove_pending = None;
                    if state.screen == AppScreen::Detail && state.view == MainView::Images {
                        state.close_detail();
                    }
                }
                DockerEvent::ContainerLifecycleFinished => {
                    state.container_action_pending = None;
                }
                DockerEvent::OperationFailed { context, message } => {
                    state.container_action_pending = None;
                    state.error_banner = Some(format!("{context}: {message}"));
                    if context == "inspect container"
                        && state.screen == AppScreen::Detail
                        && state.view == MainView::Containers
                    {
                        state.container_inspect = Some(ContainerInspectSnapshot {
                            started_at: "—".into(),
                            published_ports: "—".into(),
                            mounts: vec![],
                            networks: vec![],
                        });
                        state.container_inspect_json =
                            Some(format!("Could not load inspect JSON: {message}"));
                    }
                    if context == "container logs"
                        && state.screen == AppScreen::Detail
                        && state.view == MainView::Containers
                    {
                        state.container_logs = Some(format!("Could not load logs: {message}"));
                        state.container_logs_for_id = state.container_inspect_id.clone();
                    }
                    if context == "container stats"
                        && state.screen == AppScreen::Detail
                        && state.view == MainView::Containers
                    {
                        state.container_stats = Some(ContainerStatsSnapshot {
                            lines: vec![format!("Could not load stats: {message}")],
                        });
                        state.container_stats_for_id = state.container_inspect_id.clone();
                    }
                    if matches!(
                        context.as_str(),
                        "remove volume" | "list volumes after remove"
                    ) {
                        state.volume_remove_pending = None;
                    }
                    if matches!(
                        context.as_str(),
                        "remove image" | "list images after remove"
                    ) {
                        state.image_remove_pending = None;
                    }
                }
            }
            LoopCtl::Continue
        }
        AppEvent::Tick => LoopCtl::Continue,
    }
}

fn handle_terminal(
    ev: Event,
    state: &mut AppState,
    worker_cmd_tx: &mpsc::Sender<WorkerCommand>,
) -> LoopCtl {
    match ev {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            if state.image_delete_confirm {
                return handle_image_delete_confirm_key(key, state, worker_cmd_tx);
            }
            // Quit from the main list even when the error popup is visible (otherwise q/Esc only
            // dismiss the modal and a failing background refresh can immediately show it again).
            if state.screen == AppScreen::List {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                        return LoopCtl::Quit;
                    }
                    _ => {}
                }
            }
            if state.error_banner.is_some() {
                match key.code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char(' ') => {
                        state.error_banner = None;
                    }
                    KeyCode::Char('q') | KeyCode::Char('Q') => {
                        state.error_banner = None;
                    }
                    _ => {}
                }
                return LoopCtl::Continue;
            }
            match key.code {
                KeyCode::Esc => {
                    if state.screen == AppScreen::Detail {
                        state.close_detail();
                    }
                }
                KeyCode::Char('q') | KeyCode::Char('Q') => {
                    if state.screen == AppScreen::Detail {
                        state.close_detail();
                    } else {
                        return LoopCtl::Quit;
                    }
                }
                KeyCode::Char('1') => {
                    if state.screen == AppScreen::Detail && state.view == MainView::Containers {
                        state.container_detail_tab = ContainerDetailTab::Details;
                    } else if state.screen == AppScreen::List {
                        let next = MainView::Dashboard;
                        if state.view != next {
                            state.view = next;
                            state.reset_tab_scroll();
                        }
                    }
                }
                KeyCode::Char('2') => {
                    if state.screen == AppScreen::Detail && state.view == MainView::Containers {
                        state.container_detail_tab = ContainerDetailTab::Logs;
                        request_container_logs(state, worker_cmd_tx, false);
                    } else if state.screen == AppScreen::List {
                        let next = MainView::Containers;
                        if state.view != next {
                            state.view = next;
                            state.reset_tab_scroll();
                        }
                    }
                }
                KeyCode::Char('3') => {
                    if state.screen == AppScreen::Detail && state.view == MainView::Containers {
                        state.container_detail_tab = ContainerDetailTab::Inspect;
                    } else if state.screen == AppScreen::List {
                        let next = MainView::Images;
                        if state.view != next {
                            state.view = next;
                            state.reset_tab_scroll();
                        }
                    }
                }
                KeyCode::Char('4') => {
                    if state.screen == AppScreen::Detail && state.view == MainView::Containers {
                        state.container_detail_tab = ContainerDetailTab::Stats;
                        request_container_stats(state, worker_cmd_tx, false);
                    } else if state.screen == AppScreen::List {
                        let next = MainView::Networks;
                        if state.view != next {
                            state.view = next;
                            state.reset_tab_scroll();
                        }
                    }
                }
                KeyCode::Char('5') => {
                    if state.screen == AppScreen::List {
                        let next = MainView::Volumes;
                        if state.view != next {
                            state.view = next;
                            state.reset_tab_scroll();
                        }
                    }
                }
                KeyCode::Char('s')
                    if state.screen == AppScreen::List && state.view == MainView::Containers =>
                {
                    try_container_lifecycle(
                        state,
                        worker_cmd_tx,
                        ContainerLifecycleAction::Start,
                    );
                }
                KeyCode::Char('t')
                    if state.screen == AppScreen::List && state.view == MainView::Containers =>
                {
                    try_container_lifecycle(state, worker_cmd_tx, ContainerLifecycleAction::Stop);
                }
                KeyCode::Char('p')
                    if state.screen == AppScreen::List && state.view == MainView::Containers =>
                {
                    try_container_lifecycle(
                        state,
                        worker_cmd_tx,
                        ContainerLifecycleAction::Pause,
                    );
                }
                KeyCode::Char('u')
                    if state.screen == AppScreen::List && state.view == MainView::Containers =>
                {
                    try_container_lifecycle(
                        state,
                        worker_cmd_tx,
                        ContainerLifecycleAction::Unpause,
                    );
                }
                KeyCode::Char('k')
                    if state.screen == AppScreen::List && state.view == MainView::Containers =>
                {
                    try_container_lifecycle(state, worker_cmd_tx, ContainerLifecycleAction::Kill);
                }
                KeyCode::Char('d')
                    if state.screen == AppScreen::List && state.view == MainView::Containers =>
                {
                    try_container_lifecycle(
                        state,
                        worker_cmd_tx,
                        ContainerLifecycleAction::Remove,
                    );
                }
                KeyCode::Tab => {
                    if state.screen == AppScreen::List {
                        state.view = match state.view {
                            MainView::Dashboard => MainView::Containers,
                            MainView::Containers => MainView::Images,
                            MainView::Images => MainView::Networks,
                            MainView::Networks => MainView::Volumes,
                            MainView::Volumes => MainView::Dashboard,
                        };
                        state.reset_tab_scroll();
                    }
                }
                KeyCode::Enter => {
                    if state.screen != AppScreen::List {
                        return LoopCtl::Continue;
                    }
                    if state.view == MainView::Volumes {
                        if let Some(i) = state.table_state.selected() {
                            if let Some(name) = state.volumes.get(i).map(|v| v.name.clone()) {
                                if state.open_detail() {
                                    state.volume_mount_users = None;
                                    state.volume_mount_users_volume = None;
                                    state.volume_mount_users_table_state = TableState::default();
                                    let _ =
                                        worker_cmd_tx.try_send(WorkerCommand::FetchVolumeMounts {
                                            volume_name: name,
                                        });
                                }
                            }
                        }
                    } else if state.view == MainView::Networks {
                        if let Some(i) = state.table_state.selected() {
                            if let Some(net) = state.networks.get(i) {
                                let network_id = net.id.clone();
                                let is_system = net.is_system;
                                if state.open_detail() {
                                    if !is_system {
                                        state.network_containers = None;
                                        state.network_containers_network_id = None;
                                        state.network_containers_table_state =
                                            TableState::default();
                                        let _ = worker_cmd_tx.try_send(
                                            WorkerCommand::FetchNetworkContainers { network_id },
                                        );
                                    }
                                }
                            }
                        }
                    } else if state.view == MainView::Images {
                        if let Some(i) = state.table_state.selected() {
                            let image_ref = state.images.get(i).map(|im| im.id.clone());
                            if let Some(image_ref) = image_ref {
                                state.image_detail_scroll = 0;
                                state.image_detail = None;
                                state.image_detail_ref = Some(image_ref.clone());
                                if state.open_detail() {
                                    let _ = worker_cmd_tx
                                        .try_send(WorkerCommand::FetchImageInspect { image_ref });
                                }
                            }
                        }
                    } else if state.view == MainView::Containers {
                        state.container_detail_scroll = 0;
                        state.container_detail_tab = ContainerDetailTab::Details;
                        state.container_logs = None;
                        state.container_logs_for_id = None;
                        state.container_logs_scroll = 0;
                        state.container_inspect_json = None;
                        state.container_inspect_json_scroll = 0;
                        state.container_stats = None;
                        state.container_stats_for_id = None;
                        state.container_stats_scroll = 0;
                        if state.open_detail() {
                            if let Some(i) = state.table_state.selected() {
                                if let Some(c) = state.containers.get(i) {
                                    state.container_inspect_id = Some(c.id.clone());
                                    state.container_inspect = None;
                                    let _ = worker_cmd_tx.try_send(WorkerCommand::FetchContainerInspect {
                                        container_id: c.id.clone(),
                                    });
                                }
                            }
                        }
                    } else if state.view != MainView::Dashboard {
                        let _ = state.open_detail();
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if state.screen == AppScreen::Detail {
                        if state.view == MainView::Networks && state.network_containers_scrollable()
                        {
                            state.scroll_network_containers_table(1);
                        } else if state.view == MainView::Volumes
                            && state.volume_mount_users_scrollable()
                        {
                            state.scroll_volume_mount_users_table(1);
                        } else if state.view == MainView::Images {
                            state.image_detail_scroll = state.image_detail_scroll.saturating_add(1);
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Details
                        {
                            state.container_detail_scroll =
                                state.container_detail_scroll.saturating_add(1);
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Logs
                        {
                            state.container_logs_scroll =
                                state.container_logs_scroll.saturating_add(1);
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Inspect
                        {
                            state.container_inspect_json_scroll =
                                state.container_inspect_json_scroll.saturating_add(1);
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Stats
                        {
                            state.container_stats_scroll =
                                state.container_stats_scroll.saturating_add(1);
                        }
                        return LoopCtl::Continue;
                    }
                    if state.view == MainView::Dashboard {
                        state.dashboard_scroll = state.dashboard_scroll.saturating_add(1);
                    } else {
                        state.move_selection_next();
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if state.screen == AppScreen::Detail {
                        if state.view == MainView::Networks && state.network_containers_scrollable()
                        {
                            state.scroll_network_containers_table(-1);
                        } else if state.view == MainView::Volumes
                            && state.volume_mount_users_scrollable()
                        {
                            state.scroll_volume_mount_users_table(-1);
                        } else if state.view == MainView::Images {
                            state.image_detail_scroll = state.image_detail_scroll.saturating_sub(1);
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Details
                        {
                            state.container_detail_scroll =
                                state.container_detail_scroll.saturating_sub(1);
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Logs
                        {
                            state.container_logs_scroll =
                                state.container_logs_scroll.saturating_sub(1);
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Inspect
                        {
                            state.container_inspect_json_scroll =
                                state.container_inspect_json_scroll.saturating_sub(1);
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Stats
                        {
                            state.container_stats_scroll =
                                state.container_stats_scroll.saturating_sub(1);
                        }
                        return LoopCtl::Continue;
                    }
                    if state.view == MainView::Dashboard {
                        state.dashboard_scroll = state.dashboard_scroll.saturating_sub(1);
                    } else {
                        state.move_selection_prev();
                    }
                }
                KeyCode::PageDown => {
                    if state.screen == AppScreen::Detail {
                        if state.view == MainView::Networks && state.network_containers_scrollable()
                        {
                            let step = state.network_containers_viewport_rows.max(1) as isize;
                            state.scroll_network_containers_table(step);
                        } else if state.view == MainView::Volumes
                            && state.volume_mount_users_scrollable()
                        {
                            let step = state.volume_mount_users_viewport_rows.max(1) as isize;
                            state.scroll_volume_mount_users_table(step);
                        } else if state.view == MainView::Images {
                            state.image_detail_scroll = state.image_detail_scroll.saturating_add(8);
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Details
                        {
                            state.container_detail_scroll =
                                state.container_detail_scroll.saturating_add(8);
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Logs
                        {
                            state.container_logs_scroll =
                                state.container_logs_scroll.saturating_add(8);
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Inspect
                        {
                            state.container_inspect_json_scroll =
                                state.container_inspect_json_scroll.saturating_add(8);
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Stats
                        {
                            state.container_stats_scroll =
                                state.container_stats_scroll.saturating_add(8);
                        }
                        return LoopCtl::Continue;
                    }
                    if state.view == MainView::Dashboard {
                        state.dashboard_scroll = state.dashboard_scroll.saturating_add(8);
                    } else {
                        state.move_selection_page_down();
                    }
                }
                KeyCode::PageUp => {
                    if state.screen == AppScreen::Detail {
                        if state.view == MainView::Networks && state.network_containers_scrollable()
                        {
                            let step = -(state.network_containers_viewport_rows.max(1) as isize);
                            state.scroll_network_containers_table(step);
                        } else if state.view == MainView::Volumes
                            && state.volume_mount_users_scrollable()
                        {
                            let step = -(state.volume_mount_users_viewport_rows.max(1) as isize);
                            state.scroll_volume_mount_users_table(step);
                        } else if state.view == MainView::Images {
                            state.image_detail_scroll = state.image_detail_scroll.saturating_sub(8);
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Details
                        {
                            state.container_detail_scroll =
                                state.container_detail_scroll.saturating_sub(8);
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Logs
                        {
                            state.container_logs_scroll =
                                state.container_logs_scroll.saturating_sub(8);
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Inspect
                        {
                            state.container_inspect_json_scroll =
                                state.container_inspect_json_scroll.saturating_sub(8);
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Stats
                        {
                            state.container_stats_scroll =
                                state.container_stats_scroll.saturating_sub(8);
                        }
                        return LoopCtl::Continue;
                    }
                    if state.view == MainView::Dashboard {
                        state.dashboard_scroll = state.dashboard_scroll.saturating_sub(8);
                    } else {
                        state.move_selection_page_up();
                    }
                }
                KeyCode::Home => {
                    if state.screen == AppScreen::Detail {
                        if state.view == MainView::Images {
                            state.image_detail_scroll = 0;
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Details
                        {
                            state.container_detail_scroll = 0;
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Logs
                        {
                            state.container_logs_scroll = 0;
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Inspect
                        {
                            state.container_inspect_json_scroll = 0;
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Stats
                        {
                            state.container_stats_scroll = 0;
                        }
                        return LoopCtl::Continue;
                    }
                    if state.view == MainView::Dashboard {
                        state.dashboard_scroll = 0;
                    } else if state.selected_len() > 0 {
                        state.table_state.select(Some(0));
                    }
                }
                KeyCode::End => {
                    if state.screen == AppScreen::Detail {
                        if state.view == MainView::Images {
                            state.image_detail_scroll = usize::MAX;
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Details
                        {
                            state.container_detail_scroll = usize::MAX;
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Logs
                        {
                            state.container_logs_scroll = usize::MAX;
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Inspect
                        {
                            state.container_inspect_json_scroll = usize::MAX;
                        } else if state.view == MainView::Containers
                            && state.container_detail_tab == ContainerDetailTab::Stats
                        {
                            state.container_stats_scroll = usize::MAX;
                        }
                        return LoopCtl::Continue;
                    }
                    if state.view == MainView::Dashboard {
                        state.dashboard_scroll = usize::MAX;
                    } else {
                        let len = state.selected_len();
                        if len > 0 {
                            state.table_state.select(Some(len - 1));
                        }
                    }
                }
                KeyCode::Char('r') => {
                    let _ = worker_cmd_tx.try_send(WorkerCommand::Refresh);
                    if state.screen == AppScreen::Detail
                        && state.view == MainView::Containers
                        && state.container_detail_tab == ContainerDetailTab::Logs
                    {
                        request_container_logs(state, worker_cmd_tx, true);
                    }
                    if state.screen == AppScreen::Detail
                        && state.view == MainView::Containers
                        && state.container_detail_tab == ContainerDetailTab::Inspect
                    {
                        if let Some(i) = state.table_state.selected() {
                            if let Some(c) = state.containers.get(i) {
                                state.container_inspect = None;
                                state.container_inspect_json = None;
                                state.container_inspect_json_scroll = 0;
                                let _ = worker_cmd_tx.try_send(WorkerCommand::FetchContainerInspect {
                                    container_id: c.id.clone(),
                                });
                            }
                        }
                    }
                    if state.screen == AppScreen::Detail
                        && state.view == MainView::Containers
                        && state.container_detail_tab == ContainerDetailTab::Stats
                    {
                        request_container_stats(state, worker_cmd_tx, true);
                    }
                }
                KeyCode::Char('d') => {
                    if state.screen == AppScreen::Detail && state.view == MainView::Volumes {
                        if !state.volume_delete_enabled() {
                            return LoopCtl::Continue;
                        }
                        if let Some(i) = state.table_state.selected() {
                            if let Some(name) = state.volumes.get(i).map(|v| v.name.clone()) {
                                match worker_cmd_tx
                                    .try_send(WorkerCommand::RemoveVolume { name: name.clone() })
                                {
                                    Ok(()) => {
                                        state.volume_remove_pending = Some(name);
                                    }
                                    Err(_) => {}
                                }
                            }
                        }
                    } else if state.screen == AppScreen::Detail && state.view == MainView::Images {
                        if !state.image_delete_enabled() {
                            return LoopCtl::Continue;
                        }
                        state.image_delete_confirm = true;
                    }
                }
                _ => {}
            }
        }
        Event::Resize(_, _) => {}
        _ => {}
    }
    LoopCtl::Continue
}
