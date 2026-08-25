use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use adw::prelude::*;
use gtk::{Align, Orientation, gio};
use secrecy::{ExposeSecret, SecretString};

use crate::{
    api::{
        ApiError, FilterMode, NewDelivery, NewDeliveryDraft, ParcelClient,
        SupportedCarriersResponse,
    },
    carrier_detection::suggest_carrier_codes,
    i18n::{interpolate, tr},
    model::{Delivery, DeliveryStatus, demo_deliveries, merge_delivered_history},
    notifications, secrets,
    storage::{DeliveryCache, SnapshotKind},
};

const STATUS_CLASSES: [&str; 6] = [
    "status-delivered",
    "status-pickup",
    "status-today",
    "status-problem",
    "status-muted",
    "status-transit",
];
const AUTOMATIC_REFRESH_INTERVAL_SECS: u64 = 15 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryFilter {
    Active,
    InTransit,
    OutForDelivery,
    ReadyForPickup,
    Delivered,
    Recent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BannerAction {
    Connect,
    Retry,
}

#[derive(Clone)]
struct CarrierChoice {
    code: String,
    name: String,
}

impl DeliveryFilter {
    fn from_sidebar_index(index: i32) -> Self {
        match index {
            0 => Self::Active,
            1 => Self::InTransit,
            2 => Self::OutForDelivery,
            3 => Self::ReadyForPickup,
            4 => Self::Delivered,
            _ => Self::Recent,
        }
    }

    fn title(self) -> String {
        tr(match self {
            Self::Active => "Aktive Sendungen",
            Self::InTransit => "Unterwegs",
            Self::OutForDelivery => "In Zustellung",
            Self::ReadyForPickup => "Abholbereit",
            Self::Delivered => "Zugestellte Sendungen",
            Self::Recent => "Kürzlich",
        })
    }

    fn matches(self, status: DeliveryStatus) -> bool {
        match self {
            Self::Active => status.is_active(),
            Self::InTransit => matches!(
                status,
                DeliveryStatus::InTransit | DeliveryStatus::InformationReceived
            ),
            Self::OutForDelivery => status == DeliveryStatus::OutForDelivery,
            Self::ReadyForPickup => status == DeliveryStatus::ReadyForPickup,
            Self::Delivered => status == DeliveryStatus::Delivered,
            Self::Recent => matches!(status, DeliveryStatus::Delivered | DeliveryStatus::Frozen),
        }
    }
}

#[derive(Clone)]
struct DetailView {
    stack: gtk::Stack,
    empty_page: adw::StatusPage,
    carrier_badge: gtk::Label,
    carrier_name: gtk::Label,
    description: gtk::Label,
    tracking_number: gtk::Label,
    status_box: gtk::Box,
    status_icon: gtk::Image,
    status_label: gtk::Label,
    expected: gtk::Label,
    expected_detail: gtk::Label,
    progress: gtk::ProgressBar,
    timeline: gtk::Box,
    detail_menu: gio::Menu,
    local_delivery_action: gtk::glib::WeakRef<gio::SimpleAction>,
    using_real_data: Rc<Cell<bool>>,
}

impl DetailView {
    fn update(&self, delivery: &Delivery) {
        let status = delivery.effective_status();
        self.stack.set_visible_child_name("delivery");
        self.carrier_badge.set_label(&delivery.initials());
        self.carrier_name.set_label(&delivery.carrier_name);
        self.description.set_label(&delivery.description);
        self.tracking_number.set_label(&delivery.tracking_number);
        self.status_icon.set_icon_name(Some(status.icon_name()));
        self.status_label
            .set_label(&delivery_status_label(delivery));
        let expected = delivery
            .expected
            .clone()
            .unwrap_or_else(|| tr("Noch offen"));
        self.expected.set_label(&expected);
        let expected_detail = if delivery.is_locally_delivered() {
            let parcel_status = delivery.status.label();
            interpolate(
                tr("Nur in Ankunft auf diesem Gerät · Parcel-Status: {status}"),
                &[("status", &parcel_status)],
            )
        } else {
            delivery.expected_detail.clone().unwrap_or_default()
        };
        self.expected_detail.set_label(&expected_detail);
        self.progress.set_fraction(delivery.status.progress());

        for class in STATUS_CLASSES {
            self.status_box.remove_css_class(class);
        }
        self.status_box.add_css_class(status.css_class());

        while self.detail_menu.n_items() > 1 {
            self.detail_menu.remove(1);
        }
        let can_change_local_status = self.using_real_data.get()
            && (delivery.status != DeliveryStatus::Delivered || delivery.is_locally_delivered());
        if can_change_local_status {
            let label = if delivery.is_locally_delivered() {
                tr("Lokale Markierung zurücknehmen")
            } else {
                tr("Lokal als zugestellt markieren")
            };
            self.detail_menu
                .append(Some(&label), Some("win.toggle-local-delivery"));
        }
        if let Some(action) = self.local_delivery_action.upgrade() {
            action.set_enabled(can_change_local_status);
        }

        while let Some(child) = self.timeline.first_child() {
            self.timeline.remove(&child);
        }

        if delivery.events.is_empty() {
            let empty = adw::StatusPage::builder()
                .icon_name("mail-unread-symbolic")
                .title(tr("Noch keine Ereignisse"))
                .description(tr(
                    "Parcel hat für diese Sendung noch keine Details bereitgestellt.",
                ))
                .vexpand(true)
                .build();
            self.timeline.append(&empty);
        } else {
            for (index, event) in delivery.events.iter().enumerate() {
                self.timeline.append(&timeline_row(
                    &event.title,
                    &event.date,
                    event.location.as_deref(),
                    event.additional.as_deref(),
                    index == 0,
                    index + 1 == delivery.events.len(),
                ));
            }
        }
    }

    fn show_empty(&self, title: &str, description: &str) {
        self.empty_page.set_title(title);
        self.empty_page.set_description(Some(description));
        self.stack.set_visible_child_name("empty");
        while self.detail_menu.n_items() > 1 {
            self.detail_menu.remove(1);
        }
        if let Some(action) = self.local_delivery_action.upgrade() {
            action.set_enabled(false);
        }
    }
}

pub fn build_window(app: &adw::Application) {
    let (initial_deliveries, cache, cache_warning, cached_at) = load_initial_deliveries();
    let deliveries = Rc::new(RefCell::new(initial_deliveries));
    let current_filter = Rc::new(RefCell::new(DeliveryFilter::Active));
    let visible_indices = Rc::new(RefCell::new(Vec::<usize>::new()));
    let search_query = Rc::new(RefCell::new(String::new()));
    let using_real_data = Rc::new(Cell::new(cached_at.is_some()));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(tr("Ankunft"))
        .default_width(1280)
        .default_height(790)
        .width_request(900)
        .height_request(620)
        .build();

    let local_delivery_action = gio::SimpleAction::new("toggle-local-delivery", None);
    local_delivery_action.set_enabled(false);
    window.add_action(&local_delivery_action);

    let toast_overlay = adw::ToastOverlay::new();
    window.set_content(Some(&toast_overlay));

    let root = gtk::Box::new(Orientation::Vertical, 0);
    toast_overlay.set_child(Some(&root));

    let header = adw::HeaderBar::new();
    header.add_css_class("flat-header");
    let title = adw::WindowTitle::new(&tr("Ankunft"), &tr("Deine Lieferungen auf einen Blick"));
    header.set_title_widget(Some(&title));

    let search = gtk::SearchEntry::builder()
        .placeholder_text(tr("Sendungen durchsuchen"))
        .width_request(260)
        .build();
    search.set_key_capture_widget(Some(&window));
    header.pack_start(&search);

    let refresh_button = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text(tr("Sendungen aktualisieren"))
        .build();
    header.pack_end(&refresh_button);

    let account_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text(tr("Parcel-Verbindung verwalten"))
        .build();
    let account_popover = gtk::Popover::new();
    let account_actions = gtk::Box::new(Orientation::Vertical, 4);
    account_actions.set_margin_start(8);
    account_actions.set_margin_end(8);
    account_actions.set_margin_top(8);
    account_actions.set_margin_bottom(8);
    let change_connection_button = gtk::Button::with_label(&tr("API-Schlüssel ändern"));
    change_connection_button.add_css_class("flat");
    let disconnect_button = gtk::Button::with_label(&tr("Verbindung entfernen"));
    disconnect_button.add_css_class("flat");
    disconnect_button.add_css_class("destructive-action");
    account_actions.append(&change_connection_button);
    account_actions.append(&disconnect_button);
    account_popover.set_child(Some(&account_actions));
    account_button.set_popover(Some(&account_popover));
    header.pack_end(&account_button);

    let add_button = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text(tr("Neue Sendung"))
        .build();
    add_button.add_css_class("suggested-action");
    header.pack_end(&add_button);
    root.append(&header);

    let main_paned = gtk::Paned::new(Orientation::Horizontal);
    main_paned.set_vexpand(true);
    main_paned.set_position(228);
    main_paned.set_resize_start_child(false);
    main_paned.set_shrink_start_child(false);
    root.append(&main_paned);

    let sidebar = build_sidebar(&deliveries.borrow());
    main_paned.set_start_child(Some(&sidebar.container));

    let content_paned = gtk::Paned::new(Orientation::Horizontal);
    content_paned.set_position(410);
    content_paned.set_resize_start_child(false);
    content_paned.set_shrink_start_child(false);
    main_paned.set_end_child(Some(&content_paned));

    let list_column = gtk::Box::new(Orientation::Vertical, 0);
    list_column.add_css_class("list-column");
    let connection_banner = adw::Banner::builder()
        .title(tr("Prototypmodus · Sichere Beispieldaten"))
        .button_label(tr("API verbinden"))
        .revealed(true)
        .build();
    list_column.append(&connection_banner);

    let list_header = gtk::Box::new(Orientation::Horizontal, 10);
    list_header.set_margin_start(20);
    list_header.set_margin_end(18);
    list_header.set_margin_top(18);
    list_header.set_margin_bottom(10);
    let list_title = gtk::Label::new(Some(&DeliveryFilter::Active.title()));
    list_title.set_xalign(0.0);
    list_title.set_hexpand(true);
    list_title.add_css_class("title-3");
    let list_count = gtk::Label::new(None);
    list_count.add_css_class("count-pill");
    list_header.append(&list_title);
    list_header.append(&list_count);
    list_column.append(&list_header);

    let delivery_list = gtk::ListBox::new();
    delivery_list.set_selection_mode(gtk::SelectionMode::Single);
    delivery_list.add_css_class("delivery-list");
    delivery_list.set_margin_start(12);
    delivery_list.set_margin_end(12);
    delivery_list.set_margin_bottom(16);
    let delivery_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&delivery_list)
        .build();
    list_column.append(&delivery_scroll);
    content_paned.set_start_child(Some(&list_column));

    let (detail_widget, detail_view) = build_detail_view(
        &toast_overlay,
        &local_delivery_action,
        Rc::clone(&using_real_data),
    );
    content_paned.set_end_child(Some(&detail_widget));

    let rebuild: Rc<dyn Fn()> = {
        let deliveries = deliveries.clone();
        let current_filter = current_filter.clone();
        let visible_indices = visible_indices.clone();
        let search_query = search_query.clone();
        let delivery_list = delivery_list.clone();
        let list_title = list_title.clone();
        let list_count = list_count.clone();
        let detail_view = detail_view.clone();

        Rc::new(move || {
            while let Some(child) = delivery_list.first_child() {
                delivery_list.remove(&child);
            }

            let filter = *current_filter.borrow();
            let query = search_query.borrow();
            let delivery_data = deliveries.borrow();
            let indices: Vec<usize> = delivery_data
                .iter()
                .enumerate()
                .filter(|(_, delivery)| {
                    filter.matches(delivery.effective_status()) && delivery.matches_query(&query)
                })
                .map(|(index, _)| index)
                .collect();

            for index in &indices {
                delivery_list.append(&delivery_row(&delivery_data[*index]));
            }
            *visible_indices.borrow_mut() = indices;

            let count = visible_indices.borrow().len();
            list_title.set_label(&filter.title());
            list_count.set_label(&count.to_string());

            if let Some(row) = delivery_list.row_at_index(0) {
                delivery_list.select_row(Some(&row));
                if let Some(index) = visible_indices.borrow().first() {
                    detail_view.update(&delivery_data[*index]);
                }
            } else if filter == DeliveryFilter::Delivered && query.trim().is_empty() {
                detail_view.show_empty(
                    &tr("Noch keine zugestellten Sendungen"),
                    &tr(
                        "Parcel stellt nur kürzlich abgeschlossene Sendungen bereit. Ankunft bewahrt künftig lokal beobachtete Zustellungen auf diesem Gerät auf.",
                    ),
                );
            } else if delivery_data.is_empty() {
                detail_view.show_empty(
                    &tr("Keine Sendungen vorhanden"),
                    &tr("In deinem Parcel-Konto sind derzeit keine Sendungen gespeichert."),
                );
            } else if query.trim().is_empty() {
                detail_view.show_empty(
                    &tr("Keine passenden Sendungen"),
                    &tr("Für diesen Bereich gibt es momentan keine Sendungen."),
                );
            } else {
                detail_view.show_empty(
                    &tr("Keine Suchtreffer"),
                    &tr(
                        "Versuche es mit einer anderen Beschreibung, Paketnummer oder einem Paketdienst.",
                    ),
                );
            }
        })
    };

    {
        let deliveries = deliveries.clone();
        let visible_indices = visible_indices.clone();
        let detail_view = detail_view.clone();
        delivery_list.connect_row_selected(move |_, row| {
            let Some(row) = row else {
                return;
            };
            let visible_position = row.index() as usize;
            if let Some(delivery_index) = visible_indices.borrow().get(visible_position) {
                detail_view.update(&deliveries.borrow()[*delivery_index]);
            }
        });
    }

    {
        let current_filter = current_filter.clone();
        let rebuild = rebuild.clone();
        sidebar.list.connect_row_selected(move |_, row| {
            let Some(row) = row else {
                return;
            };
            let filter = DeliveryFilter::from_sidebar_index(row.index());
            *current_filter.borrow_mut() = filter;
            rebuild();
        });
    }

    {
        let search_query = search_query.clone();
        let rebuild = rebuild.clone();
        search.connect_search_changed(move |entry| {
            *search_query.borrow_mut() = entry.text().to_string();
            rebuild();
        });
    }

    let live_ui = LiveUi {
        deliveries: deliveries.clone(),
        application: app.clone(),
        sidebar: sidebar.clone(),
        banner: connection_banner.clone(),
        toast_overlay: toast_overlay.clone(),
        refresh_button: refresh_button.clone(),
        add_button: add_button.clone(),
        rebuild: rebuild.clone(),
        loading: Rc::new(Cell::new(false)),
        dialog_open: Rc::new(Cell::new(false)),
        cache,
        using_real_data,
        last_fetched_at: Rc::new(Cell::new(cached_at)),
        banner_action: Rc::new(Cell::new(BannerAction::Connect)),
    };

    {
        let window = window.clone();
        let live_ui = live_ui.clone();
        refresh_button.connect_clicked(move |_| {
            restore_connection(&window, &live_ui, true, false);
        });
    }

    {
        let delivery_list = delivery_list.clone();
        let visible_indices = visible_indices.clone();
        let live_ui = live_ui.clone();
        local_delivery_action.connect_activate(move |action, _| {
            let Some(row) = delivery_list.selected_row() else {
                return;
            };
            let Some(delivery_index) = visible_indices.borrow().get(row.index() as usize).copied()
            else {
                return;
            };
            toggle_local_delivery(&live_ui, action, delivery_index);
        });
    }

    {
        let window = window.clone();
        let live_ui = live_ui.clone();
        add_button.connect_clicked(move |_| {
            begin_add_delivery(&window, &live_ui);
        });
    }

    {
        let window = window.clone();
        let live_ui = live_ui.clone();
        connection_banner.connect_button_clicked(move |_| match live_ui.banner_action.get() {
            BannerAction::Connect => show_connection_dialog(&window, &live_ui),
            BannerAction::Retry => restore_connection(&window, &live_ui, true, false),
        });
    }

    {
        let window = window.clone();
        let live_ui = live_ui.clone();
        let account_popover = account_popover.clone();
        change_connection_button.connect_clicked(move |_| {
            account_popover.popdown();
            show_connection_dialog(&window, &live_ui);
        });
    }

    {
        let window = window.clone();
        let live_ui = live_ui.clone();
        let account_popover = account_popover.clone();
        disconnect_button.connect_clicked(move |_| {
            account_popover.popdown();
            confirm_disconnect(&window, &live_ui);
        });
    }

    rebuild();
    if let Some(row) = sidebar.list.row_at_index(0) {
        sidebar.list.select_row(Some(&row));
    }

    window.present();
    if let Some(warning) = cache_warning {
        let message = interpolate(
            tr("Der Offline-Speicher ist nicht verfügbar: {warning}"),
            &[("warning", &warning)],
        );
        show_toast(&live_ui, &message);
    }
    restore_connection(&window, &live_ui, false, true);
}

