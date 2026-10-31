# BRA Admin · FAQ and Operations

> Troubleshooting and operations for BRA. Installation lives in the [Installation Guide](install.html); daily usage in the [User Guide](usage.html).

## Contents

1. [5. Common questions and operations](#5-common-questions-and-operations)

## 5. Common questions and operations

### 5.1 Forgot the superadmin password

There is no email-based reset. Choose one:

1. **Reset in the database**: generate an argon2id hash for a new password and update `admin.password` for the superadmin. Prefer creating a second superadmin through the app when you still have access.
2. **Restore a backup** of the database.
3. **Recreate the database** as a last resort (this loses all data).

After resetting, log in and change the password in Profile. Resetting the password does **not** disable TOTP; clear the TOTP columns and backup-code rows if the authenticator is also lost.

### 5.2 Account locked

Default: 5 failed passwords for the same account or source IP lock it for 10 minutes. Wait, or have a superadmin disable/re-enable the account or reset its password. Wrong captcha answers do not count toward lockout.
A dynamic-code failure does count.

### 5.3 Where are language and theme changed?

Top bar: language (简体中文 / English) and light/dark theme. Both are stored in the **local browser**, not the account; set them again on another computer.
The initial language follows the browser, the theme follows the system.

### 5.4 Avatar upload fails

- Only PNG/JPEG, max 512 KB. The browser compresses larger images; if compression still exceeds 512 KB, pick a smaller file.
- Check `upload_dir` and its permissions; in Docker the upload directory is `/home/uploads` and is lost when the container is recreated.
- If you upgraded only the backend or only the frontend, upgrade both: the old base64 avatar endpoint was removed.

### 5.5 A published notice is not visible to some users

- Drafts are invisible; both the notice and attachments must be published.
- The unread bell lists at most the latest 50 unread notices.
- Check that the user is enabled and can log in; login logs show failures.
- Notice email push is opt-in per account (email must be present and decryption must work) and excludes the publisher.

### 5.6 A scheduled job did not run

- Check `[job] enabled` in `app.conf`; when off, jobs are not scheduled (manual **Trigger once** still works).
- Check the job interval and the last run/result columns. Jobs run serially; a long job delays others.
- In multi-instance deployments enable scheduling on only one instance.
- Look at run history for errors (database backup, export, notification retry, etc.).

### 5.7 Why re-login after an upgrade?

The frontend and backend must be the same version. The frontend uses the current API contract; an old token/frontend may be rejected after an upgrade. Log in again.
For upgrades, replace both the binary and the frontend dist; use `install.sh --keep-conf` for configuration-preserving upgrades.

### 5.8 What is a “security scan hit” in the audit log?

The built-in request scanner (`security-rust`) detects attack signatures (SQLi, command injection, SSRF, XXE, Log4Shell, path traversal, etc.). By default `scan = off` only logs a WARN and an audit record
(level and detector name, not the raw payload). `scan = high`/`critical` returns 403 with `err = security.blocked`. XSS is detected but scored Low and is not blocked by the high/critical threshold;
multipart uploads are not scanned. Referer/Origin are scanned only after `scheme://authority` to avoid false SSRF hits on internal IPs.

### 5.9 Operations: cleaning plaintext backups

If encrypted columns were ever backed up as plaintext (e.g., `admin_email_phone_backup`), restrict access to the database and backups, then delete or re-encrypt those copies according to your retention policy.
Never expose database dumps or the `admin` table to untrusted users. Back up `app.conf` securely: losing `encrypt_key` makes encrypted columns permanently unreadable.

### 5.10 Operations: database backup and restore

- The built-in `db_backup` job runs `mysqldump` to disk (files mode 0600, configurable directory and retention, default 7 copies); run history is in `job_log`. Compose images include `mariadb-client`; bare-metal installs need their own MySQL client. Stale `backup_*.sql.part` files older than 24 hours are reclaimed before the next backup.
- Restore is manual: stop the service, import the dump (`mysql < dump`), then start the service. Test restores regularly.
- Back up `admin/conf/app.conf` separately; `encrypt_key` and `hashids_salt` cannot be regenerated without data loss.
- The `notify_retry` ledger and other retention jobs are configured in System Config / jobs.

### 5.11 Notifications are not sent / test email fails

- **Webhook**: check the URL starts with `http://`/`https://`; link-local/unspecified IPs are rejected at save and send time; redirects are not followed; the endpoint must return 2xx within 5 seconds.
  Signatures use `X-BRA-Timestamp` and `X-BRA-Signature: sha256=…` over `"<ts>.<raw body>"`. Failed deliveries are retried by `notify_retry` with backoff (max 3 retries); check `notify_delivery`.
- **SMTP**: check server, port and encryption (STARTTLS 587, SSL/TLS 465), username/password and sender. Test email goes to your own account email using saved settings.
  A blank password means “unchanged”; use **Clear password** to remove it. Check the SMTP provider’s quota/rate limits; notice email push is one SMTP session per recipient and is best-effort (no retry).
- Permissions: `system:notify:list` to open the page; `system:notify:edit` to save/send.
