//! Module and global-search settings DocTypes (Desk section **Build**).

use crate::registry::{DoctypeMeta, FieldMeta};

use super::consts::BUILD_MODULE;

use super::permissions::{system_manager, system_manager_single};

/// A module an app groups its DocTypes under.
#[must_use]
pub fn module_def_meta() -> DoctypeMeta {
    DoctypeMeta::new("Module Def", BUILD_MODULE)
        .description("A module an app groups its DocTypes under.")
        .title_field("module_name")
        .sort("modified", "desc")
        .allow_delete()
        .field(FieldMeta::section_break("section_details").label("Details"))
        .field(
            FieldMeta::data("module_name")
                .label("Module Name")
                .in_list_view(),
        )
        .field(FieldMeta::data("app_name").label("App Name"))
        .field(
            FieldMeta::select(
                "module_type",
                "Modules\nSetup\nDesk\nCustomization\nIntegrations",
            )
            .label("Module Type"),
        )
        .field(FieldMeta::text("workspaces").label("Workspaces"))
        // A Frappe-style `scheduler_events` map, `kind -> [job]`, stored as JSON.
        // The scheduler role reads the enabled rows at boot and registers each
        // job; the same field is what an app's fixtures export.
        .field(
            FieldMeta::code("scheduler_events")
                .label("Scheduler Events")
                .description(
                    "JSON map of cadence to job names ({\"daily\": [\"app.sweep\"]}); \
                     registered on every scheduler boot",
                ),
        )
        .permission(system_manager())
}

/// Single: what the global search indexes and how many hits it returns.
#[must_use]
pub fn global_search_settings_meta() -> DoctypeMeta {
    DoctypeMeta::new("Global Search Settings", BUILD_MODULE)
        .single()
        .description("What the global search indexes and how many hits it returns.")
        .field(
            FieldMeta::check("enabled")
                .label("Enabled")
                .default_value("1"),
        )
        .field(
            FieldMeta::int("max_results")
                .label("Max Results")
                .default_value("20"),
        )
        .permission(system_manager_single())
}
