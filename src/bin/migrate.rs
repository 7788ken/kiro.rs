#[cfg(feature = "mysql")]
fn main() {
    let mut args = std::env::args().skip(1);
    let sqlite = args
        .next()
        .unwrap_or_else(|| "/app/config/accounts.db".to_string());
    let url = args
        .next()
        .or_else(|| std::env::var("MYSQL_URL").ok())
        .unwrap_or_else(|| {
            eprintln!("用法: kiro-rs-migrate <accounts.db> <mysql://user:password@host/database>");
            std::process::exit(2);
        });
    if let Err(error) = kiro_rs::accounts::migration::migrate(&sqlite, &url) {
        eprintln!("迁移失败: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(feature = "mysql"))]
fn main() {
    eprintln!("迁移工具需要 --features mysql");
    std::process::exit(1);
}