#[derive(Clone)]
struct LiveUi {
    deliveries: Rc<RefCell<Vec<Delivery>>>,
    application: adw::Application,
    sidebar: Sidebar,
    banner: adw::Banner,
    toast_overlay: adw::ToastOverlay,
    refresh_button: gtk::Button,
    add_button: gtk::Button,
    rebuild: Rc<dyn Fn()>,
    loading: Rc<Cell<bool>>,
    dialog_open: Rc<Cell<bool>>,
    cache: Option<Arc<DeliveryCache>>,
    using_real_data: Rc<Cell<bool>>,
    last_fetched_at: Rc<Cell<Option<u64>>>,
    banner_action: Rc<Cell<BannerAction>>,
}

fn load_initial_deliveries() -> (
    Vec<Delivery>,
    Option<Arc<DeliveryCache>>,
    Option<String>,
    Option<u64>,
) {
    match DeliveryCache::shared_default() {
        Ok(cache) => {
            let snapshot = cache.snapshot(SnapshotKind::Recent).and_then(|snapshot| {
                if snapshot.is_some() {
                    Ok(snapshot)
                } else {
                    cache.snapshot(SnapshotKind::Active)
                }
            });
            match snapshot {
                Ok(Some(snapshot)) => {
                    let fetched_at = snapshot.fetched_at_unix_secs;
                    (snapshot.deliveries, Some(cache), None, Some(fetched_at))
                }
                Ok(None) => (demo_deliveries(), Some(cache), None, None),
                Err(error) => (
                    demo_deliveries(),
                    Some(cache),
                    Some(error.localized_message()),
                    None,
                ),
            }
        }
        Err(error) => (
            demo_deliveries(),
            None,
            Some(error.localized_message()),
            None,
        ),
    }
}

