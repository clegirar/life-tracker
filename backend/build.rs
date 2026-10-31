// sqlx::migrate! embarque les fichiers : recompiler quand une migration change.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
