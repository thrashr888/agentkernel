//! SCIM 2.0 provisioning resources and durable tenant-scoped storage.
//!
//! The HTTP API deliberately keeps SCIM in its own module.  SCIM records are
//! stored in the same SQLite database as the other durable resources, but the
//! tenant key is included in every query and every uniqueness constraint.

use chrono::Utc;
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use rusqlite::types::Value as SqlValue;
use rusqlite::{OptionalExtension, Transaction, params, params_from_iter};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use uuid::Uuid;

use crate::durable_storage::DurableStorage;
use crate::http_api::{BoxBody, full, read_body_bytes};

pub const SCIM_MEDIA_TYPE: &str = "application/scim+json";
const USER_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
const GROUP_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:Group";
const LIST_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";
const PATCH_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:PatchOp";
const SERVICE_PROVIDER_CONFIG_SCHEMA: &str =
    "urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig";
const RESOURCE_TYPE_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:ResourceType";
const SCHEMA_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:Schema";
const MAX_SCIM_BODY_BYTES: usize = 1024 * 1024;
const MAX_PAGE_SIZE: usize = 100;

#[derive(Debug, Clone)]
pub struct ScimStore {
    storage: DurableStorage,
    mappings: Arc<Vec<crate::config::ScimGroupMapping>>,
}

impl ScimStore {
    /// Open the durable SCIM store with explicit group authorization mappings.
    /// Invalid mappings disable the store rather than granting a partial or
    /// surprising authorization set.
    pub fn open_default_with_mappings(
        mappings: Vec<crate::config::ScimGroupMapping>,
    ) -> anyhow::Result<Self> {
        validate_mappings(&mappings)
            .map_err(|error| anyhow::anyhow!("invalid SCIM group mapping: {error}"))?;
        let store = Self {
            storage: DurableStorage::open_default()?,
            mappings: Arc::new(mappings),
        };
        store.rematerialize_all_bindings()?;
        Ok(store)
    }

    #[cfg(test)]
    pub fn new(storage: DurableStorage) -> Self {
        Self {
            storage,
            mappings: Arc::new(Vec::new()),
        }
    }

    #[cfg(test)]
    pub fn new_with_mappings(
        storage: DurableStorage,
        mappings: Vec<crate::config::ScimGroupMapping>,
    ) -> anyhow::Result<Self> {
        validate_mappings(&mappings)
            .map_err(|error| anyhow::anyhow!("invalid SCIM group mapping: {error}"))?;
        let store = Self {
            storage,
            mappings: Arc::new(mappings),
        };
        store.rematerialize_all_bindings()?;
        Ok(store)
    }

    fn connection(&self) -> Result<rusqlite::Connection, ScimError> {
        self.storage
            .open_connection()
            .map_err(|e| ScimError::internal(format!("failed to open SCIM storage: {e}")))
    }