fn restore_connection(
    parent: &adw::ApplicationWindow,
    ui: &LiveUi,
    prompt_when_missing: bool,
    prefer_fresh_cache: bool,
) {
    if ui.loading.replace(true) {
        return;
    }

    ui.refresh_button.set_sensitive(false);
    ui.banner
        .set_title(&tr("GNOME-Schlüsselbund wird geprüft …"));
    ui.banner.set_button_label(None);
    ui.banner.set_revealed(true);

    let parent = parent.clone();
    let ui = ui.clone();
    gtk::glib::spawn_future_local(async move {
        let stored_key = secrets::load_api_key().await;
        ui.loading.set(false);

        match stored_key {
            Ok(Some(api_key)) if prefer_fresh_cache && cached_snapshot_is_fresh(&ui) => {
                drop(api_key);
                ui.refresh_button.set_sensitive(true);
                update_sidebar(&ui.sidebar, &ui.deliveries.borrow(), true);
                ui.banner.set_revealed(false);
            }
            Ok(Some(api_key)) => sync_with_key(&ui, api_key, false),
            Ok(None) => {
                show_disconnected(&ui);
                if prompt_when_missing {
                    show_connection_dialog(&parent, &ui);
                }
            }
            Err(error) => {
                show_disconnected(&ui);
                show_toast(&ui, &error.localized_message());
                if prompt_when_missing {
                    show_connection_dialog(&parent, &ui);
                }
            }
        }
    });
}

fn cached_snapshot_is_fresh(ui: &LiveUi) -> bool {
    let Some(fetched_at) = ui.last_fetched_at.get() else {
        return false;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now.checked_sub(fetched_at)
        .is_some_and(|age| age < AUTOMATIC_REFRESH_INTERVAL_SECS)
}

fn begin_add_delivery(parent: &adw::ApplicationWindow, ui: &LiveUi) {
    if ui.dialog_open.get() || ui.loading.replace(true) {
        return;
    }

    ui.refresh_button.set_sensitive(false);
    ui.add_button.set_sensitive(false);
    ui.banner.set_title(&tr("Paketdienste werden geladen …"));
    ui.banner.set_button_label(None);
    ui.banner.set_revealed(true);

    let parent = parent.clone();
    let ui = ui.clone();
    gtk::glib::spawn_future_local(async move {
        match secrets::load_api_key().await {
            Ok(Some(api_key)) => {
                let worker_key = SecretString::from(api_key.expose_secret());
                let carriers = gio::spawn_blocking(move || {
                    let client = ParcelClient::new(worker_key)?;
                    client.supported_carriers()
                })
                .await;

                ui.loading.set(false);
                ui.refresh_button.set_sensitive(true);
                ui.add_button.set_sensitive(true);

                match carriers {
                    Ok(Ok(carriers)) => {
                        restore_auxiliary_banner(&ui);
                        show_add_delivery_dialog(&parent, &ui, api_key, carriers);
                    }
                    Ok(Err(error)) => {
                        restore_auxiliary_banner(&ui);
                        let error = error.localized_message();
                        let message = interpolate(
                            tr("Paketdienste konnten nicht geladen werden: {error}"),
                            &[("error", &error)],
                        );
                        show_toast(&ui, &message);
                    }
                    Err(_) => {
                        restore_auxiliary_banner(&ui);
                        show_toast(
                            &ui,
                            &tr("Die Paketdienstliste konnte nicht geladen werden."),
                        );
                    }
                }
            }
            Ok(None) => {
                ui.loading.set(false);
                ui.refresh_button.set_sensitive(true);
                ui.add_button.set_sensitive(true);
                show_disconnected(&ui);
                show_toast(
                    &ui,
                    &tr("Verbinde zuerst dein Parcel-Konto, um eine Sendung hinzuzufügen."),
                );
                show_connection_dialog(&parent, &ui);
            }
            Err(error) => {
                ui.loading.set(false);
                ui.refresh_button.set_sensitive(true);
                ui.add_button.set_sensitive(true);
                restore_auxiliary_banner(&ui);
                show_toast(&ui, &error.localized_message());
            }
        }
    });
}

fn restore_auxiliary_banner(ui: &LiveUi) {
    if ui.using_real_data.get() {
        ui.banner.set_revealed(false);
    } else {
        show_disconnected(ui);
    }
}

fn show_add_delivery_dialog(
    parent: &adw::ApplicationWindow,
    ui: &LiveUi,
    api_key: SecretString,
    carriers: SupportedCarriersResponse,
) {
    let choices = Rc::new(carrier_choices(carriers));
    show_add_delivery_dialog_with_choices(parent, ui, api_key, choices, None);
}

fn show_add_delivery_dialog_with_choices(
    parent: &adw::ApplicationWindow,
    ui: &LiveUi,
    api_key: SecretString,
    choices: Rc<Vec<CarrierChoice>>,
    prefill: Option<NewDelivery>,
) {
    if ui.loading.get() || ui.dialog_open.replace(true) {
        return;
    }

    if choices.is_empty() {
        ui.dialog_open.set(false);
        show_toast(ui, &tr("Parcel hat keine Paketdienste bereitgestellt."));
        return;
    }

    let description = adw::EntryRow::builder()
        .title(tr("Beschreibung"))
        .activates_default(true)
        .build();
    let tracking_number = adw::EntryRow::builder()
        .title(tr("Sendungsnummer"))
        .activates_default(true)
        .build();

    let carrier_labels = choices
        .iter()
        .map(|choice| choice.name.as_str())
        .collect::<Vec<_>>();
    let carrier_model = gtk::StringList::new(&carrier_labels);
    let carrier_expression = gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        gtk::Expression::NONE,
        "string",
    );
    let carrier = adw::ComboRow::builder()
        .title(tr("Paketdienst"))
        .enable_search(true)
        .search_match_mode(gtk::StringFilterMatchMode::Substring)
        .expression(&carrier_expression)
        .model(&carrier_model)
        .build();
    carrier.set_selected(gtk::INVALID_LIST_POSITION);

    let primary_group = adw::PreferencesGroup::new();
    primary_group.add(&tracking_number);
    primary_group.add(&carrier);
    primary_group.add(&description);

    let postcode = adw::EntryRow::builder()
        .title(tr("Postleitzahl (optional)"))
        .activates_default(true)
        .build();
    let email = adw::EntryRow::builder()
        .title(tr("E-Mail-Adresse (optional)"))
        .input_purpose(gtk::InputPurpose::Email)
        .activates_default(true)
        .build();
    let optional_group = adw::PreferencesGroup::builder()
        .title(tr("Optionale Angaben"))
        .description(tr(
            "Manche Paketdienste benötigen eine Postleitzahl oder E-Mail-Adresse.",
        ))
        .build();
    optional_group.add(&postcode);
    optional_group.add(&email);

    let form = gtk::Box::new(Orientation::Vertical, 16);
    form.set_size_request(460, -1);
    form.set_margin_top(8);
    form.append(&primary_group);
    form.append(&optional_group);

    let dialog = adw::AlertDialog::builder()
        .heading(tr("Neue Sendung hinzufügen"))
        .body(tr(
            "Parcel prüft die Sendungsnummer nach dem Absenden. Ungültige Versuche zählen zum täglichen API-Limit.",
        ))
        .default_response("add")
        .close_response("cancel")
        .focus_widget(&tracking_number)
        .extra_child(&form)
        .build();
    dialog.add_response("cancel", &tr("Abbrechen"));
    dialog.add_response("add", &tr("Hinzufügen"));
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_response_enabled("add", false);

    if let Some(prefill) = prefill.as_ref() {
        description.set_text(&prefill.description);
        tracking_number.set_text(&prefill.tracking_number);
        postcode.set_text(prefill.postcode.as_deref().unwrap_or(""));
        email.set_text(prefill.email.as_deref().unwrap_or(""));
        if let Some(index) = choices
            .iter()
            .position(|choice| choice.code == prefill.carrier_code)
        {
            carrier.set_selected(index as u32);
        }
    }

    connect_carrier_suggestion(&tracking_number, &carrier, &choices);

    connect_add_dialog_validation(
        &dialog,
        &description,
        &tracking_number,
        &carrier,
        choices.len(),
    );

    let parent = parent.clone();
    let ui = ui.clone();
    let response = dialog.choose_future(Some(&parent));
    gtk::glib::spawn_future_local(async move {
        let response = response.await;
        ui.dialog_open.set(false);

        if response != "add" {
            clear_add_form(&description, &tracking_number, &postcode, &email);
            return;
        }

        let selected = carrier.selected() as usize;
        let carrier_code = choices
            .get(selected)
            .map(|choice| choice.code.clone())
            .unwrap_or_default();
        let draft = NewDeliveryDraft {
            tracking_number: tracking_number.text().to_string(),
            carrier_code,
            description: description.text().to_string(),
            language: crate::i18n::initialize().to_owned(),
            send_push_confirmation: false,
            postcode: postcode.text().to_string(),
            email: email.text().to_string(),
        };
        clear_add_form(&description, &tracking_number, &postcode, &email);

        match NewDelivery::try_from(draft) {
            Ok(delivery) => {
                submit_new_delivery(&parent, &ui, api_key, delivery, Rc::clone(&choices))
            }
            Err(error) => show_toast(&ui, &error.localized_message()),
        }
    });
}

