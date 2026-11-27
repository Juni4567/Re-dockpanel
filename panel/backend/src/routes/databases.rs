use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use base64::Engine;
use uuid::Uuid;

use crate::auth::{AuthUser, ServerScope};
use crate::error::{internal_error, err, agent_error, paginate, ApiError};
use crate::services::agent::AgentError;
use crate::AppState;

/// Hard per-account cap on total databases, independent of any reseller quota.
/// Bounds how much of the shared, host-wide DB port pool + host RAM a single
/// tenant can consume so one account cannot deny database creation to everyone.
const MAX_DATABASES_PER_USER: i64 = 25;

/// Database names that collide with a system/admin identity. Since v2.20.0 the tenant
/// `{name}` becomes a real postgres role that OWNS database `{name}` (M1: the tenant is
/// no longer the cluster superuser), so a name matching the cluster's own superuser /
/// template DBs (postgres) or system schemas (mariadb) must be rejected. Case-insensitive.
fn is_reserved_db_name(name: &str) -> bool {
    const RESERVED: &[&str] = &[
        "postgres",
        "template0",
        "template1",
        "mysql",
        "sys",
        "information_schema",
        "performance_schema",
    ];
    let lower = name.to_ascii_lowercase();
    RESERVED.contains(&lower.as_str())
}

/// Convert an agent error to a user-facing error for SQL operations.
/// Unlike `agent_error()`, this passes through the actual SQL error message.
fn sql_error(e: AgentError) -> ApiError {
    match e {
        AgentError::Status(_code, body) => {
            // Try to extract "error" field from agent JSON response
            let msg = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str().map(String::from)))
                .unwrap_or(body);
            // Strip the Docker-daemon infra prefix so only the DB engine's SQL
            // diagnostic is surfaced (don't leak container/daemon internals).
            let msg = msg
                .strip_prefix("Error response from daemon: ")
                .unwrap_or(&msg)
                .to_string();
            err(StatusCode::BAD_REQUEST, &msg)
        }
        other => agent_error("SQL query", other),
    }
}

#[derive(serde::Deserialize)]
pub struct ListQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(serde::Deserialize)]
pub struct CreateDbRequest {
    /// Exactly one of `site_id` / `stack_id` must be set — see `resolve_owner`.
    pub site_id: Option<Uuid>,
    pub stack_id: Option<Uuid>,
    pub name: String,
    pub engine: Option<String>,
}

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct Database {
    pub id: Uuid,
    pub site_id: Option<Uuid>,
    pub stack_id: Option<Uuid>,
    pub name: String,
    pub engine: String,
    pub db_user: String,
    pub container_id: Option<String>,
    pub port: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// GET /api/databases — List all databases for the current user.
pub async fn list(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    ServerScope(server_id, _agent): ServerScope,
    Query(params): Query<ListQuery>,
) -> Result<Json<Vec<Database>>, ApiError> {
    let (limit, offset) = paginate(params.limit, params.offset);

    // Same admin-widening create() already grants (helpers::SITE_CALLER_PREDICATE) —
    // without it, a database an admin creates on a tenant's site via the local-box
    // arm becomes invisible to its own creator (v2.216.0 audit finding).
    //
    // LEFT JOIN both possible owner tables and COALESCE: a row has exactly one
    // non-null owner FK (chk_databases_owner), so each row matches at most one
    // owner across the two joins and COALESCE picks whichever is present — one
    // query covers site-owned and stack-owned databases without a UNION.
    let dbs: Vec<Database> = sqlx::query_as(
        "SELECT d.id, d.site_id, d.stack_id, d.name, d.engine, d.db_user, d.container_id, d.port, d.created_at \
         FROM databases d \
         LEFT JOIN sites s ON d.site_id = s.id \
         LEFT JOIN docker_stacks st ON d.stack_id = st.id \
         WHERE (COALESCE(s.user_id, st.user_id) = $1 OR EXISTS (SELECT 1 FROM users u, servers sv \
         WHERE u.id = $1 AND u.role = 'admin' AND sv.id = COALESCE(s.server_id, st.server_id) \
         AND (sv.is_local OR sv.user_id = u.id))) \
         AND COALESCE(s.server_id, st.server_id) = $2 ORDER BY d.created_at DESC LIMIT $3 OFFSET $4",
    )
    .bind(claims.sub)
    .bind(server_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await
    .map_err(|e| internal_error("list databases", e))?;

    Ok(Json(dbs))
}

/// Resolve the host + a caller-facing label for a database's owner — a site or a
/// stack (GH #64: standalone DB provisioning), exactly one of which the caller must
/// supply. The label only names `agent_for_site_server`'s refusal log line; no domain
/// is loaded on these paths, so callers still identify the target by database name.
async fn resolve_owner(
    state: &AppState,
    site_id: Option<Uuid>,
    stack_id: Option<Uuid>,
    user_id: Uuid,
) -> Result<(Option<Uuid>, String), ApiError> {
    match (site_id, stack_id) {
        (Some(sid), None) => {
            let row: Option<(Option<Uuid>, String)> = sqlx::query_as(&format!(
                "SELECT s.server_id, s.domain FROM sites s WHERE {}",
                crate::helpers::SITE_CALLER_PREDICATE
            ))
            .bind(sid)
            .bind(user_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| internal_error("resolve database owner", e))?;
            row.ok_or_else(|| err(StatusCode::NOT_FOUND, "Site not found"))
        }
        (None, Some(kid)) => {
            let row: Option<(Option<Uuid>, String)> = sqlx::query_as(&format!(
                "SELECT st.server_id, st.name FROM docker_stacks st WHERE {}",
                crate::helpers::STACK_CALLER_PREDICATE
            ))
            .bind(kid)
            .bind(user_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| internal_error("resolve database owner", e))?;
            row.ok_or_else(|| err(StatusCode::NOT_FOUND, "Stack not found"))
        }
        _ => Err(err(
            StatusCode::BAD_REQUEST,
            "Specify exactly one of site_id or stack_id",
        )),
    }
}

/// POST /api/databases — Create a new database.
pub async fn create(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Json(body): Json<CreateDbRequest>,
) -> Result<(StatusCode, Json<Database>), ApiError> {
    // Verify ownership — and take the host from the same row while we are here. The
    // container is created on whichever host the handle points at and then recorded
    // against this owner, so a misdispatch leaves a `databases` row naming a
    // container that does not exist on the owner's own machine, while the wrong
    // machine keeps the container and the port. The row that decides WHICH OWNER has
    // to decide WHICH HOST too.
    let (owner_server_id, owner_label) =
        resolve_owner(&state, body.site_id, body.stack_id, claims.sub).await?;

    let agent =
        crate::helpers::agent_for_site_server(&state, owner_server_id, &owner_label).await?;

    // Hard per-account cap on total databases (independent of reseller quota) so a
    // single tenant cannot exhaust the shared, host-wide DB port pool / host RAM.
    // Counts both site-owned and stack-owned rows (COALESCE — chk_databases_owner
    // guarantees exactly one of the two joins matches per row).
    let (db_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM databases d \
         LEFT JOIN sites s ON d.site_id = s.id \
         LEFT JOIN docker_stacks st ON d.stack_id = st.id \
         WHERE COALESCE(s.user_id, st.user_id) = $1",
    )
    .bind(claims.sub)
    .fetch_one(&state.db)
    .await
    .map_err(|e| internal_error("create databases", e))?;
    if db_count >= MAX_DATABASES_PER_USER {
        return Err(err(
            StatusCode::FORBIDDEN,
            &format!("Database limit reached ({MAX_DATABASES_PER_USER} per account)"),
        ));
    }

    // Validate name
    if body.name.is_empty() || body.name.len() > 63 {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid database name"));
    }
    if !body
        .name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Database name must be alphanumeric with underscores",
        ));
    }
    // ⛔ The AGENT is the authority on this name — it is what creates the
    // container — and its `is_valid_name` demands an alphanumeric FIRST
    // character. Without this clause the panel accepted `_mydata`, called the
    // agent, and handed the operator the agent's own refusal: a form the product
    // had just validated, answered by a machine. The rule belongs where the
    // person typing it can read it.
    //
    // ⚠ The 63 above is NOT the same divergence and must not be "aligned" to the
    // agent's 64: a PostgreSQL identifier is 63 bytes, so 63 is the cross-engine
    // cap and the stricter side is the correct one. It refuses in the panel's own
    // words, which is the behaviour this clause exists to produce.
    if !body
        .name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Database name must start with a letter or a number",
        ));
    }

    if is_reserved_db_name(&body.name) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Database name is reserved (system/admin name)",
        ));
    }

    // Check uniqueness per-owner (site OR stack — whichever the caller supplied;
    // resolve_owner already refused a request naming both or neither).
    let existing: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM databases WHERE site_id IS NOT DISTINCT FROM $1 \
         AND stack_id IS NOT DISTINCT FROM $2 AND name = $3",
    )
    .bind(body.site_id)
    .bind(body.stack_id)
    .bind(&body.name)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| internal_error("create databases", e))?;

    if existing.is_some() {
        return Err(err(
            StatusCode::CONFLICT,
            "Database name already exists for this owner",
        ));
    }

    let engine = body.engine.as_deref().unwrap_or("postgres");
    if !["postgres", "mysql", "mariadb"].contains(&engine) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Engine must be postgres, mysql, or mariadb",
        ));
    }

    // Generate password and find available port
    let password = uuid::Uuid::new_v4().to_string().replace('-', "");
    let port = find_available_port(&state, engine).await?;

    // Encrypt the password before storing in the database
    let encrypted_password = crate::services::secrets_crypto::encrypt_credential(&password, &state.config.jwt_secret)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &format!("Encryption failed: {e}")))?;

    // Atomically reserve one slot against the caller's reseller quota (if any). The old
    // check-then-increment was TOCTOU-racy — concurrent creates all read the same stale
    // `used_databases` and each incremented past `max`. A conditional UPDATE ... RETURNING
    // makes the check and the increment one statement. Released on any failure below.
    let reserved_quota = reserve_reseller_db_slot(&state, claims.sub).await?;

    // Insert DB record first to atomically claim the port (unique index prevents races).
    // container_id is empty until the agent creates it.
    let db_record: Database = match sqlx::query_as(
        "INSERT INTO databases (site_id, stack_id, name, engine, db_user, db_password_enc, container_id, port) \
         VALUES ($1, $2, $3, $4, $5, $6, '', $7) \
         RETURNING id, site_id, stack_id, name, engine, db_user, container_id, port, created_at",
    )
    .bind(body.site_id)
    .bind(body.stack_id)
    .bind(&body.name)
    .bind(engine)
    .bind(&body.name)
    .bind(&encrypted_password)
    .bind(port)
    .fetch_one(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            if reserved_quota { release_reseller_db_slot(&state, claims.sub).await; }
            if e.to_string().contains("unique") || e.to_string().contains("duplicate") {
                return Err(err(StatusCode::CONFLICT, "Port or database name conflict, please retry"));
            }
            return Err(internal_error("create databases", e));
        }
    };

    // Call agent to create container. `stack_id` tells the agent to join the
    // stack's own private bridge network (already used to give the stack's other
    // containers name-based reachability) instead of the shared, ICC-disabled
    // `dockpanel-db` bridge every site-owned database uses — the stack network is
    // single-tenant by construction, so this introduces no new cross-tenant
    // exposure, and every panel-side query/backup/credential operation already
    // reaches the container via `docker exec`, which is unaffected by which
    // network the container is on.
    let agent_body = serde_json::json!({
        "name": body.name,
        "engine": engine,
        "password": password,
        "port": port,
        "stack_id": body.stack_id.map(|id| id.to_string()),
    });

    let result = match agent.post("/databases", Some(agent_body)).await {
        Ok(r) => r,
        Err(e) => {
            // Clean up the DB record + release the reserved quota slot if the agent fails.
            let _ = sqlx::query("DELETE FROM databases WHERE id = $1")
                .bind(db_record.id)
                .execute(&state.db)
                .await;
            if reserved_quota { release_reseller_db_slot(&state, claims.sub).await; }
            return Err(agent_error("Database creation", e));
        }
    };

    let container_id = result
        .get("container_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Update with the actual container_id
    sqlx::query("UPDATE databases SET container_id = $1 WHERE id = $2")
        .bind(&container_id)
        .bind(db_record.id)
        .execute(&state.db)
        .await
        .map_err(|e| internal_error("create databases", e))?;

    // (Reseller counter already incremented atomically by reserve_reseller_db_slot above.)

    tracing::info!("Database created: {} ({}, port {})", body.name, engine, port);

    Ok((StatusCode::CREATED, Json(db_record)))
}

/// GET /api/databases/{id}/credentials — Get database connection details.
pub async fn credentials(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Admin-widened to match create()/list() — see the v2.216.0 audit note on list().
    // LEFT JOIN both owner tables + COALESCE, same pattern as list() — see its comment.
    let row: Option<(String, String, String, Option<i32>, Option<String>)> = sqlx::query_as(
        "SELECT d.name, d.engine, d.db_password_enc, d.port, d.container_id \
         FROM databases d \
         LEFT JOIN sites s ON d.site_id = s.id \
         LEFT JOIN docker_stacks st ON d.stack_id = st.id \
         WHERE d.id = $1 AND (COALESCE(s.user_id, st.user_id) = $2 OR EXISTS (SELECT 1 FROM users u, servers sv \
         WHERE u.id = $2 AND u.role = 'admin' AND sv.id = COALESCE(s.server_id, st.server_id) \
         AND (sv.is_local OR sv.user_id = u.id)))",
    )
    .bind(id)
    .bind(claims.sub)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| internal_error("credentials", e))?;

    let (name, engine, password_enc, port, container_id) =
        row.ok_or_else(|| err(StatusCode::NOT_FOUND, "Database not found"))?;

    // Decrypt password (with legacy plaintext fallback for pre-encryption records)
    let password = crate::services::secrets_crypto::decrypt_credential_or_legacy(&password_enc, &state.config.jwt_secret);

    let host = container_id
        .as_deref()
        .map(|_| format!("dockpanel-db-{name}"))
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let port = port.unwrap_or(5432);

    let connection_string = match engine.as_str() {
        "mysql" | "mariadb" => format!("mysql://{name}:{password}@127.0.0.1:{port}/{name}"),
        _ => format!("postgresql://{name}:{password}@127.0.0.1:{port}/{name}"),
    };

    Ok(Json(serde_json::json!({
        "host": "127.0.0.1",
        "port": port,
        "database": name,
        "username": name,
        "password": password,
        "engine": engine,
        "connection_string": connection_string,
        "internal_host": host,
    })))
}

