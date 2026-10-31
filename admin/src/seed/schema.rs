// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 建表 / 补列 / 约束 / 存量加密迁移。

use super::*;

/// 建表 + 补列（只在启动时跑，幂等）。
///
/// 从本地自写的 `syncdb` 换到上游的 `migrate` 之后，**约束不再由模型属性声明**：
/// 上游的字段属性只有 `pk / auto / ignore / auto_now_add / auto_now / soft_delete /
/// sql_type / fk` —— 没有 unique、没有 index、也没有列长度。所以：
///   - 列长度改用 `#[bee(sql_type = "VARCHAR(n)")]`（`SqlType::Raw` 原样渲染）
///   - **唯一键与索引改由这里建**（下方 `CONSTRAINTS`），与既有的连接表、复合唯一键同一套做法
/// 命名沿用迁移前 syncdb 的约定（`uk_{表}_{列}` / `idx_{表}_{列}`）——名字变了会在已有库上
/// 重复建一份同名不同名的索引。
pub async fn migrate(db: &Pool) -> Result<(), OrmError> {
    // 1) 复合主键/复合唯一键的表：上游 migrate 只建单列主键，表达不了，走裸 DDL。
    //    必须建在 sync 之前：否则 sync 先建出一张没有复合约束的表，IF NOT EXISTS 就再补不上。
    //    `id` 一律不带 AUTO_INCREMENT：主键由代码发号（雪花），列定义要与 migrate 生成的
    //    其它表一致 —— 留着自增会让人以为这里还能靠数据库发号（旧的库上该列仍是自增的，
    //    但应用侧总是显式给值，无害）。
    for ddl in [
        "CREATE TABLE IF NOT EXISTS dict_item (
           id BIGINT NOT NULL,
           type_code VARCHAR(64) NOT NULL DEFAULT '',
           label VARCHAR(64) NOT NULL DEFAULT '',
           value VARCHAR(64) NOT NULL DEFAULT '',
           sort INT NOT NULL DEFAULT 0,
           status TINYINT NOT NULL DEFAULT 0,
           remark VARCHAR(255) NOT NULL DEFAULT '',
           created_at DATETIME NOT NULL,
           updated_at DATETIME NOT NULL,
           PRIMARY KEY (id),
           UNIQUE KEY uk_type_value (type_code, value)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
        "CREATE TABLE IF NOT EXISTS job_log (
           id BIGINT NOT NULL,
           job_code VARCHAR(64) NOT NULL DEFAULT '',
           started_at DATETIME NOT NULL,
           duration_ms BIGINT NOT NULL DEFAULT 0,
           status TINYINT NOT NULL DEFAULT 0,
           msg VARCHAR(255) NOT NULL DEFAULT '',
           PRIMARY KEY (id),
           KEY idx_code_time (job_code, started_at)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
        "CREATE TABLE IF NOT EXISTS admin_role (
           admin_id BIGINT NOT NULL,
           role_id BIGINT NOT NULL,
           PRIMARY KEY (admin_id, role_id)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
        "CREATE TABLE IF NOT EXISTS role_menu (
           role_id BIGINT NOT NULL,
           menu_id BIGINT NOT NULL,
           PRIMARY KEY (role_id, menu_id)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
        "CREATE TABLE IF NOT EXISTS role_dept (
           role_id BIGINT NOT NULL,
           dept_id BIGINT NOT NULL,
           PRIMARY KEY (role_id, dept_id)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
        // 公告已读：同一公告同一人只有一行（重复标记靠 INSERT IGNORE 幂等）
        "CREATE TABLE IF NOT EXISTS notice_read (
           notice_id BIGINT NOT NULL,
           admin_id BIGINT NOT NULL,
           read_at DATETIME NOT NULL,
           PRIMARY KEY (notice_id, admin_id)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
        // email / phone 存量加密前的**明文备份**（`encrypt_legacy` 写）。密钥丢了
        // 密文就解不开了，这是唯一的退路；只进不改（INSERT IGNORE，首份为准）。
        "CREATE TABLE IF NOT EXISTS admin_email_phone_backup (
           id BIGINT NOT NULL,
           email VARCHAR(255) NOT NULL DEFAULT '',
           phone VARCHAR(64) NOT NULL DEFAULT '',
           backed_up_at DATETIME NOT NULL,
           PRIMARY KEY (id)
         ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
    ] {
        db.execute(ddl, &[]).await?;
    }

    // 2) 各模型：建表 + 补缺列，绝不删列、不改类型（与迁移前的 Safe 模式一致）
    migrate::sync::<Admin, _>(db).await?;
    migrate::sync::<crate::models::Dept, _>(db).await?;
    migrate::sync::<crate::models::Role, _>(db).await?;
    migrate::sync::<Menu, _>(db).await?;
    migrate::sync::<crate::models::LoginLog, _>(db).await?;
    migrate::sync::<crate::models::AuditLog, _>(db).await?;
    migrate::sync::<crate::models::DictType, _>(db).await?;
    migrate::sync::<crate::models::DictItem, _>(db).await?;
    migrate::sync::<Job, _>(db).await?;
    migrate::sync::<crate::models::JobLog, _>(db).await?;
    migrate::sync::<crate::models::Notice, _>(db).await?;
    migrate::sync::<crate::models::SysConfig, _>(db).await?;
    migrate::sync::<crate::models::ExportTask, _>(db).await?;
    migrate::sync::<crate::models::AdminLoginDevice, _>(db).await?;
    migrate::sync::<crate::models::Attachment, _>(db).await?;
    // F4：TOTP 备用码 + Webhook 失败账本
    migrate::sync::<crate::models::AdminBackupCode, _>(db).await?;
    migrate::sync::<crate::models::NotifyDelivery, _>(db).await?;

    // 3) 单列唯一键与索引：原先写在模型属性上（`#[bee(unique)]` / `#[bee(index)]`），
    //    上游属性里没有这两个概念，移到这儿。
    //    job_log 建表 DDL 已有 (job_code, started_at)，单列 idx_job_log_job_code 只是最左
    //    前缀的重复；从 CONSTRAINTS 移除后，还要把已有库上的那份删掉（幂等）。
    drop_index_if_exists(db, "job_log", "idx_job_log_job_code").await?;
    for (table, index, ddl) in CONSTRAINTS {
        ensure_index(db, table, index, ddl).await?;
    }

    // 4) 新增列的存量行补值：`sync` 只加列、不给默认值（新列在旧行上是 NULL），
    //    而模型里 totp_secret / totp_enabled 都不是 Option —— 解 NULL 会直接报错。
    //    必须先补值再改列宽：MySQL 严格模式下 `MODIFY … NOT NULL` 遇到 NULL 会拒绝。
    db.execute("UPDATE admin SET totp_secret = '' WHERE totp_secret IS NULL", &[]).await?;
    db.execute("UPDATE admin SET totp_enabled = 0 WHERE totp_enabled IS NULL", &[]).await?;

    // 5) 列宽：email / phone / totp_secret 落库存密文（比明文长 ≈1/3 再加 30 字节），
    //    旧库的 128 / 20 放不下。上游 migrate 只加列不改类型，只能裸 DDL 改。
    ensure_encrypted_width(db).await?;
    Ok(())
}