    /// Return the materialized SCIM grants for an active, non-deleted user.
    /// The query is tenant-scoped and intentionally returns no rows for an
    /// inactive/deleted user, so stale callers cannot retain SCIM grants.
    #[allow(dead_code)]
    pub fn principal_bindings(
        &self,
        tenant: &str,
        principal_id: &str,
    ) -> Result<(Vec<String>, Vec<String>), ScimError> {
        validate_tenant(tenant)?;
        validate_field(principal_id, "principal subject", 256)?;
        let conn = self.connection()?;
        let active: bool = conn
            .query_row(
                "SELECT active = 1 AND deleted = 0 FROM scim_users
                 WHERE tenant_id = ?1 AND external_id = ?2",
                params![tenant, principal_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| ScimError::internal(format!("failed to read SCIM principal: {e}")))?
            .unwrap_or(false);
        if !active {
            return Ok((Vec::new(), Vec::new()));
        }
        let mut stmt = conn
            .prepare(
                "SELECT role, team_id FROM scim_principal_bindings
                 WHERE tenant_id = ?1 AND user_id =
                    (SELECT id FROM scim_users WHERE tenant_id = ?1 AND external_id = ?2
                      AND active = 1 AND deleted = 0)",
            )
            .map_err(|e| ScimError::internal(format!("failed to read SCIM bindings: {e}")))?;
        let rows = stmt
            .query_map(params![tenant, principal_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| ScimError::internal(format!("failed to read SCIM bindings: {e}")))?;
        let mut roles = BTreeSet::new();
        let mut teams = BTreeSet::new();
        for row in rows {
            let (role, team) = row
                .map_err(|e| ScimError::internal(format!("failed to decode SCIM binding: {e}")))?;
            if !role.is_empty() {
                roles.insert(role);
            }
            if !team.is_empty() {
                teams.insert(team);
            }
        }
        Ok((roles.into_iter().collect(), teams.into_iter().collect()))
    }

    fn refresh_bindings(&self, tx: &Transaction<'_>, tenant: &str) -> Result<(), ScimError> {
        tx.execute(
            "DELETE FROM scim_principal_bindings WHERE tenant_id = ?1",
            [tenant],
        )
        .map_err(|e| ScimError::internal(format!("failed to clear SCIM bindings: {e}")))?;
        let now = Utc::now().to_rfc3339();
        for mapping in self.mappings.iter().filter(|m| m.tenant_id == tenant) {
            for role in &mapping.roles {
                let (group_predicate, group_value) = mapping_group_predicate(mapping);
                let sql = format!(
                    "INSERT INTO scim_principal_bindings
                       (tenant_id, user_id, group_id, role, team_id, updated_at)
                     SELECT ?1, gm.user_id, gm.group_id, ?2, ?3, ?4
                       FROM scim_group_members gm
                       JOIN scim_groups g ON g.id = gm.group_id
                       JOIN scim_users u ON u.id = gm.user_id
                      WHERE g.tenant_id = ?1 AND {group_predicate} AND g.active = 1
                        AND u.tenant_id = ?1 AND u.active = 1 AND u.deleted = 0"
                );
                tx.execute(
                    &sql,
                    params![
                        tenant,
                        role,
                        mapping.team_id.as_deref().unwrap_or(""),
                        now,
                        group_value,
                    ],
                )
                .map_err(|e| {
                    ScimError::internal(format!("failed to materialize SCIM role binding: {e}"))
                })?;
            }
            if mapping.roles.is_empty()
                && let Some(team_id) = mapping.team_id.as_deref()
            {
                let (group_predicate, group_value) = mapping_group_predicate(mapping);
                let sql = format!(
                    "INSERT INTO scim_principal_bindings
                       (tenant_id, user_id, group_id, role, team_id, updated_at)
                     SELECT ?1, gm.user_id, gm.group_id, '', ?2, ?3
                       FROM scim_group_members gm
                       JOIN scim_groups g ON g.id = gm.group_id
                       JOIN scim_users u ON u.id = gm.user_id
                      WHERE g.tenant_id = ?1 AND {group_predicate} AND g.active = 1
                        AND u.tenant_id = ?1 AND u.active = 1 AND u.deleted = 0"
                );
                tx.execute(&sql, params![tenant, team_id, now, group_value])
                    .map_err(|e| {
                        ScimError::internal(format!("failed to materialize SCIM team binding: {e}"))
                    })?;
            }
        }
        Ok(())
    }

    /// Rebuild every materialized grant at startup. Mappings are configuration,
    /// not durable authorization, so removing or changing one must revoke its
    /// old rows immediately on restart.
    fn rematerialize_all_bindings(&self) -> anyhow::Result<()> {
        let mut conn = self
            .storage
            .open_connection()
            .map_err(|e| anyhow::anyhow!("failed to open SCIM storage: {e}"))?;
        let tx = conn
            .transaction()
            .map_err(|e| anyhow::anyhow!("failed to start SCIM binding refresh: {e}"))?;
        let mut tenants = BTreeSet::new();
        {
            let mut statement = tx.prepare(
                "SELECT tenant_id FROM scim_users
                 UNION SELECT tenant_id FROM scim_groups
                 UNION SELECT tenant_id FROM scim_principal_bindings",
            )?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows {
                tenants.insert(row?);
            }
        }
        tenants.extend(
            self.mappings
                .iter()
                .map(|mapping| mapping.tenant_id.clone()),
        );
        for tenant in tenants {
            self.refresh_bindings(&tx, &tenant)
                .map_err(|error| anyhow::anyhow!("failed to refresh tenant {tenant}: {error:?}"))?;
        }
        tx.commit()
            .map_err(|e| anyhow::anyhow!("failed to commit SCIM binding refresh: {e}"))?;
        Ok(())
    }

    pub fn create_user(&self, tenant: &str, input: &UserInput) -> Result<UserResource, ScimError> {
        validate_tenant(tenant)?;
        let user_name = required_text(input.user_name.as_deref(), "userName")?;
        validate_field(user_name, "userName", 256)?;
        let external_id = optional_text(input.external_id.as_deref(), "externalId", 256)?;
        let display_name = optional_text(input.display_name.as_deref(), "displayName", 256)?;
        let given_name = input.name.as_ref().and_then(|name| name.given_name.clone());
        let family_name = input
            .name
            .as_ref()
            .and_then(|name| name.family_name.clone());
        validate_optional_field(given_name.as_deref(), "givenName", 256)?;
        validate_optional_field(family_name.as_deref(), "familyName", 256)?;
        let email = primary_email(input.emails.as_deref())?;
        let locale = optional_text(input.locale.as_deref(), "locale", 128)?;
        let timezone = optional_text(input.timezone.as_deref(), "timezone", 128)?;
        let now = Utc::now().to_rfc3339();
        let id = Uuid::now_v7().to_string();
        let mut conn = self.connection()?;
        let tx = conn
            .transaction()
            .map_err(|e| ScimError::internal(format!("failed to start user transaction: {e}")))?;
        tx.execute(
            "INSERT INTO scim_users
             (id, tenant_id, external_id, user_name, active, display_name,
              given_name, family_name, email, locale, timezone, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
            params![
                id,
                tenant,
                external_id,
                user_name,
                input.active.unwrap_or(true),
                display_name,
                given_name,
                family_name,
                email,
                locale,
                timezone,
                now,
            ],
        )
        .map_err(map_sql_error)?;
        self.refresh_bindings(&tx, tenant)?;
        tx.commit()
            .map_err(|e| ScimError::internal(format!("failed to commit user: {e}")))?;
        self.get_user(tenant, &id)
    }

    pub fn get_user(&self, tenant: &str, id: &str) -> Result<UserResource, ScimError> {
        validate_tenant(tenant)?;
        validate_id(id)?;
        let conn = self.connection()?;
        let user = conn
            .query_row(
                "SELECT id, tenant_id, external_id, user_name, active, display_name,
                        given_name, family_name, email, locale, timezone, created_at, updated_at
                   FROM scim_users WHERE tenant_id = ?1 AND id = ?2 AND deleted = 0",
                params![tenant, id],
                row_to_user,
            )
            .optional()
            .map_err(|e| ScimError::internal(format!("failed to read SCIM user: {e}")))?
            .ok_or_else(|| ScimError::not_found("User not found"))?;
        Ok(user.into_resource())
    }

    pub fn list_users(
        &self,
        tenant: &str,
        filter: Option<&ScimFilter>,
        start_index: usize,
        count: usize,
    ) -> Result<(Vec<UserResource>, usize), ScimError> {
        validate_tenant(tenant)?;
        let conn = self.connection()?;
        let column = filter.and_then(|filter| filter_column(&filter.attribute));
        if filter.is_some() && column.is_none() {
            return Err(ScimError::invalid_filter(
                "Unsupported user filter attribute",
            ));
        }
        let mut where_sql = String::from("tenant_id = ?1 AND deleted = 0");
        let mut filter_values = vec![SqlValue::Text(tenant.to_string())];
        if let Some(column) = column {
            where_sql.push_str(" AND lower(");
            where_sql.push_str(column);
            where_sql.push_str(") = lower(?2)");
            filter_values.push(SqlValue::Text(filter.expect("filter exists").value.clone()));
        }
        let total_sql = format!("SELECT COUNT(1) FROM scim_users WHERE {where_sql}");
        let total: i64 = conn
            .query_row(&total_sql, params_from_iter(filter_values.iter()), |row| {
                row.get(0)
            })
            .map_err(|e| ScimError::internal(format!("failed to count SCIM users: {e}")))?;
        let mut page_values = filter_values;
        page_values.push(SqlValue::Integer(i64::try_from(count).unwrap_or(i64::MAX)));
        page_values.push(SqlValue::Integer(
            i64::try_from(start_index.saturating_sub(1)).unwrap_or(i64::MAX),
        ));
        let select_sql = format!(
            "SELECT id, tenant_id, external_id, user_name, active, display_name,
                    given_name, family_name, email, locale, timezone, created_at, updated_at
               FROM scim_users WHERE {where_sql} ORDER BY id ASC LIMIT ?{} OFFSET ?{}",
            if filter.is_some() { 3 } else { 2 },
            if filter.is_some() { 4 } else { 3 },
        );
        let mut stmt = conn
            .prepare(&select_sql)
            .map_err(|e| ScimError::internal(format!("failed to list SCIM users: {e}")))?;
        let rows = stmt
            .query_map(params_from_iter(page_values.iter()), row_to_user)
            .map_err(|e| ScimError::internal(format!("failed to list SCIM users: {e}")))?;
        let resources = rows
            .map(|row| {
                row.map(|user| user.into_resource())
                    .map_err(|e| ScimError::internal(format!("failed to decode SCIM user: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((resources, usize::try_from(total).unwrap_or(usize::MAX)))
    }

    pub fn replace_user(
        &self,
        tenant: &str,
        id: &str,
        input: &UserInput,
    ) -> Result<UserResource, ScimError> {
        validate_tenant(tenant)?;
        validate_id(id)?;
        let user_name = required_text(input.user_name.as_deref(), "userName")?;
        validate_field(user_name, "userName", 256)?;
        let external_id = optional_text(input.external_id.as_deref(), "externalId", 256)?;
        let display_name = optional_text(input.display_name.as_deref(), "displayName", 256)?;
        let given_name = input.name.as_ref().and_then(|name| name.given_name.clone());
        let family_name = input
            .name
            .as_ref()
            .and_then(|name| name.family_name.clone());
        validate_optional_field(given_name.as_deref(), "givenName", 256)?;
        validate_optional_field(family_name.as_deref(), "familyName", 256)?;
        let email = primary_email(input.emails.as_deref())?;
        let locale = optional_text(input.locale.as_deref(), "locale", 128)?;
        let timezone = optional_text(input.timezone.as_deref(), "timezone", 128)?;
        let now = Utc::now().to_rfc3339();
        let mut conn = self.connection()?;
        let tx = conn
            .transaction()
            .map_err(|e| ScimError::internal(format!("failed to start user transaction: {e}")))?;
        let changed = tx
            .execute(
                "UPDATE scim_users SET external_id = ?1, user_name = ?2, active = ?3,
                    display_name = ?4, given_name = ?5, family_name = ?6, email = ?7,
                    locale = ?8, timezone = ?9, updated_at = ?10
                 WHERE tenant_id = ?11 AND id = ?12 AND deleted = 0",
                params![
                    external_id,
                    user_name,
                    input.active.unwrap_or(true),
                    display_name,
                    given_name,
                    family_name,
                    email,
                    locale,
                    timezone,
                    now,
                    tenant,
                    id,
                ],
            )
            .map_err(map_sql_error)?;
        if changed == 0 {
            return Err(ScimError::not_found("User not found"));
        }
        self.refresh_bindings(&tx, tenant)?;
        tx.commit()
            .map_err(|e| ScimError::internal(format!("failed to commit user: {e}")))?;
        self.get_user(tenant, id)
    }

    /// Tombstone a SCIM user while retaining its durable audit record.
    /// PATCH `active: false` remains the non-deleting deactivation operation.
    pub fn delete_user(&self, tenant: &str, id: &str) -> Result<(), ScimError> {
        validate_tenant(tenant)?;
        validate_id(id)?;
        let mut conn = self.connection()?;
        let tx = conn
            .transaction()
            .map_err(|e| ScimError::internal(format!("failed to start user transaction: {e}")))?;
        let changed = tx
            .execute(
                "UPDATE scim_users SET active = 0, deleted = 1, updated_at = ?1
                   WHERE tenant_id = ?2 AND id = ?3 AND deleted = 0",
                params![Utc::now().to_rfc3339(), tenant, id],
            )
            .map_err(|e| ScimError::internal(format!("failed to delete SCIM user: {e}")))?;
        if changed == 0 {
            return Err(ScimError::not_found("User not found"));
        }
        self.refresh_bindings(&tx, tenant)?;
        tx.commit()
            .map_err(|e| ScimError::internal(format!("failed to commit user: {e}")))?;
        Ok(())
    }

    pub fn patch_user(
        &self,
        tenant: &str,
        id: &str,
        patch: &PatchRequest,
    ) -> Result<UserResource, ScimError> {
        let current = self.get_user(tenant, id)?;
        let mut input = current.to_input();
        for operation in &patch.operations {
            if !matches!(
                operation.op.to_ascii_lowercase().as_str(),
                "replace" | "add"
            ) {
                return Err(ScimError::invalid_request(
                    "Only add and replace PATCH operations are supported",
                ));
            }
            let path = operation.path.as_deref().unwrap_or("");
            if path.eq_ignore_ascii_case("active") {
                input.active = operation
                    .value
                    .as_bool()
                    .ok_or_else(|| ScimError::invalid_request("active must be a boolean"))
                    .map(Some)?;
            } else if path.eq_ignore_ascii_case("userName") {
                input.user_name = operation.value.as_str().map(str::to_string);
            } else if path.eq_ignore_ascii_case("externalId") {
                input.external_id = operation.value.as_str().map(str::to_string);
            } else if path.eq_ignore_ascii_case("displayName") {
                input.display_name = operation.value.as_str().map(str::to_string);
            } else if path.is_empty() {
                let object = operation
                    .value
                    .as_object()
                    .ok_or_else(|| ScimError::invalid_request("PATCH value must be an object"))?;
                apply_user_object(&mut input, object)?;
            } else {
                return Err(ScimError::invalid_request(format!(
                    "Unsupported PATCH path: {path}"
                )));
            }
        }
        self.replace_user(tenant, id, &input)
    }

    pub fn create_group(
        &self,
        tenant: &str,
        input: &GroupInput,
    ) -> Result<GroupResource, ScimError> {
        validate_tenant(tenant)?;
        let display_name = required_text(input.display_name.as_deref(), "displayName")?;
        validate_field(display_name, "displayName", 256)?;
        let external_id = optional_text(input.external_id.as_deref(), "externalId", 256)?;
        let members = validate_members(input.members.as_deref())?;
        let now = Utc::now().to_rfc3339();
        let id = Uuid::now_v7().to_string();
        let conn = self.connection()?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| ScimError::internal(format!("failed to start group transaction: {e}")))?;
        tx.execute(
            "INSERT INTO scim_groups
             (id, tenant_id, external_id, display_name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, tenant, external_id, display_name, now],
        )
        .map_err(map_sql_error)?;
        insert_members(&tx, tenant, &id, &members, &now)?;
        self.refresh_bindings(&tx, tenant)?;
        tx.commit()
            .map_err(|e| ScimError::internal(format!("failed to commit group: {e}")))?;
        self.get_group(tenant, &id)
    }

    pub fn get_group(&self, tenant: &str, id: &str) -> Result<GroupResource, ScimError> {
        validate_tenant(tenant)?;
        validate_id(id)?;
        let conn = self.connection()?;
        let group = conn
            .query_row(
                "SELECT id, tenant_id, external_id, display_name, created_at, updated_at, active
                   FROM scim_groups WHERE tenant_id = ?1 AND id = ?2 AND active = 1",
                params![tenant, id],
                row_to_group,
            )
            .optional()
            .map_err(|e| ScimError::internal(format!("failed to read SCIM group: {e}")))?
            .ok_or_else(|| ScimError::not_found("Group not found"))?;
        let members = load_members(&conn, tenant, &group.id)?;
        Ok(group.into_resource(members))
    }

    pub fn list_groups(
        &self,
        tenant: &str,
        filter: Option<&ScimFilter>,
        start_index: usize,
        count: usize,
    ) -> Result<(Vec<GroupResource>, usize), ScimError> {
        validate_tenant(tenant)?;
        let conn = self.connection()?;
        let column = filter.and_then(|filter| match filter.attribute.as_str() {
            "displayName" => Some("display_name"),
            "externalId" => Some("external_id"),
            _ => None,
        });
        if filter.is_some() && column.is_none() {
            return Err(ScimError::invalid_filter(
                "Unsupported group filter attribute",
            ));
        }
        let mut where_sql = String::from("tenant_id = ?1 AND active = 1");
        let mut filter_values = vec![SqlValue::Text(tenant.to_string())];
        if let Some(column) = column {
            where_sql.push_str(" AND lower(");
            where_sql.push_str(column);
            where_sql.push_str(") = lower(?2)");
            filter_values.push(SqlValue::Text(filter.expect("filter exists").value.clone()));
        }
        let total_sql = format!("SELECT COUNT(1) FROM scim_groups WHERE {where_sql}");
        let total: i64 = conn
            .query_row(&total_sql, params_from_iter(filter_values.iter()), |row| {
                row.get(0)
            })
            .map_err(|e| ScimError::internal(format!("failed to count SCIM groups: {e}")))?;
        let mut page_values = filter_values;
        page_values.push(SqlValue::Integer(i64::try_from(count).unwrap_or(i64::MAX)));
        page_values.push(SqlValue::Integer(
            i64::try_from(start_index.saturating_sub(1)).unwrap_or(i64::MAX),
        ));
        let select_sql = format!(
            "SELECT id, tenant_id, external_id, display_name, created_at, updated_at, active
               FROM scim_groups WHERE {where_sql} ORDER BY id ASC LIMIT ?{} OFFSET ?{}",
            if filter.is_some() { 3 } else { 2 },
            if filter.is_some() { 4 } else { 3 },
        );
        let mut stmt = conn
            .prepare(&select_sql)
            .map_err(|e| ScimError::internal(format!("failed to list SCIM groups: {e}")))?;
        let rows = stmt
            .query_map(params_from_iter(page_values.iter()), row_to_group)
            .map_err(|e| ScimError::internal(format!("failed to list SCIM groups: {e}")))?;
        let groups = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| ScimError::internal(format!("failed to decode SCIM groups: {e}")))?;
        let resources = groups
            .into_iter()
            .map(|group| {
                let members = load_members(&conn, tenant, &group.id)?;
                Ok(group.into_resource(members))
            })
            .collect::<Result<Vec<_>, ScimError>>()?;
        Ok((resources, usize::try_from(total).unwrap_or(usize::MAX)))
    }

    pub fn replace_group(
        &self,
        tenant: &str,
        id: &str,
        input: &GroupInput,
    ) -> Result<GroupResource, ScimError> {
        validate_tenant(tenant)?;
        validate_id(id)?;
        let display_name = required_text(input.display_name.as_deref(), "displayName")?;
        validate_field(display_name, "displayName", 256)?;
        let external_id = optional_text(input.external_id.as_deref(), "externalId", 256)?;
        let members = validate_members(input.members.as_deref())?;
        let now = Utc::now().to_rfc3339();
        let conn = self.connection()?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| ScimError::internal(format!("failed to start group transaction: {e}")))?;
        let changed = tx
            .execute(
                "UPDATE scim_groups SET external_id = ?1, display_name = ?2, updated_at = ?3
                 WHERE tenant_id = ?4 AND id = ?5 AND active = 1",
                params![external_id, display_name, now, tenant, id],
            )
            .map_err(map_sql_error)?;
        if changed == 0 {
            return Err(ScimError::not_found("Group not found"));
        }
        tx.execute("DELETE FROM scim_group_members WHERE group_id = ?1", [id])
            .map_err(|e| ScimError::internal(format!("failed to replace group members: {e}")))?;
        insert_members(&tx, tenant, id, &members, &now)?;
        self.refresh_bindings(&tx, tenant)?;
        tx.commit()
            .map_err(|e| ScimError::internal(format!("failed to commit group: {e}")))?;
        self.get_group(tenant, id)
    }

    /// Tombstone a group while preserving its durable membership history.
    pub fn delete_group(&self, tenant: &str, id: &str) -> Result<(), ScimError> {
        validate_tenant(tenant)?;
        validate_id(id)?;
        let mut conn = self.connection()?;
        let tx = conn
            .transaction()
            .map_err(|e| ScimError::internal(format!("failed to start group transaction: {e}")))?;
        let changed = tx
            .execute(
                "UPDATE scim_groups SET active = 0, updated_at = ?1
                   WHERE tenant_id = ?2 AND id = ?3 AND active = 1",
                params![Utc::now().to_rfc3339(), tenant, id],
            )
            .map_err(|e| ScimError::internal(format!("failed to delete SCIM group: {e}")))?;
        if changed == 0 {
            return Err(ScimError::not_found("Group not found"));
        }
        self.refresh_bindings(&tx, tenant)?;
        tx.commit()
            .map_err(|e| ScimError::internal(format!("failed to commit group: {e}")))?;
        Ok(())
    }

    pub fn patch_group(
        &self,
        tenant: &str,
        id: &str,
        patch: &PatchRequest,
    ) -> Result<GroupResource, ScimError> {
        let current = self.get_group(tenant, id)?;
        let mut input = current.to_input();
        for operation in &patch.operations {
            let op = operation.op.to_ascii_lowercase();
            let path = operation.path.as_deref().unwrap_or("");
            if !matches!(op.as_str(), "add" | "replace" | "remove") {
                return Err(ScimError::invalid_request(format!(
                    "Unsupported group PATCH operation: {}",
                    operation.op
                )));
            }
            if path.eq_ignore_ascii_case("displayName") {
                if op == "remove" {
                    return Err(ScimError::invalid_request(
                        "displayName is required and cannot be removed",
                    ));
                }
                input.display_name = Some(
                    operation
                        .value
                        .as_str()
                        .ok_or_else(|| ScimError::invalid_request("displayName must be a string"))?
                        .to_string(),
                );
                continue;
            }
            if path.is_empty() || path.eq_ignore_ascii_case("members") {
                let values = member_values(&operation.value)?;
                if op == "add" {
                    let mut seen: HashSet<String> = input
                        .members
                        .get_or_insert_default()
                        .iter()
                        .map(|m| m.value.clone())
                        .collect();
                    let members = input.members.get_or_insert_default();
                    for value in values {
                        if seen.insert(value.clone()) {
                            members.push(MemberInput {
                                value,
                                display: None,
                            });
                        }
                    }
                } else if op == "remove" {
                    let remove: HashSet<String> = values.into_iter().collect();
                    input
                        .members
                        .get_or_insert_default()
                        .retain(|member| !remove.contains(&member.value));
                } else if op == "replace" {
                    input.members = Some(
                        values
                            .into_iter()
                            .map(|value| MemberInput {
                                value,
                                display: None,
                            })
                            .collect(),
                    );
                } else {
                    return Err(ScimError::invalid_request(format!(
                        "Unsupported group PATCH operation: {}",
                        operation.op
                    )));
                }
                continue;
            }
            if op == "remove" && path.starts_with("members[") {
                let value = parse_member_filter(path)?;
                input
                    .members
                    .get_or_insert_default()
                    .retain(|member| member.value != value);
                continue;
            }
            return Err(ScimError::invalid_request(format!(
                "Unsupported group PATCH path: {path}"
            )));
        }
        self.replace_group(tenant, id, &input)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserResource {
    pub schemas: Vec<String>,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    pub user_name: String,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<NameResource>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub emails: Vec<EmailResource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    pub meta: MetaResource,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupResource {
    pub schemas: Vec<String>,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    pub display_name: String,
    pub members: Vec<MemberResource>,
    pub meta: MetaResource,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NameResource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub given_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailResource {
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberResource {
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(rename = "$ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaResource {
    pub resource_type: String,
    pub created: String,
    pub last_modified: String,
    pub version: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserInput {
    #[serde(default)]
    pub schemas: Vec<String>,
    pub external_id: Option<String>,
    pub user_name: Option<String>,
    pub active: Option<bool>,
    pub display_name: Option<String>,
    pub name: Option<NameInput>,
    pub emails: Option<Vec<EmailInput>>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NameInput {
    pub given_name: Option<String>,
    pub family_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EmailInput {
    pub value: String,
    #[serde(rename = "type")]
    pub type_: Option<String>,
    pub primary: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GroupInput {
    #[serde(default)]
    pub schemas: Vec<String>,
    pub external_id: Option<String>,
    pub display_name: Option<String>,
    pub members: Option<Vec<MemberInput>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemberInput {
    pub value: String,
    pub display: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PatchRequest {
    #[serde(default)]
    pub schemas: Vec<String>,
    #[serde(rename = "Operations", alias = "operations")]
    pub operations: Vec<PatchOperation>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PatchOperation {
    pub op: String,
    pub path: Option<String>,
    #[serde(default)]
    pub value: Value,
}

#[derive(Debug, Clone)]
pub struct ScimFilter {
    pub attribute: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct ListParams {
    pub filter: Option<ScimFilter>,
    pub start_index: usize,
    pub count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ListResponse<T> {
    schemas: Vec<String>,
    total_results: usize,
    start_index: usize,
    items_per_page: usize,
    #[serde(rename = "Resources")]
    resources: Vec<T>,
}

#[derive(Debug, Serialize)]
struct ScimErrorResponse {
    schemas: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "scimType")]
    scim_type: Option<String>,
    detail: String,
    status: String,
}

#[derive(Debug, Clone)]
pub struct ScimError {
    pub status: StatusCode,
    pub detail: String,
    pub scim_type: Option<String>,
}

impl ScimError {
    fn invalid_request(detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            detail: detail.into(),
            scim_type: None,
        }
    }

    fn invalid_filter(detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            detail: detail.into(),
            scim_type: Some("invalidFilter".to_string()),
        }
    }

    fn not_found(detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            detail: detail.into(),
            scim_type: None,
        }
    }

    fn conflict(detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            detail: detail.into(),
            scim_type: Some("uniqueness".to_string()),
        }
    }

    fn internal(detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            detail: detail.into(),
            scim_type: None,
        }
    }
}

fn map_sql_error(error: rusqlite::Error) -> ScimError {
    if matches!(
        error,
        rusqlite::Error::SqliteFailure(ref err, _)
            if err.code == rusqlite::ErrorCode::ConstraintViolation
    ) {
        ScimError::conflict(
            "A resource with the same userName, externalId, or displayName already exists",
        )
    } else {
        ScimError::internal(format!("SCIM storage operation failed: {error}"))
    }
}

#[derive(Debug)]
struct DbUser {
    id: String,
    external_id: Option<String>,
    user_name: String,
    active: bool,
    display_name: Option<String>,
    given_name: Option<String>,
    family_name: Option<String>,
    email: Option<String>,
    locale: Option<String>,
    timezone: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug)]
struct DbGroup {
    id: String,
    external_id: Option<String>,
    display_name: String,
    created_at: String,
    updated_at: String,
    #[allow(dead_code)]
    active: bool,
}

fn row_to_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<DbUser> {
    Ok(DbUser {
        id: row.get(0)?,
        external_id: row.get(2)?,
        user_name: row.get(3)?,
        active: row.get::<_, i64>(4)? != 0,
        display_name: row.get(5)?,
        given_name: row.get(6)?,
        family_name: row.get(7)?,
        email: row.get(8)?,
        locale: row.get(9)?,
        timezone: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn row_to_group(row: &rusqlite::Row<'_>) -> rusqlite::Result<DbGroup> {
    Ok(DbGroup {
        id: row.get(0)?,
        external_id: row.get(2)?,
        display_name: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        active: row.get::<_, i64>(6)? != 0,
    })
}

fn load_members(
    conn: &rusqlite::Connection,
    tenant: &str,
    group_id: &str,
) -> Result<Vec<MemberResource>, ScimError> {
    let mut stmt = conn
        .prepare(
            "SELECT u.id, u.user_name
               FROM scim_group_members m
               JOIN scim_users u ON u.id = m.user_id AND u.tenant_id = ?1 AND u.deleted = 0
              WHERE m.group_id = ?2 ORDER BY u.id ASC",
        )
        .map_err(|e| ScimError::internal(format!("failed to read group members: {e}")))?;
    stmt.query_map(params![tenant, group_id], |row| {
        let id: String = row.get(0)?;
        let name: String = row.get(1)?;
        Ok(MemberResource {
            value: id.clone(),
            display: Some(name),
            reference: None,
            type_: Some("User".to_string()),
        })
    })
    .map_err(|e| ScimError::internal(format!("failed to list group members: {e}")))?
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(|e| ScimError::internal(format!("failed to decode group members: {e}")))
}

fn insert_members(
    tx: &rusqlite::Transaction<'_>,
    tenant: &str,
    group_id: &str,
    members: &[MemberInput],
    now: &str,
) -> Result<(), ScimError> {
    for member in members {
        let exists: Option<String> = tx
            .query_row(
                "SELECT id FROM scim_users WHERE tenant_id = ?1 AND id = ?2 AND deleted = 0",
                params![tenant, member.value],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| ScimError::internal(format!("failed to validate group member: {e}")))?;
        if exists.is_none() {
            return Err(ScimError::invalid_request(format!(
                "Group member user {} does not exist in this tenant",
                member.value
            )));
        }
        tx.execute(
            "INSERT INTO scim_group_members(group_id, user_id, created_at)
             VALUES (?1, ?2, ?3)",
            params![group_id, member.value, now],
        )
        .map_err(map_sql_error)?;
    }
    Ok(())
}

fn validate_members(members: Option<&[MemberInput]>) -> Result<Vec<MemberInput>, ScimError> {
    let members = members.unwrap_or_default();
    if members.len() > 1000 {
        return Err(ScimError::invalid_request(
            "A group may contain at most 1000 members",
        ));
    }
    let mut seen = HashSet::new();
    let mut result = Vec::with_capacity(members.len());
    for member in members {
        validate_id(&member.value)?;
        validate_optional_field(member.display.as_deref(), "member display", 256)?;
        if !seen.insert(member.value.clone()) {
            return Err(ScimError::invalid_request("Group members must be unique"));
        }
        result.push(member.clone());
    }
    Ok(result)
}

fn primary_email(emails: Option<&[EmailInput]>) -> Result<Option<String>, ScimError> {
    let emails = emails.unwrap_or_default();
    if emails.len() > 32 {
        return Err(ScimError::invalid_request(
            "A user may contain at most 32 email addresses",
        ));
    }
    let primary = emails
        .iter()
        .find(|email| email.primary.unwrap_or(false))
        .or_else(|| emails.first());
    if emails
        .iter()
        .filter(|email| email.primary.unwrap_or(false))
        .count()
        > 1
    {
        return Err(ScimError::invalid_request(
            "Only one primary email is allowed",
        ));
    }
    if let Some(email) = primary {
        validate_field(&email.value, "email", 512)?;
        validate_optional_field(email.type_.as_deref(), "email type", 64)?;
        if !email.value.contains('@') {
            return Err(ScimError::invalid_request("email must contain @"));
        }
        Ok(Some(email.value.clone()))
    } else {
        Ok(None)
    }
}

fn required_text<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str, ScimError> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ScimError::invalid_request(format!("{field} is required")))?;
    Ok(value)
}

fn optional_text(
    value: Option<&str>,
    field: &str,
    max_length: usize,
) -> Result<Option<String>, ScimError> {
    let Some(value) = value else { return Ok(None) };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    validate_field(value, field, max_length)?;
    Ok(Some(value.to_string()))
}

fn validate_optional_field(
    value: Option<&str>,
    field: &str,
    max_length: usize,
) -> Result<(), ScimError> {
    if let Some(value) = value {
        validate_field(value, field, max_length)?;
    }
    Ok(())
}

fn validate_field(value: &str, field: &str, max_length: usize) -> Result<(), ScimError> {
    if value.is_empty() || value.chars().count() > max_length || value.chars().any(char::is_control)
    {
        return Err(ScimError::invalid_request(format!(
            "{field} must be non-empty, at most {max_length} characters, and contain no control characters"
        )));
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<(), ScimError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(ScimError::invalid_request("resource id is invalid"));
    }
    Ok(())
}

fn validate_tenant(tenant: &str) -> Result<(), ScimError> {
    if tenant.is_empty()
        || tenant.len() > 128
        || !tenant
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(ScimError::internal("configured SCIM tenant id is invalid"));
    }
    Ok(())
}

fn validate_mappings(mappings: &[crate::config::ScimGroupMapping]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for mapping in mappings {
        validate_mapping_text(&mapping.tenant_id, "tenant_id", 256)?;
        let selectors = usize::from(mapping.group_id.is_some())
            + usize::from(mapping.group_external_id.is_some());
        if selectors != 1 {
            return Err(format!(
                "mapping for tenant '{}' must set exactly one of group_id or group_external_id",
                mapping.tenant_id
            ));
        }
        if let Some(group_id) = mapping.group_id.as_deref() {
            validate_mapping_text(group_id, "group_id", 256)?;
        }
        if let Some(external_id) = mapping.group_external_id.as_deref() {
            validate_mapping_text(external_id, "group_external_id", 256)?;
        }
        if mapping.roles.is_empty() && mapping.team_id.is_none() {
            return Err("mapping must grant at least one role or team".to_string());
        }
        for role in &mapping.roles {
            validate_mapping_text(role, "role", 128)?;
        }
        if let Some(team_id) = mapping.team_id.as_deref() {
            validate_mapping_text(team_id, "team_id", 256)?;
        }
        let selector = mapping
            .group_id
            .as_deref()
            .or(mapping.group_external_id.as_deref())
            .expect("exactly one selector was checked");
        let key = (mapping.tenant_id.as_str(), selector);
        if !seen.insert(key) {
            return Err(format!(
                "duplicate SCIM group mapping for tenant '{}' and group selector '{selector}'",
                mapping.tenant_id
            ));
        }
    }
    Ok(())
}

fn validate_mapping_text(value: &str, field: &str, max: usize) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if trimmed != value {
        return Err(format!(
            "{field} must not have leading or trailing whitespace"
        ));
    }
    if value.len() > max {
        return Err(format!("{field} exceeds {max} bytes"));
    }
    Ok(())
}

fn mapping_group_predicate(mapping: &crate::config::ScimGroupMapping) -> (&'static str, &str) {
    if let Some(group_id) = mapping.group_id.as_deref() {
        ("g.id = ?5", group_id)
    } else {
        (
            "g.external_id = ?5",
            mapping
                .group_external_id
                .as_deref()
                .expect("mapping validation requires a group selector"),
        )
    }
}

impl DbUser {
    fn into_resource(self) -> UserResource {
        let name = if self.given_name.is_some() || self.family_name.is_some() {
            Some(NameResource {
                given_name: self.given_name,
                family_name: self.family_name,
            })
        } else {
            None
        };
        let emails = self
            .email
            .map(|value| {
                vec![EmailResource {
                    value,
                    type_: Some("work".to_string()),
                    primary: Some(true),
                }]
            })
            .unwrap_or_default();
        UserResource {
            schemas: vec![USER_SCHEMA.to_string()],
            id: self.id,
            external_id: self.external_id,
            user_name: self.user_name,
            active: self.active,
            display_name: self.display_name,
            name,
            emails,
            locale: self.locale,
            timezone: self.timezone,
            meta: MetaResource {
                resource_type: "User".to_string(),
                created: self.created_at,
                last_modified: self.updated_at.clone(),
                version: format!("W/\"{}\"", self.updated_at),
            },
        }
    }
}

impl UserResource {
    fn to_input(&self) -> UserInput {
        UserInput {
            schemas: vec![USER_SCHEMA.to_string()],
            external_id: self.external_id.clone(),
            user_name: Some(self.user_name.clone()),
            active: Some(self.active),
            display_name: self.display_name.clone(),
            name: self.name.as_ref().map(|name| NameInput {
                given_name: name.given_name.clone(),
                family_name: name.family_name.clone(),
            }),
            emails: (!self.emails.is_empty()).then(|| {
                self.emails
                    .iter()
                    .map(|email| EmailInput {
                        value: email.value.clone(),
                        type_: email.type_.clone(),
                        primary: email.primary,
                    })
                    .collect()
            }),
            locale: self.locale.clone(),
            timezone: self.timezone.clone(),
        }
    }
}

impl DbGroup {
    fn into_resource(self, members: Vec<MemberResource>) -> GroupResource {
        GroupResource {
            schemas: vec![GROUP_SCHEMA.to_string()],
            id: self.id,
            external_id: self.external_id,
            display_name: self.display_name,
            members,
            meta: MetaResource {
                resource_type: "Group".to_string(),
                created: self.created_at,
                last_modified: self.updated_at.clone(),
                version: format!("W/\"{}\"", self.updated_at),
            },
        }
    }
}

impl GroupResource {
    fn to_input(&self) -> GroupInput {
        GroupInput {
            schemas: vec![GROUP_SCHEMA.to_string()],
            external_id: self.external_id.clone(),
            display_name: Some(self.display_name.clone()),
            members: Some(
                self.members
                    .iter()
                    .map(|member| MemberInput {
                        value: member.value.clone(),
                        display: member.display.clone(),
                    })
                    .collect(),
            ),
        }
    }
}

fn apply_user_object(
    input: &mut UserInput,
    object: &serde_json::Map<String, Value>,
) -> Result<(), ScimError> {
    for (key, value) in object {
        match key.as_str() {
            "active" => {
                input.active = Some(
                    value
                        .as_bool()
                        .ok_or_else(|| ScimError::invalid_request("active must be a boolean"))?,
                )
            }
            "userName" => {
                input.user_name = Some(
                    value
                        .as_str()
                        .ok_or_else(|| ScimError::invalid_request("userName must be a string"))?
                        .to_string(),
                )
            }
            "externalId" => {
                input.external_id = Some(
                    value
                        .as_str()
                        .ok_or_else(|| ScimError::invalid_request("externalId must be a string"))?
                        .to_string(),
                )
            }
            "displayName" => {
                input.display_name = Some(
                    value
                        .as_str()
                        .ok_or_else(|| ScimError::invalid_request("displayName must be a string"))?
                        .to_string(),
                )
            }
            "locale" => {
                input.locale = Some(
                    value
                        .as_str()
                        .ok_or_else(|| ScimError::invalid_request("locale must be a string"))?
                        .to_string(),
                )
            }
            "timezone" => {
                input.timezone = Some(
                    value
                        .as_str()
                        .ok_or_else(|| ScimError::invalid_request("timezone must be a string"))?
                        .to_string(),
                )
            }
            "name" => {
                input.name = Some(
                    serde_json::from_value(value.clone())
                        .map_err(|_| ScimError::invalid_request("name must be an object"))?,
                )
            }
            "emails" => {
                input.emails = Some(
                    serde_json::from_value(value.clone())
                        .map_err(|_| ScimError::invalid_request("emails must be an array"))?,
                )
            }
            "schemas" => {}
            _ => {}
        }
    }
    Ok(())
}

fn member_values(value: &Value) -> Result<Vec<String>, ScimError> {
    let values: Vec<String> = if let Some(array) = value.as_array() {
        array
            .iter()
            .map(|value| {
                if let Some(id) = value.as_str() {
                    return Ok(id.to_string());
                }
                value
                    .as_object()
                    .and_then(|member| member.get("value"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| {
                        ScimError::invalid_request(
                            "member value must be a user id or an object containing value",
                        )
                    })
            })
            .collect::<Result<_, _>>()?
    } else if let Some(object) = value.as_object() {
        vec![
            object
                .get("value")
                .and_then(Value::as_str)
                .ok_or_else(|| ScimError::invalid_request("member value must contain a user id"))?
                .to_string(),
        ]
    } else {
        return Err(ScimError::invalid_request(
            "members value must be an array or object",
        ));
    };
    for id in &values {
        validate_id(id)?;
    }
    Ok(values)
}

fn parse_member_filter(path: &str) -> Result<String, ScimError> {
    let prefix = "members[value eq ";
    if !path.starts_with(prefix) || !path.ends_with(']') {
        return Err(ScimError::invalid_request(
            "Only members[value eq \"id\"] is supported",
        ));
    }
    let quoted = &path[prefix.len()..path.len() - 1];
    let value: String = serde_json::from_str(quoted)
        .map_err(|_| ScimError::invalid_request("Invalid member filter"))?;
    validate_id(&value)?;
    Ok(value)
}

pub fn parse_list_params(query: Option<&str>) -> Result<ListParams, ScimError> {
    let mut filter = None;
    let mut start_index = 1;
    let mut count = MAX_PAGE_SIZE;
    let Some(query) = query else {
        return Ok(ListParams {
            filter,
            start_index,
            count,
        });
    };
    if query.len() > 4096 {
        return Err(ScimError::invalid_request("query string is too long"));
    }
    for pair in query.split('&') {
        let Some((raw_key, raw_value)) = pair.split_once('=') else {
            return Err(ScimError::invalid_request("invalid query parameter"));
        };
        let key = urlencoding::decode(raw_key)
            .map_err(|_| ScimError::invalid_request("invalid query parameter encoding"))?;
        let value = urlencoding::decode(raw_value)
            .map_err(|_| ScimError::invalid_request("invalid query parameter encoding"))?;
        match key.as_ref() {
            "filter" => filter = Some(parse_filter(&value)?),
            "startIndex" => {
                start_index = value
                    .parse::<usize>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| {
                        ScimError::invalid_request("startIndex must be a positive integer")
                    })?;
            }
            "count" => {
                count = value
                    .parse::<usize>()
                    .ok()
                    .filter(|value| *value <= MAX_PAGE_SIZE)
                    .ok_or_else(|| ScimError::invalid_request("count must be between 0 and 100"))?;
            }
            "attributes" | "excludedAttributes" => {}
            "sortBy" | "sortOrder" => {
                return Err(ScimError::invalid_request("sorting is not supported"));
            }
            _ => {
                return Err(ScimError::invalid_request(format!(
                    "unsupported query parameter: {key}"
                )));
            }
        }
    }
    Ok(ListParams {
        filter,
        start_index,
        count,
    })
}

fn parse_filter(value: &str) -> Result<ScimFilter, ScimError> {
    let lowered = value.to_ascii_lowercase();
    let Some(operator_start) = lowered.find(" eq ") else {
        return Err(ScimError::invalid_filter(
            "Only the SCIM eq filter operator is supported",
        ));
    };
    if lowered[operator_start + 4..].contains(" eq ") {
        return Err(ScimError::invalid_filter(
            "Only one filter expression is supported",
        ));
    }
    let attribute = value[..operator_start].trim();
    let attribute = if attribute.eq_ignore_ascii_case("username") {
        "userName"
    } else if attribute.eq_ignore_ascii_case("externalid") {
        "externalId"
    } else if attribute.eq_ignore_ascii_case("displayname") {
        "displayName"
    } else {
        return Err(ScimError::invalid_filter("Unsupported filter attribute"));
    };
    let value: String = serde_json::from_str(value[operator_start + 4..].trim())
        .map_err(|_| ScimError::invalid_filter("Filter value must be a quoted string"))?;
    validate_field(&value, "filter value", 512)?;
    Ok(ScimFilter {
        attribute: attribute.to_string(),
        value,
    })
}

fn validate_resource_schemas(schemas: &[String], expected: &str) -> Result<(), ScimError> {
    if schemas.len() != 1 || schemas[0] != expected {
        return Err(ScimError::invalid_request(format!(
            "schemas must contain exactly the core {expected} URN"
        )));
    }
    Ok(())
}

fn filter_column(attribute: &str) -> Option<&'static str> {
    match attribute {
        "userName" => Some("user_name"),
        "externalId" => Some("external_id"),
        "displayName" => Some("display_name"),
        _ => None,
    }
}

fn json_response<T: Serialize>(status: StatusCode, data: &T) -> Response<BoxBody> {
    let body = serde_json::to_vec(data).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(status)
        .header("Content-Type", SCIM_MEDIA_TYPE)
        .body(full(body))
        .expect("valid SCIM response")
}

fn created_response<T: Serialize>(resource_type: &str, id: &str, data: &T) -> Response<BoxBody> {
    let body = serde_json::to_vec(data).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(StatusCode::CREATED)
        .header("Content-Type", SCIM_MEDIA_TYPE)
        .header("Location", format!("/scim/v2/{resource_type}s/{id}"))
        .body(full(body))
        .expect("valid SCIM response")
}

fn empty_response(status: StatusCode) -> Response<BoxBody> {
    Response::builder()
        .status(status)
        .header("Content-Type", SCIM_MEDIA_TYPE)
        .body(full(bytes::Bytes::new()))
        .expect("valid SCIM response")
}

fn error_response(error: ScimError) -> Response<BoxBody> {
    json_response(
        error.status,
        &ScimErrorResponse {
            schemas: vec!["urn:ietf:params:scim:api:messages:2.0:Error".to_string()],
            scim_type: error.scim_type,
            detail: error.detail,
            status: error.status.as_u16().to_string(),
        },
    )
}

pub fn storage_unavailable() -> Response<BoxBody> {
    error_response(ScimError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        detail: "SCIM storage is unavailable".to_string(),
        scim_type: None,
    })
}

pub fn authentication_required() -> Response<BoxBody> {
    error_response(ScimError {
        status: StatusCode::UNAUTHORIZED,
        detail: "SCIM requires API-key authentication; configure --api-key or AGENTKERNEL_API_KEY"
            .to_string(),
        scim_type: None,
    })
}

pub fn not_found_response() -> Response<BoxBody> {
    error_response(ScimError::not_found("SCIM resource not found"))
}

async fn read_json<T: for<'de> Deserialize<'de>>(
    req: Request<Incoming>,
) -> Result<T, Response<BoxBody>> {
    if req
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > MAX_SCIM_BODY_BYTES)
    {
        return Err(error_response(ScimError {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            detail: "SCIM request body exceeds 1 MiB".to_string(),
            scim_type: None,
        }));
    }
    let body = read_body_bytes(req)
        .await
        .map_err(|_| error_response(ScimError::invalid_request("failed to read request body")))?;
    if body.len() > MAX_SCIM_BODY_BYTES {
        return Err(error_response(ScimError {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            detail: "SCIM request body exceeds 1 MiB".to_string(),
            scim_type: None,
        }));
    }
    serde_json::from_slice(&body)
        .map_err(|_| error_response(ScimError::invalid_request("invalid SCIM JSON")))
}

fn list_response<T>(
    resources: Vec<T>,
    total_results: usize,
    start_index: usize,
) -> ListResponse<T> {
    ListResponse {
        schemas: vec![LIST_SCHEMA.to_string()],
        total_results,
        start_index,
        items_per_page: resources.len(),
        resources,
    }
}

pub async fn handle_service_provider_config() -> Response<BoxBody> {
    json_response(
        StatusCode::OK,
        &serde_json::json!({
            "schemas": [SERVICE_PROVIDER_CONFIG_SCHEMA],
            "patch": {"supported": true},
            "bulk": {"supported": false, "maxOperations": 0, "maxPayloadSize": 0},
            "filter": {"supported": true, "maxResults": MAX_PAGE_SIZE},
            "changePassword": {"supported": false},
            "sort": {"supported": false},
            "etag": {"supported": false},
            "authenticationSchemes": [{"type": "oauthbearertoken", "name": "Bearer API key", "description": "Authorization: Bearer <agentkernel API key>"}]
        }),
    )
}

pub async fn handle_resource_types() -> Response<BoxBody> {
    json_response(
        StatusCode::OK,
        &serde_json::json!({
            "schemas": [LIST_SCHEMA],
            "totalResults": 2,
            "startIndex": 1,
            "itemsPerPage": 2,
            "Resources": [
                {"schemas": [RESOURCE_TYPE_SCHEMA], "id": "User", "name": "User", "endpoint": "/scim/v2/Users", "schema": USER_SCHEMA, "meta": {"resourceType": "ResourceType"}},
                {"schemas": [RESOURCE_TYPE_SCHEMA], "id": "Group", "name": "Group", "endpoint": "/scim/v2/Groups", "schema": GROUP_SCHEMA, "meta": {"resourceType": "ResourceType"}}
            ]
        }),
    )
}

pub async fn handle_resource_type(id: &str) -> Response<BoxBody> {
    match id {
        "User" => json_response(
            StatusCode::OK,
            &serde_json::json!({
                "schemas": [RESOURCE_TYPE_SCHEMA], "id": "User", "name": "User",
                "endpoint": "/scim/v2/Users", "schema": USER_SCHEMA,
                "meta": {"resourceType": "ResourceType"}
            }),
        ),
        "Group" => json_response(
            StatusCode::OK,
            &serde_json::json!({
                "schemas": [RESOURCE_TYPE_SCHEMA], "id": "Group", "name": "Group",
                "endpoint": "/scim/v2/Groups", "schema": GROUP_SCHEMA,
                "meta": {"resourceType": "ResourceType"}
            }),
        ),
        _ => error_response(ScimError::not_found("ResourceType not found")),
    }
}

pub async fn handle_schemas() -> Response<BoxBody> {
    json_response(
        StatusCode::OK,
        &serde_json::json!({
            "schemas": [LIST_SCHEMA],
            "totalResults": 2,
            "startIndex": 1,
            "itemsPerPage": 2,
            "Resources": [user_schema_resource(), group_schema_resource()]
        }),
    )
}

fn user_schema_resource() -> Value {
    serde_json::json!({
        "schemas": [SCHEMA_SCHEMA],
        "id": USER_SCHEMA,
        "name": "User",
        "description": "SCIM User core schema",
        "attributes": [
            {"name": "externalId", "type": "string", "multiValued": false, "required": false, "caseExact": false, "mutability": "readWrite", "returned": "default"},
            {"name": "userName", "type": "string", "multiValued": false, "required": true, "caseExact": false, "mutability": "readWrite", "returned": "default", "uniqueness": "server"},
            {"name": "active", "type": "boolean", "multiValued": false, "required": false, "caseExact": false, "mutability": "readWrite", "returned": "default"},
            {"name": "displayName", "type": "string", "multiValued": false, "required": false, "caseExact": false, "mutability": "readWrite", "returned": "default"},
            {"name": "name", "type": "complex", "multiValued": false, "required": false, "mutability": "readWrite", "returned": "default", "subAttributes": [
                {"name": "givenName", "type": "string", "multiValued": false, "required": false, "caseExact": false, "mutability": "readWrite", "returned": "default"},
                {"name": "familyName", "type": "string", "multiValued": false, "required": false, "caseExact": false, "mutability": "readWrite", "returned": "default"}
            ]},
            {"name": "emails", "type": "complex", "multiValued": true, "required": false, "mutability": "readWrite", "returned": "default", "subAttributes": [
                {"name": "value", "type": "string", "multiValued": false, "required": true, "caseExact": false, "mutability": "readWrite", "returned": "default"},
                {"name": "type", "type": "string", "multiValued": false, "required": false, "caseExact": false, "mutability": "readWrite", "returned": "default"},
                {"name": "primary", "type": "boolean", "multiValued": false, "required": false, "mutability": "readWrite", "returned": "default"}
            ]},
            {"name": "locale", "type": "string", "multiValued": false, "required": false, "caseExact": false, "mutability": "readWrite", "returned": "default"},
            {"name": "timezone", "type": "string", "multiValued": false, "required": false, "caseExact": false, "mutability": "readWrite", "returned": "default"}
        ],
        "meta": {"resourceType": "Schema"}
    })
}

fn group_schema_resource() -> Value {
    serde_json::json!({
        "schemas": [SCHEMA_SCHEMA],
        "id": GROUP_SCHEMA,
        "name": "Group",
        "description": "SCIM Group core schema",
        "attributes": [
            {"name": "externalId", "type": "string", "multiValued": false, "required": false, "caseExact": false, "mutability": "readWrite", "returned": "default"},
            {"name": "displayName", "type": "string", "multiValued": false, "required": true, "caseExact": false, "mutability": "readWrite", "returned": "default", "uniqueness": "server"},
            {"name": "members", "type": "complex", "multiValued": true, "required": false, "mutability": "readWrite", "returned": "default", "subAttributes": [
                {"name": "value", "type": "string", "multiValued": false, "required": true, "caseExact": true, "mutability": "readWrite", "returned": "default"},
                {"name": "display", "type": "string", "multiValued": false, "required": false, "caseExact": false, "mutability": "readOnly", "returned": "default"},
                {"name": "$ref", "type": "reference", "multiValued": false, "required": false, "mutability": "readOnly", "returned": "default"},
                {"name": "type", "type": "string", "multiValued": false, "required": false, "mutability": "readOnly", "returned": "default"}
            ]}
        ],
        "meta": {"resourceType": "Schema"}
    })
}

pub async fn handle_schema(id: &str) -> Response<BoxBody> {
    let id = match urlencoding::decode(id) {
        Ok(id) => id,
        Err(_) => return error_response(ScimError::not_found("Schema not found")),
    };
    let resource = match id.as_ref() {
        USER_SCHEMA => user_schema_resource(),
        GROUP_SCHEMA => group_schema_resource(),
        _ => return error_response(ScimError::not_found("Schema not found")),
    };
    json_response(StatusCode::OK, &resource)
}

pub async fn handle_list_users(
    store: Arc<ScimStore>,
    tenant: &str,
    query: Option<&str>,
) -> Response<BoxBody> {
    let params = match parse_list_params(query) {
        Ok(params) => params,
        Err(error) => return error_response(error),
    };
    match store.list_users(
        tenant,
        params.filter.as_ref(),
        params.start_index,
        params.count,
    ) {
        Ok((resources, total)) => json_response(
            StatusCode::OK,
            &list_response(resources, total, params.start_index),
        ),
        Err(error) => error_response(error),
    }
}

pub async fn handle_create_user(
    req: Request<Incoming>,
    store: Arc<ScimStore>,
    tenant: &str,
) -> Response<BoxBody> {
    let input: UserInput = match read_json(req).await {
        Ok(input) => input,
        Err(response) => return response,
    };
    if let Err(error) = validate_resource_schemas(&input.schemas, USER_SCHEMA) {
        return error_response(error);
    }
    match store.create_user(tenant, &input) {
        Ok(resource) => created_response("User", &resource.id, &resource),
        Err(error) => error_response(error),
    }
}

pub async fn handle_get_user(id: &str, store: Arc<ScimStore>, tenant: &str) -> Response<BoxBody> {
    match store.get_user(tenant, id) {
        Ok(resource) => json_response(StatusCode::OK, &resource),
        Err(error) => error_response(error),
    }
}

pub async fn handle_replace_user(
    req: Request<Incoming>,
    id: &str,
    store: Arc<ScimStore>,
    tenant: &str,
) -> Response<BoxBody> {
    let input: UserInput = match read_json(req).await {
        Ok(input) => input,
        Err(response) => return response,
    };
    if let Err(error) = validate_resource_schemas(&input.schemas, USER_SCHEMA) {
        return error_response(error);
    }
    match store.replace_user(tenant, id, &input) {
        Ok(resource) => json_response(StatusCode::OK, &resource),
        Err(error) => error_response(error),
    }
}

pub async fn handle_patch_user(
    req: Request<Incoming>,
    id: &str,
    store: Arc<ScimStore>,
    tenant: &str,
) -> Response<BoxBody> {
    let patch: PatchRequest = match read_json(req).await {
        Ok(patch) => patch,
        Err(response) => return response,
    };
    if patch.schemas.len() != 1 || patch.schemas[0] != PATCH_SCHEMA {
        return error_response(ScimError::invalid_request("Unsupported PATCH schema"));
    }
    if patch.operations.is_empty() || patch.operations.len() > 32 {
        return error_response(ScimError::invalid_request(
            "PATCH must contain between 1 and 32 operations",
        ));
    }
    match store.patch_user(tenant, id, &patch) {
        Ok(resource) => json_response(StatusCode::OK, &resource),
        Err(error) => error_response(error),
    }
}

pub async fn handle_delete_user(
    id: &str,
    store: Arc<ScimStore>,
    tenant: &str,
) -> Response<BoxBody> {
    match store.delete_user(tenant, id) {
        Ok(()) => empty_response(StatusCode::NO_CONTENT),
        Err(error) => error_response(error),
    }
}

pub async fn handle_list_groups(
    store: Arc<ScimStore>,
    tenant: &str,
    query: Option<&str>,
) -> Response<BoxBody> {
    let params = match parse_list_params(query) {
        Ok(params) => params,
        Err(error) => return error_response(error),
    };
    match store.list_groups(
        tenant,
        params.filter.as_ref(),
        params.start_index,
        params.count,
    ) {
        Ok((resources, total)) => json_response(
            StatusCode::OK,
            &list_response(resources, total, params.start_index),
        ),
        Err(error) => error_response(error),
    }
}

pub async fn handle_create_group(
    req: Request<Incoming>,
    store: Arc<ScimStore>,
    tenant: &str,
) -> Response<BoxBody> {
    let input: GroupInput = match read_json(req).await {
        Ok(input) => input,
        Err(response) => return response,
    };
    if let Err(error) = validate_resource_schemas(&input.schemas, GROUP_SCHEMA) {
        return error_response(error);
    }
    match store.create_group(tenant, &input) {
        Ok(resource) => created_response("Group", &resource.id, &resource),
        Err(error) => error_response(error),
    }
}

pub async fn handle_get_group(id: &str, store: Arc<ScimStore>, tenant: &str) -> Response<BoxBody> {
    match store.get_group(tenant, id) {
        Ok(resource) => json_response(StatusCode::OK, &resource),
        Err(error) => error_response(error),
    }
}

pub async fn handle_replace_group(
    req: Request<Incoming>,
    id: &str,
    store: Arc<ScimStore>,
    tenant: &str,
) -> Response<BoxBody> {
    let input: GroupInput = match read_json(req).await {
        Ok(input) => input,
        Err(response) => return response,
    };
    if let Err(error) = validate_resource_schemas(&input.schemas, GROUP_SCHEMA) {
        return error_response(error);
    }
    match store.replace_group(tenant, id, &input) {
        Ok(resource) => json_response(StatusCode::OK, &resource),
        Err(error) => error_response(error),
    }
}

pub async fn handle_patch_group(
    req: Request<Incoming>,
    id: &str,
    store: Arc<ScimStore>,
    tenant: &str,
) -> Response<BoxBody> {
    let patch: PatchRequest = match read_json(req).await {
        Ok(patch) => patch,
        Err(response) => return response,
    };
    if patch.schemas.len() != 1 || patch.schemas[0] != PATCH_SCHEMA {
        return error_response(ScimError::invalid_request("Unsupported PATCH schema"));
    }
    if patch.operations.is_empty() || patch.operations.len() > 32 {
        return error_response(ScimError::invalid_request(
            "PATCH must contain between 1 and 32 operations",
        ));
    }
    match store.patch_group(tenant, id, &patch) {
        Ok(resource) => json_response(StatusCode::OK, &resource),
        Err(error) => error_response(error),
    }
}

pub async fn handle_delete_group(
    id: &str,
    store: Arc<ScimStore>,
    tenant: &str,
) -> Response<BoxBody> {
    match store.delete_group(tenant, id) {
        Ok(()) => empty_response(StatusCode::NO_CONTENT),
        Err(error) => error_response(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, ScimStore) {
        let temp = tempfile::TempDir::new().unwrap();
        let durable = DurableStorage::new(temp.path().join("scim.db")).unwrap();
        (temp, ScimStore::new(durable))
    }

    #[test]
    fn user_lifecycle_is_tenant_scoped_and_deactivation_persists() {
        let (_temp, store) = store();
        let user = store
            .create_user(
                "acme",
                &UserInput {
                    user_name: Some("alice@example.com".to_string()),
                    emails: Some(vec![EmailInput {
                        value: "alice@example.com".to_string(),
                        primary: Some(true),
                        ..Default::default()
                    }]),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(store.get_user("other", &user.id).is_err());
        let patched = store
            .patch_user(
                "acme",
                &user.id,
                &PatchRequest {
                    schemas: vec![PATCH_SCHEMA.to_string()],
                    operations: vec![PatchOperation {
                        op: "replace".to_string(),
                        path: Some("active".to_string()),
                        value: Value::Bool(false),
                    }],
                },
            )
            .unwrap();
        assert!(!patched.active);
        store.delete_user("acme", &user.id).unwrap();
        assert!(store.get_user("acme", &user.id).is_err());
        let replacement = store
            .create_user(
                "acme",
                &UserInput {
                    user_name: Some("alice@example.com".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_ne!(replacement.id, user.id);
    }

    #[test]
    fn groups_sync_members_and_reject_cross_tenant_members() {
        let (_temp, store) = store();
        let user = store
            .create_user(
                "acme",
                &UserInput {
                    user_name: Some("alice".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        let group = store
            .create_group(
                "acme",
                &GroupInput {
                    display_name: Some("Engineering".to_string()),
                    members: Some(vec![MemberInput {
                        value: user.id.clone(),
                        ..Default::default()
                    }]),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(group.members.len(), 1);
        let patched = store
            .patch_group(
                "acme",
                &group.id,
                &PatchRequest {
                    schemas: vec![PATCH_SCHEMA.to_string()],
                    operations: vec![PatchOperation {
                        op: "remove".to_string(),
                        path: Some(format!("members[value eq \"{}\"]", user.id)),
                        value: Value::Null,
                    }],
                },
            )
            .unwrap();
        assert!(patched.members.is_empty());
        store.delete_group("acme", &group.id).unwrap();
        assert!(store.get_group("acme", &group.id).is_err());
        let replacement = store
            .create_group(
                "acme",
                &GroupInput {
                    display_name: Some("Engineering".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_ne!(replacement.id, group.id);
        assert_eq!(replacement.display_name, "Engineering");
        let error = store
            .create_group(
                "other",
                &GroupInput {
                    display_name: Some("Engineering".to_string()),
                    members: Some(vec![MemberInput {
                        value: user.id,
                        ..Default::default()
                    }]),
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn explicit_external_id_mapping_materializes_roles_and_teams_fail_closed() {
        let temp = tempfile::TempDir::new().unwrap();
        let db_path = temp.path().join("scim.db");
        let durable = DurableStorage::new(db_path.clone()).unwrap();
        let mappings = vec![crate::config::ScimGroupMapping {
            tenant_id: "acme".to_string(),
            group_id: None,
            group_external_id: Some("idp-engineering".to_string()),
            roles: vec!["developer".to_string()],
            team_id: Some("engineering".to_string()),
        }];
        let store = ScimStore::new_with_mappings(durable, mappings.clone()).unwrap();
        let user = store
            .create_user(
                "acme",
                &UserInput {
                    external_id: Some("idp-alice".to_string()),
                    user_name: Some("alice@example.com".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        let group = store
            .create_group(
                "acme",
                &GroupInput {
                    external_id: Some("idp-engineering".to_string()),
                    display_name: Some("Engineering".to_string()),
                    members: Some(vec![MemberInput {
                        value: user.id.clone(),
                        ..Default::default()
                    }]),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            store.principal_bindings("acme", "idp-alice").unwrap(),
            (
                vec!["developer".to_string()],
                vec!["engineering".to_string()]
            )
        );
        let cleared =
            ScimStore::new_with_mappings(DurableStorage::new(db_path.clone()).unwrap(), Vec::new())
                .unwrap();
        assert_eq!(
            cleared.principal_bindings("acme", "idp-alice").unwrap(),
            (Vec::<String>::new(), Vec::<String>::new())
        );
        let restarted =
            ScimStore::new_with_mappings(DurableStorage::new(db_path).unwrap(), mappings).unwrap();
        assert_eq!(
            restarted.principal_bindings("acme", "idp-alice").unwrap(),
            (
                vec!["developer".to_string()],
                vec!["engineering".to_string()]
            )
        );
        store
            .create_user(
                "other",
                &UserInput {
                    external_id: Some("idp-alice".to_string()),
                    user_name: Some("other-alice@example.com".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            store.principal_bindings("other", "idp-alice").unwrap(),
            (Vec::<String>::new(), Vec::<String>::new())
        );

        store
            .patch_group(
                "acme",
                &group.id,
                &PatchRequest {
                    schemas: vec![PATCH_SCHEMA.to_string()],
                    operations: vec![PatchOperation {
                        op: "remove".to_string(),
                        path: Some("members".to_string()),
                        value: serde_json::json!([user.id]),
                    }],
                },
            )
            .unwrap();
        assert_eq!(
            store.principal_bindings("acme", "idp-alice").unwrap(),
            (Vec::<String>::new(), Vec::<String>::new())
        );

        store
            .patch_group(
                "acme",
                &group.id,
                &PatchRequest {
                    schemas: vec![PATCH_SCHEMA.to_string()],
                    operations: vec![PatchOperation {
                        op: "add".to_string(),
                        path: Some("members".to_string()),
                        value: serde_json::json!([{"value": user.id}]),
                    }],
                },
            )
            .unwrap();
        store
            .patch_user(
                "acme",
                &user.id,
                &PatchRequest {
                    schemas: vec![PATCH_SCHEMA.to_string()],
                    operations: vec![PatchOperation {
                        op: "replace".to_string(),
                        path: Some("active".to_string()),
                        value: Value::Bool(false),
                    }],
                },
            )
            .unwrap();
        assert_eq!(
            store.principal_bindings("acme", "idp-alice").unwrap(),
            (Vec::<String>::new(), Vec::<String>::new())
        );
    }

    #[test]
    fn invalid_or_unmatched_mapping_grants_nothing() {
        let (_temp, store) = store();
        assert!(
            ScimStore::new_with_mappings(
                DurableStorage::new(tempfile::tempdir().unwrap().path().join("scim.db")).unwrap(),
                vec![crate::config::ScimGroupMapping {
                    tenant_id: "acme".to_string(),
                    group_id: Some("group".to_string()),
                    group_external_id: Some("also-group".to_string()),
                    roles: vec!["admin".to_string()],
                    team_id: None,
                }],
            )
            .is_err()
        );
        assert_eq!(
            store
                .principal_bindings("acme", "unprovisioned-sub")
                .unwrap(),
            (Vec::<String>::new(), Vec::<String>::new())
        );
    }

    #[test]
    fn filters_are_strict_and_pagination_is_one_based() {
        let params = parse_list_params(Some(
            "filter=userName%20eq%20%22alice%22&startIndex=2&count=10",
        ))
        .unwrap();
        assert_eq!(params.filter.unwrap().value, "alice");
        assert_eq!(params.start_index, 2);
        assert_eq!(params.count, 10);
        let case_insensitive =
            parse_list_params(Some("filter=USERNAME%20EQ%20%22alice%22")).unwrap();
        assert_eq!(case_insensitive.filter.unwrap().attribute, "userName");
        assert!(parse_list_params(Some("filter=userName%20ne%20%22alice%22")).is_err());
        assert!(parse_list_params(Some("count=101")).is_err());
    }

    #[test]
    fn resource_names_are_case_insensitively_unique_and_pagination_counts_all_rows() {
        let (_temp, store) = store();
        for user_name in ["Alice", "Bob", "Carol"] {
            store
                .create_user(
                    "acme",
                    &UserInput {
                        user_name: Some(user_name.to_string()),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        let duplicate_user = store.create_user(
            "acme",
            &UserInput {
                user_name: Some("alice".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(duplicate_user.unwrap_err().status, StatusCode::CONFLICT);

        let filter = ScimFilter {
            attribute: "userName".to_string(),
            value: "bob".to_string(),
        };
        let (filtered, total) = store.list_users("acme", Some(&filter), 1, 1).unwrap();
        assert_eq!(total, 1);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].user_name, "Bob");

        let (page, total) = store.list_users("acme", None, 2, 1).unwrap();
        assert_eq!(total, 3);
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].user_name, "Bob");

        store
            .create_group(
                "acme",
                &GroupInput {
                    display_name: Some("Engineering".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        let group_filter =
            parse_list_params(Some("filter=DISPLAYNAME%20eQ%20%22engineering%22&count=1"))
                .unwrap()
                .filter;
        let (groups, group_total) = store
            .list_groups("acme", group_filter.as_ref(), 1, 1)
            .unwrap();
        assert_eq!(group_total, 1);
        assert_eq!(groups.len(), 1);
        let duplicate_group = store.create_group(
            "acme",
            &GroupInput {
                display_name: Some("engineering".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(duplicate_group.unwrap_err().status, StatusCode::CONFLICT);
    }

    #[test]
    fn resource_body_requires_exact_core_schema() {
        assert!(validate_resource_schemas(&[], USER_SCHEMA).is_err());
        assert!(
            validate_resource_schemas(&["urn:example:extension".to_string()], USER_SCHEMA).is_err()
        );
        assert!(validate_resource_schemas(&[USER_SCHEMA.to_string()], USER_SCHEMA).is_ok());
        assert!(validate_resource_schemas(&[GROUP_SCHEMA.to_string()], GROUP_SCHEMA).is_ok());
    }

    #[test]
    fn resource_serialization_uses_scim_wire_names_and_schema_urns() {
        let (_temp, store) = store();
        let user = store
            .create_user(
                "acme",
                &UserInput {
                    user_name: Some("alice".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        let value = serde_json::to_value(user).unwrap();
        assert_eq!(value["schemas"][0], USER_SCHEMA);
        assert_eq!(value["userName"], "alice");
        assert!(value.get("user_name").is_none());
    }

    #[test]
    fn list_response_uses_canonical_capitalized_resources_field() {
        let response = list_response(vec![serde_json::json!({"id": "user-1"})], 1, 1);
        let value = serde_json::to_value(response).unwrap();
        assert!(value.get("Resources").is_some());
        assert!(value.get("resources").is_none());
    }

    #[test]
    fn patch_deserializes_canonical_capitalized_operations_field() {
        let patch: PatchRequest = serde_json::from_str(&format!(
            "{{\"schemas\":[\"{PATCH_SCHEMA}\"],\"Operations\":[{{\"op\":\"replace\",\"path\":\"active\",\"value\":false}}]}}"
        ))
        .unwrap();
        assert_eq!(patch.operations.len(), 1);
        assert_eq!(patch.operations[0].op, "replace");
    }

    #[test]
    fn group_patch_accepts_canonical_member_object_arrays() {
        let values = member_values(&serde_json::json!([
            {"value": "user-1"},
            {"value": "user-2", "display": "Ignored display metadata"}
        ]))
        .unwrap();
        assert_eq!(values, vec!["user-1", "user-2"]);
    }

    #[test]
    fn schema_discovery_resources_declare_the_schema_resource_urn() {
        assert_eq!(
            SCHEMA_SCHEMA,
            "urn:ietf:params:scim:schemas:core:2.0:Schema"
        );
        for resource in [user_schema_resource(), group_schema_resource()] {
            assert_eq!(resource["schemas"][0], SCHEMA_SCHEMA);
            assert!(resource["attributes"].as_array().is_some_and(|attrs| {
                attrs.iter().any(|attr| {
                    attr["type"] == "complex"
                        && attr["subAttributes"]
                            .as_array()
                            .is_some_and(|sub| !sub.is_empty())
                })
            }));
        }
    }
}