fn carrier_choices(carriers: SupportedCarriersResponse) -> Vec<CarrierChoice> {
    let locale = crate::i18n::initialize();
    let language = locale.split(['_', '-']).next().unwrap_or(locale);
    let mut choices = carriers
        .into_iter()
        .map(|(code, carrier)| {
            let name = carrier
                .name_variations
                .get(language)
                .cloned()
                .unwrap_or(carrier.name);
            CarrierChoice { code, name }
        })
        .collect::<Vec<_>>();
    choices.sort_by_cached_key(|choice| choice.name.to_lowercase());
    choices
}

fn connect_carrier_suggestion(
    tracking_number: &adw::EntryRow,
    carrier: &adw::ComboRow,
    choices: &Rc<Vec<CarrierChoice>>,
) {
    let automatic_selection = Rc::new(Cell::new(None::<u32>));
    let applying_suggestion = Rc::new(Cell::new(false));

    {
        let automatic_selection = Rc::clone(&automatic_selection);
        let applying_suggestion = Rc::clone(&applying_suggestion);
        carrier.connect_selected_notify(move |carrier| {
            if !applying_suggestion.get() {
                automatic_selection.set(None);
                carrier.set_subtitle("");
            }
        });
    }

    let locale = carrier_locale_hint();
    let choices = Rc::clone(choices);
    let carrier = carrier.clone();
    tracking_number.connect_changed(move |tracking_number| {
        let current = carrier.selected();
        if current != gtk::INVALID_LIST_POSITION && automatic_selection.get() != Some(current) {
            return;
        }

        let suggestion = suggested_carrier_index(&choices, &tracking_number.text(), &locale);
        applying_suggestion.set(true);
        carrier.set_selected(suggestion.unwrap_or(gtk::INVALID_LIST_POSITION));
        if suggestion.is_some() {
            carrier.set_subtitle(&tr("Aus der Sendungsnummer vorgeschlagen – bitte prüfen."));
        } else {
            carrier.set_subtitle("");
        }
        automatic_selection.set(suggestion);
        applying_suggestion.set(false);
    });
}

fn suggested_carrier_index(
    choices: &[CarrierChoice],
    tracking_number: &str,
    locale: &str,
) -> Option<u32> {
    suggest_carrier_codes(tracking_number, locale)
        .into_iter()
        .find_map(|code| {
            choices
                .iter()
                .position(|choice| choice.code == code)
                .map(|index| index as u32)
        })
}

fn carrier_locale_hint() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| crate::i18n::initialize().to_owned())
}

fn connect_add_dialog_validation(
    dialog: &adw::AlertDialog,
    description: &adw::EntryRow,
    tracking_number: &adw::EntryRow,
    carrier: &adw::ComboRow,
    carrier_count: usize,
) {
    let update = Rc::new({
        let dialog = dialog.downgrade();
        let description = description.downgrade();
        let tracking_number = tracking_number.downgrade();
        let carrier = carrier.downgrade();
        move || {
            let (Some(dialog), Some(description), Some(tracking_number), Some(carrier)) = (
                dialog.upgrade(),
                description.upgrade(),
                tracking_number.upgrade(),
                carrier.upgrade(),
            ) else {
                return;
            };
            let selected = carrier.selected() as usize;
            dialog.set_response_enabled(
                "add",
                !description.text().trim().is_empty()
                    && !tracking_number.text().trim().is_empty()
                    && selected < carrier_count,
            );
        }
    });

    {
        let update = update.clone();
        description.connect_changed(move |_| update());
    }
    {
        let update = update.clone();
        tracking_number.connect_changed(move |_| update());
    }
    {
        let update = update.clone();
        carrier.connect_selected_notify(move |_| update());
    }
    update();
}

fn clear_add_form(
    description: &adw::EntryRow,
    tracking_number: &adw::EntryRow,
    postcode: &adw::EntryRow,
    email: &adw::EntryRow,
) {
    description.set_text("");
    tracking_number.set_text("");
    postcode.set_text("");
    email.set_text("");
}

fn submit_new_delivery(
    parent: &adw::ApplicationWindow,
    ui: &LiveUi,
    api_key: SecretString,
    delivery: NewDelivery,
    choices: Rc<Vec<CarrierChoice>>,
) {
    if ui.loading.replace(true) {
        return;
    }

    ui.refresh_button.set_sensitive(false);
    ui.add_button.set_sensitive(false);
    ui.banner.set_title(&tr("Sendung wird hinzugefügt …"));
    ui.banner.set_button_label(None);
    ui.banner.set_revealed(true);

    let worker_key = SecretString::from(api_key.expose_secret());
    let parent = parent.clone();
    let ui = ui.clone();
    gtk::glib::spawn_future_local(async move {
        let result = gio::spawn_blocking(move || {
            let result =
                ParcelClient::new(worker_key).and_then(|client| client.add_delivery(&delivery));
            (result, delivery)
        })
        .await;

        ui.loading.set(false);
        ui.refresh_button.set_sensitive(true);
        ui.add_button.set_sensitive(true);

        match result {
            Ok((Ok(()), _delivery)) => {
                show_toast(
                    &ui,
                    &tr(
                        "Sendung wurde hinzugefügt. Trackingdaten können nach der ersten Parcel-Aktualisierung erscheinen.",
                    ),
                );
                sync_with_key(&ui, api_key, false);
            }
            Ok((Err(error), delivery)) => {
                restore_auxiliary_banner(&ui);
                let error = error.localized_message();
                let message = interpolate(
                    tr("Sendung konnte nicht hinzugefügt werden: {error}"),
                    &[("error", &error)],
                );
                show_toast(&ui, &message);
                show_add_delivery_dialog_with_choices(
                    &parent,
                    &ui,
                    api_key,
                    choices,
                    Some(delivery),
                );
            }
            Err(_) => {
                restore_auxiliary_banner(&ui);
                show_toast(&ui, &tr("Die Sendung konnte nicht hinzugefügt werden."));
            }
        }
    });
}