/// DELETE /api/databases/{id} — Delete a database and its container.
pub async fn remove(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Verify ownership through site — and take the host from the same row while we
    // are here. The join to `sites` was already carrying the ownership term; the
    // host it also knows about was thrown away, and the agent came from the request
    // header instead. That is a destructive mismatch in both directions: the panel
    // row is deleted whether or not the container was, so dispatching to the wrong
    // machine either destroys a same-named `dockpanel-db-{name}` container that
    // belongs to somebody else's tenant, or fails to find one while the real
    // container is orphaned on its own host with its row already gone.
    // Admin-widened to match create()/list() — see the v2.216.0 audit note on list().
    // LEFT JOIN both owner tables + COALESCE, same pattern as list() — see its comment.
    let db: Option<(Uuid, String, Option<String>, Option<Uuid>, String)> = sqlx::query_as(
        "SELECT d.id, d.name, d.container_id, COALESCE(s.server_id, st.server_id), \
         COALESCE(s.domain, st.name) FROM databases d \
         LEFT JOIN sites s ON d.site_id = s.id \
         LEFT JOIN docker_stacks st ON d.stack_id = st.id \
         WHERE d.id = $1 AND (COALESCE(s.user_id, st.user_id) = $2 OR EXISTS (SELECT 1 FROM users u, servers sv \
         WHERE u.id = $2 AND u.role = 'admin' AND sv.id = COALESCE(s.server_id, st.server_id) \
         AND (sv.is_local OR sv.user_id = u.id)))",
    )
    .bind(id)
    .bind(claims.sub)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| internal_error("remove databases", e))?;

    let (_, name, container_id, owner_server_id, owner_label) =
        db.ok_or_else(|| err(StatusCode::NOT_FOUND, "Database not found"))?;

    let agent =
        crate::helpers::agent_for_site_server(&state, owner_server_id, &owner_label).await?;

    // Remove container via agent (must succeed before DB deletion)
    if let Some(cid) = &container_id {
        let agent_path = format!("/databases/{cid}");
        agent.delete(&agent_path).await
            .map_err(|e| agent_error("Database removal", e))?;
    }

    // Delete from DB
    sqlx::query("DELETE FROM databases WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(|e| internal_error("remove databases", e))?;

    // Decrement reseller database counter
    let _ = sqlx::query(
        "UPDATE reseller_profiles SET used_databases = GREATEST(used_databases - 1, 0), updated_at = NOW() \
         WHERE user_id = (SELECT reseller_id FROM users WHERE id = $1 AND reseller_id IS NOT NULL)"
    ).bind(claims.sub).execute(&state.db).await;

    purge_dumps_if_unclaimed(&state, &agent, &name, owner_server_id).await;

    tracing::info!("Database deleted: {name}");

    Ok(Json(serde_json::json!({ "ok": true, "name": name })))
}

/// Remove a database's dump directory, but only once nothing else can claim it.
///
/// ⛔ ORDERING IS THE ONLY GUARD, and getting it wrong is silent. The `shared`
/// probe has no self-exclusion term, so every row still present answers the
/// question about ITSELF: call this before the row is gone and `shared` is
/// always true, nothing is ever purged, and the log line reads exactly like
/// correct behaviour. Call it only AFTER the row (or the cascade that takes it)
/// has committed.
pub(crate) async fn purge_dumps_if_unclaimed(
    state: &AppState,
    agent: &crate::services::agent::AgentHandle,
    name: &str,
    site_server_id: Option<Uuid>,
) {
    let name = name.to_string();
    // Take the dumps with it — but only once nothing else is using them.
    //
    // The dump directory is named for the DATABASE and nothing else, while the
    // name is unique only per SITE, so two live databases belonging to two
    // different sites on this host share one directory. Purging on the first
    // delete would take the survivor's backups with it, which is a worse failure
    // than the leak being closed and would be discovered at a restore.
    //
    // ⚠ A row whose site names no server is counted as possibly-here rather than
    // elsewhere. It cannot be acted on through the panel — the agent lookup above
    // refuses it — but its dumps, if any, are on some host, and this host is one
    // of the candidates. Doubt keeps the files; only positive evidence that
    // nothing else claims the name removes them. The reverse default trades a
    // disclosed leak for silent data loss.
    let shared: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM databases d \
         LEFT JOIN sites s ON s.id = d.site_id \
         LEFT JOIN docker_stacks st ON st.id = d.stack_id \
         WHERE d.name = $1 AND (COALESCE(s.server_id, st.server_id) = $2 \
         OR COALESCE(s.server_id, st.server_id) IS NULL))",
    )
    .bind(&name)
    .bind(site_server_id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(true);

    if shared {
        tracing::info!(
            "Keeping the dump directory for {name}: another database on this server still uses that name"
        );
    } else {
        match agent.delete(&format!("/db-backups/{name}")).await {
            Ok(v) => tracing::info!(
                "Purged dump directory for {name}: {} file(s) removed, {} kept, directory removed: {}",
                v.get("removed").and_then(|x| x.as_u64()).unwrap_or(0),
                v.get("kept").and_then(|x| x.as_u64()).unwrap_or(0),
                v.get("dir_removed").and_then(|x| x.as_bool()).unwrap_or(false),
            ),
            // Best-effort on purpose: the database IS deleted by now, and failing
            // the request over leftover files would report a destructive operation
            // that succeeded as though it had not.
            Err(e) => tracing::warn!("Could not purge dump directory for {name}: {e}"),
        }
    }

}


/// Helper: fetch database info (name, engine, password, port) — and the id of the server
/// the owning site actually runs on — with an ownership check.
///
/// The join to `sites` was here all along for the ownership term, and `sites` is where the
/// authoritative `server_id` lives; the column simply was not selected. So every caller
/// below answered WHICH DATABASE from this row and then took WHICH HOST from `ServerScope`
/// — an `X-Server-Id` request header that falls back to the LOCAL agent whenever the caller
/// owns no `servers` row. No non-admin ever owns one (the only INSERT is admin-gated), so
/// for a tenant whose database lives on a fleet member that fallback was always the wrong
/// machine, and nothing in the response said so.
///
/// What that leaked matters as much as where it dispatched: this helper returns the
/// DECRYPTED database password, and every caller forwards it to the agent it chose, which
/// spends it on a `docker exec -e MYSQL_PWD=…` child process. A misdispatch therefore
/// handed one tenant's live credential to a host their database does not run on, in the
/// argv/environment of a process on that host — a disclosure that happened on ordinary
/// read-only calls like listing tables, with no error for anyone to notice.
///
/// The row is the authority for the host exactly as it is for the ownership check, so the
/// server id now comes back in the same tuple and each caller resolves its handle from it
/// via `agent_for_site_server`, which REFUSES rather than substituting a reachable host.
/// That helper's third argument only labels its refusal log line; no domain is loaded on
/// these paths, so callers identify the target by database name.
///
/// `Option<Uuid>` because the column is `NOT NULL` in the schema but optional in the
/// structs that predate the fleet work — a `None` means a row older than the backfill, and
/// guessing a host for it is the thing that must not happen.
async fn get_db_info(
    state: &AppState,
    id: Uuid,
    user_id: Uuid,
) -> Result<(String, String, String, i32, Option<Uuid>), ApiError> {
    // Admin-widened to match create()/list() — see the v2.216.0 audit note on list().
    // This is the shared chokepoint for tables/table_schema/query/table_indexes/
    // foreign_keys/schema_overview/update_pitr_config/pitr_restore/reset_password/
    // dumps/import — one fix here covers all 11 callers.
    // LEFT JOIN both owner tables + COALESCE, same pattern as list() — see its comment.
    let row: Option<(String, String, String, Option<i32>, Option<String>, Option<Uuid>)> = sqlx::query_as(
        "SELECT d.name, d.engine, d.db_password_enc, d.port, d.container_id, \
         COALESCE(s.server_id, st.server_id) \
         FROM databases d \
         LEFT JOIN sites s ON d.site_id = s.id \
         LEFT JOIN docker_stacks st ON d.stack_id = st.id \
         WHERE d.id = $1 AND (COALESCE(s.user_id, st.user_id) = $2 OR EXISTS (SELECT 1 FROM users u, servers sv \
         WHERE u.id = $2 AND u.role = 'admin' AND sv.id = COALESCE(s.server_id, st.server_id) \
         AND (sv.is_local OR sv.user_id = u.id)))",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| internal_error("remove databases", e))?;

    let (name, engine, password_enc, port, container_id, site_server_id) =
        row.ok_or_else(|| err(StatusCode::NOT_FOUND, "Database not found"))?;
    // Security (H1, v2.19.0): every exec/reset path resolves the target container by
    // the (per-site, NON-unique) name `dockpanel-db-{name}`. create() inserts the row
    // with an EMPTY container_id and only fills it after the agent confirms the
    // container was created; on a global Docker name collision the row is deleted, but
    // during the agent round-trip a tenant's transient colliding-name row is visible.
    // Docker's global name uniqueness guarantees a row with a NON-EMPTY container_id
    // owns its uniquely-named container, so refusing to operate on an empty container_id
    // closes the cross-tenant reset/query race for every get_db_info caller at once.
    if container_id.as_deref().unwrap_or("").is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "Database not found"));
    }
    let port = port.unwrap_or(5432);
    // Decrypt password for agent use (with legacy plaintext fallback)
    let password = crate::services::secrets_crypto::decrypt_credential_or_legacy(&password_enc, &state.config.jwt_secret);
    Ok((name, engine, password, port, site_server_id))
}

/// GET /api/databases/{id}/tables — List tables in the database.
pub async fn tables(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (name, engine, password, _port, site_server_id) =
        get_db_info(&state, id, claims.sub).await?;
    let agent = crate::helpers::agent_for_site_server(&state, site_server_id, &name).await?;

    let sql = match engine.as_str() {
        "mysql" | "mariadb" => {
            "SELECT table_name, table_type, table_rows, \
             ROUND((data_length + index_length) / 1024, 1) AS size_kb \
             FROM information_schema.tables WHERE table_schema = DATABASE() \
             ORDER BY table_name"
                .to_string()
        }
        _ => {
            "SELECT t.table_name, t.table_type, \
             pg_catalog.pg_size_pretty(pg_catalog.pg_total_relation_size(quote_ident(t.table_name))) AS size \
             FROM information_schema.tables t \
             WHERE t.table_schema = 'public' ORDER BY t.table_name"
                .to_string()
        }
    };

    let container = format!("dockpanel-db-{name}");
    let agent_body = serde_json::json!({
        "container": container,
        "engine": engine,
        "user": name,
        "password": password,
        "database": name,
        "sql": sql,
    });

    agent
        .post("/databases/query", Some(agent_body))
        .await
        .map(Json)
        .map_err(sql_error)
}

/// The one charset any table name is interpolated under.
///
/// `/databases/query` has no bind-parameter channel — the agent's `QueryDbRequest`
/// carries `sql` and nothing else, and the statement is handed verbatim to
/// `mariadb -e` / `psql -c`. So every table name that reaches a statement is
/// interpolated, and this predicate is the only thing standing between a path
/// segment and that statement. It admits no quote, no backslash and no shell
/// metacharacter, which is what makes `'{table}'` safe to build.
///
/// It lives here as ONE function on purpose. There were two copies with two
/// different charsets — the interpolating one refused `-`, the other allowed it —
/// and a divergence between a validator and the statement it guards is the bug
/// this file already shipped once.
fn is_safe_table_name(table: &str) -> bool {
    !table.is_empty()
        && table.len() <= 128
        && table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// GET /api/databases/{id}/tables/{table} — Get table schema (columns, types).
pub async fn table_schema(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path((id, table)): Path<(Uuid, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !is_safe_table_name(&table) {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid table name"));
    }

    let (name, engine, password, _port, site_server_id) =
        get_db_info(&state, id, claims.sub).await?;
    let agent = crate::helpers::agent_for_site_server(&state, site_server_id, &name).await?;

    // Interpolated, not bound. `29d959b` rewrote these two statements to use `?`/`$1`
    // for injection safety and sent the values under a `"params"` key — but the agent
    // has never had a `params` field to receive them, so serde dropped it and the
    // placeholder reached the engine unbound. Every column listing 400'd from
    // 2026-03-23 until this fix, invisibly, because the caller swallowed the error.
    // The sibling `table_indexes` below always interpolated and always worked; this
    // now matches it, which also keeps the fix backend-only — a bind channel would
    // need the agent to ship in lockstep, and an old agent would still fail.
    let sql = match engine.as_str() {
        "mysql" | "mariadb" => format!(
            "SELECT column_name, column_type, is_nullable, column_default, column_key, extra \
             FROM information_schema.columns \
             WHERE table_schema = DATABASE() AND table_name = '{table}' \
             ORDER BY ordinal_position"
        ),
        _ => format!(
            "SELECT column_name, data_type, character_maximum_length, is_nullable, column_default \
             FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = '{table}' \
             ORDER BY ordinal_position"
        ),
    };

    let container = format!("dockpanel-db-{name}");
    let agent_body = serde_json::json!({
        "container": container,
        "engine": engine,
        "user": name,
        "password": password,
        "database": name,
        "sql": sql,
    });

    agent
        .post("/databases/query", Some(agent_body))
        .await
        .map(Json)
        .map_err(sql_error)
}

#[derive(serde::Deserialize)]
pub struct SqlQueryRequest {
    pub sql: String,
}

/// POST /api/databases/{id}/query — Execute a SQL query.
pub async fn query(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SqlQueryRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if body.sql.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "Query is empty"));
    }
    if body.sql.len() > 10_000 {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Query too long (max 10KB)",
        ));
    }

    let (name, engine, password, _port, site_server_id) =
        get_db_info(&state, id, claims.sub).await?;
    // The host comes from the site's row, never from the caller's server switcher. This
    // request body carries the tenant's DECRYPTED database password to the agent, which
    // puts it in a `docker exec -e MYSQL_PWD=…` child process; sent to a machine the
    // database does not run on, the credential is disclosed to that host — and the query
    // either runs against a same-named container belonging to someone else or fails with a
    // "not found" that reads like an ordinary error.
    let agent = crate::helpers::agent_for_site_server(&state, site_server_id, &name).await?;

    let container = format!("dockpanel-db-{name}");
    let agent_body = serde_json::json!({
        "container": container,
        "engine": engine,
        "user": name,
        "password": password,
        "database": name,
        "sql": body.sql,
    });

    agent
        .post("/databases/query", Some(agent_body))
        .await
        .map(Json)
        .map_err(sql_error)
}