/// 密文列（email / phone / totp_secret）的目标列宽 = 模型上的 `sql_type`。
/// 密文长度 = 4 × ceil((明文字节数 + 30) / 3)：128 字节明文 → 212 字符，
/// 20 字节 → 68 字符，255 都有余量（实测值见 crypto.rs 的 COL_MAX）。
const ENCRYPTED_COLUMNS: &[(&str, &str, &str)] = &[
    ("admin", "email", "VARCHAR(255)"),
    ("admin", "phone", "VARCHAR(255)"),
    // 动态验证码密钥：base32 明文 32 字符（项 7），落库同样是密文
    ("admin", "totp_secret", "VARCHAR(255)"),
];

/// 幂等改列宽：已是目标类型就跳过（每次启动都跑，不能每次 ALTER）。
async fn ensure_encrypted_width(db: &Pool) -> Result<(), OrmError> {
    for (table, column, want) in ENCRYPTED_COLUMNS {
        // **必须显式起别名**：information_schema 里的列名本身是全大写（COLUMN_TYPE），
        // 结果集的行键跟着走，`r.get("column_type")` 会取到 None —— 那样这段会静默
        // 跳过 ALTER，旧库启动时直接「Data too long for column 'phone'」（踩过）。
        // 上游 migrate.rs 也是这个写法（`AS name`）。
        let rows = db
            .query(
                "SELECT column_type AS ty FROM information_schema.columns \
                 WHERE table_schema = DATABASE() AND table_name = ? AND column_name = ?",
                &[Value::from(*table), Value::from(*column)],
            )
            .await?;
        let Some(current) = rows.first().and_then(|r| r.get("ty")).and_then(|v| v.as_str()) else {
            // 走到这儿说明列不存在：sync 已经建过表，不该发生 —— 响一声，别静默跳过
            tracing::warn!("查不到 {table}.{column} 的列类型，跳过列宽检查");
            continue;
        };
        if current.eq_ignore_ascii_case(want) {
            continue;
        }
        // MODIFY COLUMN 要写全定义，否则 NOT NULL / DEFAULT 会被一起抹掉
        let ddl = format!("ALTER TABLE {table} MODIFY COLUMN {column} {want} NOT NULL DEFAULT ''");
        tracing::info!("列宽迁移：{table}.{column} 由 {current} 改为 {want}（要放密文）");
        db.execute(&ddl, &[]).await?;
    }
    Ok(())
}