fn show_connection_dialog(parent: &adw::ApplicationWindow, ui: &LiveUi) {
    if ui.loading.get() || ui.dialog_open.replace(true) {
        return;
    }

    let entry = adw::PasswordEntryRow::builder()
        .title(tr("Parcel API-Schlüssel"))
        .activates_default(true)
        .build();

    let group = adw::PreferencesGroup::new();
    group.add(&entry);

    let link = gtk::LinkButton::with_label(
        "https://web.parcelapp.net/",
        &tr("API-Schlüssel in Parcel Web erzeugen"),
    );
    link.set_halign(Align::Center);

    let extra = gtk::Box::new(Orientation::Vertical, 12);
    extra.set_margin_top(8);
    extra.append(&group);
    extra.append(&link);

    let dialog = adw::AlertDialog::builder()
        .heading(tr("Mit Parcel verbinden"))
        .body(tr(
            "Dein persönlicher Premium-API-Schlüssel wird ausschließlich im geschützten GNOME-Schlüsselbund gespeichert.",
        ))
        .default_response("connect")
        .close_response("cancel")
        .extra_child(&extra)
        .build();
    dialog.add_response("cancel", &tr("Abbrechen"));
    dialog.add_response("connect", &tr("Verbinden"));
    dialog.set_response_appearance("connect", adw::ResponseAppearance::Suggested);
    dialog.set_response_enabled("connect", false);

    {
        let dialog = dialog.clone();
        entry.connect_changed(move |entry| {
            dialog.set_response_enabled("connect", secrets::parse_api_key(&entry.text()).is_ok());
        });
    }

    let ui = ui.clone();
    let response = dialog.choose_future(Some(parent));
    gtk::glib::spawn_future_local(async move {
        let response = response.await;
        ui.dialog_open.set(false);

        if response != "connect" {
            entry.set_text("");
            return;
        }

        let raw_key = entry.text().to_string();
        entry.set_text("");
        match secrets::parse_api_key_owned(raw_key) {
            Ok(api_key) => sync_with_key(&ui, api_key, true),
            Err(error) => show_toast(&ui, &error.localized_message()),
        }
    });
}

fn sync_with_key(ui: &LiveUi, api_key: SecretString, persist_key: bool) {
    if ui.loading.replace(true) {
        return;
    }

    ui.refresh_button.set_sensitive(false);
    ui.banner.set_title(&tr("Live-Sendungen werden geladen …"));
    ui.banner.set_button_label(None);
    ui.banner.set_revealed(true);

    let worker_key = SecretString::from(api_key.expose_secret());
    let cache = ui.cache.clone();
    let ui = ui.clone();
    gtk::glib::spawn_future_local(async move {
        let result = gio::spawn_blocking(move || {
            let client = ParcelClient::new(worker_key)?;
            let fetched_deliveries = client.deliveries(FilterMode::Recent)?;
            let fetched_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let (deliveries, previous_snapshot, cache_error) = if let Some(cache) = cache {
                match cache.snapshot(SnapshotKind::Recent) {
                    Ok(cached_snapshot) => {
                        let deliveries = merge_delivered_history(
                            fetched_deliveries,
                            cached_snapshot
                                .as_ref()
                                .map(|snapshot| snapshot.deliveries.as_slice())
                                .unwrap_or(&[]),
                        );
                        match cache.replace_snapshot(
                            SnapshotKind::Recent,
                            fetched_at,
                            deliveries.clone(),
                        ) {
                            Ok(previous) => (deliveries, previous, None),
                            Err(error) if error.write_was_committed() => {
                                (deliveries, cached_snapshot, Some(error))
                            }
                            Err(error) => (deliveries, None, Some(error)),
                        }
                    }
                    Err(error) => (
                        merge_delivered_history(fetched_deliveries, &[]),
                        None,
                        Some(error),
                    ),
                }
            } else {
                (merge_delivered_history(fetched_deliveries, &[]), None, None)
            };
            Ok::<_, ApiError>((deliveries, previous_snapshot, cache_error, fetched_at))
        })
        .await;

        match result {
            Ok(Ok((live_deliveries, previous_snapshot, cache_error, fetched_at))) => {
                let key_stored = if persist_key {
                    match secrets::store_api_key(&api_key).await {
                        Ok(()) => true,
                        Err(error) => {
                            show_toast(&ui, &error.localized_message());
                            false
                        }
                    }
                } else {
                    true
                };

                *ui.deliveries.borrow_mut() = live_deliveries;
                ui.using_real_data.set(true);
                ui.last_fetched_at.set(Some(fetched_at));
                update_sidebar(&ui.sidebar, &ui.deliveries.borrow(), key_stored);
                (ui.rebuild)();

                if cache_error
                    .as_ref()
                    .is_none_or(|error| error.write_was_committed())
                {
                    notifications::notify_delivery_status_changes(
                        &ui.application,
                        previous_snapshot.as_ref(),
                        &ui.deliveries.borrow(),
                    );
                }

                if let Some(error) = cache_error {
                    let committed = error.write_was_committed();
                    let error = error.localized_message();
                    let message = if committed {
                        error
                    } else {
                        interpolate(
                            tr("Live-Daten geladen, aber nicht offline gespeichert: {error}"),
                            &[("error", &error)],
                        )
                    };
                    show_toast(&ui, &message);
                }

                if key_stored {
                    ui.banner.set_revealed(false);
                    show_toast(&ui, &tr("Live-Sendungen wurden aktualisiert"));
                } else {
                    ui.banner
                        .set_title(&tr("Live-Daten geladen · Schlüssel nicht gespeichert"));
                    ui.banner.set_button_label(Some(&tr("Erneut verbinden")));
                    ui.banner_action.set(BannerAction::Connect);
                    ui.banner.set_revealed(true);
                }
            }
            Ok(Err(error)) => {
                update_sidebar(&ui.sidebar, &ui.deliveries.borrow(), false);
                ui.banner
                    .set_title(&tr("Parcel konnte nicht aktualisiert werden"));
                ui.banner.set_button_label(Some(&tr("Verbindung prüfen")));
                ui.banner_action.set(if persist_key {
                    BannerAction::Connect
                } else {
                    BannerAction::Retry
                });
                ui.banner.set_revealed(true);
                let error = error.localized_message();
                let message = interpolate(
                    tr("Synchronisierung fehlgeschlagen: {error}"),
                    &[("error", &error)],
                );
                show_toast(&ui, &message);
            }
            Err(_) => {
                ui.banner
                    .set_title(&tr("Die Synchronisierung wurde unerwartet beendet"));
                ui.banner.set_button_label(Some(&tr("Erneut versuchen")));
                ui.banner_action.set(if persist_key {
                    BannerAction::Connect
                } else {
                    BannerAction::Retry
                });
                ui.banner.set_revealed(true);
                show_toast(&ui, &tr("Die Live-Daten konnten nicht geladen werden"));
            }
        }

        ui.loading.set(false);
        ui.refresh_button.set_sensitive(true);
    });
}