/// GET /api/databases/{id}/indexes/{table} — Get indexes for a table.
pub async fn table_indexes(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path((id, table)): Path<(Uuid, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !is_safe_table_name(&table) {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid table name"));
    }

    let (name, engine, password, _port, site_server_id) =
        get_db_info(&state, id, claims.sub).await?;
    let agent = crate::helpers::agent_for_site_server(&state, site_server_id, &name).await?;

    let sql = match engine.as_str() {
        "mysql" | "mariadb" => format!(
            "SELECT index_name, GROUP_CONCAT(column_name ORDER BY seq_in_index) AS columns, \
             non_unique, index_type \
             FROM information_schema.statistics \
             WHERE table_schema = DATABASE() AND table_name = '{}' \
             GROUP BY index_name, non_unique, index_type \
             ORDER BY index_name", table
        ),
        _ => format!(
            "SELECT indexname AS index_name, indexdef AS definition \
             FROM pg_indexes \
             WHERE schemaname = 'public' AND tablename = '{}' \
             ORDER BY indexname", table
        ),
    };

    let container = format!("dockpanel-db-{name}");
    let agent_body = serde_json::json!({
        "container": container,
        "engine": engine,
        "user": name,
        "password": password,
        "database": name,
        "sql": sql,
    });

    agent.post("/databases/query", Some(agent_body)).await.map(Json).map_err(sql_error)
}

/// GET /api/databases/{id}/foreign-keys — Get all foreign key relationships.
pub async fn foreign_keys(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (name, engine, password, _port, site_server_id) =
        get_db_info(&state, id, claims.sub).await?;
    let agent = crate::helpers::agent_for_site_server(&state, site_server_id, &name).await?;

    let sql = match engine.as_str() {
        "mysql" | "mariadb" =>
            "SELECT kcu.table_name AS source_table, kcu.column_name AS source_column, \
             kcu.referenced_table_name AS target_table, kcu.referenced_column_name AS target_column, \
             rc.constraint_name, rc.update_rule, rc.delete_rule \
             FROM information_schema.key_column_usage kcu \
             JOIN information_schema.referential_constraints rc \
               ON kcu.constraint_name = rc.constraint_name AND kcu.constraint_schema = rc.constraint_schema \
             WHERE kcu.table_schema = DATABASE() AND kcu.referenced_table_name IS NOT NULL \
             ORDER BY kcu.table_name, kcu.column_name".to_string(),
        _ =>
            "SELECT \
               tc.table_name AS source_table, \
               kcu.column_name AS source_column, \
               ccu.table_name AS target_table, \
               ccu.column_name AS target_column, \
               tc.constraint_name, \
               rc.update_rule, rc.delete_rule \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu ON tc.constraint_name = kcu.constraint_name \
             JOIN information_schema.constraint_column_usage ccu ON tc.constraint_name = ccu.constraint_name \
             JOIN information_schema.referential_constraints rc ON tc.constraint_name = rc.constraint_name \
             WHERE tc.constraint_type = 'FOREIGN KEY' AND tc.table_schema = 'public' \
             ORDER BY tc.table_name, kcu.column_name".to_string(),
    };

    let container = format!("dockpanel-db-{name}");
    let agent_body = serde_json::json!({
        "container": container,
        "engine": engine,
        "user": name,
        "password": password,
        "database": name,
        "sql": sql,
    });

    agent.post("/databases/query", Some(agent_body)).await.map(Json).map_err(sql_error)
}

/// GET /api/databases/{id}/schema-overview — Full schema overview for visual browser.
/// Returns tables, columns, indexes, and foreign keys in a single call.
pub async fn schema_overview(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (name, engine, password, _port, site_server_id) =
        get_db_info(&state, id, claims.sub).await?;
    // Both halves of the join below go through this one handle, so it has to be the
    // site's own host — otherwise the two concurrent queries carry the decrypted
    // password to a foreign machine twice over.
    let agent = crate::helpers::agent_for_site_server(&state, site_server_id, &name).await?;
    let container = format!("dockpanel-db-{name}");

    // Execute all queries concurrently
    let tables_sql = match engine.as_str() {
        "mysql" | "mariadb" =>
            "SELECT table_name, table_type, table_rows, \
             ROUND((data_length + index_length) / 1024, 1) AS size_kb \
             FROM information_schema.tables WHERE table_schema = DATABASE() \
             ORDER BY table_name".to_string(),
        _ =>
            "SELECT t.table_name, t.table_type, \
             pg_stat_user_tables.n_live_tup AS table_rows, \
             pg_total_relation_size(quote_ident(t.table_name))/1024 AS size_kb \
             FROM information_schema.tables t \
             LEFT JOIN pg_stat_user_tables ON pg_stat_user_tables.relname = t.table_name \
             WHERE t.table_schema = 'public' ORDER BY t.table_name".to_string(),
    };

    let fk_sql = match engine.as_str() {
        "mysql" | "mariadb" =>
            "SELECT kcu.table_name AS source_table, kcu.column_name AS source_column, \
             kcu.referenced_table_name AS target_table, kcu.referenced_column_name AS target_column, \
             rc.constraint_name \
             FROM information_schema.key_column_usage kcu \
             JOIN information_schema.referential_constraints rc \
               ON kcu.constraint_name = rc.constraint_name AND kcu.constraint_schema = rc.constraint_schema \
             WHERE kcu.table_schema = DATABASE() AND kcu.referenced_table_name IS NOT NULL".to_string(),
        _ =>
            "SELECT tc.table_name AS source_table, kcu.column_name AS source_column, \
             ccu.table_name AS target_table, ccu.column_name AS target_column, \
             tc.constraint_name \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu ON tc.constraint_name = kcu.constraint_name \
             JOIN information_schema.constraint_column_usage ccu ON tc.constraint_name = ccu.constraint_name \
             WHERE tc.constraint_type = 'FOREIGN KEY' AND tc.table_schema = 'public'".to_string(),
    };

    let (tables_res, fk_res) = tokio::join!(
        agent.post("/databases/query", Some(serde_json::json!({
            "container": &container, "engine": &engine, "user": &name,
            "password": &password, "database": &name, "sql": &tables_sql,
        }))),
        agent.post("/databases/query", Some(serde_json::json!({
            "container": &container, "engine": &engine, "user": &name,
            "password": &password, "database": &name, "sql": &fk_sql,
        })))
    );

    let tables = tables_res.map_err(sql_error)?;
    let foreign_keys = fk_res.unwrap_or_else(|_| serde_json::json!({"columns":[],"rows":[]}));

    Ok(Json(serde_json::json!({
        "tables": tables,
        "foreign_keys": foreign_keys,
        "engine": engine,
    })))
}

/// Atomically reserve one database slot against the caller's reseller quota.
/// Returns Ok(true) if a slot was reserved (caller must release it on a later
/// failure), Ok(false) if the user has no reseller (nothing to enforce/release),
/// or Err(403) if the reseller's database quota is exhausted. The conditional
/// UPDATE ... RETURNING makes the quota check and the increment a single atomic
/// statement, closing the check-then-increment TOCTOU.
pub(crate) async fn reserve_reseller_db_slot(state: &AppState, user_id: Uuid) -> Result<bool, ApiError> {
    let reserved: Option<i32> = sqlx::query_scalar(
        "UPDATE reseller_profiles SET used_databases = used_databases + 1, updated_at = NOW() \
         WHERE user_id = (SELECT reseller_id FROM users WHERE id = $1 AND reseller_id IS NOT NULL) \
           AND (max_databases IS NULL OR used_databases < max_databases) \
         RETURNING 1",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| internal_error("reserve reseller quota", e))?;

    if reserved.is_some() {
        return Ok(true); // slot reserved + counter incremented atomically
    }

    // 0 rows updated: either the user has no reseller/profile (nothing to enforce),
    // or the profile exists but the quota condition failed (quota exhausted).
    let has_profile: Option<(Uuid,)> = sqlx::query_as(
        "SELECT rp.user_id FROM reseller_profiles rp \
         WHERE rp.user_id = (SELECT reseller_id FROM users WHERE id = $1 AND reseller_id IS NOT NULL)",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| internal_error("reserve reseller quota", e))?;

    if has_profile.is_some() {
        return Err(err(StatusCode::FORBIDDEN, "Reseller database quota exceeded"));
    }
    Ok(false) // no reseller → nothing reserved
}

/// Release a reserved reseller database slot (compensating decrement) when a
/// create fails after `reserve_reseller_db_slot` succeeded.
pub(crate) async fn release_reseller_db_slot(state: &AppState, user_id: Uuid) {
    let _ = sqlx::query(
        "UPDATE reseller_profiles SET used_databases = GREATEST(used_databases - 1, 0), updated_at = NOW() \
         WHERE user_id = (SELECT reseller_id FROM users WHERE id = $1 AND reseller_id IS NOT NULL)",
    )
    .bind(user_id)
    .execute(&state.db)
    .await;
}

/// Find an available port using a single SQL query to find the first gap.
pub(crate) async fn find_available_port(state: &AppState, engine: &str) -> Result<i32, ApiError> {
    // Choose port range based on engine. Ranges are intentionally wide so that the
    // per-account cap (MAX_DATABASES_PER_USER) leaves ample headroom and one tenant
    // cannot exhaust the shared, host-wide pool.
    let (range_start, range_end) = match engine {
        "mysql" | "mariadb" => (3307, 3600),
        _ => (5433, 5700),
    };

    // Find first unused port in range with a single query
    let row: Option<(i32,)> = sqlx::query_as(
        "SELECT s.port FROM generate_series($1::int, $2::int) AS s(port) \
         WHERE s.port NOT IN (SELECT port FROM databases WHERE port IS NOT NULL) \
         LIMIT 1"
    )
    .bind(range_start)
    .bind(range_end)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| internal_error("query", e))?;

    row.map(|(p,)| p).ok_or_else(|| err(
        StatusCode::INTERNAL_SERVER_ERROR,
        "No available ports for database",
    ))
}

pub(crate) struct SiteMariaDbCredentials {
    pub db_id: Uuid,
    pub db_name: String,
    pub db_user: String,
    pub db_pass: String,
    pub db_host: String,
    pub container_id: Option<String>,
    pub reserved_quota: bool,
    pub newly_created: bool,
}

/// Sanitize a domain into a valid MySQL/PostgreSQL database identifier (max 63 chars).
fn sanitize_wp_db_name(domain: &str) -> String {
    let mut base: String = domain
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if !base
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        base = format!("wp_{base}");
    }
    base.truncate(59);
    let candidate = if base.ends_with("_wp") {
        base
    } else {
        format!("{base}_wp")
    };
    candidate
}

/// Reuse an existing site MariaDB/MySQL database or provision a new MariaDB container for WordPress.
pub(crate) async fn ensure_site_mariadb(
    state: &AppState,
    agent: &crate::services::agent::AgentHandle,
    user_id: Uuid,
    site_id: Uuid,
    domain: &str,
) -> Result<SiteMariaDbCredentials, ApiError> {
    if let Some((db_id, db_name, db_user, db_password_enc, port)) = sqlx::query_as::<_, (Uuid, String, String, String, i32)>(
        "SELECT id, name, db_user, db_password_enc, port \
         FROM databases \
         WHERE site_id = $1 AND engine IN ('mariadb', 'mysql') AND port IS NOT NULL \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(site_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| internal_error("lookup site database", e))?
    {
        let db_pass = crate::services::secrets_crypto::decrypt_credential_or_legacy(
            &db_password_enc,
            &state.config.jwt_secret,
        );
        return Ok(SiteMariaDbCredentials {
            db_id,
            db_name,
            db_user,
            db_pass,
            db_host: format!("127.0.0.1:{port}"),
            container_id: None,
            reserved_quota: false,
            newly_created: false,
        });
    }

    let (db_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM databases d \
         LEFT JOIN sites s ON d.site_id = s.id \
         LEFT JOIN docker_stacks st ON d.stack_id = st.id \
         WHERE COALESCE(s.user_id, st.user_id) = $1",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| internal_error("count databases", e))?;
    if db_count >= MAX_DATABASES_PER_USER {
        return Err(err(
            StatusCode::FORBIDDEN,
            &format!("Database limit reached ({MAX_DATABASES_PER_USER} per account)"),
        ));
    }

    let mut db_name = sanitize_wp_db_name(domain);
    let taken: Option<(i32,)> = sqlx::query_as("SELECT 1 FROM databases WHERE name = $1 LIMIT 1")
        .bind(&db_name)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| internal_error("check database name", e))?;
    if taken.is_some() || is_reserved_db_name(&db_name) {
        let suffix = &Uuid::new_v4().to_string().replace('-', "")[..8];
        let max_prefix = 63usize.saturating_sub(suffix.len() + 1);
        let mut prefix = sanitize_wp_db_name(domain);
        prefix.truncate(max_prefix);
        db_name = format!("{prefix}_{suffix}");
    }

    let password = Uuid::new_v4().to_string().replace('-', "");
    let port = find_available_port(state, "mariadb").await?;
    let encrypted_password =
        crate::services::secrets_crypto::encrypt_credential(&password, &state.config.jwt_secret)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &format!("Encryption failed: {e}")))?;

    let reserved_quota = reserve_reseller_db_slot(state, user_id).await?;
    let db_id: Uuid = match sqlx::query_scalar(
        "INSERT INTO databases (site_id, stack_id, name, engine, db_user, db_password_enc, container_id, port) \
         VALUES ($1, NULL, $2, 'mariadb', $3, $4, '', $5) RETURNING id",
    )
    .bind(site_id)
    .bind(&db_name)
    .bind(&db_name)
    .bind(&encrypted_password)
    .bind(port)
    .fetch_one(&state.db)
    .await
    {
        Ok(id) => id,
        Err(e) => {
            if reserved_quota {
                release_reseller_db_slot(state, user_id).await;
            }
            return Err(internal_error("insert site database", e));
        }
    };

    let agent_body = serde_json::json!({
        "name": db_name,
        "engine": "mariadb",
        "password": password,
        "port": port,
    });
    let result = match agent.post("/databases", Some(agent_body)).await {
        Ok(r) => r,
        Err(e) => {
            let _ = sqlx::query("DELETE FROM databases WHERE id = $1")
                .bind(db_id)
                .execute(&state.db)
                .await;
            if reserved_quota {
                release_reseller_db_slot(state, user_id).await;
            }
            return Err(agent_error("Database creation", e));
        }
    };

    let container_id = result
        .get("container_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let _ = sqlx::query("UPDATE databases SET container_id = $1 WHERE id = $2")
        .bind(&container_id)
        .bind(db_id)
        .execute(&state.db)
        .await;

    tracing::info!("Auto-provisioned MariaDB {db_name} (port {port}) for WordPress install on {domain}");

    Ok(SiteMariaDbCredentials {
        db_id,
        db_name: db_name.clone(),
        db_user: db_name,
        db_pass: password,
        db_host: format!("127.0.0.1:{port}"),
        container_id: Some(container_id),
        reserved_quota,
        newly_created: true,
    })
}

