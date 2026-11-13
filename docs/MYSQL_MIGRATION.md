# MySQL 部署、迁移与回滚

## 生产架构

- 数据库：现有 MySQL 8.4 平台，独立数据库 `kiro_rs`，表使用 InnoDB。
- 连接：`MYSQL_URL=mysql://用户:密码@数据库主机:3306/kiro_rs`，通过服务器 `.env.mysql` 注入。
- 生产 Docker 镜像使用 `--features mysql`，必须设置 `MYSQL_URL`，不会自动回退到 SQLite。
- 数据库账号只授予 `kiro_rs` 所需权限，不使用 root；MySQL 端口不要暴露到公网。
- `config/` 继续保存 `config.json`、`credentials.json` 和备份，账户与账务不再写入 `accounts.db`。

## 配置

在服务器创建 `.env.mysql`，权限设为 `600`：

```dotenv
MYSQL_URL=mysql://kiro_rs_app:请替换为独立随机密码@new-api-mysql:3306/kiro_rs
```

Compose 同时挂载 `config/` 和 `.env.mysql`，并连接到 MySQL 所在 Docker 网络。不要把 `.env.mysql`、真实数据库密码或备份提交到 Git。

初始化表结构由应用启动自动完成，包含账户上级 `parent_id` 的增量迁移；旧账户的上级为空，仅超管可管理。凭证归属 `registeredBy` 和登记人名称保存在 `credentials.json`，历史无归属凭证仅超管可管理。SQLite 导入 MySQL 时保留已有上级关系，没有该列的旧备份按无上级处理。数据库、独立账号和最小权限需由管理员预先创建。MySQL 表使用 `utf8mb4`、InnoDB、外键及条件更新。计费预授权、实际结算、失败退款和重复结算保护均在事务内完成。

## 本次迁移结果

- SQLite 一致性备份：`/opt/kiro-rs/config/backups/accounts-mysql-cutover-20261112.db`。
- 迁移内容：3 个用户、188 条流水、1 条计费配置、0 个有效网页会话。
- 用户 ID、密码哈希、API Key 哈希、积分余额、原始流水和计费配置均保留。
- 迁移工具支持按旧 SQLite 表结构补齐新增日志字段，目标有冲突数据时拒绝覆盖。
- 生产日志已明确显示 `用户、权限及积分账本已启用: MySQL`。
- 已验证：管理后台、账户与用户接口、历史流水、密码登录、真实 `/cc/v1/messages` 流式调用及实际扣费流水。

## 手动迁移

先备份并确认完整性：

```bash
cd /opt/kiro-rs
sqlite3 config/accounts.db ".backup config/backups/accounts-before-mysql.db"
sqlite3 -readonly config/backups/accounts-before-mysql.db "PRAGMA integrity_check;"
```

使用包含 `migrate` 的应用镜像执行（`.env.mysql` 提供容器内的 `MYSQL_URL`）：

```bash
cd /opt/kiro-rs
docker run --rm \
  --network deploy_new-api-network \
  --env-file .env.mysql \
  -v /opt/kiro-rs/config:/config:ro \
  kiro-rs:local \
  sh -c 'exec /app/migrate /config/backups/accounts-before-mysql.db "$MYSQL_URL"'
```

目标表为空时执行导入；重复执行使用 `INSERT IGNORE`，但若目标记录数与 SQLite 不一致会拒绝覆盖。迁移完成后逐表核对数量、用户 ID 和余额，再切换应用。历史 SQLite 文件保留为回滚备份，不删除。

## 回滚

当前生产使用蓝绿部署，候选实例验收后切流，旧实例优雅排空，详见 [部署手册](DEPLOYMENT.md#8-蓝绿无感升级)。数据库恢复与不可逆迁移仍需单独制定维护方案，不能把数据库回滚当成无中断操作。

若新版本启动失败：

1. 保留 MySQL 数据，不删除数据库、不清空表。
2. 使用上一个正常镜像恢复容器；需要回退 SQLite 时，使用迁移前的备份恢复 `accounts.db`。
3. 如果切换后 MySQL 已产生新账务，不能直接用旧 SQLite 覆盖回去，应先停止写入并评估补回新增流水，避免余额回退。
4. 检查 `/admin`、账户接口、历史流水及最小真实调用后恢复流量。

MySQL 日常备份应使用 `mysqldump --single-transaction --quick --routines --triggers kiro_rs` 或等价一致性快照，并与 `config.json`、`credentials.json` 一并保存。