fn confirm_disconnect(parent: &adw::ApplicationWindow, ui: &LiveUi) {
    if ui.loading.get() || ui.dialog_open.replace(true) {
        return;
    }

    let dialog = adw::AlertDialog::builder()
        .heading(tr("Parcel-Verbindung entfernen?"))
        .body(tr(
            "Der API-Schlüssel wird aus dem GNOME-Schlüsselbund gelöscht. Du kannst dich jederzeit wieder verbinden.",
        ))
        .default_response("cancel")
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", &tr("Abbrechen"));
    dialog.add_response("disconnect", &tr("Entfernen"));
    dialog.set_response_appearance("disconnect", adw::ResponseAppearance::Destructive);

    let response = dialog.choose_future(Some(parent));
    let ui = ui.clone();
    gtk::glib::spawn_future_local(async move {
        let response = response.await;
        ui.dialog_open.set(false);
        if response != "disconnect" {
            return;
        }

        ui.loading.set(true);
        ui.refresh_button.set_sensitive(false);
        match secrets::clear_api_key().await {
            Ok(()) => {
                let cache_warning = if let Some(cache) = ui.cache.clone() {
                    match gio::spawn_blocking(move || cache.clear()).await {
                        Ok(Ok(())) => None,
                        Ok(Err(error)) => {
                            Some((error.write_was_committed(), error.localized_message()))
                        }
                        Err(_) => Some((false, tr("Der Offline-Speicher antwortet nicht."))),
                    }
                } else {
                    Some((
                        false,
                        tr(
                            "Der Offline-Speicher war beim Start nicht verfügbar; lokale Sendungsdaten konnten nicht entfernt werden.",
                        ),
                    ))
                };

                *ui.deliveries.borrow_mut() = demo_deliveries();
                ui.using_real_data.set(false);
                ui.last_fetched_at.set(None);
                (ui.rebuild)();
                show_disconnected(&ui);
                show_toast(&ui, &tr("Parcel-Verbindung wurde entfernt"));
                if let Some((committed, warning)) = cache_warning {
                    let message = if committed {
                        warning
                    } else {
                        interpolate(
                            tr("Offline-Daten nicht entfernt: {warning}"),
                            &[("warning", &warning)],
                        )
                    };
                    show_toast(&ui, &message);
                }
            }
            Err(error) => {
                ui.loading.set(false);
                ui.refresh_button.set_sensitive(true);
                show_toast(&ui, &error.localized_message());
            }
        }
    });
}

fn show_disconnected(ui: &LiveUi) {
    ui.loading.set(false);
    ui.refresh_button.set_sensitive(true);
    if ui.using_real_data.get() {
        ui.banner
            .set_title(&tr("Offline · Zuletzt gespeicherte Sendungen"));
    } else {
        ui.banner
            .set_title(&tr("Prototypmodus · Sichere Beispieldaten"));
    }
    ui.banner.set_button_label(Some(&tr("API verbinden")));
    ui.banner_action.set(BannerAction::Connect);
    ui.banner.set_revealed(true);
    update_sidebar(&ui.sidebar, &ui.deliveries.borrow(), false);
    if ui.using_real_data.get() {
        ui.sidebar.sync_title.set_label(&tr("Offline verfügbar"));
        ui.sidebar
            .sync_text
            .set_label(&tr("Der zuletzt gespeicherte Parcel-Stand wird angezeigt."));
    }
}

fn show_toast(ui: &LiveUi, message: &str) {
    let toast = adw::Toast::new(message);
    toast.set_timeout(4);
    ui.toast_overlay.add_toast(toast);
}

fn toggle_local_delivery(ui: &LiveUi, action: &gio::SimpleAction, delivery_index: usize) {
    if ui.loading.replace(true) {
        return;
    }

    let Some(cache) = ui.cache.clone() else {
        ui.loading.set(false);
        let error = tr("Der Offline-Speicher antwortet nicht.");
        let message = interpolate(
            tr("Die lokale Markierung konnte nicht gespeichert werden: {error}"),
            &[("error", &error)],
        );
        show_toast(ui, &message);
        return;
    };

    let mut next_deliveries = ui.deliveries.borrow().clone();
    let Some(delivery) = next_deliveries.get_mut(delivery_index) else {
        ui.loading.set(false);
        return;
    };

    let removed = if delivery.is_locally_delivered() {
        delivery.clear_local_status_override();
        true
    } else if delivery.status != DeliveryStatus::Delivered {
        delivery.mark_locally_delivered();
        false
    } else {
        ui.loading.set(false);
        return;
    };

    action.set_enabled(false);
    let action = action.downgrade();
    ui.refresh_button.set_sensitive(false);
    ui.add_button.set_sensitive(false);
    let fetched_at = ui.last_fetched_at.get().unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    });
    let persisted_deliveries = next_deliveries.clone();
    let ui = ui.clone();
    gtk::glib::spawn_future_local(async move {
        let result = gio::spawn_blocking(move || {
            cache.replace_snapshot(SnapshotKind::Recent, fetched_at, persisted_deliveries)
        })
        .await;

        ui.loading.set(false);
        ui.refresh_button.set_sensitive(true);
        ui.add_button.set_sensitive(true);

        match result {
            Ok(Ok(_)) => {
                apply_local_delivery_update(&ui, next_deliveries);
                show_toast(
                    &ui,
                    &tr(if removed {
                        "Die lokale Markierung wurde zurückgenommen."
                    } else {
                        "Die Sendung wurde lokal als zugestellt markiert."
                    }),
                );
            }
            Ok(Err(error)) if error.write_was_committed() => {
                apply_local_delivery_update(&ui, next_deliveries);
                show_toast(&ui, &error.localized_message());
            }
            Ok(Err(error)) => {
                if let Some(action) = action.upgrade() {
                    action.set_enabled(true);
                }
                let error = error.localized_message();
                let message = interpolate(
                    tr("Die lokale Markierung konnte nicht gespeichert werden: {error}"),
                    &[("error", &error)],
                );
                show_toast(&ui, &message);
            }
            Err(_) => {
                if let Some(action) = action.upgrade() {
                    action.set_enabled(true);
                }
                let error = tr("Der Offline-Speicher antwortet nicht.");
                let message = interpolate(
                    tr("Die lokale Markierung konnte nicht gespeichert werden: {error}"),
                    &[("error", &error)],
                );
                show_toast(&ui, &message);
            }
        }
    });
}

fn apply_local_delivery_update(ui: &LiveUi, deliveries: Vec<Delivery>) {
    *ui.deliveries.borrow_mut() = deliveries;
    update_sidebar_counts(&ui.sidebar, &ui.deliveries.borrow());
    (ui.rebuild)();
}

#[derive(Clone)]
struct Sidebar {
    container: gtk::Box,
    list: gtk::ListBox,
    count_labels: Vec<gtk::Label>,
    sync_title: gtk::Label,
    sync_text: gtk::Label,
}