/// Remove a newly provisioned MariaDB database if WordPress installation fails before completion.
pub(crate) async fn cleanup_provisioned_db(
    state: &AppState,
    agent: &crate::services::agent::AgentHandle,
    user_id: Uuid,
    creds: &SiteMariaDbCredentials,
) {
    if !creds.newly_created {
        return;
    }
    if let Some(ref cid) = creds.container_id {
        if !cid.is_empty() {
            let _ = agent.delete(&format!("/databases/{cid}")).await;
        }
    }
    let _ = sqlx::query("DELETE FROM databases WHERE id = $1")
        .bind(creds.db_id)
        .execute(&state.db)
        .await;
    if creds.reserved_quota {
        release_reseller_db_slot(state, user_id).await;
    }
}

// ─── Point-in-Time Recovery (PITR) ─────────────────────────────

#[derive(serde::Deserialize)]
pub struct PitrConfigRequest {
    pub pitr_enabled: Option<bool>,
    pub retention_hours: Option<i32>,
}

/// GET /api/databases/{id}/pitr — Get PITR configuration.
pub async fn pitr_config(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Verify ownership — admin-widened to match create()/list() (v2.216.0 audit note).
    // Owner is either a site or a stack (chk_databases_owner enforces exactly one).
    let _db: (String,) = sqlx::query_as(
        "SELECT name FROM databases WHERE id = $1 AND (\
         site_id IN (SELECT s.id FROM sites s WHERE s.user_id = $2 OR EXISTS \
         (SELECT 1 FROM users u, servers sv WHERE u.id = $2 AND u.role = 'admin' \
         AND sv.id = s.server_id AND (sv.is_local OR sv.user_id = u.id))) \
         OR stack_id IN (SELECT st.id FROM docker_stacks st WHERE st.user_id = $2 OR EXISTS \
         (SELECT 1 FROM users u, servers sv WHERE u.id = $2 AND u.role = 'admin' \
         AND sv.id = st.server_id AND (sv.is_local OR sv.user_id = u.id))))"
    )
        .bind(id).bind(claims.sub)
        .fetch_optional(&state.db).await
        .map_err(|e| internal_error("pitr config", e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Database not found"))?;

    let config: Option<(bool, i32, Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>, i64)> =
        sqlx::query_as(
            "SELECT pitr_enabled, retention_hours, last_backup_at, last_wal_at, backup_size_bytes \
             FROM db_pitr_config WHERE database_id = $1"
        )
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| internal_error("pitr config", e))?;

    match config {
        Some((enabled, hours, last_backup, last_wal, size)) => {
            Ok(Json(serde_json::json!({
                "pitr_enabled": enabled,
                "retention_hours": hours,
                "last_backup_at": last_backup,
                "last_wal_at": last_wal,
                "backup_size_bytes": size,
            })))
        }
        None => Ok(Json(serde_json::json!({
            "pitr_enabled": false,
            "retention_hours": 24,
        }))),
    }
}

/// PUT /api/databases/{id}/pitr — Enable/configure PITR.
pub async fn update_pitr_config(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    _scope: ServerScope,
    Json(body): Json<PitrConfigRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Ownership + container-readiness check (get_db_info also enforces the H1 guard).
    // No agent call on this path — the handler only persists the intent flag (see the
    // note below), so the server id this now returns has nothing to dispatch.
    let (name, _engine, _password, _port, _site_server_id) =
        get_db_info(&state, id, claims.sub).await?;
    let enabled = body.pitr_enabled.unwrap_or(false);
    let hours = body.retention_hours.unwrap_or(24).max(1).min(720);

    // NOTE (v2.19.0): the previous implementation ran `ALTER SYSTEM SET archive_mode='on'`
    // on enable but the disable path had no inverse — so archiving was un-revertable through
    // the panel and grew WAL without bound after a container restart (the archive_command
    // targeted a directory that the accompanying mkdir/pg_basebackup never created, because
    // those posted to a non-existent agent `/exec` route and were swallowed with `let _ =`).
    // The harmful/dead container mutation is removed; we persist only the intent flag until
    // real, symmetric WAL archiving is implemented (pitr_restore returns 501 until then).
    // Save config


    sqlx::query(
        "INSERT INTO db_pitr_config (database_id, pitr_enabled, retention_hours) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (database_id) DO UPDATE SET \
         pitr_enabled = $2, retention_hours = $3, updated_at = NOW()"
    )
    .bind(id)
    .bind(enabled)
    .bind(hours)
    .execute(&state.db)
    .await
    .map_err(|e| internal_error("update pitr config", e))?;

    crate::services::activity::log_activity(
        &state.db, claims.sub, &claims.email,
        if enabled { "database.pitr_enabled" } else { "database.pitr_disabled" },
        Some("database"), Some(&name), None, None, claims.key_id,
    ).await;

    Ok(Json(serde_json::json!({ "ok": true })))
}

/// POST /api/databases/{id}/pitr/restore — Restore to a specific point in time.
pub async fn pitr_restore(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    _scope: ServerScope,
    Json(_body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Ownership + container-readiness check (get_db_info also enforces the H1 guard).
    let _ = get_db_info(&state, id, claims.sub).await?;

    // Honesty (v2.19.0): point-in-time restore was never functional and used to LIE.
    // The postgres branch only wrote a WAL restore-point marker (pg_create_restore_point
    // performs NO rollback) then returned {ok:true, restored_to:...}; the mysql branch +
    // the WAL-archive setup posted to a non-existent agent `/exec` route so nothing ran.
    // Return an explicit 501 instead of a false success until real PITR is implemented
    // (this also retires the dead mysqlbinlog shell string that only reached the 404 route).
    Err(err(
        StatusCode::NOT_IMPLEMENTED,
        "Point-in-time restore is not yet implemented",
    ))
}

/// POST /api/databases/{id}/reset-password — Reset the database password.
/// Generates a new random password, updates the database container via the agent,
/// then stores the encrypted password in the panel DB and returns it to the user.
pub async fn reset_password(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Fetch current db info with ownership check (returns decrypted old password)
    let (name, engine, old_password, _port, site_server_id) =
        get_db_info(&state, id, claims.sub).await?;

    // The host MUST come from the site's row here, and this is the handler where taking
    // it from the request header did the most damage. Container names are
    // `dockpanel-db-{name}` and are unique only per host, so a request dispatched to the
    // wrong machine acts on ANY same-named container living there, for a tenant who has
    // nothing to do with the caller. `ServerScope` made that the DEFAULT rather than the
    // edge case: a non-admin owns no `servers` row, so their scope always fell back to
    // the local agent even when their database lives on a fleet member. The panel row is
    // then updated with the new password regardless, so the real database is left with a
    // credential the panel no longer knows.
    //
    // The agent's mysql/mariadb branch used to reach the account as container-root and
    // ignore the `old_password` this body sends. That never worked — the branch could not
    // authenticate at all — and it now authenticates AS THE TENANT with this password, so
    // the value below is load-bearing rather than decorative. See the note on
    // `services::database::reset_password` in the agent crate for the measurement.
    let agent = crate::helpers::agent_for_site_server(&state, site_server_id, &name).await?;

    // Generate a new random password (same pattern as create())
    let new_password = uuid::Uuid::new_v4().to_string().replace('-', "");

    // Reset password in the actual database container via the agent
    let container = format!("dockpanel-db-{name}");
    let agent_body = serde_json::json!({
        "container": container,
        "engine": engine,
        "user": name,
        "old_password": old_password,
        "new_password": new_password,
    });

    agent
        .post("/databases/reset-password", Some(agent_body))
        .await
        .map_err(|e| agent_error("Database password reset", e))?;

    // Encrypt the new password
    let encrypted = crate::services::secrets_crypto::encrypt_credential(&new_password, &state.config.jwt_secret)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &format!("Encryption failed: {e}")))?;

    // Update the encrypted password in the panel database
    sqlx::query("UPDATE databases SET db_password_enc = $1 WHERE id = $2")
        .bind(&encrypted)
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(|e| internal_error("reset password", e))?;

    // Log activity
    let ip = crate::routes::client_ip(&headers);
    crate::services::activity::log_activity(
        &state.db, claims.sub, &claims.email, "database.password_reset",
        Some("database"), Some(&name), None, ip.as_deref(), claims.key_id,
    ).await;
    crate::services::security_hardening::audit_log(
        &state.db,
        "database.password_reset",
        Some(&claims.email),
        ip.as_deref(),
        Some("database"),
        Some(&name),
        None,
        None,
        "warning",
        claims.key_id,
    ).await;

    tracing::info!("Database password reset: {name}");

    Ok(Json(serde_json::json!({
        "ok": true,
        "password": new_password,
    })))
}

/// GET /api/databases/{id}/dumps — What is sitting in this database's dump directory.
///
/// The intake pattern here is the migration wizard's, not the file manager's: the
/// operator puts the dump on the server themselves and the panel takes it by NAME. That
/// is deliberate. A dump travels as base64-in-JSON through two axum services with a 2 MiB
/// body limit each, so the file manager tops out around 1.5 MB — and a `.sql` under
/// 1.5 MB is close to a rounding error. `Migration.tsx` has told operators to SFTP the
/// archive up and hand the panel a path since the wizard shipped, and a 40 GB cPanel
/// backup already moves through this product that way.
///
/// ⛔ ADMIN ONLY, and this is a security boundary rather than a tidiness rule. The
/// directory is keyed by DATABASE NAME, and a name is reusable. Deleting a database now
/// also purges that directory — but only when no surviving row on the same host still
/// answers to the name, so the door below still has to assume leftovers exist: dumps
/// written before this behaviour shipped are still there, and a shared name keeps them
/// deliberately. `databases.name` is unique only per SITE (the global
/// constraint was dropped in `20260312000000_data_integrity.sql`). So a tenant who
/// creates a database with a name a previous tenant used would, under a
/// tenant-reachable listing, see and be able to import that tenant's leftover dumps —
/// which are unencrypted for ad-hoc backups. Row-scoping (what
/// `/api/backup-orchestrator/db-backups` does) cannot fix it, because the whole point of
/// this door is files that have no row. `require_admin` can, and costs nothing real:
/// putting a file in this directory needs root on the box, there is no per-site SFTP,
/// and an admin can already read `/var/backups` directly.
pub async fn dumps(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (name, engine, _password, _port, site_server_id) =
        get_db_info(&state, id, claims.sub).await?;
    let agent = crate::helpers::agent_for_site_server(&state, site_server_id, &name).await?;

    let listing = agent
        .get(&format!("/db-backups/{name}/list"))
        .await
        .map_err(|e| agent_error("Database dump listing", e))?;

    // The agent answers "is this a backup at all". Whether a given backup can go into
    // THIS database is a second question it cannot answer, because it does not know the
    // engine — so it is answered here and annotated onto each entry, and the import
    // handler enforces the identical rule below. Annotating without enforcing would be a
    // UI that merely asks nicely.
    let mut entries = match listing {
        serde_json::Value::Array(a) => a,
        // A shape we do not recognise is not an empty directory, and must not render as
        // one — the empty state is a positive claim about the disk.
        other => return Ok(Json(other)),
    };
    for entry in &mut entries {
        let filename = entry.get("filename").and_then(|f| f.as_str()).unwrap_or("").to_string();
        if entry.get("unsupported").and_then(|u| u.as_str()).is_some() {
            continue; // Already explained: not a backup file at all.
        }
        if let Some(reason) = import_blocked_reason(&filename, &engine) {
            entry["import_blocked"] = serde_json::Value::String(reason);
        }
    }

    Ok(Json(serde_json::Value::Array(entries)))
}

