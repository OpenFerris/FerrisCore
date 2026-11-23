//! Ferris API - generic REST layer over the JSON document store.

pub mod activity;
pub mod authz;
pub mod collab;
pub mod core_doctypes;
pub mod custom_docperm;
pub mod customization;
pub mod dashboard_layout;
pub mod dashboard_layout_doc;
pub mod dashboard_settings;
pub mod dashboards;
pub mod data;
pub mod desired_state;
pub mod desk;
pub mod doc_meta;
pub mod doctypes;
pub mod engagement;
pub mod form_tours;
pub mod graphql;
pub mod log;
pub mod method;
pub mod mfa;
pub mod module;
pub mod notifications;
pub mod notify;
pub mod oauth;
pub mod openapi;
pub mod panel;
pub mod permission_admin;
pub mod router;
pub mod rpc;
pub mod seed;
pub(crate) mod service;
pub mod session;
pub mod shutdown;
pub mod site;
pub mod store;
pub mod tags;
pub mod themes;
pub mod types;
pub mod ui;
pub mod widgets;
pub mod workspace_store;
pub mod workspaces;

pub use activity::activity;
pub use authz::{PolicyInstall, PolicySource, install_policies, install_policies_from_path};
pub use collab::{
    add_assignment, add_attachment, add_share, list_assignments, list_attachments, list_shares,
    remove_assignment, remove_attachment, remove_share,
};
pub use core_doctypes::register_core_doctypes;
pub use customization::{
    delete_custom_field, delete_property_setter, ensure_can_customize, export_customization,
    get_customization, put_custom_field, put_property_setter, read_customization,
    remove_custom_field, remove_property_setter, save_custom_field, save_property_setter,
};
pub use desired_state::{DesiredState, DesiredStateStore, PublishOutcome};
pub use desk::{modules, search};
pub use engagement::{add_comment, engagement, list_comments, toggle_like};
pub use graphql::{GraphQlError, GraphQlRequest, GraphQlResponse, execute_query};
pub use log::client_report;
pub use method::{MethodHandler, MethodRegistry};
pub use mfa::{activate, disable, enable};
pub use module::{
    Module, ModuleOrder, ModuleRegistry, ModuleState, OpenApiFragment, ServiceContext, ServiceRoles,
};
pub use oauth::{OAuthState, oauth_router, oauth_router_with_manager, oauth_router_with_state};
pub use openapi::openapi_spec;
pub use panel::{
    events, get_notification_settings, mark_notifications_read, notifications,
    put_notification_settings,
};
pub use router::{ApiState, api_router_with_app_state, api_router_with_modules};
pub use rpc::{JsonRpcError, JsonRpcRequest, JsonRpcResponse, dispatch, dispatch_batch};
pub use seed::{SeedReport, seed_singletons};
pub use session::{
    SessionHandlers, login, logout, logout_all, session as session_me, set_language, verify_mfa,
};
pub use site::site_settings;
pub use store::{DocStore, effective_meta, ensure_writable};
pub use themes::{
    ThemeSpec, all_themes, builtin_themes, dark_theme, light_theme, register_theme,
    register_themes, themes as themes_handler,
};
pub use types::{ApiError, ListParams, ListResponse, Order};
pub use ui::ui_extensions;
pub use widgets::{script_by_name, script_by_path, stylesheet_by_name};
pub use workspace_store::{WorkspaceSeedReport, load_workspaces, seed_workspaces};

/// The canonical wire contract, re-exported so `ferris-api` and `ferris-web`
/// can name the same types without either having to depend on `ferris-schema`
/// directly.
pub use ferris_schema::contract;
