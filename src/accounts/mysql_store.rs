use super::store::AccountError;
use mysql::{Opts, OptsBuilder, PooledConn, Pool, PoolConstraints, PoolOpts, Value, prelude::Queryable};
use parking_lot::Mutex;

pub type Params = Vec<Value>;

pub struct MySqlStore {
    pool: Pool,
    connection: Mutex<PooledConn>,
}

impl MySqlStore {
    pub fn open(url: &str) -> Result<Self, AccountError> {
        let opts = Opts::from_url(url).map_err(|error| AccountError::Internal(format!("MySQL 连接配置无效: {error}")))?;
        let pool = Pool::new(OptsBuilder::from_opts(opts).pool_opts(PoolOpts::new().with_constraints(PoolConstraints::new(1, 16).unwrap()))).map_err(mysql_error)?;
        let connection = pool.get_conn().map_err(mysql_error)?;
        let store = Self { pool, connection: Mutex::new(connection) };
        store.init()?;
        Ok(store)
    }

    fn init(&self) -> Result<(), AccountError> {
        let mut connection = self.connection.lock();
        connection.query_drop("SET NAMES utf8mb4").map_err(mysql_error)?;
        for statement in [
            "CREATE TABLE IF NOT EXISTS users (id BIGINT PRIMARY KEY AUTO_INCREMENT, username VARCHAR(191) NOT NULL, password_hash VARCHAR(255) NOT NULL, api_key_hash VARCHAR(64) NOT NULL UNIQUE, key_prefix VARCHAR(191) NOT NULL, role VARCHAR(20) NOT NULL CHECK (role IN ('super_admin', 'admin', 'user')), balance_micro BIGINT NOT NULL DEFAULT 0 CHECK (balance_micro >= 0), enabled TINYINT NOT NULL DEFAULT 1, created_at VARCHAR(64) NOT NULL, UNIQUE KEY uq_users_username (username)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
            "CREATE TABLE IF NOT EXISTS billing_config (id TINYINT PRIMARY KEY CHECK (id = 1), input_per_million DOUBLE NOT NULL, output_per_million DOUBLE NOT NULL) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
            "CREATE TABLE IF NOT EXISTS credit_transactions (id VARCHAR(64) PRIMARY KEY, user_id BIGINT NOT NULL, actor_id BIGINT NULL, kind VARCHAR(32) NOT NULL, amount_micro BIGINT NOT NULL, balance_after_micro BIGINT NOT NULL, model VARCHAR(191) NULL, input_tokens BIGINT NOT NULL DEFAULT 0, output_tokens BIGINT NOT NULL DEFAULT 0, reserved_micro BIGINT NOT NULL DEFAULT 0, input_rate DOUBLE NOT NULL DEFAULT 10.0, output_rate DOUBLE NOT NULL DEFAULT 50.0, status VARCHAR(32) NOT NULL, note TEXT NULL, created_at VARCHAR(64) NOT NULL, completed_at VARCHAR(64) NULL, request_id VARCHAR(64) NULL, path VARCHAR(255) NULL, stream TINYINT NOT NULL DEFAULT 0, balance_before_micro BIGINT NULL, reserved_balance_micro BIGINT NULL, http_status INT NULL, error_message TEXT NULL, duration_ms BIGINT NULL, first_token_ms BIGINT NULL, CONSTRAINT fk_transactions_user FOREIGN KEY (user_id) REFERENCES users(id), CONSTRAINT fk_transactions_actor FOREIGN KEY (actor_id) REFERENCES users(id)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
            "CREATE TABLE IF NOT EXISTS web_sessions (token_hash VARCHAR(64) PRIMARY KEY, user_id BIGINT NOT NULL, expires_at VARCHAR(64) NOT NULL, CONSTRAINT fk_sessions_user FOREIGN KEY (user_id) REFERENCES users(id)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
            "CREATE INDEX idx_transactions_user ON credit_transactions(user_id, created_at DESC)",
            "CREATE INDEX idx_transactions_pending ON credit_transactions(status)",
            "CREATE INDEX idx_transactions_created ON credit_transactions(created_at DESC)",
            "CREATE INDEX idx_transactions_request ON credit_transactions(request_id, created_at ASC)",
            "CREATE INDEX idx_sessions_expiry ON web_sessions(expires_at)",
        ] {
            if statement.starts_with("CREATE INDEX") {
                let name = statement.split_whitespace().nth(2).unwrap();
                let table = if name == "idx_sessions_expiry" { "web_sessions" } else { "credit_transactions" };
                let exists: Option<i64> = connection.exec_first("SELECT COUNT(*) FROM information_schema.statistics WHERE table_schema = DATABASE() AND table_name = ? AND index_name = ?", (table, name)).map_err(mysql_error)?;
                if exists.unwrap_or(0) > 0 { continue; }
            }
            connection.query_drop(statement).map_err(mysql_error)?;
        }
        connection.query_drop("INSERT IGNORE INTO billing_config VALUES (1, 10.0, 50.0)").map_err(mysql_error)?;
        Ok(())
    }

    pub fn execute(&self, sql: &str, params: Params) -> Result<u64, AccountError> {
        let mut connection = self.connection.lock();
        connection.exec_drop(sql, params).map_err(mysql_error)?;
        Ok(connection.affected_rows())
    }

    pub fn query<T: mysql::prelude::FromRow>(&self, sql: &str, params: Params) -> Result<Vec<T>, AccountError> {
        self.connection.lock().exec(sql, params).map_err(mysql_error)
    }

    pub fn map<T: mysql::prelude::FromRow, U>(&self, sql: &str, params: Params, map: impl FnMut(T) -> U) -> Result<Vec<U>, AccountError> {
        self.connection.lock().exec_map(sql, params, map).map_err(mysql_error)
    }

    pub fn first<T: mysql::prelude::FromRow>(&self, sql: &str, params: Params) -> Result<Option<T>, AccountError> {
        self.connection.lock().exec_first(sql, params).map_err(mysql_error)
    }

    pub fn transaction<T>(&self, callback: impl FnOnce(&mut mysql::Transaction<'_>) -> Result<T, AccountError>) -> Result<T, AccountError> {
        let mut connection = self.connection.lock();
        let mut transaction = connection.start_transaction(mysql::TxOpts::default()).map_err(mysql_error)?;
        let result = callback(&mut transaction)?;
        transaction.commit().map_err(mysql_error)?;
        Ok(result)
    }
}

pub fn mysql_error(error: mysql::Error) -> AccountError {
    AccountError::Internal(format!("MySQL 数据库错误: {error}"))
}
