use ferris_db::Db;

pub(super) fn admin() -> ferris_auth::User {
    let mut user = ferris_auth::User::system_user("admin@local".to_string(), "Admin".to_string());
    user.roles = vec!["System Manager".to_string()];
    user
}

pub(super) async fn pool() -> Db {
    ferris_db::memory_db().await.unwrap()
}
