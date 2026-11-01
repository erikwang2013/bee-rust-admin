# BRA · Bee Rust Admin

[简体中文](README.md) · English

An RBAC admin panel built on the bee-rust framework — Rust backend + React frontend, ready to run once the database exists.

> Project site: [https://package.erik.xyz/bra/](https://package.erik.xyz/bra/)

`JWT login` · `Admins / Roles / Menus / Departments / Dicts / Scheduled jobs / Notices / Login logs / Audit logs` · `Menu + button permissions` · `Department data scopes` · `Bilingual (Chinese/English)` · `One-command Docker deploy`

---

## Project pet

![Keeper · the BRA project pet: an on-duty bee standing on the hexagon hive-gate platform, three keys hooked on its claws](docs/pet/keeper-onduty.svg)

**Keeper (阿守)** — the hive's on-duty bee.

The framework pet [Rusty](docs/rusty.svg) is the worker bee that builds the hive; Keeper is the one standing at the gate: **checking credentials, handing out keys, logging visitors**.
It is this admin panel's personality in bee form — permissions are not a wall in your way, they are someone carefully watching the door.

| Design detail | Feature / lore |
|---|---|
| **Hexagon badge** on the chest (check / gold star / exclamation) | Hive cell + verification result: normal / superadmin / alert |
| Carrying **a key** | Granting and revoking permissions (mapped to the three join tables `admin_role` / `role_menu` / `role_dept`) |
| **Wings** folded behind normally | JWT is silent most of the time; they spread only for the superadmin (bypass permission codes) |
| **Stinger** folded into a small dot normally | Raised only when a key file is detected in the repository |
| Teal accents only on wings and badge | The same palette as the admin UI, as an accent, never the main event |

**Five states** (the same mechanism as the framework pet, switched automatically by project state; artwork rendered by `docs/pet/render.mjs`):

![Keeper's five states: on duty / checking / superadmin / dozing / stinger up](docs/pet/keeper-states.png)

| State | Face | Meaning | Trigger |
|:--:|:--:|---|---|
| On duty | `(⌐■_■)` | Gate normal, permission table and menu tree consistent | All tests green · no uncommitted changes |
| Checking | `(・_・)?` | Changes pending verification — credentials, please | Uncommitted changes awaiting review |
| Superadmin | `(＾▽＾)` | Superadmin on site, bypasses all permission codes | On main · just tagged |
| Dozing | `(-_-)zZ` | Nobody visiting at this hour, login log empty | Late night · no visitors |
| Stinger up | `(#°益°)` | Stinger raised — someone committed a secret file! | Key file detected in the repository |

## Project pet expression forms · Keeper

![Keeper's 12 expression forms: smile / joy / cry / depressed / terrified / angry / coquettish / cute / pleasing / knocking / facepalm / tongue](docs/pet/keeper-expressions.png)

| Name | Shortcode | Name | Shortcode |
|:--:|---|---|:--:|
| Smile | `:emoji-smile:` | Joy | `:emoji-joy:` |
| Cry | `:emoji-cry:` | Depressed | `:emoji-depressed:` |
| Terrified | `:emoji-terrified:` | Angry | `:emoji-angry:` |
| Coquettish | `:emoji-coquettish:` | Cute | `:emoji-cute:` |
| Pleasing | `:emoji-pleasing:` | Knocking | `:emoji-knocking:` |
| Facepalm | `:emoji-facepalm:` | Tongue | `:emoji-tongue:` |

> 12 colored animated forms: smile / joy / cry / depressed / terrified / angry / coquettish / cute / pleasing / knocking / facepalm / tongue. Each SVG ships with its own expression, emotion effects and overall animation, and respects `prefers-reduced-motion`.

## Visual Assets License

The project pet, images, icons and emoji in this project (including the SVG / PNG files under `docs/pet-expressions/` and similar directories) are **not open-source assets**; copyright belongs to the author. Copying, modifying, redistributing or using them commercially without written permission is not allowed; commercial use requires a commercial license — contact **erik@erik.xyz**. See [ASSETS-LICENSE.md](ASSETS-LICENSE.md).

> The code is still released under the repository's `LICENSE`; third-party assets (e.g. the Go Gopher where explicitly noted) remain under their own licenses.

## Overview

**BRA (Bee Rust Admin)** is an out-of-the-box admin panel: the backend is written with this repository's
[bee-rust](https://github.com/erikwang2013/bee-rust) framework (axum routing + custom ORM execution layer +
INI configuration), and the frontend is a Vite 8 + React 19 + Ant Design 6 single-page app. The two talk through one unified
`{code, msg, data}` API contract (error responses additionally carry a stable business error code `err` and arguments `args`, from which the frontend renders localized text).

It solves a practical problem: **permissions must reach down to buttons, data must be visible only within scope, and logins must leave a trace**. Hence:

- **The permission model has two levels, menu and button**: roles are granted down to button-level permission codes (e.g. `system:admin:resetPwd`),
  the frontend renders menus and buttons from that, and the backend **re-checks on every endpoint** — hiding in the frontend is UX, not a line of defense.
- **There are five data scopes**: All / This department and below / This department only / Self only / Custom (pick departments),
  injected as conditions at the list-query layer with parameter binding, never by concatenating user input.
- **Sessions are stateless but support force logout**: the JWT carries `token_version`; changing the password, disabling the account or logging out invalidates old tokens.
- **Every login success / failure / logout is recorded**: IP, User-Agent and failure reason are stored and searchable.

On startup the server creates the tables and seeds the superadmin plus the menu permission tree — **no manual SQL migrations**.

## Architecture

![Architecture](docs/diagrams/architecture.svg)

Five layers, top to bottom:

| Layer | Components | Notes |
|---|---|---|
| Client | Browser SPA | React 19 + TypeScript + Ant Design 6 (built with Vite 8) |
| Access | nginx :8081 | Serves the frontend assets, reverse-proxies `/api/` to the backend and passes `X-Real-IP` through |
| Application | `bee_admin` | bee_router (axum) · Auth extractor · 97 API handlers (v1 + v2) · datascope data permissions · unified response envelope |
| Framework | bee_orm / bee_config / bee_logs | Multi-backend connection pool, QuerySet, CRUD, M2M, migrate; INI configuration; tracing logs |
| Data | MySQL 8.4 | 22 tables (17 business tables + 4 relation tables + 1 plaintext backup table) |

## Feature design

![Feature design](docs/diagrams/features.svg)

**The permission model is a three-part many-to-many chain**: admin ←`admin_role`→ role ←`role_menu`→ menus and buttons;
role ←`role_dept`→ departments (used by the "custom" data scope). The superadmin (`is_super`) bypasses all permission-code checks.

**The five data scopes** are injected as query conditions in list endpoints: department sets from multiple roles are unioned, any role set to "All" adds no condition,
and when there is no enabled role at all it falls back to `WHERE 1 = 0` (show nothing rather than everything).

## Request cycle

![Request cycle](docs/diagrams/request-cycle.svg)

The 12-step path for `GET /api/v1/admins`: browser → nginx → bee_router → Auth extractor
(verify signature / `ver` / account status, load roles and permission codes) → handler (`require(permission code)` first, then resolve the data scope)
→ bee_orm (prepared statements + parameter binding) → MySQL → unified envelope. **401 (not logged in) and 403 (no permission) short-circuit before the database is touched.**

## Service lifecycle

![Service lifecycle](docs/diagrams/lifecycle.svg)

Startup path: read config (JWT secret shorter than 32 chars or `changeme` → **refuse to start**) → connect MySQL →
`migrate::sync` synchronizes the schema (**only creates tables / adds columns, never drops columns or changes types**; a new field on a model adds the column on the next start;
unique keys and indexes are created by the raw DDL in `seed/schema.rs` — the upstream model attributes express only columns, primary keys and foreign keys, with no unique/index) →
on first start (`admin` table empty) write the superadmin, and **idempotently top up** the built-in menus and jobs on every start (filling gaps by permission code;
**not transactional** — a mid-way failure resumes on the next start and never leaves a half set) → listen on `127.0.0.1:8080` → run →
exit on `SIGTERM` (systemd policy `on-failure` restarts automatically).

## Features

| Module | Capabilities |
|---|---|
| **Login & auth** | Username/password login (argon2id; username 3–64 characters, password 8–1024 characters, oversized input rejected before rate limiting / DB lookup / hashing), JWT issuing and validation, logout, password change in the profile (forces re-login afterwards) |
| **Two-factor codes & login devices** | Optional second step (TOTP, RFC 6238, 6 digits / 30 seconds / one step of tolerance either way): binding requires confirmation with a current code, the secret is stored AES-256-GCM encrypted (no plaintext in exports or logs), unbinding clears the secret and the backup codes together; at login the code prompt appears **only after the password is verified** (showing it earlier would leak "this account has 2FA enabled"). **Backup codes**: 10 codes generated in the profile from the current TOTP (10 characters, confusable characters removed), plaintext returned once only, only SHA-256(pepper‖code) stored, each usable once, regenerating invalidates all older codes; at login a 6-digit numeric input is treated as a TOTP code, anything else as a backup code, and misses count toward the failure count too. The profile shows the **login device** list (grouped by UA, the current one marked) and can **kick a single device**; **generating backup codes / unbinding** requires entering the current code on the spot, and 5 consecutive wrong entries cause 429 for 5 minutes (in-process per-admin rate limit, not shared across instances). If both the authenticator and the backup codes are lost, an operator with database access clears the totp columns (`UPDATE admin SET totp_secret='', totp_enabled=0, totp_last_step=NULL WHERE id=<id>;` and deletes the backup codes `DELETE FROM admin_backup_code WHERE admin_id=<id>;`), then logs in again and re-binds, or restores a backup (resetting the password does not remove the binding) |
| **Notification channels** | Two outbound channels: **Webhook** (events `notice.published` on notice publish / `account.locked` on account lockout → POST JSON `{event,at,data}`; optional **HMAC-SHA256 signature**, headers `X-BRA-Timestamp` + `X-BRA-Signature: sha256=…`; link-local / unspecified-IP literals are rejected on save, and **domains are resolved and blocked again at send time**, loopback `127.0.0.1` allowed; **redirects are not followed** (3xx counts as a delivery failure, so 302/307 cannot bypass the block); failures go to the `notify_delivery` ledger, and `notify_retry` retries every 5 minutes with 1/5/15-minute backoff, at most 3 times, ledger kept 30 days) and **SMTP** (the settings page can send one test email to **your own** mailbox, reporting the reason on failure; **notice email push** can also be enabled: publishing a notice sends one email to every enabled account with an email address, **except the publisher**, off by default). Delivery is asynchronous (business responses don't wait for it); configuration lives under the `notify.*` keys of system config (never public, no restart needed after changes); the SMTP password and webhook signing secret are stored encrypted and the API only answers "whether it is set"; the whole `notify.*` namespace is owned by the notification settings page — the generic config page does not list, add, change or delete it |
| **Dashboard** | Four counts on the home page (admins / roles / departments / notices) plus 7-day login and failed-login trends (hand-drawn SVG mini bar chart, no chart library), available to any logged-in user without a permission code; days without data are filled with 0, all-zero shows an empty state |
| **Admin management** | Paged list (filter by username / status / department), create/edit/delete, assign roles, reset password, enable/disable (disabling kicks the user offline); edits are **full replacements**, missing fields return 400, so roles/departments cannot be cleared by accident |
| **Role management** | CRUD, menu permission tree checkboxes, five data scopes, custom departments, deletion blocked while referenced by admins |
| **Menu management** | Three levels — directory / menu / button — permission codes, icons and ordering, cycle prevention, deletion blocked while child nodes or roles reference it |
| **Department management** | Department tree, leader and ordering, deletion blocked while child departments or active admins exist |
| **Login logs** | Paged query by username / status / time range, clear by condition, records constrained by data scope |
| **Audit logs** | Operations with side effects (POST / PUT / DELETE) are recorded automatically (time / operator / module / action / request / IP), filtered by username / module / time range, exportable and clearable by condition (the clear itself is recorded too), records constrained by data scope. Editing admins / roles / departments / dicts also records a **change summary** (which fields, old→new); admin email / phone are encrypted columns, so the summary only says "changed" without the value |
| **Dictionary management** | Two-level maintenance of dict types + dict items, values unique within a type, deleting a type cascades to its items, CSV export; `GET /dicts/{code}/items` is a dropdown data source available to any logged-in user (only enabled items) |
| **Data import** | Three pages — admins / departments / dict items: download template (the header row is the contract) → upload CSV → per-row report (row number + reason). **Single transaction, all or nothing** (one non-conforming row means nothing is written), every row passes the initiator's own permissions and data scope; UTF-8 only; 5000 rows per import for departments / dict items, 300 for admins (argon2 per row) |
| **Export center** | Asynchronous export: start (audit logs / login logs, sharing **the same rendering** as synchronous export and producing byte-identical output) → a scheduled job runs it (in-process concurrency limit 2, no pile-up across triggers) → list auto-polls (with a progress column) → download / cancel / delete. Two permission gates (`system:export:list` + the view code for that type), **personal workbench** (you only see your own jobs); artifacts are named by generation, expired ones are cleaned up together with the record, and manually deleting a job removes all its generation files |
| **Scheduled jobs** | **8 built-in jobs** are **registered in code** (the code is the registration key; the database holds only schedules and run records; the UI cannot add or rename them), fixed-interval (seconds) scheduling + enable/disable, manual trigger once, paged run history; with the `[job] enabled` master switch off, manual runs still work |
| **Database backup** | Built-in job `db_backup` dumps the whole database to disk with `mysqldump` (files 0600, configurable directory and retention, default 7 copies); runs are recorded in `job_log`; restore is a **manual step** (stop the service → `mysql < dump`, see faq.md §5.10) |
| **Notices** | Draft / published states (publishing writes `published_at`, reverting to draft does not clear it — the trace stays), CRUD, deleting a notice cascades in the same transaction to read records and attachment rows (disk files go to attachment orphan cleanup); v2 list/unread return summaries only (editing and the bell's "view full text" fetch the body on demand), v1 keeps full rows for compatibility; the top-bar bell shows the unread count |
| **Attachment center** | General attachment library: chunked upload (13 extensions on a whitelist, 10 MB per file, total per user bounded by `[attach] quota_mb`), list filtering, download, delete, and **linking** attachments to business objects (v1 is notice images, binding carries ordering); **downloads are authenticated** — attachments bound to a published notice are downloadable by all logged-in users, the rest follow the uploader's data scope; identical content is stored once per month (file name is the content MD5, subdirectory by month), **deleting removes the row but not the disk file** (files are reclaimed by the built-in job `attach_orphan_clean`: 5-minute grace period, hourly by default, interval taken from the existing DB row and not overwritten on upgrade); the notice editor has an "images" section (upload binds immediately) |
| **System config** | Key/value configuration (key / value / type / group / remark + public flag), group-form editing that submits only changed items (the public flag and the value are saved in one go); keys marked "public" are served by `GET /configs/public` (no login required) for the login page and footer (subtitle, copyright, project URL, footer visibility), with built-in defaults as frontend fallback when unavailable; built-in keys are idempotently topped up at startup, changes show on refresh. `notify.*` is owned exclusively by the notification settings page; the generic config page does not list or change it |
| **Bilingual UI** | Two sets of UI strings (Chinese / English, switched from the top bar, following the browser language, stored in localStorage); antd component strings and date formats switch with it; **the backend has no localized messages** — error responses carry a stable business error code `err` + arguments `args`, the frontend looks up the text, and falls back to the backend's Chinese `msg` when it cannot (there is always something to say) |
| **Site info** | One line on the login page and at the bottom of the sidebar: version (injected at build time; release artifacts show the real tag, local builds show `dev`), copyright, project URL (links to the repository) — copyright text / project URL / visibility are read from public system config keys; including the version in bug reports saves the most time |

The documentation site indexes 97 API entries (95 business entries + `/api/v1/health` liveness + `/api/v1/version`;
the notice list/unread each have one entry under v1 and v2). Full definitions of paths and fields are in the
[design doc §5.3](docs/superpowers/specs/2026-10-05-bee-rust-admin-design.md),
and the contracts for dicts / jobs / notices / i18n are in the [Group C design](docs/superpowers/plans/2026-10-06-bra-v1.5-c-modules.md).

**API versions**: the version is in the path prefix, `/api/v1` is the existing contract, `/api/v2` is the current notice contract;
**breaking changes only go into a new version**, released versions never drop fields or change their meaning (new fields are the exception).

- Notice list / unread: v1 keeps full rows (including `content`, `created_by`); v2 returns summaries (without the body),
  the body is fetched via `GET /api/v2/notices/{id}`.
- Notice create/update/delete, detail, status and mark-as-read are available under both the v1 and v2 prefixes (the same handlers);
  new clients should use v2 throughout.
- The other modules currently only have v1. Only the notice module uses `httpV2` in the frontend; the rest still use `http` (v1).
- New endpoints go into the current version by default; to change the shape of an existing response, copy it to a v2 route and keep the original handler on v1.

## Project structure

```
crates/                 bee-rust framework (workspace members, this admin depends on them directly)
  bee_orm/              ★ ORM execution layer: multi-backend pool / QuerySet / CRUD / M2M / migrate
  bee_orm_macro/        ★ #[derive(Model)]: metadata + row mapping + parameter binding codegen
  bee_router/           routing built on axum
  bee_config/           INI configuration (parsing and watching included)
  bee_logs/             tracing log initialization
  bee_cli/ bee_kv/ ...  other framework crates
admin/                  admin panel backend (crate bee_admin)
  src/main.rs           startup: config → DB → migrate → seed → routes → listen
  src/config.rs         INI config and startup validation (JWT secret strength, DSN override)
  src/error.rs          ApiError + unified envelope (code/msg/err/args) + error code table + request body extractor
  src/auth.rs           JWT issue/validate + Auth extractor (permission code loading, token_version check)
  src/datascope.rs      data scope resolution and query injection (department subtree / self / custom)
  src/api/v1/           admin · attach · audit_log · auth · avatar · config · dashboard · dept · dict ·
                        export · import · job · login_log · menu · notice · notify · role · totp
                        eighteen module directories (v1 entry is api/v1/mod.rs; csv is an internal helper module with no endpoints)
  src/api/v2/           modules with breaking changes: notice (list/unread summaries; the full v1 rows
                        live in api/v1/notice/compat.rs)
  src/models/           15 models (tables created by migrate). Column lengths are declared with
                        `sql_type = "VARCHAR(n)"`; **unique keys and indexes are not part of models**
                        (the upstream attributes have no such concept) and are created in CONSTRAINTS
                        in `src/seed/schema.rs`. The relation tables (admin_role/role_menu/
                        role_dept/notice_read) have composite primary keys and no model structs;
                        they use seed's raw DDL, reads/writes in src/relations.rs
  src/seed/             table creation, relation-table DDL, first-run seed (superadmin + menu tree): schema / defaults / menus
  conf/                 app.conf (gitignored) · app.conf.example · app.conf.test
  deploy/               systemd units + nginx site config
  tests/                integration tests: start a real process + connect to a real database (including full-stack)
  web/                  frontend SPA (React + antd, see below)
    src/api/            API layer (contract types + axios interceptors + error code → text mapping)
    src/i18n/           typed dictionaries (zh/en) + I18nProvider + language switch
    src/auth/           AuthContext + button-level permission component
    src/layouts/        dynamic menu layout
    src/pages/          login / home / admins / roles / menus / departments / dicts / jobs / notices /
                        system config / notification settings / export center / attachment center / login logs / audit logs / profile
docs/
  install.md            ★ installation guide (three ways, config generation, common pitfalls)
  usage.md              ★ user guide (how the permission system lands, how each module works)
  faq.md                ★ FAQ and operations (usage chapter 5 as a separate book: troubleshooting, backup/restore, cleaning plaintext backups)
  release-checklist.md  ★ release / upgrade checklist (tick each item)
  release.md            how to release (scripts/release.sh usage, artifact conventions, post-release checks)
  ponytail-debt.md      deliberately deferred items (summary view of the ponytail: comments in the source)
  scaffold.md           module scaffolding usage (generated output must compile before it is written)
  diagrams/             the 4 SVG diagrams referenced by this README
  pet/                  project pet "Keeper" five-state SVG + renderer (node render.mjs)
  social/               social preview image 1280×640 (upload the PNG under Settings → Social preview)
  superpowers/specs/    design docs (data model, API list, data scope rules)
  superpowers/plans/    implementation plans (ORM execution layer, backend, frontend)
install.sh              ★ one-command installer (fetch release artifact / build from source → generate config → install systemd + nginx → self-check)
scripts/
  lib/                  install.sh step implementations (install-*.sh and shared functions; install.sh fails immediately if missing)
  scaffold-module.sh    ★ module scaffold entry: one command to generate a CRUD module skeleton (implementation split across scaffold-*.sh)
  release.sh            release: build → strip → package → artifact self-check → gh release create
  publish.sh            publish each workspace crate not yet on crates.io, one by one
  smoke-install.sh      install.sh smoke test (install a scratch instance in /tmp + upgrade once + clean up)
Dockerfile              backend image (multi-stage build)
docker-compose.yml      single-host MySQL + init + backend + frontend in one stack
docker-compose.tls.yml  HTTPS overlay (certs/ + TLS nginx template)
```

> `install.sql` sits in the repository root but **has nothing to do with this project** — it is another tool's memory schema.
> Installing BRA neither needs nor uses it; don't be fooled by the name.

## Usage

> **Want it running fast?** See [`docs/install.md`](docs/install.md) — full commands for the three install methods,
> key generation commands, a ready-to-edit `app.conf`, and common pitfalls. Command-line people, directly:
>
> ```bash
> git clone --depth 1 https://github.com/erikwang2013/bee-rust-admin.git
> cd bee-rust-admin
> less install.sh && bash install.sh      # interactive; --help lists all options
> ```
>
> `install.sh` is only the entry point; the actual steps live in `scripts/lib/install-*.sh` in the same repository — **the old
> "download only install.sh" usage is retired**; you must clone the full repository first (missing `scripts/lib/` makes it exit with an error).
>
> Once you are **using** it, the guide is [`docs/usage.md`](docs/usage.md) (how the permission system lands, how to operate each module);
> troubleshooting and operations (forgot password, job didn't run, backup/restore) are in [`docs/faq.md`](docs/faq.md).
>
> Note: the detailed docs linked above are written in Chinese; English versions live on the site:
> [install](https://package.erik.xyz/bra/en/install.html) · [usage](https://package.erik.xyz/bra/en/usage.html) · [FAQ](https://package.erik.xyz/bra/en/faq.html).

### Requirements

| Component | Version |
|---|---|
| Rust | 1.99+ (edition 2024) |
| MySQL | 8.0+ (developed on 8.4, utf8mb4) |
| Node / pnpm | Node 22.12+ / pnpm 10+ |
| nginx | any recent version (only needed for production deployment) |

> ⚠️ **Prebuilt artifacts (the default mode of `install.sh`) require glibc ≥ 2.38** (Debian 13 / Ubuntu 24.04+).
> On Ubuntu 22.04 / Debian 12 and older, use `install.sh --mode build` or Docker —
> the script detects this itself and exits **before writing anything**, printing those two alternatives.

### Quick start

**1. Create the database** (tables are created automatically at startup; no manual DDL)

```sql
CREATE DATABASE bee_admin DEFAULT CHARSET utf8mb4 COLLATE utf8mb4_0900_ai_ci;
```

**2. Configure**

```bash
cp admin/conf/app.conf.example admin/conf/app.conf
# Three required items:
#   [db] dsn                              MySQL account and password
#   [jwt] secret                          ≥32 random characters, and not changeme
#   [app] encrypt_key                     base64:<32 bytes>, generate: openssl rand -base64 32 | sed 's/^/base64:/'
# Two recommended items (must differ between deployments):
#   [app] hashids_salt                    salt for public short ids
#   [app] snowflake_worker / snowflake_dc must differ per process across instances, otherwise IDs collide
```

> ⚠️ The service **refuses to start when `encrypt_key` is missing** (deliberate: running while
> "thinking it is encrypted when it is not" is not allowed), and it **cannot be changed** — change the key and the encrypted emails/phones
> in the database can never be decrypted again.
> Likewise, changing `hashids_salt` invalidates every short id already handed out.

Other optional switches are in the example file: `[log] retain_days` (log retention days, `0` = forever, applies to
login / audit / job run records), `[job] enabled` (scheduled-job master switch; manual runs still work when off),
`[auth] max_fail` / `lock_minutes` (login failure lockout), `[auth] captcha` (login captcha, on by default),
`[auth] captcha_trajectory` (slider drag trajectory validation, **off by default**).
With `captcha_trajectory` on, slider answers must include a trajectory (plugin defaults: ≥4 sample points, 300–5000 ms,
linearity ≤0.99); clients that submit only the bare value always fail — the new frontend always reports a trajectory, so enabling it needs no frontend
redeploy; the cost is that "very fast flicks" (<300 ms) are judged as bots (symptom: the slider always says "captcha error" no matter how you drag);
relaxing that requires changing the plugin's `min_duration_ms`.

> When upgrading an existing deployment: captcha credentials are submitted by the new frontend. **If you switch the backend first and do not rebuild the frontend for now, set
> `[auth] captcha` to `false` first**, otherwise logins from the old frontend are rejected with 400 (captcha error).

**3. Start the service** (first start creates tables and seeds the superadmin and menu permissions)

```bash
cargo run -p bee_admin
curl http://127.0.0.1:8080/api/v1/health    # => OK
curl http://127.0.0.1:8080/api/v1/version   # => {"version":"dev"} (injected at build time, see BRA_VERSION in release.sh)
```

**4. Start the frontend**

```bash
cd admin/web && pnpm i && pnpm dev
# open http://127.0.0.1:5173 (vite already proxies /api to 127.0.0.1:8080)
```

You can also preview a production build locally (same port as the live nginx, also proxying `/api`):

```bash
cd admin/web && pnpm build && pnpm preview   # http://localhost:8081
```

### Initial account

| Account | Password |
|---|---|
| `admin` | `[seed] initial_admin_password` in `admin/conf/app.conf` (`change-me-please-1` in the example config) |

The superadmin is written only on first start (`admin` table empty). **Change the password in "Profile" immediately after logging in**;
in production, also change `initial_admin_password` and `[jwt] secret`.

> Tip: `BEE_ADMIN_CONF` sets the config file path; `BEE_ADMIN_DB_DSN` overrides the database DSN;
> `BEE_ADMIN_HTTP_ADDR` overrides the listen address (required when binding `0.0.0.0:8080` in containers).

### Request security scanning (security-rust)

32 built-in attack-pattern detectors (SQLi / command injection / SSRF / XXE / Log4Shell / path traversal …),
**report-only by default**:

```ini
[security]
# off (default): hits only leave a trace (WARN log + one audit entry with the level and detector name, no raw payload)
# high / critical: requests at that level or above get 403 (err = security.blocked)
scan = off
```

False positives are real (users with `' or 1=1--` in their password, code snippets pasted into notices), so it does not block by default.
**Before turning on `high`, know two things:**

- **XSS is not blocked**: XSS only scores Low in the plugin, and the thresholds are only off/high/critical —
  it can be detected and logged, but never 403'd
- Scanning **covers both the raw string and the decoded form** (attack payloads are mostly percent-encoded;
  scanning only the raw text misses the vast majority of injections)

It scans the URL, selected headers (`host` / `user-agent` / `referer` / `origin` / `x-forwarded-host`)
and JSON bodies ≤64KB (**read and put back untouched**). **multipart is never touched** (touching it would buffer the whole upload
in memory and break avatar chunked uploads).

> Only the part of `referer` / `origin` **after** `scheme://authority` is scanned — a same-origin
> Referer the browser adds automatically would be judged **Critical** by the plugin's SSRF detector if it is a loopback/private IP; scanning it literally would make
> `scan = high` 403 **every** normal request on deployments accessed by IP.

### API documentation site (apidoc-rust)

The backend bundles apidoc-rust's API documentation site, **off by default** (the docs page exposes the whole backend API surface,
request/response structures included; it is a dev/delivery tool and should not ship to the public with a production instance). To enable it, write in `app.conf`:

```ini
[apidoc]
enabled = true
password = <pick your own password>
```

- `enabled = true` with an empty `password` → **refuse to start** (same attitude as `encrypt_key`: running while
  "thinking there is a password when there is not" is not allowed); invalid boolean values also refuse to start
- Mount points: `/apidoc` (UI, open directly in a browser), `/apidoc/api.json` (raw API list, 97 entries)
- The docs site is **outside the audit / security-scan layer** (it merges after all middleware; reading docs is not a business
  operation, logging it is just noise), and the whole tree is outside `/api/v1` and `/api/v2`, so it does not affect business endpoints

Data routes (`api.json` / `export` / `mock` / `share` / `generate`) require a token: **POST the password for a
token** (the password goes in the body, not the URL), then pass the token as a query parameter:

```bash
TOKEN=$(curl -s -X POST http://127.0.0.1:8080/apidoc/auth \
  -H 'Content-Type: application/json' -d '{"password":"your-password"}' \
  | sed 's/.*"token":"\([^"]*\)".*/\1/')
curl -G --data-urlencode "token=$TOKEN" http://127.0.0.1:8080/apidoc/api.json
```

`/apidoc` (the UI page itself) has no guard; the password is only checked when fetching a token — don't treat the UI as "a password-protected site".
Failed logins back off (the first 3 are free, then 1→2→… capped at 10 seconds); tokens last 24 hours by default, and a container/process restart
generates a new random `secret_key` by default (old tokens immediately invalid; multiple instances sharing tokens must configure `secret_key` themselves).

> ⚠️ **A token in the query string is equivalent to a replayable pass** — once the URL appears in an access log / browser history /
> bastion host record, whoever grabs it can use it until it expires. So **only enable it on trusted networks (intranet / bastion)**,
> never on the public internet.

### One-command Docker deployment

Container configuration is included:

| File | Purpose |
|---|---|
| `Dockerfile` | Backend image: multi-stage build (Rust compile → Debian runtime), runs as a non-root user |
| `admin/web/Dockerfile` | Frontend image: pnpm build → nginx, `/api/` upstream injected via the `BACKEND_UPSTREAM` environment variable |
| `docker-compose.yml` | Single-host MySQL + init + backend + frontend in one stack, HTTP `8081` by default |
| `docker-compose.tls.yml` | HTTPS overlay: mounts `certs/` and the TLS nginx template, 80→443 |
| `compose-up.sh` | Generates/migrates secrets into `secrets/`, renders `.env`/app.conf, picks HTTP/HTTPS automatically and starts the stack |
| `deploy/nginx-tls.conf.template` | TLS site template (certificates, redirect, reverse proxy, security headers) |
| `certs/README.md` | Where to put domain certificates (the only manual configuration) |
| `deploy/Dockerfile.prebuilt` | **Prebuilt** version of the backend image: packs `target/release-assets/bee_admin`, unused unless releasing (see below) |

```bash
./compose-up.sh --build     # build from source and start the stack; drop --build when images already exist
# default HTTP http://localhost:8081; initial account admin
# the initial password is in secrets/initial_admin_password (also rendered into the initial-password key in app.conf)
#
# Domain certificates (the only manual step): certs/fullchain.pem + certs/privkey.pem,
#   set SERVER_NAME in .env, then ./compose-up.sh --tls
#
# All missing passwords/keys are generated automatically into secrets/ (0600); never copy placeholders by hand.
# Regenerate: ./compose-up.sh --regen <name> (all for everything), old values in secrets/backups/.
```

- **Services separated**: `mysql` + one-shot `mysql-init` + `bee-admin` backend + `bee-admin-web` nginx;
  MySQL publishes no port, the backend uses a dedicated account `bra` (not root), root is only used during initialization
- **Data is not in the container layer**: uploads/attachments/export artifacts/database backups live in the host's `data/bra/`
  (mounted at `/home/uploads` in the backend); MySQL data is in the named volume `mysql-data` (host Docker data directory)
- **Backups work out of the box**: the runtime image includes `mariadb-client`'s `mysqldump`, so the built-in `db_backup`
  runs directly; bare-metal/systemd deployments must install a MySQL client themselves
- **Temp files are reclaimed**: `_header/`, `.part` left by interrupted uploads and
  `backup_*.sql.part` from interrupted backups are swept by the built-in cleanup job after 24 hours; nothing piles up in `data/bra/`
- **Logs/caches don't bloat containers**: all services use the `json-file` log driver with rotation (10m × 5); the backend's
  `/tmp` and nginx's `/var/cache/nginx`, `/var/run` use tmpfs, released on exit
- **Secrets managed locally**: `compose-up.sh` generates missing MySQL root/app passwords, JWT secret,
  `encrypt_key`, `hashids_salt` and the initial admin password into `secrets/` (0600), then renders
  `.env` (0600) and `admin/conf/app.conf` (0600); existing values are never overwritten
  - Regenerate: `./compose-up.sh --regen <name>`; `all` for everything; old values are backed up to
    `secrets/backups/`. MySQL passwords only change files; existing databases must be changed manually per `docs/faq.md`
  - **Changing `encrypt_key` makes encrypted emails/phones in the database permanently unreadable**; changing `hashids_salt`
    invalidates all short ids already issued — backups are in `secrets/backups/`, don't lose `secrets/`
- **Domain certificates are the only manual configuration**: put `certs/fullchain.pem` and `certs/privkey.pem`,
  set `SERVER_NAME` in `.env`; when certificates are present `compose-up.sh` automatically layers
  `docker-compose.tls.yml` for 80→443, otherwise HTTP `:8081` by default
- The backend runs as the host's `APP_UID/APP_GID` so it can read the 600 app.conf and write `data/bra`; a new
  `.env` picks the current user automatically (explicit values in an old `.env` are kept), and root changes the owner of app.conf and `data/bra`
  to that uid; to switch users, change `APP_UID/APP_GID` explicitly in `.env`
- Upgrading an existing compose deployment: copy `/home/uploads` from the old container to `data/bra/` first (the old container loses
  its layer when recreated), then run `./compose-up.sh`; keys in the old `.env`/app.conf are migrated to `secrets/`
  automatically (including external MySQL passwords that only existed in the old `[db] dsn`). A leftover
  `HTTP_PORT=80` from the old TLS mode is reset to 8081 on `--http`; root only recursively chowns `data/bra/` when ownership mismatches
- To use an existing MySQL: remove `mysql`/`mysql-init` and their dependencies from compose, change `MYSQL_*` and
  the rendered DSN in `.env` (or edit `[db] dsn` in app.conf directly)
- If `bra-backend`/`bra-web` images already exist locally (e.g. `docker load`ed from an offline package), skip
  `--build`; to build images only: `docker build -t bra-backend .` and
  `docker build -t bra-web admin/web`; the release script uses the prebuilt
  `deploy/Dockerfile.prebuilt`, packing the stripped binary directly
- **Offline package**: releases ship `bra-images-vX.Y.Z.tar.gz` and `bra-compose-vX.Y.Z.tar.gz`
  (compose + compose-up + TLS template + `.env` + certificate notes); unpack, `docker load`, run
  `./compose-up.sh`, and put domain certificates per `certs/README.md`

### Tests

```bash
# Framework: unit tests + real-database integration (integration tests skip with a printed reason when no DSN is set)
cargo test -p bee_orm
BEE_ORM_TEST_DSN='mysql://user:pass@127.0.0.1:3306/bee_orm_test' cargo test -p bee_orm -- --nocapture

# Admin: unit tests (pure functions: config / JWT / data scope / cycle prevention …)
cargo test -p bee_admin --bins

# Admin: whole package (unit tests + all real-database integration suites; don't list --test one by one, so new suites aren't missed in CI)
# Note: drops all tables in the test database and recreates them; only run against a test database
BEE_ADMIN_DB_DSN='mysql://user:pass@127.0.0.1:3306/bee_admin_test' \
  cargo test -p bee_admin -- --nocapture
```

`BEE_ADMIN_DB_DSN` overrides `[db] dsn` from the config (so test credentials need not be committed); integration tests have a
**production-database guardrail** — they refuse to run when the database name in the DSN does not end with `_test`.

### Deployment

systemd runs the service (listening on `127.0.0.1:8080`) and nginx serves port 8081: static assets point at the frontend build output
`admin/web/dist`, and `/api/` reverse-proxies to the backend passing `X-Real-IP` through (login logs need the real IP).

```bash
cargo build --release -p bee_admin
cd admin/web && pnpm build

sudo cp admin/deploy/bee-admin.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now bee-admin

sudo cp admin/deploy/nginx.conf.example /usr/local/nginx/conf/vhost/bee-admin.conf
sudo nginx -t && sudo systemctl reload nginx
```

The comments in the files describe each file's install location and prerequisites; design doc §7 covers the deployment approach in full.

> On machines without root, use the **user-level unit** `admin/deploy/bee-admin.user.service`: install it under
> `~/.config/systemd/user/`, run every command with `--user`; capabilities are equivalent. Auto-start at boot requires that user to enable
> linger (`loginctl show-user <user> -p Linger`) — installation and management details are in the
> [installation guide (c)](docs/install.md#c-手工部署).

> These two files are **examples of the manual path**, with paths and usernames hard-coded to this machine
> (`/home/wwwroot/bee-rust-admin`, `erik`); adjust them when following along.
> **To skip the copying and editing, use [`install.sh`](install.sh)**: it renders these two files from the actual `--prefix` / user / port,
> generates the config, creates the database and runs a self-check. Full instructions are in the "one-command script" section of
> [`docs/install.md`](docs/install.md).
> Before **upgrading an existing deployment**, go through [`docs/release-checklist.md`](docs/release-checklist.md)
> (config keys, same version front and back, rollback with a single `mv`).
> **To cut a release**, see [`docs/release.md`](docs/release.md): one `bash scripts/release.sh vX.Y.Z --notes-file …`
> covers build, strip, packaging, self-check and `gh release create` (`--dry-run` validates without releasing).

## Copyright

© 2026 erik · [https://erik.xyz](https://erik.xyz)

This project is released under [Apache-2.0](LICENSE).