/// 一次性存量迁移（幂等、可重跑）：把 `admin.email` / `admin.phone` 里还没加密的
/// 明文加密回写。每行搬迁前先把**原值**写进 `admin_email_phone_backup`（密钥丢了
/// 就靠它）。已加密的跳过 —— 重跑不会双重加密。
///
/// 放在**启动流程**而不是注册成定时任务，两个理由：
/// - `[job] enabled=false` 时调度循环根本不启动；安全迁移不该被「临时关掉定时」
///   这个开关顺带关掉，也不该等一个间隔周期才跑；
/// - 数据正确性不依赖它（读路径兼容明文），但越早跑完越少明文滞留 —— 启动时扫
///   一遍最省心，且幂等，多起几次没有副作用。
///
/// 每批 500 行、按 id keyset 前进（id 是雪花，正数且单调），内存只与一批成正比。
pub async fn encrypt_legacy(db: &Pool, guard: &encryptable::guard::Guard) -> Result<(), OrmError> {
    const BATCH: i64 = 500;
    let (mut scanned, mut encrypted, mut skipped, mut failed) = (0u64, 0u64, 0u64, 0u64);
    let mut last = 0i64;
    loop {
        let rows = db
            .query(
                &format!(
                    "SELECT id, email, phone FROM admin WHERE id > ? ORDER BY id ASC LIMIT {BATCH}"
                ),
                &[Value::from(last)],
            )
            .await?;
        if rows.is_empty() {
            break;
        }
        for row in &rows {
            scanned += 1;
            let Some(id) = row.get("id").and_then(|v| v.as_i64()) else { continue };
            last = id;
            let email = row.get("email").and_then(|v| v.as_str()).unwrap_or("");
            let phone = row.get("phone").and_then(|v| v.as_str()).unwrap_or("");
            if guard.is_encrypted(email) && guard.is_encrypted(phone) {
                skipped += 1;
                continue;
            }

            // 先备份原值再动它：写入失败也只是多一条备份（INSERT IGNORE 不覆盖首份）
            db.execute(
                "INSERT IGNORE INTO admin_email_phone_backup (id, email, phone, backed_up_at) \
                 VALUES (?, ?, ?, ?)",
                &[Value::from(id), Value::from(email), Value::from(phone), Value::from(now())],
            )
            .await?;

            // 逐个字段判定：部分加密的行只补没加密的那列
            let sealed = |value: &str| -> Result<String, String> {
                if guard.is_encrypted(value) {
                    Ok(value.to_owned())
                } else {
                    crypto::write(guard, value)
                }
            };
            let (new_email, new_phone) = match (sealed(email), sealed(phone)) {
                (Ok(e), Ok(p)) => (e, p),
                (e, p) => {
                    // 单行搬不动（比如明文是多字节字符、密文撑爆列宽）不能让整个进程
                    // 起不来：留在明文，日志点名，下次启动再试。
                    failed += 1;
                    tracing::warn!(
                        "管理员 {id} 的 email/phone 加密失败，暂留明文：{}",
                        e.err().or(p.err()).unwrap_or_default()
                    );
                    continue;
                }
            };
            db.execute(
                "UPDATE admin SET email = ?, phone = ? WHERE id = ?",
                &[
                    Value::from(new_email.as_str()),
                    Value::from(new_phone.as_str()),
                    Value::from(id),
                ],
            )
            .await?;
            encrypted += 1;
        }
    }
    if encrypted > 0 || failed > 0 {
        tracing::info!(
            "存量加密迁移：扫描 {scanned} 行，加密 {encrypted} 行，跳过 {skipped} 行（已是密文），失败 {failed} 行；原值备份在 admin_email_phone_backup"
        );
    } else {
        tracing::debug!("存量加密迁移：无明文（扫描 {scanned} 行）");
    }

    // 备份还在就提醒运维「可以清理了」：它是密钥丢失时的唯一退路，但也是明文 PII，
    // 无限期留着本身就是风险。数行失败只记 debug —— 这条迁移是「尽力而为」，
    // 不能因为一次查询把启动搞挂。
    match db.query("SELECT COUNT(*) AS n FROM admin_email_phone_backup", &[]).await {
        Ok(rows) => {
            let n = rows.first().and_then(|r| r.get("n")).and_then(|v| v.as_i64()).unwrap_or(0);
            if n > 0 {
                tracing::warn!(
                    "admin_email_phone_backup 仍有 {n} 行明文备份：确认新密钥下 email/phone 解密正常（登录后看个人资料的邮箱/手机号原文）后，可按 docs/faq.md 的「运维 · 清理明文备份」清理"
                );
            }
        }
        Err(e) => tracing::debug!("admin_email_phone_backup 行数统计失败（不影响启动）：{e}"),
    }
    Ok(())
}

