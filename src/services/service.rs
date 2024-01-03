use serde::Deserialize;
use std::sync::Arc;

use crate::global::AsyncGlobal;

use super::http::service::HttpService;

pub trait Service {
    fn run(&self, global: Arc<AsyncGlobal>);
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ServiceType {
    #[serde(rename = "http")]
    Http(HttpService),
}

impl Service for ServiceType {
    fn run(&self, global: Arc<AsyncGlobal>) {
        match self {
            ServiceType::Http(service) => service.run(global),
        }
    }
}
