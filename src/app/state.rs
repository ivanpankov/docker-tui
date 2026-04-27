use ratatui::widgets::TableState;

use crate::docker::types::{
    ContainerInspectSnapshot, ContainerRow, ContainerStatsSnapshot, DashboardSnapshot,
    ImageDetailSnapshot, ImageRow, NetworkAttachedContainerRow, NetworkRow,
    VolumeContainerMountRow, VolumeRow,
};
use crate::events::{AppScreen, ContainerDetailTab, MainView};

/// UI and data snapshot for one frame.
pub struct AppState {
    pub view: MainView,
    pub screen: AppScreen,
    /// First visible line index for the Dashboard scroll view.
    pub dashboard_scroll: usize,
    /// First visible line index for the Images detail (three-panel) scroll view.
    pub image_detail_scroll: usize,
    /// First visible line index for the Containers detail (four-panel) scroll view.
    pub container_detail_scroll: usize,
    /// First visible line for the container Logs sub-tab.
    pub container_logs_scroll: usize,
    /// `GET /images/{id}/json` payload for the open image (`None` until loaded).
    pub image_detail: Option<ImageDetailSnapshot>,
    /// Short image ID from the list row for this request (stale-while-revalidate on list refresh).
    pub image_detail_ref: Option<String>,
    pub dashboard: Option<DashboardSnapshot>,
    pub containers: Vec<ContainerRow>,
    pub images: Vec<ImageRow>,
    pub networks: Vec<NetworkRow>,
    pub volumes: Vec<VolumeRow>,
    pub table_state: TableState,
    /// Data rows visible in the last table paint (for PageUp/PageDown step).
    pub table_viewport_rows: usize,
    pub status_line: String,
    pub error_banner: Option<String>,
    /// `None` until [`crate::events::DockerEvent::VolumeMountsLoaded`] for the open volume.
    pub volume_mount_users: Option<Vec<VolumeContainerMountRow>>,
    /// Volume name that [`Self::volume_mount_users`] belongs to (for stale-while-revalidate on list refresh).
    pub volume_mount_users_volume: Option<String>,
    /// Scroll state for the "Containers using volume" table (separate from the main list [`Self::table_state`]).
    pub volume_mount_users_table_state: TableState,
    /// Last viewport height (data rows) for that table; used for PageUp/PageDown scroll step.
    pub volume_mount_users_viewport_rows: usize,
    /// Set while a [`crate::runtime::WorkerCommand::RemoveVolume`] is in flight (blocks repeat `d`).
    pub volume_remove_pending: Option<String>,
    /// Set while a [`crate::runtime::WorkerCommand::RemoveImage`] is in flight (blocks repeat `d`).
    pub image_remove_pending: Option<String>,
    /// Set while a container lifecycle command is in flight from the containers list (blocks repeat shortcuts).
    pub container_action_pending: Option<String>,
    /// `d` on image detail opened the delete confirmation dialog (blocks interaction until dismissed).
    pub image_delete_confirm: bool,
    /// `None` until [`crate::events::DockerEvent::NetworkContainersLoaded`] for the open network (non-system only).
    pub network_containers: Option<Vec<NetworkAttachedContainerRow>>,
    /// Network ID that [`Self::network_containers`] belongs to (stale-while-revalidate on list refresh).
    pub network_containers_network_id: Option<String>,
    /// Scroll state for the "Containers in network" table (separate from the main list [`Self::table_state`]).
    pub network_containers_table_state: TableState,
    /// Last viewport height (data rows) for the containers table; used for PageUp/PageDown scroll step.
    pub network_containers_viewport_rows: usize,
    /// Short container ID for the open container detail (inspect ref).
    pub container_inspect_id: Option<String>,
    /// Result of `docker inspect` for the open container (`None` until loaded).
    pub container_inspect: Option<ContainerInspectSnapshot>,
    /// Pretty-printed full inspect JSON for the Inspect sub-tab (`None` until loaded).
    pub container_inspect_json: Option<String>,
    /// Scroll offset for the Inspect sub-tab JSON view.
    pub container_inspect_json_scroll: usize,
    /// Active section within container detail (`1`–`4`); default is [`ContainerDetailTab::Details`].
    pub container_detail_tab: ContainerDetailTab,
    /// Log text for [`Self::container_logs_for_id`] (`None` while loading).
    pub container_logs: Option<String>,
    /// Container ID the current [`Self::container_logs`] belongs to (`None` if not loaded).
    pub container_logs_for_id: Option<String>,
    /// One-shot stats for [`Self::container_stats_for_id`] (`None` while loading).
    pub container_stats: Option<ContainerStatsSnapshot>,
    /// Container ID for [`Self::container_stats`] (`None` if not loaded).
    pub container_stats_for_id: Option<String>,
    /// Scroll offset for the Stats sub-tab.
    pub container_stats_scroll: usize,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            view: MainView::default(),
            screen: AppScreen::default(),
            dashboard_scroll: 0,
            image_detail_scroll: 0,
            container_detail_scroll: 0,
            container_logs_scroll: 0,
            image_detail: None,
            image_detail_ref: None,
            dashboard: None,
            containers: Vec::new(),
            images: Vec::new(),
            networks: Vec::new(),
            volumes: Vec::new(),
            table_state: TableState::default().with_selected(0),
            table_viewport_rows: 8,
            status_line: String::new(),
            error_banner: None,
            volume_mount_users: None,
            volume_mount_users_volume: None,
            volume_mount_users_table_state: TableState::default(),
            volume_mount_users_viewport_rows: 8,
            volume_remove_pending: None,
            image_remove_pending: None,
            container_action_pending: None,
            image_delete_confirm: false,
            network_containers: None,
            network_containers_network_id: None,
            network_containers_table_state: TableState::default(),
            network_containers_viewport_rows: 8,
            container_inspect_id: None,
            container_inspect: None,
            container_inspect_json: None,
            container_inspect_json_scroll: 0,
            container_detail_tab: ContainerDetailTab::default(),
            container_logs: None,
            container_logs_for_id: None,
            container_stats: None,
            container_stats_for_id: None,
            container_stats_scroll: 0,
        }
    }

    pub fn selected_len(&self) -> usize {
        match self.view {
            MainView::Dashboard => 0,
            MainView::Containers => self.containers.len(),
            MainView::Images => self.images.len(),
            MainView::Networks => self.networks.len(),
            MainView::Volumes => self.volumes.len(),
        }
    }

    /// Call after switching tabs: return to list, scroll to top, selection to first row.
    pub fn reset_tab_scroll(&mut self) {
        self.screen = AppScreen::List;
        self.volume_mount_users = None;
        self.volume_mount_users_volume = None;
        self.volume_mount_users_table_state = TableState::default();
        self.network_containers = None;
        self.network_containers_network_id = None;
        self.network_containers_table_state = TableState::default();
        self.container_inspect_id = None;
        self.container_inspect = None;
        self.container_inspect_json = None;
        self.container_inspect_json_scroll = 0;
        self.container_detail_tab = ContainerDetailTab::default();
        self.container_logs_scroll = 0;
        self.container_logs = None;
        self.container_logs_for_id = None;
        self.container_stats = None;
        self.container_stats_for_id = None;
        self.container_stats_scroll = 0;
        self.dashboard_scroll = 0;
        self.image_detail_scroll = 0;
        self.container_detail_scroll = 0;
        self.image_detail = None;
        self.image_detail_ref = None;
        self.image_delete_confirm = false;
        self.container_action_pending = None;
        *self.table_state.offset_mut() = 0;
        let len = self.selected_len();
        if len > 0 {
            self.table_state.select(Some(0));
        } else {
            self.table_state.select(None);
        }
    }

    pub fn move_selection_next(&mut self) {
        let len = self.selected_len();
        if len == 0 {
            self.table_state.select(None);
            return;
        }
        let i = self.table_state.selected().unwrap_or(0);
        let last = len.saturating_sub(1);
        let next = (i + 1).min(last);
        self.table_state.select(Some(next));
    }

    pub fn move_selection_prev(&mut self) {
        let len = self.selected_len();
        if len == 0 {
            self.table_state.select(None);
            return;
        }
        let i = self.table_state.selected().unwrap_or(0);
        let prev = i.saturating_sub(1);
        self.table_state.select(Some(prev));
    }

    pub fn move_selection_page_down(&mut self) {
        let len = self.selected_len();
        if len == 0 {
            return;
        }
        let page = self.table_viewport_rows.max(1);
        let i = self.table_state.selected().unwrap_or(0);
        let last = len.saturating_sub(1);
        let next = (i + page).min(last);
        self.table_state.select(Some(next));
    }

    pub fn move_selection_page_up(&mut self) {
        let len = self.selected_len();
        if len == 0 {
            return;
        }
        let page = self.table_viewport_rows.max(1);
        let i = self.table_state.selected().unwrap_or(0);
        let prev = i.saturating_sub(page);
        self.table_state.select(Some(prev));
    }

    pub fn clamp_selection(&mut self) {
        let len = self.selected_len();
        if len == 0 {
            self.table_state.select(None);
            return;
        }
        let i = self.table_state.selected().unwrap_or(0).min(len - 1);
        self.table_state.select(Some(i));
    }

    /// Human-readable label for the current selection (detail header); empty if none.
    pub fn detail_selection_label(&self) -> String {
        let Some(i) = self.table_state.selected() else {
            return "—".into();
        };
        match self.view {
            MainView::Dashboard => String::new(),
            MainView::Containers => self
                .containers
                .get(i)
                .map(|c| format!("{} — {}", c.id, c.names))
                .unwrap_or_else(|| "—".into()),
            MainView::Images => self
                .images
                .get(i)
                .map(|im| format!("{} — {}", im.id, im.tag))
                .unwrap_or_else(|| "—".into()),
            MainView::Networks => self
                .networks
                .get(i)
                .map(|n| {
                    let short: String = n.id.chars().take(12).collect();
                    format!("{short} — {}", n.name)
                })
                .unwrap_or_else(|| "—".into()),
            MainView::Volumes => self
                .volumes
                .get(i)
                .map(|v| v.name.clone())
                .unwrap_or_else(|| "—".into()),
        }
    }

    pub fn close_detail(&mut self) {
        self.screen = AppScreen::List;
        self.image_detail_scroll = 0;
        self.container_detail_scroll = 0;
        self.container_logs_scroll = 0;
        self.image_detail = None;
        self.image_detail_ref = None;
        self.image_delete_confirm = false;
        self.volume_mount_users = None;
        self.volume_mount_users_volume = None;
        self.volume_mount_users_table_state = TableState::default();
        self.network_containers = None;
        self.network_containers_network_id = None;
        self.network_containers_table_state = TableState::default();
        self.container_inspect_id = None;
        self.container_inspect = None;
        self.container_inspect_json = None;
        self.container_inspect_json_scroll = 0;
        self.container_detail_tab = ContainerDetailTab::default();
        self.container_logs = None;
        self.container_logs_for_id = None;
        self.container_stats = None;
        self.container_stats_for_id = None;
        self.container_stats_scroll = 0;
    }

    /// Opens detail when on a resource tab with a selected row. Returns false if nothing opened.
    pub fn open_detail(&mut self) -> bool {
        if self.view == MainView::Dashboard || self.selected_len() == 0 {
            return false;
        }
        self.screen = AppScreen::Detail;
        true
    }

    pub fn network_containers_scrollable(&self) -> bool {
        self.network_containers
            .as_ref()
            .is_some_and(|r| !r.is_empty())
    }

    pub fn volume_mount_users_scrollable(&self) -> bool {
        self.volume_mount_users
            .as_ref()
            .is_some_and(|r| !r.is_empty())
    }

    /// Scroll the "Containers using volume" table by `delta` rows (`delta` may be negative).
    pub fn scroll_volume_mount_users_table(&mut self, delta: isize) {
        let Some(rows) = self.volume_mount_users.as_ref() else {
            return;
        };
        if rows.is_empty() {
            return;
        }
        let n = rows.len();
        let viewport = self.volume_mount_users_viewport_rows.max(1);
        let max_off = n.saturating_sub(viewport);
        let cur = self.volume_mount_users_table_state.offset() as isize;
        let next = (cur + delta).clamp(0, max_off as isize) as usize;
        *self.volume_mount_users_table_state.offset_mut() = next;
    }

    /// Scroll the "Containers in network" table by `delta` rows (`delta` may be negative).
    pub fn scroll_network_containers_table(&mut self, delta: isize) {
        let Some(rows) = self.network_containers.as_ref() else {
            return;
        };
        if rows.is_empty() {
            return;
        }
        let n = rows.len();
        let viewport = self.network_containers_viewport_rows.max(1);
        let max_off = n.saturating_sub(viewport);
        let cur = self.network_containers_table_state.offset() as isize;
        let next = (cur + delta).clamp(0, max_off as isize) as usize;
        *self.network_containers_table_state.offset_mut() = next;
    }

    /// Whether `d` may remove the open volume: only when mounts are loaded and no container uses it.
    pub fn volume_delete_enabled(&self) -> bool {
        if self.volume_remove_pending.is_some() {
            return false;
        }
        if self.screen != AppScreen::Detail || self.view != MainView::Volumes {
            return true;
        }
        match &self.volume_mount_users {
            None => false,
            Some(rows) => rows.is_empty(),
        }
    }

    /// Whether `d` may remove the open image: only after inspect has loaded.
    pub fn image_delete_enabled(&self) -> bool {
        if self.image_remove_pending.is_some() || self.image_delete_confirm {
            return false;
        }
        if self.screen != AppScreen::Detail || self.view != MainView::Images {
            return true;
        }
        self.image_detail.is_some()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
