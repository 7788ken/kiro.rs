# kiro-rs 部署手册

本文档适用于本分支（用户、角色与积分计费版本）的 Docker 部署、升级与运维。默认示例使用 `6091` 端口；当前服务器通过 Caddy 对外提供 HTTPS，后端容器仅监听本机地址。

## 1. 部署架构

```text
客户端
  │ HTTPS :6091
  ▼
Caddy（TLS 终止、反向代理、蓝绿切流）
  │ HTTP 容器 IP:6091（本机 16091 / 16092 仅用于检查）
  ▼
kiro-rs 蓝 / 绿容器（共享配置和 MySQL）
  ├── config.json          服务与上游配置
  ├── credentials.json     Kiro 凭据
  └── MySQL                用户、角色、会话与积分账本
```

生产环境使用 MySQL 8.4 + InnoDB，账户数据不再写入 `accounts.db`。MySQL 连接地址通过服务器上的 `.env.mysql` 提供，密码不得提交到 Git。原 SQLite 文件只用于历史备份与一次性迁移。

后台页面的视觉、组件、交互与验收规则统一遵循 [后台 UI / UX 设计手册](ADMIN_UI_UX_GUIDE.md)。数据库配置、SQLite 迁移和回滚详见 [MySQL 迁移手册](MYSQL_MIGRATION.md)。

## 2. 环境要求

- Linux x86_64 或 arm64。
- Docker 与 Docker Compose v2。
- 可访问 GitHub、容器镜像仓库和 Kiro 上游服务。
- 使用公网服务时开放所选端口；不要把数据库端口暴露到公网。

检查环境：

```bash
docker version
docker compose version
ss -ltnp | grep -E ':(6091|16091)\b' || true
```

## 3. 获取代码与同步上游

个人二次开发仓库使用 `origin`，原始项目使用 `upstream`：

```bash
git clone https://github.com/7788ken/kiro.rs.git
cd kiro.rs
git remote add upstream https://github.com/hank9999/kiro.rs.git
```

如果已克隆，只需设置一次：

```bash
git remote set-url origin https://github.com/7788ken/kiro.rs.git
git remote add upstream https://github.com/hank9999/kiro.rs.git 2>/dev/null || \
  git remote set-url upstream https://github.com/hank9999/kiro.rs.git
```

同步上游并保留自己的修改：

```bash
git fetch upstream
git merge upstream/master
# 如有冲突，人工解决并运行测试后再提交
git push origin master
```

部署配置、真实密钥和账户数据库不要提交到 Git。部署专用 Compose 文件可保留在服务器，或使用仓库外路径。

## 4. 首次部署

### 4.1 创建目录与配置

```bash
sudo mkdir -p /opt/kiro-rs/config
sudo rsync -av --exclude=.git --exclude=config \
  ./ /opt/kiro-rs/
cd /opt/kiro-rs
```

创建 `config/config.json`：

```json
{
  "host": "0.0.0.0",
  "port": 6091,
  "apiKey": "请替换为随机生成的客户端主密钥",
  "adminApiKey": "请替换为随机生成的超管密钥",
  "region": "us-east-1",
  "kiroVersion": "0.11.133",
  "defaultEndpoint": "ide"
}
```

创建空的 `config/credentials.json`：

```json
[]
```

生成强随机密钥并限制文件权限：

```bash
openssl rand -base64 36
chmod 700 /opt/kiro-rs/config
chmod 600 /opt/kiro-rs/config/config.json \
  /opt/kiro-rs/config/credentials.json
```

说明：

- `apiKey`：原主 API 密钥，保留兼容，映射为超管身份；不要分发给普通用户。
- `adminApiKey`：原管理密钥，同时也是首次创建的 `superadmin` 账户初始密码。
- 普通用户必须在后台单独创建，使用各自的 API Key 和积分余额。
- 服务启动后应在「安全与设置」中修改超管初始密码。
- Kiro 的 `kiroVersion` 与 kiro-rs 程序版本不是一个概念，不要为了“升级”随意提高该标识。

