#[cfg(test)]
mod tests {
    // Integration-test crates do not have `super::*` in scope, so the registry
    // types and the permission builder are imported from the crate explicitly.
    use ferris_schema::{DoctypeMeta, FieldMeta, IndexMeta, RolePermission};

    #[test]
    fn doctype_new_only_sets_identity() {
        let meta = DoctypeMeta::new("Print Settings", "Printing");
        assert_eq!(meta.name, "Print Settings");
        assert_eq!(meta.module, "Printing");
        assert!(!meta.is_single);
        assert!(meta.fields.is_empty());
        assert!(meta.permissions.is_empty());
        assert!(meta.naming_series.is_empty());
    }

    #[test]
    fn builders_set_the_intended_flags() {
        let meta = DoctypeMeta::new("System Settings", "System")
            .single()
            .hidden()
            .allow_delete()
            .in_list_view()
            .sort("modified", "desc")
            .title_field("name")
            .field(FieldMeta::data("name").label("Name").required())
            .field(FieldMeta::check("enabled").default_value("1"))
            .permission(
                RolePermission::new("System Manager")
                    .read()
                    .write()
                    .create()
                    .delete(),
            );

        assert!(meta.is_single);
        assert!(meta.hidden);
        assert!(meta.allow_delete);
        assert!(meta.in_list_view);
        assert_eq!(meta.sort_field.as_deref(), Some("modified"));
        assert_eq!(meta.sort_order.as_deref(), Some("desc"));
        assert_eq!(meta.title_field.as_deref(), Some("name"));
        assert_eq!(meta.fields.len(), 2);
        assert_eq!(meta.fields[0].fieldtype, "Data");
        assert!(meta.fields[0].reqd);
        assert_eq!(meta.fields[0].label.as_deref(), Some("Name"));
        assert_eq!(meta.fields[1].default.as_deref(), Some("1"));
        assert_eq!(meta.permissions.len(), 1);
        assert_eq!(meta.permissions[0].role, "System Manager");
        assert!(meta.permissions[0].read && meta.permissions[0].delete);
        assert!(!meta.permissions[0].submit);
    }

    #[test]
    fn typed_field_constructors_pick_the_right_fieldtype() {
        assert_eq!(FieldMeta::data("a").fieldtype, "Data");
        assert_eq!(FieldMeta::check("b").fieldtype, "Check");
        assert_eq!(FieldMeta::datetime("c").fieldtype, "Datetime");
        assert_eq!(
            FieldMeta::link("d", "User").options.as_deref(),
            Some("User")
        );
        assert_eq!(
            FieldMeta::select("e", "A\nB").options.as_deref(),
            Some("A\nB")
        );
        assert_eq!(
            FieldMeta::table("f", "Child").options.as_deref(),
            Some("Child")
        );
    }

    #[test]
    fn index_builder_collects_fields_and_uniqueness() {
        let index = IndexMeta::new("idx_user", ["user", "status"]).unique();
        assert_eq!(index.name, "idx_user");
        assert_eq!(index.fields, vec!["user", "status"]);
        assert!(index.unique);
    }

    #[test]
    fn role_permission_starts_from_nothing() {
        let rule = RolePermission::new("Desk User").build();
        assert_eq!(rule.role, "Desk User");
        assert!(!rule.read && !rule.write && !rule.create && !rule.delete);
        assert!(!rule.if_owner);

        let owner = RolePermission::new("All").read().if_owner().build();
        assert!(owner.read && owner.if_owner && !owner.write);
    }
}