/// Why a real backup still cannot be imported into THIS database, if it cannot.
///
/// Split out as a pure function so the boundary is judgeable with no request, no agent
/// and no database — the `exceeds_upload_limit` shape from v2.137.0 — and so the
/// listing and the import door cannot drift apart: they are the same call.
fn import_blocked_reason(filename: &str, engine: &str) -> Option<String> {
    if filename.ends_with(".enc") {
        // Not an operator mistake. This is a backup the PANEL took and encrypted with a
        // key derived from the panel's own secret; the import door has no key to offer,
        // so the agent would answer "Encryption key required" every single time. Naming
        // the door that CAN open it is the difference between a dead end and an answer.
        return Some(
            "Encrypted panel backup. Restore it from Backup Manager, which holds the key."
                .to_string(),
        );
    }
    // `.archive.gz` is MongoDB's dump format — `dump_mongo` is its only producer. Fed to
    // a postgres or mariadb restore it becomes a BSON stream piped into `psql`, which
    // fails deep inside the pipe on content rather than at the door.
    let is_mongo = matches!(engine, "mongo" | "mongodb");
    if filename.ends_with(".archive.gz") && !is_mongo {
        return Some(format!(
            "MongoDB archive format. This database is {engine}, which reads .sql.gz."
        ));
    }
    if filename.ends_with(".sql.gz") && is_mongo {
        return Some("SQL dump. This database is MongoDB, which reads .archive.gz.".to_string());
    }
    None
}

#[derive(serde::Deserialize)]
pub struct ImportDbRequest {
    #[serde(alias = "file_name", alias = "file", alias = "dump")]
    pub filename: String,
}

/// POST /api/databases/{id}/import — Import a dump the operator placed on the server.
///
/// Answers the half of issue #121 that v2.137.0 left open. The importer itself is not new
/// and is not written here: the agent's `POST /db-backups/{db}/restore/{filename}` already
/// pins the path per database, validates every identifier it puts on an argv, passes the
/// password by environment rather than command line, and reads the child's exit status
/// instead of treating EOF as success. What did not exist was a door onto it for a dump the
/// operator brought themselves — only for one the panel had taken.
///
/// ⛔ Admin only, for the reason documented on [`dumps`] above.
pub async fn import(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    headers: axum::http::HeaderMap,
    Json(body): Json<ImportDbRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let filename = body.filename.trim();
    // The agent's `is_safe_filename` is the real gate, but this name is about to be
    // interpolated into an HTTP request line, and a character that is merely UNUSUAL
    // rather than dangerous breaks that far from anything that can explain it: a single
    // space makes `Request::builder().uri(...)` fail, which surfaced as a transport error
    // and not as "that filename cannot be used". So the charset is checked HERE, where
    // the answer can still be a sentence about the file.
    let charset_ok = !filename.is_empty()
        && filename.len() <= 255
        && !filename.contains("..")
        && filename
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if !charset_ok {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "That filename cannot be used. Backup names may contain letters, digits, \
             dash, underscore and dot only — rename the file on the server and try again.",
        ));
    }

    let (name, engine, password, _port, site_server_id) =
        get_db_info(&state, id, claims.sub).await?;

    // The same rule the listing shows, ENFORCED. A caller that posts a filename the UI
    // greyed out gets the same sentence the UI displayed, rather than a failure from
    // inside a decompression pipe.
    if let Some(reason) = import_blocked_reason(filename, &engine) {
        return Err(err(StatusCode::BAD_REQUEST, &reason));
    }

    // Same rule as `reset_password` and for the same reason: the host comes from the
    // site's row, never from the caller's scope. `dockpanel-db-{name}` is unique only per
    // machine, and this call hands the tenant's decrypted password to whichever agent
    // answers.
    let agent = crate::helpers::agent_for_site_server(&state, site_server_id, &name).await?;

    let agent_body = serde_json::json!({
        "container_name": format!("dockpanel-db-{name}"),
        "db_type": engine,
        "user": name,
        "password": password,
    });

    // ⛔ The timeout is taken HERE rather than read off the error that comes back, and
    // that is not a style choice. `AgentError::Request` is minted for a URI that would
    // not build and for a connection that dropped mid-flight as well as for a timeout,
    // and the REMOTE client does not mint it for a timeout at all — it maps every
    // failure, timeouts included, to `AgentError::Connection`. So matching on the variant
    // told fleet operators their agent was offline while the import ran fine, and told
    // local operators an import was "still running" when nothing had ever been sent.
    // `Elapsed` here is unambiguous on both transports. The inner budget is deliberately
    // larger so the inner clock can never be the one that fires.
    let budget = crate::services::agent::DB_RESTORE_TIMEOUT_SECS;
    let agent_path = format!("/db-backups/{name}/restore/{filename}");
    let call = agent.post_long(&agent_path, Some(agent_body), budget + 60);

    let result = match tokio::time::timeout(std::time::Duration::from_secs(budget), call).await {
        Ok(inner) => inner.map_err(|e| {
            tracing::error!(
                database = %name, filename = %filename,
                "Database import agent error: {e:?}"
            );
            agent_error("Database import", e)
        })?,
        Err(_elapsed) => {
            tracing::warn!(
                database = %name, filename = %filename,
                "import outlasted the panel's request timeout; the agent is still running it"
            );
            // Deliberately NOT reported as a failure. The import is still running inside
            // the container, and the operator's natural response to "failed" is to run it
            // again — on top of a database that is mid-write, which is the one genuinely
            // destructive thing this information could cause.
            return Err(err(
                StatusCode::GATEWAY_TIMEOUT,
                &format!(
                    "The import is still running on the server — it was NOT cancelled and \
                     must not be started again. A dump that takes longer than {budget}s \
                     outlasts the panel's request timeout, so there is nothing left here to \
                     watch it with. Browse the tables in a few minutes to see it finish."
                ),
            ));
        }
    };

    let ip = crate::routes::client_ip(&headers);
    crate::services::activity::log_activity_on_server(
        &state.db, claims.sub, &claims.email, "database.import",
        Some("database"), Some(&name), Some(filename), ip.as_deref(), site_server_id, claims.key_id,
    ).await;
    crate::services::security_hardening::audit_log(
        &state.db,
        "database.import",
        Some(&claims.email),
        ip.as_deref(),
        Some("database"),
        Some(&name),
        Some(filename),
        None,
        "info",
        claims.key_id,
    ).await;

    crate::services::extensions::fire_event(&state.db, "database.imported", serde_json::json!({
        "database": &name,
        "database_id": id.to_string(),
        "filename": filename,
    }));

    tracing::info!("Database import completed: {name} from {filename}");

    Ok(Json(result))
}

#[derive(serde::Deserialize)]
pub struct UploadDumpRequest {
    #[serde(alias = "file_name", alias = "name")]
    pub filename: String,
    pub content: Option<String>,
    pub content_base64: Option<String>,
    #[serde(default, alias = "autoImport")]
    pub auto_import: Option<bool>,
    #[serde(default)]
    pub apply_compat_fixes: Option<bool>,
}

fn scan_cloud_sql_compat_issues(sql_bytes: &[u8], engine: &str) -> (Option<String>, Vec<String>) {
    if engine != "mysql" && engine != "mariadb" {
        return (None, Vec::new());
    }
    let text = String::from_utf8_lossy(sql_bytes);
    let mut first_err: Option<String> = None;
    let mut fixes: Vec<String> = Vec::new();
    let mut push = |err_msg: &str, fix: &str| {
        if first_err.is_none() {
            first_err = Some(err_msg.to_string());
        }
        if !fixes.iter().any(|x| x == fix) {
            fixes.push(fix.to_string());
        }
    };

    for line in text.lines() {
        let t = line.trim();
        let low = t.to_ascii_lowercase();
        if low.is_empty() || low.starts_with("insert") || low.starts_with("replace") || low.starts_with('(') {
            continue;
        }
        if engine == "mariadb" && low.contains("utf8mb4_0900_") {
            push(
                &format!("Incompatible MySQL 8 collation (utf8mb4_0900_*) on MariaDB: {}", t),
                "Convert utf8mb4_0900_* collations to utf8mb4_unicode_ci / utf8mb4_bin",
            );
        }
        if engine == "mysql" && (low.contains("with system versioning") || low.contains("page_checksum=") || low.contains("utf8mb4_uca1400_")) {
            push(
                &format!("MariaDB-only DDL feature detected on MySQL target: {}", t),
                "Downgrade utf8mb4_uca1400_* collations and strip MariaDB-only clauses (WITH SYSTEM VERSIONING, PAGE_CHECKSUM)",
            );
        }
        if low.contains("default x as `") || low.contains("default b as `") {
            push(
                &format!("Invalid phpMyAdmin binary/bit DEFAULT ... AS literal: {}", t),
                "Rewrite broken phpMyAdmin DEFAULT x/b AS literals into valid DEFAULT X'...' / b'...' hex literals",
            );
        }
        if low.contains("definer=") || low.contains("definer =") {
            push(
                &format!("Remote host DEFINER clause found: {}", t),
                "Strip remote DEFINER=... clauses from views/triggers/routines",
            );
        }
        if low.contains("gtid_purged") || low.contains("sql_log_bin") {
            push(
                &format!("Privileged GTID_PURGED / SQL_LOG_BIN directive detected: {}", t),
                "Remove @@GLOBAL.GTID_PURGED and @@SESSION.SQL_LOG_BIN session statements",
            );
        }
        if low.contains("type=innodb") || low.contains("type=myisam") || low.contains("row_format=fixed") {
            push(
                &format!("Legacy pre-5.5 table syntax detected: {}", t),
                "Upgrade legacy TYPE= to ENGINE= and normalize ROW_FORMAT=DYNAMIC",
            );
        }
    }
    (first_err, fixes)
}

fn rewrite_cloud_sql_compat_bytes(sql_bytes: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(sql_bytes);
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let t = line.trim_start();
        if (t.len() >= 6 && t[..6].eq_ignore_ascii_case("insert"))
            || (t.len() >= 7 && t[..7].eq_ignore_ascii_case("replace"))
            || t.starts_with('(')
        {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let mut s = line
            .replace("utf8mb4_0900_as_cs", "utf8mb4_bin")
            .replace("utf8mb4_0900_ai_ci", "utf8mb4_unicode_ci")
            .replace("utf8mb4_0900_as_ci", "utf8mb4_unicode_ci")
            .replace("utf8mb4_ja_0900_as_cs", "utf8mb4_unicode_ci")
            .replace("ROW_FORMAT=FIXED", "ROW_FORMAT=DYNAMIC")
            .replace("TYPE=InnoDB", "ENGINE=InnoDB")
            .replace("TYPE=MyISAM", "ENGINE=MyISAM")
            .replace(" WITH SYSTEM VERSIONING", "")
            .replace(" PAGE_CHECKSUM=1", "")
            .replace(" PAGE_CHECKSUM=0", "");
        let low = s.to_ascii_lowercase();
        if low.contains("gtid_purged") || low.contains("sql_log_bin") {
            s = "-- dockpanel skipped GTID_PURGED / SQL_LOG_BIN directive".to_string();
        }
        // Rewrite phpMyAdmin `DEFAULT x AS \`...\`` lines safely
        if low.contains("default x as `") {
            if let Some(pos) = low.find("default x as `") {
                if let Some(end_rel) = s[pos + 14..].find('`') {
                    let hex = &s[pos + 14..pos + 14 + end_rel];
                    if !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                        s = format!("{}DEFAULT X'{}'{}", &s[..pos], hex, &s[pos + 14 + end_rel + 1..]);
                    }
                }
            }
        }
        out.push_str(&s);
        out.push('\n');
    }
    out.into_bytes()
}

/// POST /api/databases/{id}/export — Trigger an on-demand dump of the database.
pub async fn export(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (name, engine, password, _port, site_server_id) =
        get_db_info(&state, id, claims.sub).await?;
    let agent = crate::helpers::agent_for_site_server(&state, site_server_id, &name).await?;

    let agent_body = serde_json::json!({
        "container_name": format!("dockpanel-db-{name}"),
        "db_name": name,
        "db_type": engine,
        "user": name,
        "password": password,
    });

    let res = agent
        .post_long("/db-backups/dump", Some(agent_body), 600)
        .await
        .map_err(|e| agent_error("Database export", e))?;

    Ok(Json(res))
}