### 4.2 创建部署 Compose 文件

在 `/opt/kiro-rs/docker-compose.deploy.yml` 写入：

```yaml
services:
  kiro-rs:
    pull_policy: never
    build:
      context: .
      dockerfile: Dockerfile
    image: kiro-rs:local
    ports: !override
      - "127.0.0.1:16091:6091"
```

该配置只把后端绑定到本机，供 Caddy 转发。若不使用反向代理，需要直接暴露服务时，可改为：

```yaml
    ports: !override
      - "6091:6091"
```

此时只能使用 `http://服务器IP:6091`，并应自行评估明文传输密钥的风险。若已有进程占用 `6091`，先检查 `ss -ltnp`，不要停止其他项目来腾端口。

### 4.3 构建与启动

```bash
cd /opt/kiro-rs
docker compose -f docker-compose.yml -f docker-compose.deploy.yml build
docker compose -f docker-compose.yml -f docker-compose.deploy.yml up -d --no-build
```

`docker-compose.yml` 提供容器名、配置卷和自动重启；部署文件覆盖镜像构建与端口。不要执行不带部署文件的 `docker compose up`，否则会使用默认 `8990` 配置或上游镜像。

查看状态和日志：

```bash
docker ps --filter name=^/kiro-rs$
docker logs --tail 100 kiro-rs
docker exec kiro-rs ./kiro-rs --version
curl -fsS http://127.0.0.1:16091/admin -o /dev/null
```

长时间构建建议在服务器后台执行，避免 SSH 断开导致构建取消：

```bash
systemd-run --unit=kiro-rs-build \
  --working-directory=/opt/kiro-rs \
  --property=Type=oneshot \
  /usr/bin/docker compose -f docker-compose.yml \
  -f docker-compose.deploy.yml build
journalctl -u kiro-rs-build -f
```

## 5. 配置 Caddy HTTPS

当前服务器使用 Caddy 的 IP 短期证书。`/etc/caddy/Caddyfile` 中可加入独立站点块：

```caddy
https://服务器IP:6091 {
    tls {
        issuer acme {
            profile shortlived
        }
    }
    encode zstd gzip
    reverse_proxy 127.0.0.1:16091 {
        flush_interval -1
        transport http {
            read_timeout 600s
            write_timeout 600s
            dial_timeout 10s
            keepalive 120s
        }
    }
}
```

有域名时，建议将 `https://服务器IP:6091` 改为实际域名，并使用常规 ACME 证书。证书签发依赖公网可达性和 CA 支持，不能保证所有网络环境都能获得 IP 证书。

应用配置并验证：

```bash
caddy validate --config /etc/caddy/Caddyfile
systemctl reload caddy
systemctl status caddy --no-pager
```

访问地址：

- 管理页面：`https://服务器IP:6091/admin`
- Anthropic API：`https://服务器IP:6091/v1`
- Claude Code 兼容接口：`https://服务器IP:6091/cc/v1`

IP 地址证书可能被浏览器提示需要确认。HTTPS 站点不能使用 `http://` 访问，否则会收到 `Client sent an HTTP request to an HTTPS server` 的 400 响应。

如果使用防火墙：

```bash
ufw allow 6091/tcp comment 'kiro-rs HTTPS'
ufw status
```

只开放 Caddy 对外端口即可，`16091` 不应开放到公网。

## 6. 初始化账户与凭据

1. 打开 `/admin`，用原 `adminApiKey` 登录，或使用 `superadmin` 和该密钥作为初始密码登录。
2. 在「用户与积分 → 安全与设置」修改超管密码。
3. 在「用户管理」创建管理员或普通用户，设置初始密码与初始积分。
4. 将创建成功时显示的 API Key 通过安全渠道交给用户；关闭后无法再次查看完整 Key。
5. 管理角色在「上游凭据」添加 Kiro 凭据。普通管理员可创建直属下级代理，并管理本人及后代代理登记的凭据；其他分支凭据只读。无登记人的历史凭据仅超管可管理。无有效上游凭据时，账户和计费功能仍可运行，但实际对话会返回上游错误。

