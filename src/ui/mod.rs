use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::{Align, Orientation, gio};

use crate::model::{Delivery, DeliveryStatus, demo_deliveries};

const STATUS_CLASSES: [&str; 6] = [
    "status-delivered",
    "status-pickup",
    "status-today",
    "status-problem",
    "status-muted",
    "status-transit",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryFilter {
    Active,
    InTransit,
    OutForDelivery,
    ReadyForPickup,
    Recent,
}

impl DeliveryFilter {
    fn title(self) -> &'static str {
        match self {
            Self::Active => "Aktive Sendungen",
            Self::InTransit => "Unterwegs",
            Self::OutForDelivery => "In Zustellung",
            Self::ReadyForPickup => "Abholbereit",
            Self::Recent => "Kürzlich",
        }
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
            Self::Recent => matches!(status, DeliveryStatus::Delivered | DeliveryStatus::Frozen),
        }
    }
}

#[derive(Clone)]
struct DetailView {
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
}

impl DetailView {
    fn update(&self, delivery: &Delivery) {
        self.carrier_badge.set_label(&delivery.initials());
        self.carrier_name.set_label(&delivery.carrier_name);
        self.description.set_label(&delivery.description);
        self.tracking_number.set_label(&delivery.tracking_number);
        self.status_icon
            .set_icon_name(Some(delivery.status.icon_name()));
        self.status_label.set_label(delivery.status.label());
        self.expected
            .set_label(delivery.expected.as_deref().unwrap_or("Noch offen"));
        self.expected_detail
            .set_label(delivery.expected_detail.as_deref().unwrap_or(""));
        self.progress.set_fraction(delivery.status.progress());

        for class in STATUS_CLASSES {
            self.status_box.remove_css_class(class);
        }
        self.status_box.add_css_class(delivery.status.css_class());

        while let Some(child) = self.timeline.first_child() {
            self.timeline.remove(&child);
        }

        if delivery.events.is_empty() {
            let empty = adw::StatusPage::builder()
                .icon_name("mail-unread-symbolic")
                .title("Noch keine Ereignisse")
                .description("Parcel hat für diese Sendung noch keine Details bereitgestellt.")
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
}

pub fn build_window(app: &adw::Application) {
    let deliveries = Rc::new(demo_deliveries());
    let current_filter = Rc::new(RefCell::new(DeliveryFilter::Active));
    let visible_indices = Rc::new(RefCell::new(Vec::<usize>::new()));
    let search_query = Rc::new(RefCell::new(String::new()));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Ankunft")
        .default_width(1280)
        .default_height(790)
        .width_request(900)
        .height_request(620)
        .build();

    let toast_overlay = adw::ToastOverlay::new();
    window.set_content(Some(&toast_overlay));

    let root = gtk::Box::new(Orientation::Vertical, 0);
    toast_overlay.set_child(Some(&root));

    let header = adw::HeaderBar::new();
    header.add_css_class("flat-header");
    let title = adw::WindowTitle::new("Ankunft", "Deine Lieferungen auf einen Blick");
    header.set_title_widget(Some(&title));

    let search = gtk::SearchEntry::builder()
        .placeholder_text("Sendungen durchsuchen")
        .width_request(260)
        .build();
    search.set_key_capture_widget(Some(&window));
    header.pack_start(&search);

    let refresh_button = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Sendungen aktualisieren")
        .build();
    header.pack_end(&refresh_button);

    let add_button = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Neue Sendung")
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

    let sidebar = build_sidebar(&deliveries);
    main_paned.set_start_child(Some(&sidebar.container));

    let content_paned = gtk::Paned::new(Orientation::Horizontal);
    content_paned.set_position(410);
    content_paned.set_resize_start_child(false);
    content_paned.set_shrink_start_child(false);
    main_paned.set_end_child(Some(&content_paned));

    let list_column = gtk::Box::new(Orientation::Vertical, 0);
    list_column.add_css_class("list-column");
    let demo_banner = adw::Banner::builder()
        .title("Prototypmodus · Sichere Beispieldaten")
        .button_label("API verbinden")
        .revealed(true)
        .build();
    list_column.append(&demo_banner);

    let list_header = gtk::Box::new(Orientation::Horizontal, 10);
    list_header.set_margin_start(20);
    list_header.set_margin_end(18);
    list_header.set_margin_top(18);
    list_header.set_margin_bottom(10);
    let list_title = gtk::Label::new(Some(DeliveryFilter::Active.title()));
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

    let (detail_widget, detail_view) = build_detail_view(&toast_overlay);
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
            let indices: Vec<usize> = deliveries
                .iter()
                .enumerate()
                .filter(|(_, delivery)| {
                    filter.matches(delivery.status) && delivery.matches_query(&query)
                })
                .map(|(index, _)| index)
                .collect();

            for index in &indices {
                delivery_list.append(&delivery_row(&deliveries[*index]));
            }
            *visible_indices.borrow_mut() = indices;

            let count = visible_indices.borrow().len();
            list_title.set_label(filter.title());
            list_count.set_label(&count.to_string());

            if let Some(row) = delivery_list.row_at_index(0) {
                delivery_list.select_row(Some(&row));
                if let Some(index) = visible_indices.borrow().first() {
                    detail_view.update(&deliveries[*index]);
                }
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
                detail_view.update(&deliveries[*delivery_index]);
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
            let filter = match row.index() {
                0 => DeliveryFilter::Active,
                1 => DeliveryFilter::InTransit,
                2 => DeliveryFilter::OutForDelivery,
                3 => DeliveryFilter::ReadyForPickup,
                _ => DeliveryFilter::Recent,
            };
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

    {
        let toast_overlay = toast_overlay.clone();
        refresh_button.connect_clicked(move |_| {
            let toast = adw::Toast::new("Beispieldaten sind bereits aktuell");
            toast.set_timeout(3);
            toast_overlay.add_toast(toast);
        });
    }

    {
        let toast_overlay = toast_overlay.clone();
        add_button.connect_clicked(move |_| {
            toast_overlay.add_toast(adw::Toast::new(
                "Das Hinzufügen wird mit der sicheren API-Einrichtung aktiviert",
            ));
        });
    }

    {
        let toast_overlay = toast_overlay.clone();
        demo_banner.connect_button_clicked(move |_| {
            toast_overlay.add_toast(adw::Toast::new(
                "Der Einrichtungsassistent folgt im nächsten Schritt",
            ));
        });
    }

    rebuild();
    if let Some(row) = sidebar.list.row_at_index(0) {
        sidebar.list.select_row(Some(&row));
    }

    window.present();
}

struct Sidebar {
    container: gtk::Box,
    list: gtk::ListBox,
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
    let eyebrow = gtk::Label::new(Some("PARCEL PREMIUM"));
    eyebrow.set_xalign(0.0);
    eyebrow.add_css_class("eyebrow");
    let title = gtk::Label::new(Some("Meine Sendungen"));
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
            "Aktiv",
            deliveries
                .iter()
                .filter(|item| item.status.is_active())
                .count(),
        ),
        (
            "go-next-symbolic",
            "Unterwegs",
            deliveries
                .iter()
                .filter(|item| {
                    matches!(
                        item.status,
                        DeliveryStatus::InTransit | DeliveryStatus::InformationReceived
                    )
                })
                .count(),
        ),
        (
            "send-to-symbolic",
            "In Zustellung",
            deliveries
                .iter()
                .filter(|item| item.status == DeliveryStatus::OutForDelivery)
                .count(),
        ),
        (
            "folder-download-symbolic",
            "Abholbereit",
            deliveries
                .iter()
                .filter(|item| item.status == DeliveryStatus::ReadyForPickup)
                .count(),
        ),
        (
            "document-open-recent-symbolic",
            "Kürzlich",
            deliveries
                .iter()
                .filter(|item| {
                    matches!(
                        item.status,
                        DeliveryStatus::Delivered | DeliveryStatus::Frozen
                    )
                })
                .count(),
        ),
    ];

    for (icon, label, count) in filters {
        let row = gtk::ListBoxRow::new();
        let content = gtk::Box::new(Orientation::Horizontal, 10);
        content.set_margin_start(10);
        content.set_margin_end(10);
        content.set_margin_top(9);
        content.set_margin_bottom(9);
        let image = gtk::Image::from_icon_name(icon);
        image.set_pixel_size(17);
        let label = gtk::Label::new(Some(label));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        let count = gtk::Label::new(Some(&count.to_string()));
        count.add_css_class("sidebar-count");
        content.append(&image);
        content.append(&label);
        content.append(&count);
        row.set_child(Some(&content));
        list.append(&row);
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
    let sync_title = gtk::Label::new(Some("Noch nicht verbunden"));
    sync_title.set_xalign(0.0);
    sync_title.add_css_class("heading");
    let sync_text = gtk::Label::new(Some(
        "Dein API-Schlüssel wird später sicher im GNOME-Schlüsselbund gespeichert.",
    ));
    sync_text.set_xalign(0.0);
    sync_text.set_wrap(true);
    sync_text.add_css_class("caption");
    sync_text.add_css_class("dim-label");
    sync_card.append(&sync_title);
    sync_card.append(&sync_text);
    container.append(&sync_card);

    Sidebar { container, list }
}

fn delivery_row(delivery: &Delivery) -> gtk::ListBoxRow {
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

    let expected = gtk::Label::new(Some(delivery.expected.as_deref().unwrap_or("Offen")));
    expected.set_valign(Align::Start);
    expected.add_css_class("expected-small");
    top.append(&expected);
    card.append(&top);

    let status_row = gtk::Box::new(Orientation::Horizontal, 7);
    let status_icon = gtk::Image::from_icon_name(delivery.status.icon_name());
    status_icon.set_pixel_size(14);
    let status = gtk::Label::new(Some(delivery.status.label()));
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

fn build_detail_view(toast_overlay: &adw::ToastOverlay) -> (gtk::ScrolledWindow, DetailView) {
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
        .tooltip_text("Weitere Optionen")
        .valign(Align::Start)
        .build();
    let menu = gio::Menu::new();
    menu.append(Some("In Parcel Web öffnen"), Some("app.open-web"));
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
    let expected_caption = gtk::Label::new(Some("VORAUSSICHTLICH"));
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
    let tracking_caption = gtk::Label::new(Some("SENDUNGSNUMMER"));
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
        .tooltip_text("Sendungsnummer kopieren")
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
    let timeline_title = gtk::Label::new(Some("Sendungsverlauf"));
    timeline_title.set_xalign(0.0);
    timeline_title.set_hexpand(true);
    timeline_title.add_css_class("title-3");
    let live_badge = gtk::Label::new(Some("LIVE"));
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

    {
        let tracking_number = tracking_number.clone();
        let toast_overlay = toast_overlay.clone();
        copy_button.connect_clicked(move |_| {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&tracking_number.text());
                toast_overlay.add_toast(adw::Toast::new("Sendungsnummer kopiert"));
            }
        });
    }

    (
        scroll,
        DetailView {
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
