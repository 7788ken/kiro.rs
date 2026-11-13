use mysql::{Conn, Opts, prelude::Queryable};
use rusqlite::Connection;
use std::collections::BTreeMap;

type Rows = Vec<Vec<rusqlite::types::Value>>;

pub fn migrate(sqlite_path: &str, mysql_url: &str) -> Result<(), String> {
    let sqlite =
        Connection::open_with_flags(sqlite_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("打开 SQLite 备份失败: {e}"))?;
    let mut mysql =
        Conn::new(Opts::from_url(mysql_url).map_err(|e| format!("MySQL 配置无效: {e}"))?)
            .map_err(|e| format!("连接 MySQL 失败: {e}"))?;
    let _ = super::mysql_store::MySqlStore::open(mysql_url)
        .map_err(|e| format!("初始化 MySQL 表结构失败: {e}"))?;
    mysql
        .query_drop("SET NAMES utf8mb4")
        .map_err(|e| e.to_string())?;
    mysql
        .query_drop("SET FOREIGN_KEY_CHECKS = 0")
        .map_err(|e| e.to_string())?;
    let result = migrate_inner(&sqlite, &mut mysql);
    let _ = mysql.query_drop("SET FOREIGN_KEY_CHECKS = 1");
    result
}

fn migrate_inner(sqlite: &Connection, mysql: &mut Conn) -> Result<(), String> {
    let users = read_users(sqlite)?;
    let billing = read_rows(
        sqlite,
        "billing_config",
        &["id", "input_per_million", "output_per_million"],
    )?;
    let transactions = read_transactions(sqlite)?;
    let sessions = read_rows(
        sqlite,
        "web_sessions",
        &["token_hash", "user_id", "expires_at"],
    )?;
    for (table, count) in [
        ("users", users.len()),
        ("billing_config", billing.len()),
        ("credit_transactions", transactions.len()),
        ("web_sessions", sessions.len()),
    ] {
        let current: i64 = mysql
            .query_first(format!("SELECT COUNT(*) FROM {table}"))
            .map_err(|e| e.to_string())?
            .unwrap_or(0);
        if current != 0 && current as usize != count {
            return Err(format!(
                "目标表 {table} 已有 {current} 条记录，与 SQLite 的 {count} 条不一致，拒绝覆盖"
            ));
        }
    }
    let mut tx = mysql
        .start_transaction(mysql::TxOpts::default())
        .map_err(|e| e.to_string())?;
    let tables = [
        ("users", &users),
        ("billing_config", &billing),
        ("credit_transactions", &transactions),
        ("web_sessions", &sessions),
    ];
    for (table, rows) in tables {
        let columns: &[&str] = match table {
            "users" => &[
                "id",
                "username",
                "password_hash",
                "api_key_hash",
                "key_prefix",
                "role",
                "balance_micro",
                "enabled",
                "created_at",
                "parent_id",
            ],
            "billing_config" => &["id", "input_per_million", "output_per_million"],
            "credit_transactions" => &[
                "id",
                "user_id",
                "actor_id",
                "kind",
                "amount_micro",
                "balance_after_micro",
                "model",
                "input_tokens",
                "output_tokens",
                "reserved_micro",
                "input_rate",
                "output_rate",
                "status",
                "note",
                "created_at",
                "completed_at",
                "request_id",
                "path",
                "stream",
                "balance_before_micro",
                "reserved_balance_micro",
                "http_status",
                "error_message",
                "duration_ms",
                "first_token_ms",
            ],
            _ => &["token_hash", "user_id", "expires_at"],
        };
        for row in rows {
            let placeholders = row.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let values = row.iter().map(to_mysql).collect::<Vec<_>>();
            tx.exec_drop(
                format!(
                    "INSERT IGNORE INTO {table} ({}) VALUES ({placeholders})",
                    columns.join(",")
                ),
                values,
            )
            .map_err(|e| format!("迁移 {table} 失败: {e}"))?;
        }
    }
    let max_id: i64 = tx
        .query_first("SELECT COALESCE(MAX(id), 0) FROM users")
        .map_err(|e| e.to_string())?
        .unwrap_or(0);
    tx.query_drop(format!("ALTER TABLE users AUTO_INCREMENT = {}", max_id + 1))
        .map_err(|e| e.to_string())?;
    for (table, count) in [
        ("users", users.len()),
        ("billing_config", billing.len()),
        ("credit_transactions", transactions.len()),
        ("web_sessions", sessions.len()),
    ] {
        let current: i64 = tx
            .query_first(format!("SELECT COUNT(*) FROM {table}"))
            .map_err(|e| e.to_string())?
            .unwrap_or(0);
        if current as usize != count {
            return Err(format!(
                "{table} 校验失败：SQLite {count} 条，MySQL {current} 条"
            ));
        }
    }
    let sqlite_balance: BTreeMap<i64, i64> = read_rows(sqlite, "users", &["id", "balance_micro"])?
        .into_iter()
        .map(|r| (int(&r[0]), int(&r[1])))
        .collect();
    for (id, balance) in sqlite_balance {
        let target: i64 = tx
            .exec_first("SELECT balance_micro FROM users WHERE id = ?", (id,))
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("用户 {id} 未迁移"))?;
        if target != balance {
            return Err(format!(
                "用户 {id} 余额不一致：SQLite {balance}，MySQL {target}"
            ));
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    println!(
        "迁移完成：users={} billing_config={} credit_transactions={} web_sessions={}",
        users.len(),
        billing.len(),
        transactions.len(),
        sessions.len()
    );
    Ok(())
}

fn read_users(connection: &Connection) -> Result<Rows, String> {
    let has_parent = connection
        .prepare("PRAGMA table_info(users)")
        .map_err(|e| e.to_string())?
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?
        .contains(&"parent_id".to_string());
    let mut rows = read_rows(
        connection,
        "users",
        &[
            "id",
            "username",
            "password_hash",
            "api_key_hash",
            "key_prefix",
            "role",
            "balance_micro",
            "enabled",
            "created_at",
        ],
    )?;
    if has_parent {
        let parents = read_rows(connection, "users", &["id", "parent_id"])?;
        for row in &mut rows {
            let id = int(&row[0]);
            row.push(
                parents
                    .iter()
                    .find(|parent| int(&parent[0]) == id)
                    .map(|parent| parent[1].clone())
                    .unwrap_or(rusqlite::types::Value::Null),
            );
        }
    } else {
        for row in &mut rows {
            row.push(rusqlite::types::Value::Null);
        }
    }
    Ok(rows)
}

fn read_transactions(connection: &Connection) -> Result<Rows, String> {
    let mut statement = connection.prepare("SELECT id,user_id,actor_id,kind,amount_micro,balance_after_micro,model,input_tokens,output_tokens,reserved_micro,input_rate,output_rate,status,note,created_at,completed_at FROM credit_transactions").map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let mut values = (0..16)
                .map(|i| row.get::<_, rusqlite::types::Value>(i))
                .collect::<Result<Vec<_>, _>>()?;
            values.extend((0..9).map(|_| rusqlite::types::Value::Null));
            Ok(values)
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Rows, _>>().map_err(|e| e.to_string())
}

fn read_rows(connection: &Connection, table: &str, columns: &[&str]) -> Result<Rows, String> {
    let sql = format!("SELECT {} FROM {table}", columns.join(","));
    let mut statement = connection
        .prepare(&sql)
        .map_err(|e| format!("读取 SQLite {table} 失败: {e}"))?;
    statement
        .query_map([], |row| (0..columns.len()).map(|i| row.get(i)).collect())
        .map_err(|e| e.to_string())?
        .collect::<Result<Rows, _>>()
        .map_err(|e| e.to_string())
}

fn to_mysql(value: &rusqlite::types::Value) -> mysql::Value {
    match value {
        rusqlite::types::Value::Null => mysql::Value::NULL,
        rusqlite::types::Value::Integer(value) => (*value).into(),
        rusqlite::types::Value::Real(value) => (*value).into(),
        rusqlite::types::Value::Text(value) => value.clone().into(),
        rusqlite::types::Value::Blob(value) => value.clone().into(),
    }
}

fn int(value: &rusqlite::types::Value) -> i64 {
    match value {
        rusqlite::types::Value::Integer(value) => *value,
        _ => 0,
    }
}
