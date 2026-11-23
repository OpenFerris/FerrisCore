//! REST router: `/api/resource/{doctype}[/{name}]` and `/api/method/{method}`.

mod doctype;
mod error;
mod locale;
mod methods;
mod resource;
mod role_permissions;
mod routes;
mod state;
mod widget_assets;

pub use error::error_response;
pub use locale::RequestLocale;
pub use routes::{api_router_with_app_state, api_router_with_modules};
pub use state::ApiState;

#[cfg(test)]
mod tests;