/// POST /api/databases/{id}/upload-dump — Upload a dump file and optionally import it immediately.
pub async fn upload_dump(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    headers: axum::http::HeaderMap,
    Json(body): Json<UploadDumpRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (name, engine, password, _port, site_server_id) =
        get_db_info(&state, id, claims.sub).await?;

    let raw_filename = body.filename.trim();
    let sanitized_filename = raw_filename
        .replace(['/', '\\', ' ', ':', ';', '&', '|', '`', '$'], "_");
    if sanitized_filename.is_empty() || sanitized_filename.contains("..") {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid filename"));
    }

    let raw_content_str = body.content.as_deref()
        .or(body.content_base64.as_deref())
        .unwrap_or_default();
    let trimmed_content = raw_content_str.trim().replace(['\r', '\n', ' '], "");
    let decoded_bytes = base64::engine::general_purpose::STANDARD
        .decode(&trimmed_content)
        .or_else(|_| {
            base64::engine::general_purpose::STANDARD_NO_PAD
                .decode(&trimmed_content)
        })
        .map_err(|e| err(StatusCode::BAD_REQUEST, &format!("Invalid base64 encoding: {e}")))?;

    if decoded_bytes.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "File is empty"));
    }

    let _is_gzipped = decoded_bytes.starts_with(&[0x1f, 0x8b]);
    let is_zip = decoded_bytes.starts_with(&[0x50, 0x4b, 0x03, 0x04]);

    // If a ZIP archive was uploaded, extract the internal .sql dump
    let effective_bytes = if is_zip {
        let temp_zip_path = format!("/tmp/dockpanel_dump_upload_{}.zip", uuid::Uuid::new_v4());
        tokio::fs::write(&temp_zip_path, &decoded_bytes)
            .await
            .map_err(|e| internal_error("write temp zip", e))?;

        let unzip_output = tokio::process::Command::new("unzip")
            .args(["-p", &temp_zip_path, "*.sql"])
            .output()
            .await;

        let _ = tokio::fs::remove_file(&temp_zip_path).await;

        match unzip_output {
            Ok(output) if output.status.success() && !output.stdout.is_empty() => output.stdout,
            _ => {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    "ZIP archive does not contain a .sql database dump file. Ensure your .zip contains a database dump.",
                ));
            }
        }
    } else {
        decoded_bytes
    };

    let is_effective_gzipped = effective_bytes.starts_with(&[0x1f, 0x8b]);
    let auto_import = body.auto_import.unwrap_or(false);
    let apply_compat = body.apply_compat_fixes.unwrap_or(false);

    let effective_bytes = if auto_import && !is_effective_gzipped && (engine == "mysql" || engine == "mariadb") {
        let (compat_err, compat_fixes) = scan_cloud_sql_compat_issues(&effective_bytes, &engine);
        if !compat_fixes.is_empty() && !apply_compat {
            return Err((
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": compat_err.unwrap_or_else(|| "SQL dump contains compatibility issues requiring confirmation.".to_string()),
                    "code": "compat_confirm_required",
                    "requires_compat_confirm": true,
                    "compat_fixes": compat_fixes,
                    "engine": engine,
                })),
            ));
        }
        if apply_compat && !compat_fixes.is_empty() {
            rewrite_cloud_sql_compat_bytes(&effective_bytes)
        } else {
            effective_bytes
        }
    } else {
        effective_bytes
    };

    let target_filename = if is_effective_gzipped {
        if sanitized_filename.ends_with(".gz") {
            sanitized_filename
        } else {
            format!("{sanitized_filename}.gz")
        }
    } else {
        let base_name = sanitized_filename.trim_end_matches(".zip");
        if base_name.ends_with(".sql") {
            base_name.to_string()
        } else {
            format!("{base_name}.sql")
        }
    };

    let target_dir = format!("/var/backups/dockpanel/databases/{name}");
    tokio::fs::create_dir_all(&target_dir)
        .await
        .map_err(|e| internal_error("create dump directory", e))?;

    let target_path = format!("{target_dir}/{target_filename}");
    tokio::fs::write(&target_path, &effective_bytes)
        .await
        .map_err(|e| internal_error("write dump file", e))?;

    // If file is plain SQL and not gzipped, compress it with gzip and delete plain file
    let final_filename = if !is_effective_gzipped && target_filename.ends_with(".sql") {
        let gz_path = format!("{target_path}.gz");
        let gz_status = tokio::process::Command::new("gzip")
            .arg("-c")
            .arg(&target_path)
            .output()
            .await;
        match gz_status {
            Ok(output) if output.status.success() && !output.stdout.is_empty() => {
                let _ = tokio::fs::write(&gz_path, &output.stdout).await;
                let _ = tokio::fs::remove_file(&target_path).await;
                format!("{target_filename}.gz")
            }
            _ => target_filename,
        }
    } else {
        target_filename
    };

    let mut import_result = None;

    if auto_import {
        let agent = crate::helpers::agent_for_site_server(&state, site_server_id, &name).await?;
        let agent_body = serde_json::json!({
            "container_name": format!("dockpanel-db-{name}"),
            "db_type": engine,
            "user": name,
            "password": password,
        });
        let budget = crate::services::agent::DB_RESTORE_TIMEOUT_SECS;
        let agent_path = format!("/db-backups/{name}/restore/{final_filename}");
        let call = agent.post_long(&agent_path, Some(agent_body), budget + 60);
        match tokio::time::timeout(std::time::Duration::from_secs(budget), call).await {
            Ok(Ok(val)) => {
                import_result = Some(val);
            }
            Ok(Err(e)) => {
                return Err(agent_error("Database import", e));
            }
            Err(_) => {
                import_result = Some(serde_json::json!({
                    "status": "in_progress",
                    "message": "Import was started and is running in background"
                }));
            }
        }
    }

    let ip = crate::routes::client_ip(&headers);
    crate::services::activity::log_activity_on_server(
        &state.db, claims.sub, &claims.email, "database.upload_dump",
        Some("database"), Some(&name), Some(&final_filename), ip.as_deref(), site_server_id, claims.key_id,
    ).await;

    Ok(Json(serde_json::json!({
        "success": true,
        "filename": final_filename,
        "auto_imported": auto_import,
        "result": import_result,
    })))
}

/// GET /api/databases/{id}/dumps/{filename}/download — Download dump file directly.
pub async fn download_dump(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path((id, filename)): Path<(Uuid, String)>,
) -> Result<axum::response::Response, ApiError> {
    let (name, _engine, _password, _port, _site_server_id) =
        get_db_info(&state, id, claims.sub).await?;

    let clean_filename = filename.trim();
    if clean_filename.contains("..") || clean_filename.contains('/') || clean_filename.contains('\\') {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid filename"));
    }

    let file_path = format!("/var/backups/dockpanel/databases/{name}/{clean_filename}");
    let bytes = tokio::fs::read(&file_path)
        .await
        .map_err(|_| err(StatusCode::NOT_FOUND, "Dump file not found on disk"))?;

    let content_disposition = format!("attachment; filename=\"{clean_filename}\"");

    Ok(axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "application/octet-stream")
        .header(axum::http::header::CONTENT_DISPOSITION, content_disposition)
        .body(axum::body::Body::from(bytes))
        .map_err(|e| internal_error("build download response", e))?)
}

/// DELETE /api/databases/{id}/dumps/{filename} — Delete dump file from disk.
pub async fn delete_dump(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path((id, filename)): Path<(Uuid, String)>,
) -> Result<axum::Json<serde_json::Value>, ApiError> {
    let (name, _engine, _password, _port, _site_server_id) =
        get_db_info(&state, id, claims.sub).await?;

    let clean_filename = filename.trim();
    if clean_filename.contains("..") || clean_filename.contains('/') || clean_filename.contains('\\') {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid filename"));
    }

    let file_path = format!("/var/backups/dockpanel/databases/{name}/{clean_filename}");
    if tokio::fs::metadata(&file_path).await.is_err() {
        return Err(err(StatusCode::NOT_FOUND, "Dump file not found on disk"));
    }

    tokio::fs::remove_file(&file_path)
        .await
        .map_err(|e| internal_error("delete dump file", e))?;

    Ok(axum::Json(serde_json::json!({
        "success": true,
        "filename": clean_filename
    })))
}

/// GET /api/databases/sizes — Return disk usage in bytes for all database containers on this server.
pub async fn get_database_sizes(
    State(_state): State<AppState>,
    AuthUser(_claims): AuthUser,
    ServerScope(_server_id, agent): ServerScope,
) -> Result<Json<serde_json::Value>, ApiError> {
    let res = agent.get("/databases/sizes").await.unwrap_or_else(|_| serde_json::json!({ "sizes": {} }));
    Ok(Json(res))
}

// ── Database Health, Optimization & Pruning Tools (Cloud) ──────────────────

fn api_err_msg(e: &ApiError) -> String {
    e.1 .0
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| e.0.canonical_reason().unwrap_or("Request failed"))
        .to_string()
}

async fn run_agent_sql(
    agent: &crate::services::agent::AgentHandle,
    container: &str,
    engine: &str,
    user: &str,
    password: &str,
    database: &str,
    sql: &str,
) -> Result<serde_json::Value, ApiError> {
    agent
        .post(
            "/databases/query",
            Some(serde_json::json!({
                "container": container,
                "engine": engine,
                "user": user,
                "password": password,
                "database": database,
                "sql": sql,
            })),
        )
        .await
        .map_err(sql_error)
}

async fn query_agent_scalar_u64(
    agent: &crate::services::agent::AgentHandle,
    container: &str,
    engine: &str,
    user: &str,
    password: &str,
    database: &str,
    sql: &str,
) -> u64 {
    let Ok(v) = run_agent_sql(agent, container, engine, user, password, database, sql).await else {
        return 0;
    };
    v["rows"]
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|row| row.as_array())
        .and_then(|cols| cols.first())
        .and_then(|cell| {
            cell.as_u64()
                .or_else(|| cell.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
        })
        .unwrap_or(0)
}