/// 从模型属性搬过来的约束。名字必须与迁移前 syncdb 生成的一致。
const CONSTRAINTS: &[(&str, &str, &str)] = &[
    ("admin", "uk_admin_username", "CREATE UNIQUE INDEX uk_admin_username ON admin (username)"),
    ("admin", "idx_admin_dept_id", "CREATE INDEX idx_admin_dept_id ON admin (dept_id)"),
    ("admin", "idx_admin_status", "CREATE INDEX idx_admin_status ON admin (status)"),
    ("role", "uk_role_code", "CREATE UNIQUE INDEX uk_role_code ON role (code)"),
    ("dict_type", "uk_dict_type_code", "CREATE UNIQUE INDEX uk_dict_type_code ON dict_type (code)"),
    (
        "dict_item",
        "idx_dict_item_type_code",
        "CREATE INDEX idx_dict_item_type_code ON dict_item (type_code)",
    ),
    ("job", "uk_job_code", "CREATE UNIQUE INDEX uk_job_code ON job (code)"),
    // 保留清理按 started_at 扫；建表 DDL 的 (job_code, started_at) 不能单独服务 started_at
    (
        "job_log",
        "idx_job_log_started_at",
        "CREATE INDEX idx_job_log_started_at ON job_log (started_at)",
    ),
    // 仪表盘的近 7 天按天窗口（项 2）每次进首页都查一遍：没有它是一次全表扫。
    // 登录记录列表的时间段筛选同样沾光。
    (
        "login_log",
        "idx_login_log_created_at",
        "CREATE INDEX idx_login_log_created_at ON login_log (created_at)",
    ),
    // 数据权限按 admin_id 收窄后再按时间区间筛；清空日志也按同一组条件 keyset 翻页
    (
        "login_log",
        "idx_login_log_admin_created_at",
        "CREATE INDEX idx_login_log_admin_created_at ON login_log (admin_id, created_at)",
    ),
    // audit_log 原先没有索引：数据权限 + 时间段筛必须走 (admin_id, created_at)，
    // 只按时间段（全量筛选 / 保留清理）另配单列 created_at
    (
        "audit_log",
        "idx_audit_log_admin_created_at",
        "CREATE INDEX idx_audit_log_admin_created_at ON audit_log (admin_id, created_at)",
    ),
    (
        "audit_log",
        "idx_audit_log_created_at",
        "CREATE INDEX idx_audit_log_created_at ON audit_log (created_at)",
    ),
    // 登录设备（项 7）只按账号查（列列表 / 每次登录找同 UA 的行），表很小但会很频繁
    (
        "admin_login_device",
        "idx_admin_login_device_admin_id",
        "CREATE INDEX idx_admin_login_device_admin_id ON admin_login_device (admin_id)",
    ),
    (
        "sys_config",
        "uk_sys_config_key",
        "CREATE UNIQUE INDEX uk_sys_config_key ON sys_config (config_key)",
    ),
    // 附件（项 5）：列表按上传人收窄、多态关联按 (entity_type, entity_id) 取「某个对象的
    // 配图」，各配一条。`stored_path` 那条**已无查询用它**（S2 后删除只删行、不再数引用），
    // 留着不删：索引删了要重建，但不该再按旧注释当成热路径
    (
        "attachment",
        "idx_attachment_admin_id",
        "CREATE INDEX idx_attachment_admin_id ON attachment (admin_id)",
    ),
    (
        "attachment",
        "idx_attachment_stored_path",
        "CREATE INDEX idx_attachment_stored_path ON attachment (stored_path)",
    ),
    (
        "attachment",
        "idx_attachment_entity",
        "CREATE INDEX idx_attachment_entity ON attachment (entity_type, entity_id)",
    ),
    // 导出中心（批次 E T4）：`export_run` 每分钟按 status 扫待处理/卡住的行，
    // `export_clean` 每天按 expire_at 扫过期的行 —— 两条各有索引，量大了才不会是全表扫
    (
        "export_task",
        "idx_export_task_status",
        "CREATE INDEX idx_export_task_status ON export_task (status)",
    ),
    (
        "export_task",
        "idx_export_task_expire_at",
        "CREATE INDEX idx_export_task_expire_at ON export_task (expire_at)",
    ),
    // F4-1：备用码登录按 (admin_id, used_at IS NULL) 取未用行；表很小，但登录是热路径
    (
        "admin_backup_code",
        "idx_admin_backup_code_admin",
        "CREATE INDEX idx_admin_backup_code_admin ON admin_backup_code (admin_id)",
    ),
    // F4-3：重投任务按 (status, next_at) 找 due 行；30 天清理按 created_at 扫
    (
        "notify_delivery",
        "idx_notify_delivery_due",
        "CREATE INDEX idx_notify_delivery_due ON notify_delivery (status, next_at)",
    ),
    (
        "notify_delivery",
        "idx_notify_delivery_created_at",
        "CREATE INDEX idx_notify_delivery_created_at ON notify_delivery (created_at)",
    ),
];