fn build_sidebar(deliveries: &[Delivery]) -> Sidebar {
    let container = gtk::Box::new(Orientation::Vertical, 0);
    container.set_width_request(228);
    container.add_css_class("sidebar");

    let heading = gtk::Box::new(Orientation::Vertical, 2);
    heading.set_margin_start(18);
    heading.set_margin_end(18);
    heading.set_margin_top(22);
    heading.set_margin_bottom(14);
    let eyebrow = gtk::Label::new(Some(&tr("PARCEL PREMIUM")));
    eyebrow.set_xalign(0.0);
    eyebrow.add_css_class("eyebrow");
    let title = gtk::Label::new(Some(&tr("Meine Sendungen")));
    title.set_xalign(0.0);
    title.add_css_class("title-2");
    heading.append(&eyebrow);
    heading.append(&title);
    container.append(&heading);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.add_css_class("navigation-list");
    list.set_margin_start(10);
    list.set_margin_end(10);

    let filters = [
        (
            "view-list-symbolic",
            tr("Aktiv"),
            deliveries
                .iter()
                .filter(|item| item.effective_status().is_active())
                .count(),
        ),
        (
            "go-next-symbolic",
            tr("Unterwegs"),
            deliveries
                .iter()
                .filter(|item| {
                    matches!(
                        item.effective_status(),
                        DeliveryStatus::InTransit | DeliveryStatus::InformationReceived
                    )
                })
                .count(),
        ),
        (
            "send-to-symbolic",
            tr("In Zustellung"),
            deliveries
                .iter()
                .filter(|item| item.effective_status() == DeliveryStatus::OutForDelivery)
                .count(),
        ),
        (
            "folder-download-symbolic",
            tr("Abholbereit"),
            deliveries
                .iter()
                .filter(|item| item.effective_status() == DeliveryStatus::ReadyForPickup)
                .count(),
        ),
        (
            "emblem-ok-symbolic",
            tr("Zugestellt"),
            deliveries
                .iter()
                .filter(|item| item.effective_status() == DeliveryStatus::Delivered)
                .count(),
        ),
        (
            "document-open-recent-symbolic",
            tr("Kürzlich"),
            deliveries
                .iter()
                .filter(|item| {
                    matches!(
                        item.effective_status(),
                        DeliveryStatus::Delivered | DeliveryStatus::Frozen
                    )
                })
                .count(),
        ),
    ];

    let mut count_labels = Vec::new();
    for (icon, label, count) in filters {
        let row = gtk::ListBoxRow::new();
        let content = gtk::Box::new(Orientation::Horizontal, 10);
        content.set_margin_start(10);
        content.set_margin_end(10);
        content.set_margin_top(9);
        content.set_margin_bottom(9);
        let image = gtk::Image::from_icon_name(icon);
        image.set_pixel_size(17);
        let label = gtk::Label::new(Some(&label));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        let count_label = gtk::Label::new(Some(&count.to_string()));
        count_label.add_css_class("sidebar-count");
        content.append(&image);
        content.append(&label);
        content.append(&count_label);
        row.set_child(Some(&content));
        list.append(&row);
        count_labels.push(count_label);
    }
    container.append(&list);

    let spacer = gtk::Box::new(Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    container.append(&spacer);

    let sync_card = gtk::Box::new(Orientation::Vertical, 5);
    sync_card.add_css_class("sync-card");
    sync_card.set_margin_start(14);
    sync_card.set_margin_end(14);
    sync_card.set_margin_bottom(16);
    sync_card.set_margin_top(16);
    let sync_title = gtk::Label::new(Some(&tr("Noch nicht verbunden")));
    sync_title.set_xalign(0.0);
    sync_title.add_css_class("heading");
    let sync_text = gtk::Label::new(Some(&tr(
        "Dein API-Schlüssel wird später sicher im GNOME-Schlüsselbund gespeichert.",
    )));
    sync_text.set_xalign(0.0);
    sync_text.set_wrap(true);
    sync_text.add_css_class("caption");
    sync_text.add_css_class("dim-label");
    sync_card.append(&sync_title);
    sync_card.append(&sync_text);
    container.append(&sync_card);

    Sidebar {
        container,
        list,
        count_labels,
        sync_title,
        sync_text,
    }
}

fn update_sidebar(sidebar: &Sidebar, deliveries: &[Delivery], connected: bool) {
    update_sidebar_counts(sidebar, deliveries);

    if connected {
        sidebar.sync_title.set_label(&tr("Mit Parcel verbunden"));
        sidebar.sync_text.set_label(&tr(
            "Der API-Schlüssel liegt geschützt im GNOME-Schlüsselbund.",
        ));
    } else {
        sidebar.sync_title.set_label(&tr("Noch nicht verbunden"));
        sidebar.sync_text.set_label(&tr(
            "Verbinde dein Parcel-Premium-Konto, um Live-Daten zu laden.",
        ));
    }
}

fn update_sidebar_counts(sidebar: &Sidebar, deliveries: &[Delivery]) {
    let counts = [
        deliveries
            .iter()
            .filter(|item| item.effective_status().is_active())
            .count(),
        deliveries
            .iter()
            .filter(|item| {
                matches!(
                    item.effective_status(),
                    DeliveryStatus::InTransit | DeliveryStatus::InformationReceived
                )
            })
            .count(),
        deliveries
            .iter()
            .filter(|item| item.effective_status() == DeliveryStatus::OutForDelivery)
            .count(),
        deliveries
            .iter()
            .filter(|item| item.effective_status() == DeliveryStatus::ReadyForPickup)
            .count(),
        deliveries
            .iter()
            .filter(|item| item.effective_status() == DeliveryStatus::Delivered)
            .count(),
        deliveries
            .iter()
            .filter(|item| {
                matches!(
                    item.effective_status(),
                    DeliveryStatus::Delivered | DeliveryStatus::Frozen
                )
            })
            .count(),
    ];

    for (label, count) in sidebar.count_labels.iter().zip(counts) {
        label.set_label(&count.to_string());
    }
}

fn delivery_row(delivery: &Delivery) -> gtk::ListBoxRow {
    let effective_status = delivery.effective_status();
    let row = gtk::ListBoxRow::new();
    row.add_css_class("delivery-row");

    let card = gtk::Box::new(Orientation::Vertical, 10);
    card.set_margin_start(12);
    card.set_margin_end(12);
    card.set_margin_top(13);
    card.set_margin_bottom(13);

    let top = gtk::Box::new(Orientation::Horizontal, 11);
    let badge = gtk::Label::new(Some(&delivery.initials()));
    badge.set_width_request(42);
    badge.set_height_request(42);
    badge.add_css_class("carrier-badge");
    top.append(&badge);

    let labels = gtk::Box::new(Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let description = gtk::Label::new(Some(&delivery.description));
    description.set_xalign(0.0);
    description.set_ellipsize(gtk::pango::EllipsizeMode::End);
    description.add_css_class("heading");
    let carrier = gtk::Label::new(Some(&delivery.carrier_name));
    carrier.set_xalign(0.0);
    carrier.set_ellipsize(gtk::pango::EllipsizeMode::End);
    carrier.add_css_class("caption");
    carrier.add_css_class("dim-label");
    labels.append(&description);
    labels.append(&carrier);
    top.append(&labels);

    let expected_text = delivery.expected.clone().unwrap_or_else(|| tr("Offen"));
    let expected = gtk::Label::new(Some(&expected_text));
    expected.set_valign(Align::Start);
    expected.add_css_class("expected-small");
    top.append(&expected);
    card.append(&top);

    let status_row = gtk::Box::new(Orientation::Horizontal, 7);
    let status_icon = gtk::Image::from_icon_name(effective_status.icon_name());
    status_icon.set_pixel_size(14);
    let status = gtk::Label::new(Some(&delivery_status_label(delivery)));
    status.set_xalign(0.0);
    status.set_hexpand(true);
    status.add_css_class("caption");
    let updated = gtk::Label::new(Some(&delivery.last_update));
    updated.add_css_class("caption");
    updated.add_css_class("dim-label");
    status_row.append(&status_icon);
    status_row.append(&status);
    status_row.append(&updated);
    card.append(&status_row);

    let progress = gtk::ProgressBar::new();
    progress.set_fraction(delivery.status.progress());
    progress.add_css_class(delivery.status.css_class());
    card.append(&progress);
    row.set_child(Some(&card));
    row
}

fn delivery_status_label(delivery: &Delivery) -> String {
    if delivery.is_locally_delivered() {
        tr("Lokal als zugestellt markiert")
    } else {
        delivery.effective_status().label()
    }
}

fn build_detail_view(
    toast_overlay: &adw::ToastOverlay,
    local_delivery_action: &gio::SimpleAction,
    using_real_data: Rc<Cell<bool>>,
) -> (gtk::Stack, DetailView) {
    let detail = gtk::Box::new(Orientation::Vertical, 0);
    detail.add_css_class("detail-surface");

    let page = gtk::Box::new(Orientation::Vertical, 22);
    page.set_margin_start(30);
    page.set_margin_end(30);
    page.set_margin_top(28);
    page.set_margin_bottom(38);

    let hero = gtk::Box::new(Orientation::Vertical, 18);
    hero.add_css_class("hero-card");
    let hero_top = gtk::Box::new(Orientation::Horizontal, 14);
    let carrier_badge = gtk::Label::new(None);
    carrier_badge.set_width_request(54);
    carrier_badge.set_height_request(54);
    carrier_badge.add_css_class("carrier-badge");
    carrier_badge.add_css_class("carrier-badge-large");
    hero_top.append(&carrier_badge);

    let identity = gtk::Box::new(Orientation::Vertical, 3);
    identity.set_hexpand(true);
    let carrier_name = gtk::Label::new(None);
    carrier_name.set_xalign(0.0);
    carrier_name.add_css_class("caption");
    carrier_name.add_css_class("dim-label");
    let description = gtk::Label::new(None);
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.add_css_class("title-1");
    identity.append(&carrier_name);
    identity.append(&description);
    hero_top.append(&identity);

    let menu_button = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text(tr("Weitere Sendungsoptionen"))
        .valign(Align::Start)
        .build();
    let menu = gio::Menu::new();
    menu.append(Some(&tr("In Parcel Web öffnen")), Some("app.open-web"));
    menu_button.set_menu_model(Some(&menu));
    hero_top.append(&menu_button);
    hero.append(&hero_top);

    let status_box = gtk::Box::new(Orientation::Horizontal, 7);
    status_box.set_halign(Align::Start);
    status_box.add_css_class("status-chip");
    let status_icon = gtk::Image::new();
    status_icon.set_pixel_size(15);
    let status_label = gtk::Label::new(None);
    status_label.add_css_class("caption");
    status_label.add_css_class("heading");
    status_box.append(&status_icon);
    status_box.append(&status_label);
    hero.append(&status_box);

    let metrics = gtk::Box::new(Orientation::Horizontal, 12);
    metrics.set_homogeneous(true);
    let expected_card = gtk::Box::new(Orientation::Vertical, 3);
    expected_card.add_css_class("metric-card");
    let expected_caption = gtk::Label::new(Some(&tr("VORAUSSICHTLICH")));
    expected_caption.set_xalign(0.0);
    expected_caption.add_css_class("eyebrow");
    let expected = gtk::Label::new(None);
    expected.set_xalign(0.0);
    expected.add_css_class("title-2");
    let expected_detail = gtk::Label::new(None);
    expected_detail.set_xalign(0.0);
    expected_detail.set_wrap(true);
    expected_detail.add_css_class("caption");
    expected_detail.add_css_class("dim-label");
    expected_card.append(&expected_caption);
    expected_card.append(&expected);
    expected_card.append(&expected_detail);
    metrics.append(&expected_card);

    let tracking_card = gtk::Box::new(Orientation::Vertical, 3);
    tracking_card.add_css_class("metric-card");
    let tracking_caption = gtk::Label::new(Some(&tr("SENDUNGSNUMMER")));
    tracking_caption.set_xalign(0.0);
    tracking_caption.add_css_class("eyebrow");
    let tracking_row = gtk::Box::new(Orientation::Horizontal, 6);
    let tracking_number = gtk::Label::new(None);
    tracking_number.set_xalign(0.0);
    tracking_number.set_hexpand(true);
    tracking_number.set_selectable(true);
    tracking_number.add_css_class("heading");
    let copy_button = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text(tr("Sendungsnummer kopieren"))
        .css_classes(["flat", "circular"])
        .build();
    tracking_row.append(&tracking_number);
    tracking_row.append(&copy_button);
    tracking_card.append(&tracking_caption);
    tracking_card.append(&tracking_row);
    metrics.append(&tracking_card);
    hero.append(&metrics);

    let progress = gtk::ProgressBar::new();
    progress.set_show_text(false);
    progress.add_css_class("hero-progress");
    hero.append(&progress);
    page.append(&hero);

    let timeline_heading = gtk::Box::new(Orientation::Horizontal, 8);
    let timeline_title = gtk::Label::new(Some(&tr("Sendungsverlauf")));
    timeline_title.set_xalign(0.0);
    timeline_title.set_hexpand(true);
    timeline_title.add_css_class("title-3");
    let live_badge = gtk::Label::new(Some(&tr("LIVE")));
    live_badge.add_css_class("live-badge");
    timeline_heading.append(&timeline_title);
    timeline_heading.append(&live_badge);
    page.append(&timeline_heading);

    let timeline = gtk::Box::new(Orientation::Vertical, 0);
    timeline.add_css_class("timeline-card");
    page.append(&timeline);
    detail.append(&page);

    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&detail)
        .build();

    let empty_page = adw::StatusPage::builder()
        .icon_name("mail-unread-symbolic")
        .title(tr("Keine Sendung ausgewählt"))
        .description(tr(
            "Wähle links eine Sendung aus, um ihren Verlauf zu sehen.",
        ))
        .build();
    let stack = gtk::Stack::new();
    stack.add_named(&scroll, Some("delivery"));
    stack.add_named(&empty_page, Some("empty"));
    stack.set_visible_child_name("empty");

    {
        let tracking_number = tracking_number.clone();
        let toast_overlay = toast_overlay.clone();
        copy_button.connect_clicked(move |_| {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&tracking_number.text());
                toast_overlay.add_toast(adw::Toast::new(&tr("Sendungsnummer kopiert")));
            }
        });
    }

    (
        stack.clone(),
        DetailView {
            stack,
            empty_page,
            carrier_badge,
            carrier_name,
            description,
            tracking_number,
            status_box,
            status_icon,
            status_label,
            expected,
            expected_detail,
            progress,
            timeline,
            detail_menu: menu,
            local_delivery_action: local_delivery_action.downgrade(),
            using_real_data,
        },
    )
}