积分调整：正数充值，负数扣减。管理员只能管理本人及后代代理的账户，不能修改计费单价或负载均衡。普通用户仅能查看自己的余额和流水。

## 7. 验收与日常检查

```bash
# 页面应返回 200
curl -kfsS https://服务器IP:6091/admin -o /dev/null

# 未带 Key 应返回 401
curl -ksS -o /dev/null -w '%{http_code}\n' \
  https://服务器IP:6091/v1/models

# 使用有效主 API Key 应返回 200
curl -kfsS https://服务器IP:6091/v1/models \
  -H 'x-api-key: 替换为主API密钥' -o /dev/null

# 后端健康检查
curl -fsS http://127.0.0.1:16091/admin -o /dev/null

# 检查容器、端口与近期错误
docker ps --filter name=^/kiro-rs$
docker logs --tail 100 kiro-rs
journalctl -u caddy --since '15 minutes ago' --no-pager
```

建议分别验证：

- 超管可管理用户、积分、单价和上游凭据。
- 管理员可创建下级代理、查看和搜索凭据、登记并管理本分支凭据；不能修改单价、负载均衡或其他管理员分支的凭据。
- 普通用户不能访问管理接口，流水仅包含本人。
- 余额不足返回 `402`；上游失败后预授权全额返还。
- 「积分流水 → 调用日志」每次调用一条：成功为实际消费，失败为 `0` 和具体错误；详情可核对首字、Token、预扣、返还及结算后余额。
- 密码登录不会轮换 API Key，退出后网页会话失效。
- 至少进行一次真实最小对话，确认上游凭据和协议转换正常。

## 8. 蓝绿无感升级

生产升级使用 `deploy/blue_green.py`，**不得再用单容器 `docker compose up -d` 直接替换活跃服务**。蓝绿流程会：

1. 保留旧实例与流量，构建并启动另一颜色的候选实例。
2. 检查 `/health/live`、`/health/ready`、管理页、MySQL 预授权状态，并用专用测试 Key 执行最小真实流式请求。
3. 通过 Caddy 管理 API 的 `ETag` / `If-Match` 原子替换 `@id kiro_rs_active` 的 upstream，避免并发覆盖。
4. 切流后再次验收。失败时立即恢复旧 upstream，旧实例未通过验收前不会停止。
5. 旧实例收到 `SIGTERM` 后停止接收新连接，等待在途响应体（包括 SSE）完成，再退出并清理。

### 8.1 Caddy 配置与持久化

- Caddy 管理 API 只监听 `127.0.0.1:2019`，不能暴露公网。
- 使用系统 `caddy-api.service`，以 `caddy run --resume` 恢复管理 API 的最新配置。普通 `caddy.service` 从 Caddyfile 启动，会丢失 API 切流状态，不能混用。
- 目标代理 handler 必须带 `@id kiro_rs_active`，脚本只修改这个 handler 的 upstream，不操作其他站点。
- 当前 Caddyfile 仍是人工审计和紧急恢复参考；管理 API 配置保存在 Caddy 的 autosave 文件中。修改 Caddyfile 后通过 API `/load` 加载，不要在生产直接重启代理。

### 8.2 服务器本地配置

复制 `deploy/deployment.example.env` 为 `/opt/kiro-rs/deployment.env`，填写服务器路径、镜像、网络、端口和专用测试 Key，权限设为 `600`。该文件与 `.env.mysql`、真实凭据和数据库备份均不提交 Git。

```bash
chmod 600 /opt/kiro-rs/deployment.env /opt/kiro-rs/.env.mysql
python3 /opt/kiro-rs/deploy/blue_green.py status --env /opt/kiro-rs/deployment.env
```

### 8.3 备份并升级

部署脚本会先在旧实例服务期间备份 `config.json`、`credentials.json` 和 MySQL 一致性快照，再构建候选实例。也可以先手动检查备份目录：

```bash
ls -ld config/backups/blue-green-*
```

```bash
cd /opt/kiro-rs
python3 deploy/blue_green.py deploy --env deployment.env
```

