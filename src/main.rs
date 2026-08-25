#[allow(dead_code, unused_imports)]
mod api;
mod app;
mod carrier_detection;
mod i18n;
mod model;
mod notifications;
mod rate_limit;
mod secrets;
mod storage;
mod ui;

fn main() {
    i18n::initialize();
    app::run();
}