/// 幂等删索引：MySQL 没有 `DROP INDEX IF EXISTS`，先查 information_schema，存在才删。
async fn drop_index_if_exists(db: &Pool, table: &str, index: &str) -> Result<(), OrmError> {
    let rows = db
        .query(
            "SELECT COUNT(*) AS n FROM information_schema.statistics \
             WHERE table_schema = DATABASE() AND table_name = ? AND index_name = ?",
            &[Value::from(table), Value::from(index)],
        )
        .await?;
    let n = rows.first().and_then(|r| r.get("n")).and_then(|v| v.as_i64()).unwrap_or(0);
    if n > 0 {
        db.execute(&format!("DROP INDEX {index} ON {table}"), &[]).await?;
    }
    Ok(())
}

/// 幂等建索引：MySQL 没有 `CREATE INDEX IF NOT EXISTS`，先查 information_schema。
async fn ensure_index(db: &Pool, table: &str, index: &str, ddl: &str) -> Result<(), OrmError> {
    let rows = db
        .query(
            "SELECT COUNT(*) AS n FROM information_schema.statistics \
             WHERE table_schema = DATABASE() AND table_name = ? AND index_name = ?",
            &[Value::from(table), Value::from(index)],
        )
        .await?;
    let n = rows.first().and_then(|r| r.get("n")).and_then(|v| v.as_i64()).unwrap_or(0);
    if n == 0 {
        db.execute(ddl, &[]).await?;
    }
    Ok(())
}