MySQL 备份使用 `--single-transaction`，不需要为了蓝绿升级停止应用。`deploy-state/active.json` 记录活跃实例、候选、镜像 ID 和阶段，权限为 `600`。`deploy.lock` 防止并发部署。

数据库变更必须遵守“扩展 → 新旧版本兼容 → 收缩”。破坏性迁移或不可逆迁移不得在蓝绿过程中执行；此类维护需要单独的数据迁移与回滚方案。

### 8.4 排空、回滚与故障恢复

- `status`：查看活跃实例、候选、退役实例及 Caddy upstream。
- `rollback`：仅在上一实例尚未排空、仍能通过就绪与真实调用检查时切回；随后优雅排空当前实例。不能恢复已停止或已删除的实例。
- `cleanup`：继续等待旧实例优雅退出，再清理已停止容器；不会强制结束活跃流。
- 候选启动或验收失败：不切流，保留旧实例。候选容器保留用于排查。
- 切流后验收失败：恢复旧 upstream，并重新验收旧实例。
- 旧实例排空超时：状态为 `awaiting_drain`，旧容器保留。不能为了“完成部署”使用 `docker kill`、`docker rm -f` 或强制清理。

```bash
python3 deploy/blue_green.py status --env deployment.env
python3 deploy/blue_green.py rollback --env deployment.env
python3 deploy/blue_green.py cleanup --env deployment.env
```

手动恢复时，先检查状态文件、容器和 Caddy upstream 是否一致，不要盲目重建或删除实例。Caddy 管理 API 支持：

```bash
curl http://127.0.0.1:2019/id/kiro_rs_active/upstreams
curl http://127.0.0.1:16091/health/ready
curl http://127.0.0.1:16092/health/ready
```

数据库恢复可能丢弃新写入，不等同于应用无感回滚。只有在确认需要回退数据时，才停止写入、保留故障副本并按 MySQL 迁移手册恢复。

## 10. 常见问题

### 管理页返回 400

检查是否用 `http://` 访问了 Caddy HTTPS 端口。使用 `https://服务器IP:6091/admin`，并区分后端 `16091` 和公网 `6091`。

### 页面返回 502

通常是后端未运行或端口映射错误：

```bash
docker ps -a --filter name=kiro-rs
docker logs --tail 100 kiro-rs
curl -v http://127.0.0.1:16091/admin
journalctl -u caddy --since '15 minutes ago'
```

### 端口已被占用

使用 `ss -ltnp | grep 6091` 查明监听进程。现有 Caddy 转发应保留，不要再让 Docker 直接绑定同一端口。不要为清端口而终止不明服务。

### 无有效上游凭据

账户管理仍可工作，对话会返回 `502` 或 `503`。到「上游凭据」添加有效 Kiro 凭据，并检查 `profileArn`、Region、Token 与代理设置。

### 用户积分没有变化

先检查请求是否到达计费入口、是否使用普通用户 Key、响应是否完成，以及是否因上游失败全额退款。通过「积分流水 → 调用日志」查看每次请求的消费或错误状态，点击详情核对预扣、返还和结算后余额；管理员身份不计用户积分。旧版记录未保存的错误和耗时无法补回。

### 数据库或密钥被误提交

立即停止分发相关提交，轮换所有可能泄露的密钥，并清理 Git 历史。`config/`、`accounts.db*`、`docker-compose.deploy.yml` 与备份文件不应推送到公开仓库。

## 11. 安全与运维清单

- 使用 HTTPS，限制后台和密钥的访问范围。
- 修改超管初始密码，普通用户使用独立 API Key。
- 不把真实密钥、Token、Profile ARN、数据库或备份提交到 Git。
- 配置目录、数据库与备份文件权限最小化。
- 升级前备份，升级后验证，保留旧镜像直到服务稳定。
- 定期执行 MySQL 一致性备份，并定期进行恢复演练。
- 将 `config/` 纳入备份计划，并定期进行恢复演练。
- 网页会话与 API Key 分离，泄露时立即重置对应用户 Key。
