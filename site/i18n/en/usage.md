# BRA Admin · User Guide

> This guide is for **users**: administrators and authorised users with an account. It explains how to get things done in the UI.
> Installation, deployment and upgrades are covered in the [Installation Guide](install.html); troubleshooting and operations
> are in the [FAQ](faq.html).
>
> Button and menu names match the admin UI; look for the same labels on screen.

## Contents

1. [Login and profile](#1-login-and-profile)
2. [How permissions work](#2-how-permissions-work)
3. [Module notes](#3-module-notes)
4. [Data scope: five levels and how to verify](#4-data-scope-five-levels-and-how-to-verify)

---

## 1. Login and profile

### 1.1 Login

1. Open the admin URL and go to the login page (BRA · Admin).
2. Enter **username** and **password**, then complete the image captcha.
3. Click **Login**. You land on the dashboard.

If your account has **TOTP** enabled, login is two-step: after the password is accepted, a **dynamic code / backup code** field appears.
Enter the current 6-digit code from your authenticator app and click **Login** again. A **backup code** (10 alphanumeric characters,
generated in your profile, **single use**) can be entered in the same field; the backend treats 6 digits as TOTP and anything else as a backup code.
Backup-code hashes derive their pepper from `encrypt_key` — **changing that key invalidates every existing backup code**.
Codes change every 30 seconds; a code can be used only once per time window, so two devices cannot both log in with the same code.

The footer shows site information (version, copyright, repository link; editable under **System Config**).

Login failures:

Usernames are limited to **64** characters and passwords to **1024**; oversized requests get the generic invalid-credentials response before any database lookup or password hash.

| Message | Meaning |
| --- | --- |
| Username and password are required | One field is empty |
| Invalid username or password | The same message for an unknown account and a wrong password (no account enumeration) |
| Captcha error | Redo the captcha; a wrong captcha does **not** count toward lockout |
| Dynamic code required | TOTP is enabled and the password was correct; enter the 6-digit code or a backup code |
| Invalid dynamic code | Wrong/expired code (30-second window), or a wrong/used backup code. Counts as a failed attempt |
| Dynamic code already used | The code was used in this window (multi-device or a recent login). Counts as a failed attempt |
| Account disabled | Ask a superadmin to enable it |
| Too many attempts, try again in N minutes | Lockout, see the FAQ |

Default policy: **5** failed passwords for the same account or source IP lock it for **10** minutes. Wrong captchas do not count.

### 1.2 Top bar

From left to right: breadcrumb, **language** (简体中文 / English), **light/dark theme**, unread-notice bell, avatar menu.

- Language and theme are local browser preferences, not account settings.
- Initial language follows the browser; initial theme follows the system. Your changes stick.
- The avatar menu has **Profile** and **Sign out**.

### 1.3 Profile

**Profile details**: nickname, email, phone. Saving affects only your profile; email and phone are encrypted at rest.

**Avatar**:

- PNG / JPEG only, max **512 KB**. Larger images are compressed in the browser (long edge 512 px, decreasing JPEG quality);
  if still over 512 KB you are asked to choose a smaller file.
- Upload shows progress; the top-bar avatar updates on success. Avatar responses carry an ETag and return 304 on revalidation.

**Change password**: current password, new password (8–1024 characters with at least two of letters/numbers/symbols), confirm.
All sessions on **every device, including this one**, are invalidated; log in again with the new password.

**Sign out other devices**: other sessions are invalidated immediately; **this device stays signed in** (the backend quietly issues a new token).

**TOTP**: click **Generate secret** → scan the QR code with an authenticator (Google Authenticator / 1Password, etc.) or enter the secret manually →
enter the 6-digit code and click **Bind**. After that, login is password + code. Notes:

- Binding and unbinding require a current code.
- The secret is stored encrypted; plaintext is shown only once. Unbinding clears the secret and backup codes.
- **Backup codes**: after binding, click **Generate backup codes**, confirm with the current 6-digit code, and save the **10 one-time codes**
  (ambiguous characters such as `0/O/1/I` are excluded). Plaintext is shown once; only hashes are stored. Regenerating invalidates old codes.
- **Current-code confirmation is rate-limited**: after 5 wrong current codes, even a correct code gets 429 for 5 minutes (per account, per process).
- If the authenticator and all backup codes are lost, only an operator with database access can clear the account’s TOTP columns and backup-code rows;
  resetting the password does **not** disable TOTP.

**Login devices**: one row per device (browser id, IP, login time, last seen); the current device is marked. **Kick** invalidates that device’s token only
(change password invalidates all). Kicked devices fail on their next request. The row is soft-deleted and retained per `[auth] device_keep_days`
(default 30 days; `0` = forever), cleaned by the `device_clean` job. Validity is cached for 30 seconds per process; kicks apply immediately in the same process
and within 30 seconds on other instances.

### 1.4 Dashboard

The first screen after login: a welcome card (account, nickname, superadmin flag, permission count, accessible menus) and four counters
(**admins / roles / departments / notices**), followed by two 7-day trend cards (successful and failed logins). Hover a bar for date and count;
missing days are zero; an all-zero week shows an empty state. The dashboard needs no permission code and is not filtered by data scope.

---

## 2. How permissions work

In one sentence:

- **What you can click = role menu permissions** (down to individual buttons);
- **What data you can see = role data scope** (five levels, section 4).

Both live on **roles**; admins get permissions by being assigned roles. The built-in superadmin (`admin`) bypasses both and sees everything.

Recommended order: **departments → roles → admins**.

1. Create departments first; roles and admins reference them.
2. Define roles (what can be clicked + what data is visible).
3. Create admins and assign roles.

### 2.1 Step 1: departments

Menu **System → Departments**. Departments form an arbitrarily deep tree.

1. Click **Add** for a top-level department, or **Add child** on a row.
2. Fill parent, name, order, owner, phone, status.
3. Delete constraints: **delete child departments first**, and **no admins may remain in the department**; a department cannot be moved under its own descendant (no cycles).

Departments affect **data scope only**, not functional permissions. A disabled department can still be selected in admin forms and is still part of “department and below” subtrees.

### 2.2 Step 2: roles

Menu **System → Roles**.

1. Click **Add**; enter a **role name** (display name, e.g. “Dept lead”) and **role code** (3–64 chars, letters/numbers/underscore/colon, e.g. `admin`).
2. Choose a **data scope** (five levels; section 4).
3. Save, then click **Permissions** on the row to open the “Assign permissions: …” drawer.

The drawer is a three-level **menu tree**: **directory → menu → button**.

- Checking a menu makes it appear in the sidebar; checking a menu without buttons lets the user open the page but hides actions.
- Button nodes are marked **(button)**, e.g. Add, Edit, Delete, Reset password.
- Changes apply immediately; no re-login required.

The drawer loads both the menu tree and the department tree, so the operator also needs **Menu management** and **Department management** view permissions
(`system:menu:list`, `system:dept:list`) in addition to the role being edited; otherwise the drawer cannot load its trees.

### 2.3 Step 3: admins

Menu **System → Admins**.

1. Click **Add**; enter username, initial password, nickname, department, roles, status.
2. Rules: username 3–64 chars (letters/numbers/underscore, unique); password 8–1024 chars with at least two of letters/numbers/symbols;
   nickname required (max 64 chars).
3. **Usernames cannot be changed** after creation; passwords are changed via **Reset password** (or by the user in Profile).
4. Give the user the username and initial password; ask them to change it immediately.

The role selector accepts a **role name filter**; at most **100** roles are listed at once (use the search box when there are more).

Important consequences:

- **Reset password** invalidates all sessions for that account. This is intentional; notify the user.
- **Disable** also invalidates all sessions and prevents future logins.
- You cannot delete the superadmin, delete yourself, change the superadmin’s status, change your own status, or reset another superadmin’s password.

Data scope also constrains whom you can manage:

- Admins outside your scope are not listed and cannot be edited/deleted; forced operations are rejected with **outside your data scope**.
- You cannot create/edit an admin into a department outside your scope.
- You cannot grant roles whose data scope is wider than yours (“All data”, “Department and below”, “Custom”) or that contain permission codes you lack;
  such roles are greyed out. On edit, only newly added roles are checked.
- The same rule applies to editing roles: you cannot modify a role wider than yours or grant menus/permissions you lack; you can only set its data scope to
  “Own department” or “Self only”. Widening your own role is rejected.

### 2.4 Permission codes

Permission codes are `module:resource:action`, for example:

- `system:admin:list` — view admins;
- `system:admin:resetPwd` — reset passwords;
- `system:notice:edit` — edit notices (publish/unpublish uses the same code).

See them under **System → Menus**; the **Permission code** column lists the code for every node, including button nodes.

Built-in codes:

| Menu | Prefix | Actions |
| --- | --- | --- |
| Admins | `system:admin` | `list`, `add`, `edit`, `remove`, `resetPwd` |
| Roles | `system:role` | `list`, `add`, `edit`, `remove` |
| Menus | `system:menu` | `list`, `add`, `edit`, `remove` |
| Departments | `system:dept` | `list`, `add`, `edit`, `remove` |
| Login logs | `system:loginlog` | `list`, `remove` (clear) |
| Audit logs | `system:auditlog` | `list`, `remove` (clear) |
| Dictionaries | `system:dict` | `list`, `add`, `edit`, `remove` (items use the same codes) |
| Jobs | `system:job` | `list`, `edit` (interval, enable/disable, trigger once) |
| Notices | `system:notice` | `list`, `add`, `edit`, `remove` |
| System config | `system:config` | `list`, `edit` |
| Notification settings | `system:notify` | `list`, `edit` |
| Export center | `system:export` | `list`, `remove` (starting/downloading also needs the data’s view code) |
| Attachments | `system:attach` | `list`, `add`, `edit`, `remove` (`edit` covers link/unlink) |

Notes:

- You can add menus/buttons and codes under **Menu management**, but a non-superadmin may only use codes they already have; otherwise the request is rejected.
  New codes affect sidebar visibility and role checkboxes only; backend endpoints only recognise codes compiled into the code.
- Pages outside the sidebar (dashboard, profile, notice bell) need no permission code; login is enough.

### 2.5 Hiding buttons is UX; the backend checks every request

- The **frontend** hides buttons and blocks routes based on your codes; a blocked route shows a 403 page with **Back to home**.
- The **backend** re-checks your **current** codes on every API request; missing permissions are rejected. Re-showing a button or calling the API directly does not grant access.
- Permissions are evaluated per request: disabling a role or unchecking a menu takes effect on the next request. The sidebar menu is cached at login; refresh to see changes.

---

## 3. Module notes

### 3.1 Admin management (system:admin)

Filter by username (fuzzy) and status; **Export** the current filter; bulk creation uses **Import**. Create with username, initial password, nickname, department, roles, status.
Username and password cannot be edited in the edit form (use Reset password or the user’s Profile). Row actions include Edit, Reset password, Delete and a status switch;
reset and disable immediately invalidate all sessions. Admins outside your data scope are not listed and cannot be operated on. Prefer disabling over deleting.

### 3.2 Role management (system:role)

The **Permissions** button opens the permission drawer (menu tree down to buttons). Each role also has a **data scope** (section 4). A role in use by an admin cannot be deleted.
Role codes are unique. Permission and data-scope changes apply immediately after a page refresh on affected accounts. Non-superadmins can only edit roles within their own scope and
with a subset of their own codes; data scope can only be set to “Own department” or “Self only”.

### 3.3 Menu management (system:menu)

Maintains the sidebar structure and permission-code table. Node types: **directory** (expandable group), **menu** (page), **button** (permission code only).
Fields: parent, name, type, permission code, route, component, icon, order, visible, status. Delete constraints: delete child nodes first; remove the node from all roles first.
**Hidden** only hides it from the sidebar; it does not affect permission checks. The 13 built-in modules are restored on every startup if deleted; to disable one, hide it or unassign it from all roles.

### 3.4 Department management (system:dept)

Arbitrarily deep department tree. Each node can add a child or edit parent, name, order, owner, phone and status. Delete constraints: no child departments and no admins in the department;
no cycles. Departments only provide coordinates for data scope. To reorganise, change the parent instead of deleting/recreating (recreation changes ids and breaks custom scopes and admins).
Bulk creation uses **Import**.

### 3.5 Dictionary management (system:dict)

Left: dictionary types; right: items of the selected type. Types have a **code** (lowercase letters/numbers/underscore, 2–64 chars, immutable), name and status.
Items have label, value (unique within the type), order and status. Deleting a type deletes all its items. Dictionaries are a data source for page dropdowns
(`GET /dicts/{code}/items` returns enabled items only). The built-in pages currently do not use dictionary dropdowns, so creating a dictionary may not change the UI.
**Export** downloads the current type’s items; bulk item creation uses **Import**.

### 3.6 Jobs (system:job)

Two tabs: **Tasks** and **Run history**. Tasks are registered in code; the database stores only scheduling and run history, so the UI can change interval and enable/disable but cannot add or rename tasks.
Eight built-in jobs: `log_retention`, `avatar_orphan_clean`, `attach_orphan_clean`, `db_backup`, `export_run`, `export_clean`, `device_clean`, `notify_retry`.
Most run daily; `export_run` every 60 seconds; `attach_orphan_clean` every hour; `notify_retry` every 5 minutes. Intervals come from the database row, so upgrades do not overwrite existing rows. The upload cleanup jobs reclaim stale `_header/`/`.part` files older than 24 hours; `db_backup` reclaims stale `backup_*.sql.part` files.
**Trigger once** runs in the background and returns immediately; the last run/result columns and run history update afterwards. Triggering a running job is rejected (409) and does not start a second copy.
Disabled jobs are not scheduled, but **Trigger once** still works. Jobs run serially; a long job delays the next checks. In multi-instance deployments enable scheduling on only one instance
(`[job] enabled = true`); the “already running” guard is per process.

### 3.7 Notices (system:notice)

New notices are drafts and invisible to users until **Publish**. Unpublish removes them from unread lists (read records remain). The first publish time is preserved across unpublish/republish.
Title max 128 chars; body is plain text with newlines preserved. The edit dialog has an **images** section (upload binds to the notice; save a new notice first). Deleting a notice cascades
to read records and attachment rows in the same transaction (files are handled by orphan cleanup). The top-bar bell shows unread count; each notice can be marked read; only the latest 50 are listed.

### 3.8 Login logs (system:loginlog)

Every login attempt (success or failure) is recorded with username, IP, time and result. Filter and **Export** the current result (or use the Export center). **Clear** removes only records matching the current filter;
the clear itself is audited. Records are constrained by data scope.

### 3.9 Audit logs (system:auditlog)

Records side-effecting operations (POST/PUT/DELETE); login requests are in Login logs. Shows time, operator, module, action, request and IP; filter by username/module/time;
**Export** or **Clear** (only the current filter; clearing is itself recorded). Modules include admins, roles, menus, departments, dictionaries, system config, import, export, login logs, audit logs,
attachments, auth, other. Job/notice operations show the raw module `job`/`notice`, which are not in the dropdown; filter by username/time instead. For four entity types (admins, roles, departments, dictionaries)
a **change summary** records changed fields as `field: old → new`; encrypted email/phone only say “changed”, and password APIs never produce summaries.

### 3.10 System config (system:config)

Grouped key/value configuration. Each item has key (immutable), value, type, group, note and a **public** switch. Only public keys are readable by the login page and footer.
Save submits only changed items in the group. Public toggles and values are saved together. Four built-in public keys: `site.copyright`, `site.repo_url`, `site.footer_enabled`, `site.login_subtitle`.
If the public endpoint is unavailable, the frontend falls back to built-in defaults. New items are private by default. Deleting a built-in public key falls back to defaults; startup restores missing built-in keys without touching existing values.
The `notify` group appears after notification settings are configured; the SMTP password and Webhook signing secret are not visible or editable on this page.

### 3.11 CSV import (admins / departments / dictionary items)

The Admins, Departments and Dictionary items pages have an **Import** button: **download template** → fill → upload → review the per-row report. Headers are part of the contract and cannot be changed.

- Departments/parent departments/roles/type codes are referenced by name or code and must already exist. Empty parent = top level; multiple roles separated by `|`; empty roles = none.
- Status: `1`, `enabled` or empty means enabled; `0`/`disabled` means disabled. Username/password validation matches the create form.
- **All or nothing**: one invalid row means no rows are written; the dialog lists row number and reason. Valid files commit in one transaction.
- Permissions and data scope are enforced per row. Import does not bypass them.
- UTF-8 only; at most 5000 rows per import. Empty data (header only) is reported as “no data”, not a failure.
- Normal requests time out after 30 seconds; imports get a 180-second ceiling. Split very large imports or use the Export center for big data.

### 3.12 Export center (system:export)

Choose a type → **Start export** → watch status (pending/running/completed/failed/cancelled) → **Download** completed rows; cancel pending/running rows.
Filters are stored with the task and applied when it runs (username, module, status, time range). **Run by**: the `export_run` job (default every minute), up to 2 concurrent exports per process;
the list auto-refreshes every 3 seconds while work is pending. Cancel removes the partial file and cannot leave a finished file behind. Progress shows rows exported (not a percentage); a heartbeat older than 10 minutes is treated as dead.
Files are retained per `[export] retain_days` (default 7 days) and cleaned by `export_clean`. Two permission gates: `system:export:list` plus the data’s view code (`system:auditlog:list` or `system:loginlog:list`).
The export center is a personal workspace: only your own tasks are visible. Data scope is re-evaluated when the task runs, using the initiator’s current scope.

### 3.13 Notification settings (system:notify)

Configures **Webhook** (event push) and **SMTP** (email). Values live in `notify.*` system config and apply immediately.

- **Webhook**: URL must start with `http://` or `https://`. Saves reject link-local/unspecified IP literals (including `169.254.0.0/16`, `fe80::/10`, `0.0.0.0`, `::`; metadata service `169.254.169.254` is blocked); loopback is allowed.
  Sending resolves the domain again and blocks link-local/unspecified results; redirects are not followed. Events: `notice.published` and `account.locked`; body is `{"event":…,"at":unix,"data":{…}}`.
  Optional HMAC-SHA256 signing headers: `X-BRA-Timestamp` and `X-BRA-Signature: sha256=…` (HMAC over `"<ts>.<raw body>"`). Failures are stored in `notify_delivery` and retried by `notify_retry`
  with 1/5/15-minute backoff, at most 3 retries; the ledger is kept for 30 days. Delivery is asynchronous.
- **SMTP**: server, port, encryption (STARTTLS 587, SSL/TLS 465, none for local relays), username, password, sender. The password is encrypted and never returned; blank = unchanged,
  filled = replace; **Clear password** is a separate action. Sender defaults to username; empty username = no auth. Test email goes to **your own account email** using saved settings.
- **Notice email push**: when enabled, publishing a notice sends one email per enabled account with an email address, excluding the publisher. Delivery is per recipient, best-effort,
  and does not retry; it requires SMTP to be configured.
- Permissions: `system:notify:list` to open; `system:notify:edit` to save and send the test email. Secrets are write-only everywhere.

### 3.14 Attachments (system:attach)

A shared attachment library. Uploads are unbound until linked to a business object (v1: notice images). Page: **Upload** → list (name/size/uploader/binding/time) → **Download / Link / Delete**;
filter by name and binding.

- Limits: 10 MB per file; whitelist of 13 extensions (png/jpg/gif/webp/bmp, pdf, txt, csv, docx/xlsx/pptx, zip); per-user quota `[attach] quota_mb` (default 200 MB, `0` = unlimited).
  Deleting releases quota immediately; disk reclaim is by the hourly cleanup. Files deleted within the 5-minute protection window still occupy disk.
- Identical content is stored once (MD5); multiple rows may point to one file. Deleting rows does not delete the file; orphan cleanup removes unreferenced files after the protection window.
- Download: attachments bound to a **published notice** are public to logged-in users; others follow the uploader’s data scope. Link/delete require `system:attach:edit` / `system:attach:remove`.
  Linking a notice is done in the attachment center (empty selection = unlink); notice editing can upload and bind images (save a new notice first).
- Deleting a notice cascades to its attachment rows in the same transaction; files go to orphan cleanup. The `attach_orphan_clean` job (hourly) removes unreferenced files older than 5 minutes.

---

## 4. Data scope: five levels and how to verify

### 4.1 The five levels

| Level | UI name | Data visible to admins with this role |
| --- | --- | --- |
| 1 | All data | No restriction; all departments and all admins |
| 2 | Department and below | Own department **plus all descendants**, recursively; other branches invisible |
| 3 | Own department | Only the exact department, **not** children |
| 4 | Self only | Only rows owned by the admin |
| 5 | Custom | A hand-picked set of departments, any branches/levels (checked in the permission drawer) |

Data scope applies to **Admin management** (by admin department), **Login logs** and **Audit logs** (by the recorded admin’s department), and **Attachments** (by uploader; published notice images are downloadable by the public rule).
Menus, roles, dictionaries and notices are global configuration; everyone who can open them sees the same data.

Edge cases:

- An admin **without a department** (“none”): “Department and below” sees nothing; “Own department” sees only other no-department admins. Assign a real department, or use “All data”/“Custom”.
- **No active role** (none assigned, or all disabled): sees nothing. It is not “all” or “own department” by default.

### 4.2 Verify it yourself

Create departments **A**, **A-team** (child of A), **B**; create test admins:

| Account | Department | Purpose |
| --- | --- | --- |
| `u1` | A | Subject |
| `u2` | A-team | Descendant of A |
| `u3` | B | Other branch |

Create two roles with only **Admin management → list** checked:

- “Dept lead”: data scope **Department and below**;
- “Global lead”: data scope **All data**.

Then:

1. Assign “Dept lead” to `u1`, log in as `u1`, open Admin management: only A and its descendants (u1, u2) plus u1 himself are listed; u3, the superadmin and other departments are hidden. Note the total.
2. As the superadmin, change u1’s role to “Global lead”; u1 refreshes: more rows, including u3 and the superadmin.
3. For **Self only**, create a role with that scope and assign it to u1; after refresh only u1 remains.
4. For **Custom**, create a role with scope **Custom**, check only **B** in the department tree, assign it to u1; u1 sees u3 in B but not u2 in A-team.

Only list counts change; menus and buttons do not (functional permissions and data scope are separate). No re-login is needed; refresh after changes. Keep filters identical when comparing counts.

### 4.3 Multiple roles

Functional permissions and data scope are both **unions** across all active roles:

- **Permissions**: union of all codes from enabled roles.
- **Data scope**: union of department scopes; any role with “All data” wins; any role with “Self only” adds “own rows” on top of the department scope.
- Disabled roles contribute nothing.
- Adding a role always widens access; to narrow it, remove the wider role.