fn parse_cell_u64(v: Option<&serde_json::Value>) -> u64 {
    v.and_then(|x| {
        x.as_u64()
            .or_else(|| x.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
    })
    .unwrap_or(0)
}

fn detect_wp_prefix(tables: &[String]) -> Option<String> {
    let lower_set: std::collections::HashSet<String> =
        tables.iter().map(|t| t.to_ascii_lowercase()).collect();
    for tbl in tables {
        let lower = tbl.to_ascii_lowercase();
        if let Some(prefix) = lower.strip_suffix("options") {
            if (prefix.is_empty() || is_safe_table_name(prefix))
                && lower_set.contains(&format!("{prefix}posts"))
                && lower_set.contains(&format!("{prefix}postmeta"))
            {
                return Some(tbl[..prefix.len()].to_string());
            }
        }
    }
    None
}

fn quote_ident(engine: &str, ident: &str) -> String {
    if matches!(engine, "mysql" | "mariadb") {
        format!("`{ident}`")
    } else {
        format!("\"{ident}\"")
    }
}

async fn cloud_wp_bloat(
    agent: &crate::services::agent::AgentHandle,
    container: &str,
    engine: &str,
    name: &str,
    password: &str,
    table_names: &[String],
) -> serde_json::Value {
    let Some(prefix) = detect_wp_prefix(table_names) else {
        return serde_json::json!({
            "detected": false,
            "table_prefix": "",
            "revisions": 0,
            "auto_drafts": 0,
            "trashed_posts": 0,
            "spam_comments": 0,
            "trashed_comments": 0,
            "orphaned_meta": 0,
            "expired_transients": 0,
            "total_transients": 0,
            "autoload_bytes": 0,
            "autoload_count": 0
        });
    };

    let posts = quote_ident(engine, &format!("{prefix}posts"));
    let postmeta = quote_ident(engine, &format!("{prefix}postmeta"));
    let comments = quote_ident(engine, &format!("{prefix}comments"));
    let commentmeta = quote_ident(engine, &format!("{prefix}commentmeta"));
    let options = quote_ident(engine, &format!("{prefix}options"));
    let is_mysql = matches!(engine, "mysql" | "mariadb");

    let has_comments = table_names
        .iter()
        .any(|t| t.eq_ignore_ascii_case(&format!("{prefix}comments")));
    let has_commentmeta = table_names
        .iter()
        .any(|t| t.eq_ignore_ascii_case(&format!("{prefix}commentmeta")));

    let revisions = query_agent_scalar_u64(
        agent, container, engine, name, password, name,
        &format!("SELECT COUNT(*) FROM {posts} WHERE post_type = 'revision'"),
    ).await;
    let auto_drafts = query_agent_scalar_u64(
        agent, container, engine, name, password, name,
        &format!("SELECT COUNT(*) FROM {posts} WHERE post_status = 'auto-draft'"),
    ).await;
    let trashed_posts = query_agent_scalar_u64(
        agent, container, engine, name, password, name,
        &format!("SELECT COUNT(*) FROM {posts} WHERE post_status = 'trash'"),
    ).await;
    let spam_comments = if has_comments {
        query_agent_scalar_u64(
            agent, container, engine, name, password, name,
            &format!("SELECT COUNT(*) FROM {comments} WHERE comment_approved = 'spam'"),
        ).await
    } else { 0 };
    let trashed_comments = if has_comments {
        query_agent_scalar_u64(
            agent, container, engine, name, password, name,
            &format!("SELECT COUNT(*) FROM {comments} WHERE comment_approved = 'trash'"),
        ).await
    } else { 0 };
    let orphan_pm = query_agent_scalar_u64(
        agent, container, engine, name, password, name,
        &format!("SELECT COUNT(*) FROM {postmeta} pm LEFT JOIN {posts} p ON p.ID = pm.post_id WHERE p.ID IS NULL"),
    ).await;
    let orphan_cm = if has_comments && has_commentmeta {
        query_agent_scalar_u64(
            agent, container, engine, name, password, name,
            &format!("SELECT COUNT(*) FROM {commentmeta} cm LEFT JOIN {comments} c ON c.comment_ID = cm.comment_id WHERE c.comment_ID IS NULL"),
        ).await
    } else { 0 };

    let total_transients = query_agent_scalar_u64(
        agent, container, engine, name, password, name,
        &format!("SELECT COUNT(*) FROM {options} WHERE option_name LIKE '\\_transient\\_%' ESCAPE '\\' OR option_name LIKE '\\_site\\_transient\\_%' ESCAPE '\\'"),
    ).await;
    let now_epoch = chrono::Utc::now().timestamp();
    let expired_sql = if is_mysql {
        format!("SELECT COUNT(*) FROM {options} WHERE (option_name LIKE '\\_transient\\_timeout\\_%' ESCAPE '\\' OR option_name LIKE '\\_site\\_transient\\_timeout\\_%' ESCAPE '\\') AND CAST(option_value AS UNSIGNED) < {now_epoch}")
    } else {
        format!("SELECT COUNT(*) FROM {options} WHERE (option_name LIKE '\\_transient\\_timeout\\_%' ESCAPE '\\' OR option_name LIKE '\\_site\\_transient\\_timeout\\_%' ESCAPE '\\') AND option_value ~ '^[0-9]+$' AND option_value::bigint < {now_epoch}")
    };
    let expired_transients = query_agent_scalar_u64(
        agent, container, engine, name, password, name, &expired_sql,
    ).await;

    let autoload_count = query_agent_scalar_u64(
        agent, container, engine, name, password, name,
        &format!("SELECT COUNT(*) FROM {options} WHERE autoload IN ('yes', 'on', 'auto', 'auto-on')"),
    ).await;
    let autoload_bytes_sql = if is_mysql {
        format!("SELECT COALESCE(SUM(OCTET_LENGTH(option_value)), 0) FROM {options} WHERE autoload IN ('yes', 'on', 'auto', 'auto-on')")
    } else {
        format!("SELECT COALESCE(SUM(octet_length(option_value)), 0) FROM {options} WHERE autoload IN ('yes', 'on', 'auto', 'auto-on')")
    };
    let autoload_bytes = query_agent_scalar_u64(
        agent, container, engine, name, password, name, &autoload_bytes_sql,
    ).await;

    serde_json::json!({
        "detected": true,
        "table_prefix": prefix,
        "revisions": revisions,
        "auto_drafts": auto_drafts,
        "trashed_posts": trashed_posts,
        "spam_comments": spam_comments,
        "trashed_comments": trashed_comments,
        "orphaned_meta": orphan_pm + orphan_cm,
        "expired_transients": expired_transients,
        "total_transients": total_transients,
        "autoload_bytes": autoload_bytes,
        "autoload_count": autoload_count,
    })
}

async fn cloud_dump_list_for_db(
    agent: &crate::services::agent::AgentHandle,
    name: &str,
) -> Vec<serde_json::Value> {
    match agent.get(&format!("/db-backups/{name}/list")).await {
        Ok(serde_json::Value::Array(arr)) => arr,
        _ => Vec::new(),
    }
}

async fn compute_cloud_health(
    agent: &crate::services::agent::AgentHandle,
    id: Uuid,
    name: &str,
    engine: &str,
    password: &str,
) -> Result<serde_json::Value, ApiError> {
    let container = format!("dockpanel-db-{name}");
    let is_mysql = matches!(engine, "mysql" | "mariadb");

    let sql = if is_mysql {
        "SELECT table_name, COALESCE(engine, 'InnoDB'), COALESCE(table_rows, 0), COALESCE(data_length, 0), COALESCE(index_length, 0), COALESCE(data_free, 0) FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY (data_length + index_length + data_free) DESC"
    } else {
        "SELECT t.table_name, 'PostgreSQL', COALESCE(st.n_live_tup, 0), pg_relation_size(quote_ident(t.table_name)), GREATEST(pg_total_relation_size(quote_ident(t.table_name)) - pg_relation_size(quote_ident(t.table_name)), 0), COALESCE(st.n_dead_tup, 0) FROM information_schema.tables t LEFT JOIN pg_stat_user_tables st ON st.relname = t.table_name WHERE t.table_schema = 'public' ORDER BY pg_total_relation_size(quote_ident(t.table_name)) DESC"
    };

    let qr = run_agent_sql(agent, &container, engine, name, password, name, sql).await?;
    let mut tables = Vec::new();
    let mut table_names = Vec::new();
    let mut data_bytes = 0u64;
    let mut index_bytes = 0u64;
    let mut overhead_bytes = 0u64;
    let mut fragmented = 0usize;

    if let Some(rows) = qr["rows"].as_array() {
        for r in rows {
            let Some(arr) = r.as_array() else { continue };
            let t_name = arr.first().and_then(|v| v.as_str()).unwrap_or("").to_string();
            if t_name.is_empty() {
                continue;
            }
            let t_engine = arr.get(1).and_then(|v| v.as_str()).unwrap_or(engine).to_string();
            let t_rows = parse_cell_u64(arr.get(2));
            let t_data = parse_cell_u64(arr.get(3));
            let t_idx = parse_cell_u64(arr.get(4));
            let third = parse_cell_u64(arr.get(5));
            let (t_overhead, dead_tuples) = if is_mysql { (third, 0) } else { (third.saturating_mul(256), third) };
            let needs_opt = if is_mysql {
                t_overhead >= 64 * 1024 || (t_data > 0 && t_overhead * 10 > t_data)
            } else {
                dead_tuples >= 100
            };
            if needs_opt {
                fragmented += 1;
            }
            data_bytes += t_data;
            index_bytes += t_idx;
            overhead_bytes += t_overhead;
            table_names.push(t_name.clone());
            tables.push(serde_json::json!({
                "name": t_name,
                "rows": t_rows,
                "data_bytes": t_data,
                "index_bytes": t_idx,
                "overhead_bytes": t_overhead,
                "engine": t_engine,
                "dead_tuples": dead_tuples,
                "needs_optimize": needs_opt,
            }));
        }
    }

    let dumps = cloud_dump_list_for_db(agent, name).await;
    let dump_count = dumps.len();
    let dump_bytes: u64 = dumps
        .iter()
        .filter_map(|d| d.get("size_bytes").and_then(|v| v.as_u64()))
        .sum();

    let wp_bloat = cloud_wp_bloat(agent, &container, engine, name, password, &table_names).await;

    Ok(serde_json::json!({
        "database_id": id,
        "database_name": name,
        "engine": engine,
        "total_bytes": data_bytes + index_bytes + overhead_bytes,
        "data_bytes": data_bytes,
        "index_bytes": index_bytes,
        "overhead_bytes": overhead_bytes,
        "table_count": tables.len(),
        "fragmented_table_count": fragmented,
        "dump_count": dump_count,
        "dump_bytes": dump_bytes,
        "integrity": "ok",
        "tables": tables,
        "wp_bloat": wp_bloat,
    }))
}

#[derive(serde::Deserialize, Default)]
pub struct CloudOptimizeRequest {
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub tables: Option<Vec<String>>,
}

#[derive(serde::Deserialize, Default)]
pub struct CloudPruneRequest {
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub keep_dumps: Option<usize>,
    #[serde(default)]
    pub older_than_days: Option<u64>,
    #[serde(default)]
    pub optimize_after: bool,
    #[serde(default)]
    pub truncate_tables: Vec<String>,
}

async fn optimize_cloud_database_inner(
    agent: &crate::services::agent::AgentHandle,
    id: Uuid,
    name: &str,
    engine: &str,
    password: &str,
    req: &CloudOptimizeRequest,
) -> Result<serde_json::Value, ApiError> {
    let action = req
        .action
        .as_deref()
        .unwrap_or("optimize")
        .trim()
        .to_ascii_lowercase();
    if !matches!(action.as_str(), "optimize" | "analyze" | "repair" | "check") {
        return Err(err(StatusCode::BAD_REQUEST, "Unsupported action (expected optimize, analyze, repair, or check)"));
    }

    let health_before = compute_cloud_health(agent, id, name, engine, password).await?;
    let size_before = health_before["total_bytes"].as_u64().unwrap_or(0);
    let container = format!("dockpanel-db-{name}");
    let is_mysql = matches!(engine, "mysql" | "mariadb");

    let mut target_tables = Vec::new();
    if let Some(ref requested) = req.tables {
        for t in requested {
            if !is_safe_table_name(t) {
                return Err(err(StatusCode::BAD_REQUEST, &format!("Invalid table name '{t}'")));
            }
            target_tables.push(t.clone());
        }
    } else if let Some(arr) = health_before["tables"].as_array() {
        for item in arr {
            if let Some(t_name) = item.get("name").and_then(|v| v.as_str()) {
                if is_safe_table_name(t_name) {
                    target_tables.push(t_name.to_string());
                }
            }
        }
    }

    let mut results = Vec::new();
    if is_mysql {
        let verb = match action.as_str() {
            "analyze" => "ANALYZE TABLE",
            "repair" => "REPAIR TABLE",
            "check" => "CHECK TABLE",
            _ => "OPTIMIZE TABLE",
        };
        for tbl in &target_tables {
            let q = format!("{verb} `{tbl}`");
            match run_agent_sql(agent, &container, engine, name, password, name, &q).await {
                Ok(v) => {
                    let msg = v["rows"]
                        .as_array()
                        .and_then(|rows| rows.last())
                        .and_then(|r| r.get(3))
                        .and_then(|m| m.as_str())
                        .unwrap_or("OK")
                        .to_string();
                    results.push(serde_json::json!({ "table": tbl, "status": "ok", "message": msg }));
                }
                Err(e) => {
                    results.push(serde_json::json!({ "table": tbl, "status": "error", "message": api_err_msg(&e) }));
                }
            }
        }
    } else if req.tables.as_ref().is_some_and(|t| !t.is_empty()) {
        for tbl in &target_tables {
            let sql = match action.as_str() {
                "analyze" => format!("ANALYZE \"{tbl}\""),
                "repair" => format!("REINDEX TABLE \"{tbl}\""),
                "check" => format!("ANALYZE \"{tbl}\""),
                _ => format!("VACUUM ANALYZE \"{tbl}\""),
            };
            match run_agent_sql(agent, &container, engine, name, password, name, &sql).await {
                Ok(_) => results.push(serde_json::json!({ "table": tbl, "status": "ok", "message": "Completed" })),
                Err(e) => results.push(serde_json::json!({ "table": tbl, "status": "error", "message": api_err_msg(&e) })),
            }
        }
    } else {
        let sql = match action.as_str() {
            "analyze" | "check" => "ANALYZE",
            "repair" => "VACUUM FULL",
            _ => "VACUUM ANALYZE",
        };
        run_agent_sql(agent, &container, engine, name, password, name, sql).await?;
        results.push(serde_json::json!({
            "table": name,
            "status": "ok",
            "message": format!("Executed {sql} on database {name}"),
        }));
    }

    let health_after = compute_cloud_health(agent, id, name, engine, password).await.unwrap_or(health_before);
    let size_after = health_after["total_bytes"].as_u64().unwrap_or(size_before);

    Ok(serde_json::json!({
        "ok": true,
        "action": action,
        "engine": engine,
        "tables_processed": results.len(),
        "size_before_bytes": size_before,
        "size_after_bytes": size_after,
        "reclaimed_bytes": size_before.saturating_sub(size_after),
        "integrity_status": "ok",
        "results": results,
    }))
}

async fn prune_cloud_dumps(
    name: &str,
    dumps: &[serde_json::Value],
    keep_latest: Option<usize>,
    older_than_days: Option<u64>,
) -> (usize, u64) {
    let mut items: Vec<(String, u64, String)> = dumps
        .iter()
        .filter_map(|d| {
            let fname = d.get("filename")?.as_str()?.to_string();
            if fname.contains("..") || fname.contains('/') || fname.contains('\\') {
                return None;
            }
            let sz = d.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
            let ts = d.get("created_at").and_then(|v| v.as_str()).unwrap_or("").to_string();
            Some((fname, sz, ts))
        })
        .collect();
    items.sort_by(|a, b| b.2.cmp(&a.2));

    let cutoff_ts = older_than_days.map(|days| chrono::Utc::now() - chrono::Duration::days(days as i64));
    let mut deleted = 0usize;
    let mut reclaimed = 0u64;

    for (idx, (fname, sz, ts)) in items.iter().enumerate() {
        let over_keep = keep_latest.map(|k| idx >= k).unwrap_or(false);
        let over_age = cutoff_ts
            .and_then(|cutoff| chrono::DateTime::parse_from_rfc3339(ts).ok().map(|dt| dt.with_timezone(&chrono::Utc) < cutoff))
            .unwrap_or(false);
        if over_keep || over_age {
            let path = format!("/var/backups/dockpanel/databases/{name}/{fname}");
            if tokio::fs::remove_file(&path).await.is_ok() {
                deleted += 1;
                reclaimed += *sz;
            }
        }
    }
    (deleted, reclaimed)
}

/// GET /api/databases/{id}/health — Inspect table sizes, fragmentation/overhead & WP bloat.
pub async fn health(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (name, engine, password, _port, site_server_id) =
        get_db_info(&state, id, claims.sub).await?;
    let agent = crate::helpers::agent_for_site_server(&state, site_server_id, &name).await?;
    let report = compute_cloud_health(&agent, id, &name, &engine, &password).await?;
    Ok(Json(report))
}

/// POST /api/databases/{id}/optimize — Optimize, analyze, repair, or vacuum tables.
pub async fn optimize(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    payload: Option<Json<CloudOptimizeRequest>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (name, engine, password, _port, site_server_id) =
        get_db_info(&state, id, claims.sub).await?;
    let agent = crate::helpers::agent_for_site_server(&state, site_server_id, &name).await?;
    let req = payload.map(|Json(v)| v).unwrap_or_default();
    let res = optimize_cloud_database_inner(&agent, id, &name, &engine, &password, &req).await?;
    Ok(Json(res))
}

/// POST /api/databases/{id}/prune — Preview (dry_run) or prune WP bloat, selected tables, and old dumps.
pub async fn prune(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    payload: Option<Json<CloudPruneRequest>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (name, engine, password, _port, site_server_id) =
        get_db_info(&state, id, claims.sub).await?;
    let agent = crate::helpers::agent_for_site_server(&state, site_server_id, &name).await?;
    let req = payload.map(|Json(v)| v).unwrap_or_default();
    let container = format!("dockpanel-db-{name}");

    let health_before = compute_cloud_health(&agent, id, &name, &engine, &password).await?;
    let size_before = health_before["total_bytes"].as_u64().unwrap_or(0);
    let mut rows_deleted = 0u64;
    let mut breakdown = serde_json::Map::new();

    let wp = &health_before["wp_bloat"];
    if wp["detected"].as_bool() == Some(true) {
        let prefix = wp["table_prefix"].as_str().unwrap_or("wp_");
        if is_safe_table_name(&format!("{prefix}posts")) {
            let posts = quote_ident(&engine, &format!("{prefix}posts"));
            let postmeta = quote_ident(&engine, &format!("{prefix}postmeta"));
            let comments = quote_ident(&engine, &format!("{prefix}comments"));
            let commentmeta = quote_ident(&engine, &format!("{prefix}commentmeta"));
            let options = quote_ident(&engine, &format!("{prefix}options"));
            let is_mysql = matches!(engine.as_str(), "mysql" | "mariadb");

            for cat in &req.categories {
                let count = match cat.as_str() {
                    "revisions" => {
                        let c = wp["revisions"].as_u64().unwrap_or(0);
                        if !req.dry_run && c > 0 {
                            let _ = run_agent_sql(&agent, &container, &engine, &name, &password, &name, &format!("DELETE FROM {posts} WHERE post_type = 'revision'")).await;
                        }
                        c
                    }
                    "auto_drafts" => {
                        let c = wp["auto_drafts"].as_u64().unwrap_or(0);
                        if !req.dry_run && c > 0 {
                            let _ = run_agent_sql(&agent, &container, &engine, &name, &password, &name, &format!("DELETE FROM {posts} WHERE post_status = 'auto-draft'")).await;
                        }
                        c
                    }
                    "trashed_posts" => {
                        let c = wp["trashed_posts"].as_u64().unwrap_or(0);
                        if !req.dry_run && c > 0 {
                            let _ = run_agent_sql(&agent, &container, &engine, &name, &password, &name, &format!("DELETE FROM {posts} WHERE post_status = 'trash'")).await;
                        }
                        c
                    }
                    "spam_comments" => {
                        let c = wp["spam_comments"].as_u64().unwrap_or(0) + wp["trashed_comments"].as_u64().unwrap_or(0);
                        if !req.dry_run && c > 0 {
                            let _ = run_agent_sql(&agent, &container, &engine, &name, &password, &name, &format!("DELETE FROM {comments} WHERE comment_approved IN ('spam', 'trash')")).await;
                        }
                        c
                    }
                    "orphaned_meta" => {
                        let c = wp["orphaned_meta"].as_u64().unwrap_or(0);
                        if !req.dry_run && c > 0 {
                            let _ = run_agent_sql(&agent, &container, &engine, &name, &password, &name, &format!("DELETE FROM {postmeta} WHERE post_id NOT IN (SELECT ID FROM {posts})")).await;
                            let _ = run_agent_sql(&agent, &container, &engine, &name, &password, &name, &format!("DELETE FROM {commentmeta} WHERE comment_id NOT IN (SELECT comment_ID FROM {comments})")).await;
                        }
                        c
                    }
                    "expired_transients" => {
                        let c = wp["expired_transients"].as_u64().unwrap_or(0);
                        if !req.dry_run && c > 0 && is_mysql {
                            let now_epoch = chrono::Utc::now().timestamp();
                            let _ = run_agent_sql(&agent, &container, &engine, &name, &password, &name, &format!("DELETE a, b FROM {options} a JOIN {options} b ON a.option_name = REPLACE(b.option_name, '_timeout_', '_') WHERE (b.option_name LIKE '\\_transient\\_timeout\\_%' ESCAPE '\\' OR b.option_name LIKE '\\_site\\_transient\\_timeout\\_%' ESCAPE '\\') AND CAST(b.option_value AS UNSIGNED) < {now_epoch}")).await;
                            let _ = run_agent_sql(&agent, &container, &engine, &name, &password, &name, &format!("DELETE FROM {options} WHERE (option_name LIKE '\\_transient\\_timeout\\_%' ESCAPE '\\' OR option_name LIKE '\\_site\\_transient\\_timeout\\_%' ESCAPE '\\') AND CAST(option_value AS UNSIGNED) < {now_epoch}")).await;
                        }
                        c
                    }
                    "all_transients" => {
                        let c = wp["total_transients"].as_u64().unwrap_or(0);
                        if !req.dry_run && c > 0 {
                            let _ = run_agent_sql(&agent, &container, &engine, &name, &password, &name, &format!("DELETE FROM {options} WHERE option_name LIKE '\\_transient\\_%' ESCAPE '\\' OR option_name LIKE '\\_site\\_transient\\_%' ESCAPE '\\'")).await;
                        }
                        c
                    }
                    _ => 0,
                };
                rows_deleted += count;
                breakdown.insert(cat.clone(), serde_json::json!(count));
            }
        }
    }

    if !req.truncate_tables.is_empty() {
        let mut trunc_rows = 0u64;
        for tbl in &req.truncate_tables {
            if !is_safe_table_name(tbl) {
                return Err(err(StatusCode::BAD_REQUEST, &format!("Invalid table name '{tbl}'")));
            }
            let ident = quote_ident(&engine, tbl);
            let cnt = query_agent_scalar_u64(&agent, &container, &engine, &name, &password, &name, &format!("SELECT COUNT(*) FROM {ident}")).await;
            if !req.dry_run {
                let _ = run_agent_sql(&agent, &container, &engine, &name, &password, &name, &format!("TRUNCATE TABLE {ident}")).await;
            }
            trunc_rows += cnt;
            rows_deleted += cnt;
        }
        breakdown.insert("truncated_rows".to_string(), serde_json::json!(trunc_rows));
    }

    let mut dumps_deleted = 0usize;
    let mut reclaimed_bytes = 0u64;
    if !req.dry_run && (req.keep_dumps.is_some() || req.older_than_days.is_some()) {
        // Admin-only guard mirrors dumps() so shared database names cannot erase another tenant's files.
        if claims.role == "admin" {
            let dumps = cloud_dump_list_for_db(&agent, &name).await;
            let (cnt, bytes) = prune_cloud_dumps(&name, &dumps, req.keep_dumps, req.older_than_days).await;
            dumps_deleted = cnt;
            reclaimed_bytes += bytes;
            breakdown.insert("dumps_deleted".to_string(), serde_json::json!(cnt));
        }
    }

    let mut optimized = false;
    if !req.dry_run && req.optimize_after {
        let opt = optimize_cloud_database_inner(
            &agent, id, &name, &engine, &password,
            &CloudOptimizeRequest { action: Some("optimize".to_string()), tables: None },
        ).await?;
        optimized = opt["ok"].as_bool() == Some(true);
        let after = opt["size_after_bytes"].as_u64().unwrap_or(size_before);
        reclaimed_bytes += size_before.saturating_sub(after);
    }

    Ok(Json(serde_json::json!({
        "ok": true,
        "dry_run": req.dry_run,
        "rows_deleted": rows_deleted,
        "dumps_deleted": dumps_deleted,
        "reclaimed_bytes": reclaimed_bytes,
        "optimized": optimized,
        "breakdown": serde_json::Value::Object(breakdown),
    })))
}

/// GET /api/databases/maintenance/summary — Server-wide DB storage, dumps, and orphan container audit.
pub async fn maintenance_summary(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    ServerScope(server_id, agent): ServerScope,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rows: Vec<(Uuid, Option<Uuid>, Option<Uuid>, String, String, Option<String>)> = sqlx::query_as(
        "SELECT d.id, d.site_id, d.stack_id, d.name, d.engine, d.container_id \
         FROM databases d \
         LEFT JOIN sites s ON d.site_id = s.id \
         LEFT JOIN docker_stacks st ON d.stack_id = st.id \
         WHERE (COALESCE(s.user_id, st.user_id) = $1 OR EXISTS (SELECT 1 FROM users u, servers sv \
         WHERE u.id = $1 AND u.role = 'admin' AND sv.id = COALESCE(s.server_id, st.server_id) \
         AND (sv.is_local OR sv.user_id = u.id))) \
         AND COALESCE(s.server_id, st.server_id) = $2 ORDER BY d.name ASC",
    )
    .bind(claims.sub)
    .bind(server_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| internal_error("maintenance summary", e))?;

    let sizes_res = agent.get("/databases/sizes").await.unwrap_or_else(|_| serde_json::json!({ "sizes": {} }));
    let sizes_map = sizes_res["sizes"].as_object();

    let mut known_containers = std::collections::HashSet::new();
    let mut total_bytes = 0u64;
    let mut total_dumps = 0usize;
    let mut total_dump_bytes = 0u64;
    let mut items = Vec::new();
    let mut orphan_databases = Vec::new();

    for (db_id, site_id, stack_id, db_name, db_engine, cid) in &rows {
        if let Some(c) = cid {
            known_containers.insert(c.clone());
        }
        let sz = sizes_map
            .and_then(|m| m.get(db_name).or_else(|| m.get(&db_id.to_string())))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let dumps = if claims.role == "admin" {
            cloud_dump_list_for_db(&agent, db_name).await
        } else {
            Vec::new()
        };
        let d_cnt = dumps.len();
        let d_bytes: u64 = dumps.iter().filter_map(|d| d.get("size_bytes").and_then(|v| v.as_u64())).sum();
        let linked = site_id.is_some() || stack_id.is_some();

        total_bytes += sz;
        total_dumps += d_cnt;
        total_dump_bytes += d_bytes;

        if !linked {
            orphan_databases.push(serde_json::json!({
                "id": db_id,
                "name": db_name,
                "engine": db_engine,
                "size_bytes": sz,
                "table_count": 0,
                "reason": "Database record is not linked to a site or stack",
            }));
        }

        items.push(serde_json::json!({
            "id": db_id,
            "name": db_name,
            "engine": db_engine,
            "linked": linked,
            "size_bytes": sz,
            "overhead_bytes": 0,
            "table_count": 0,
            "dump_count": d_cnt,
            "dump_bytes": d_bytes,
        }));
    }

    // For server admins, also detect dangling Docker `dockpanel-db-*` containers missing from `databases` table
    if claims.role == "admin" {
        if let Ok(serde_json::Value::Array(containers)) = agent.get("/databases").await {
            for c in containers {
                let cid = c.get("container_id").and_then(|v| v.as_str()).unwrap_or("");
                let cname = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let raw_name = cname.trim_start_matches('/').trim_start_matches("dockpanel-db-");
                if !cid.is_empty() && !known_containers.contains(cid) && !rows.iter().any(|r| r.3 == raw_name) {
                    let sz = sizes_map.and_then(|m| m.get(raw_name)).and_then(|v| v.as_u64()).unwrap_or(0);
                    orphan_databases.push(serde_json::json!({
                        "id": format!("container:{cid}"),
                        "name": raw_name,
                        "engine": c.get("engine").and_then(|v| v.as_str()).unwrap_or("unknown"),
                        "size_bytes": sz,
                        "table_count": 0,
                        "reason": "Orphan Docker container with no panel database record",
                    }));
                }
            }
        }
    }

    Ok(Json(serde_json::json!({
        "total_databases": items.len(),
        "total_bytes": total_bytes,
        "total_overhead_bytes": 0,
        "total_dumps": total_dumps,
        "total_dump_bytes": total_dump_bytes,
        "orphan_databases": orphan_databases,
        "databases": items,
    })))
}

#[derive(serde::Deserialize, Default)]
pub struct CloudGlobalCleanupRequest {
    #[serde(default)]
    pub optimize_all: bool,
    #[serde(default)]
    pub prune_dumps_keep: Option<usize>,
    #[serde(default)]
    pub prune_dumps_older_than_days: Option<u64>,
    #[serde(default)]
    pub delete_orphan_ids: Vec<String>,
}

/// POST /api/databases/maintenance/cleanup — Batch optimize, prune old dumps, and clean orphan containers.
pub async fn maintenance_cleanup(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    ServerScope(server_id, agent): ServerScope,
    Json(req): Json<CloudGlobalCleanupRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rows: Vec<(Uuid, Option<Uuid>, Option<Uuid>, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT d.id, d.site_id, d.stack_id, d.name, d.engine, d.db_password_enc, d.container_id \
         FROM databases d \
         LEFT JOIN sites s ON d.site_id = s.id \
         LEFT JOIN docker_stacks st ON d.stack_id = st.id \
         WHERE (COALESCE(s.user_id, st.user_id) = $1 OR EXISTS (SELECT 1 FROM users u, servers sv \
         WHERE u.id = $1 AND u.role = 'admin' AND sv.id = COALESCE(s.server_id, st.server_id) \
         AND (sv.is_local OR sv.user_id = u.id))) \
         AND COALESCE(s.server_id, st.server_id) = $2",
    )
    .bind(claims.sub)
    .bind(server_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| internal_error("maintenance cleanup", e))?;

    let mut optimized_count = 0usize;
    let mut reclaimed_bytes = 0u64;
    let mut dumps_deleted = 0usize;
    let mut orphans_deleted = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for oid in &req.delete_orphan_ids {
        if let Some(cid) = oid.strip_prefix("container:") {
            if claims.role != "admin" {
                errors.push("Admin required to remove orphan containers".to_string());
                continue;
            }
            if !cid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                errors.push("Invalid container id".to_string());
                continue;
            }
            match agent.delete(&format!("/databases/{cid}")).await {
                Ok(_) => orphans_deleted += 1,
                Err(e) => errors.push(format!("{cid}: {e}")),
            }
        }
    }

    for (db_id, _sid, _kid, db_name, db_engine, pw_enc, cid) in &rows {
        if cid.as_deref().unwrap_or("").is_empty() {
            continue;
        }
        if claims.role == "admin" && (req.prune_dumps_keep.is_some() || req.prune_dumps_older_than_days.is_some()) {
            let dumps = cloud_dump_list_for_db(&agent, db_name).await;
            let (cnt, bytes) = prune_cloud_dumps(db_name, &dumps, req.prune_dumps_keep, req.prune_dumps_older_than_days).await;
            dumps_deleted += cnt;
            reclaimed_bytes += bytes;
        }
        if req.optimize_all {
            let password = crate::services::secrets_crypto::decrypt_credential_or_legacy(pw_enc, &state.config.jwt_secret);
            match optimize_cloud_database_inner(
                &agent, *db_id, db_name, db_engine, &password,
                &CloudOptimizeRequest { action: Some("optimize".to_string()), tables: None },
            ).await {
                Ok(rep) => {
                    optimized_count += 1;
                    reclaimed_bytes += rep["reclaimed_bytes"].as_u64().unwrap_or(0);
                }
                Err(e) => errors.push(format!("{db_name}: {}", api_err_msg(&e))),
            }
        }
    }

    Ok(Json(serde_json::json!({
        "ok": errors.is_empty(),
        "optimized_count": optimized_count,
        "reclaimed_bytes": reclaimed_bytes,
        "dumps_deleted": dumps_deleted,
        "orphans_deleted": orphans_deleted,
        "errors": errors,
    })))
}

#[cfg(test)]
mod tests {
    use super::{detect_wp_prefix, is_reserved_db_name, quote_ident};

    #[test]
    fn reserved_db_names_rejected_case_insensitive() {
        for n in ["postgres", "Postgres", "TEMPLATE1", "template0", "mysql", "information_schema", "performance_schema", "sys"] {
            assert!(is_reserved_db_name(n), "{n} should be reserved");
        }
        for n in ["blog", "app_db", "wordpress", "shop", "my_data"] {
            assert!(!is_reserved_db_name(n), "{n} should be allowed");
        }
    }

    #[test]
    fn detects_wordpress_table_prefix_and_quotes_per_engine() {
        let wp = vec![
            "dp9_options".to_string(),
            "dp9_posts".to_string(),
            "dp9_postmeta".to_string(),
        ];
        assert_eq!(detect_wp_prefix(&wp).as_deref(), Some("dp9_"));

        let non_wp = vec!["users".to_string(), "orders".to_string()];
        assert_eq!(detect_wp_prefix(&non_wp), None);

        assert_eq!(quote_ident("mariadb", "wp_posts"), "`wp_posts`");
        assert_eq!(quote_ident("postgres", "wp_posts"), "\"wp_posts\"");
    }
}