fn timeline_row(
    title: &str,
    date: &str,
    location: Option<&str>,
    additional: Option<&str>,
    is_latest: bool,
    is_last: bool,
) -> gtk::Box {
    let row = gtk::Box::new(Orientation::Horizontal, 14);
    row.set_margin_start(18);
    row.set_margin_end(18);
    row.set_margin_top(16);
    row.set_margin_bottom(if is_last { 16 } else { 8 });

    let marker_column = gtk::Box::new(Orientation::Vertical, 0);
    marker_column.set_width_request(18);
    let marker = gtk::Label::new(Some(if is_latest { "●" } else { "○" }));
    marker.add_css_class(if is_latest {
        "timeline-marker-active"
    } else {
        "timeline-marker"
    });
    marker_column.append(&marker);
    if !is_last {
        let line = gtk::Separator::new(Orientation::Vertical);
        line.set_vexpand(true);
        line.set_margin_top(5);
        line.add_css_class("timeline-line");
        marker_column.append(&line);
    }
    row.append(&marker_column);

    let content = gtk::Box::new(Orientation::Vertical, 4);
    content.set_hexpand(true);
    let header = gtk::Box::new(Orientation::Horizontal, 8);
    let title_label = gtk::Label::new(Some(title));
    title_label.set_xalign(0.0);
    title_label.set_wrap(true);
    title_label.set_hexpand(true);
    title_label.add_css_class("heading");
    let date_label = gtk::Label::new(Some(date));
    date_label.set_valign(Align::Start);
    date_label.add_css_class("caption");
    date_label.add_css_class("dim-label");
    header.append(&title_label);
    header.append(&date_label);
    content.append(&header);

    if let Some(location) = location {
        let location_row = gtk::Box::new(Orientation::Horizontal, 5);
        let icon = gtk::Image::from_icon_name("mark-location-symbolic");
        icon.set_pixel_size(13);
        let label = gtk::Label::new(Some(location));
        label.set_xalign(0.0);
        label.add_css_class("caption");
        label.add_css_class("dim-label");
        location_row.append(&icon);
        location_row.append(&label);
        content.append(&location_row);
    }

    if let Some(additional) = additional {
        let label = gtk::Label::new(Some(additional));
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.add_css_class("caption");
        label.add_css_class("dim-label");
        content.append(&label);
    }
    row.append(&content);
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn sidebar_indices_keep_delivered_and_recent_as_separate_rows() {
        assert_eq!(
            DeliveryFilter::from_sidebar_index(4),
            DeliveryFilter::Delivered
        );
        assert_eq!(
            DeliveryFilter::from_sidebar_index(5),
            DeliveryFilter::Recent
        );
    }

    #[test]
    fn delivered_filter_excludes_frozen_recent_entries() {
        assert!(DeliveryFilter::Delivered.matches(DeliveryStatus::Delivered));
        assert!(!DeliveryFilter::Delivered.matches(DeliveryStatus::Frozen));
        assert!(DeliveryFilter::Recent.matches(DeliveryStatus::Delivered));
        assert!(DeliveryFilter::Recent.matches(DeliveryStatus::Frozen));
    }

    #[test]
    fn carrier_choices_are_sorted_and_keep_the_api_code() {
        let mut carriers = SupportedCarriersResponse::new();
        carriers.insert(
            "zeta".into(),
            crate::api::SupportedCarrier {
                name: "Zeta Parcel".into(),
                name_variations: BTreeMap::new(),
                extra_required: None,
            },
        );
        carriers.insert(
            "alpha".into(),
            crate::api::SupportedCarrier {
                name: "Alpha Post".into(),
                name_variations: BTreeMap::new(),
                extra_required: None,
            },
        );

        let choices = carrier_choices(carriers);

        assert_eq!(choices[0].code, "alpha");
        assert_eq!(choices[1].code, "zeta");
        assert_eq!(choices[0].name, "Alpha Post");
    }

    #[test]
    fn carrier_suggestion_uses_the_first_available_parcel_code() {
        let choices = vec![
            CarrierChoice {
                code: "dpdgpcode".into(),
                name: "DPD Group".into(),
            },
            CarrierChoice {
                code: "ups".into(),
                name: "UPS".into(),
            },
        ];

        assert_eq!(
            suggested_carrier_index(&choices, "01234567890123", "de_AT"),
            Some(0)
        );
        assert_eq!(
            suggested_carrier_index(&choices, "1Z5R89390357567127", "de_AT"),
            Some(1)
        );
        assert_eq!(
            suggested_carrier_index(&choices, "nicht-eindeutig", "de_AT"),
            None
        );
    }
}
