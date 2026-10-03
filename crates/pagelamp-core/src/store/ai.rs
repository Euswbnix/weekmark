//! Store methods of PageLamp's own model calls (schema 4): providers, the student's question-(b)
//! answers, the usage ledger and "remove all AI data". Keys are never stored here (keychain).

use rusqlite::{OptionalExtension, params};

use super::{Store, expect_changed, get_value};
use crate::Result;
use crate::ai::{AiDataRemoved, MaterialSharing, ProviderRow, UsageRecord};
use crate::model::Timestamp;

/// How long usage rows are kept (design §3.5: 13 months).
pub const USAGE_KEEP_DAYS: i64 = 13 * 31;

impl Store {
    // ----- providers ---------------------------------------------------------------------------

    /// Add a provider row (`Invalid` if the id exists).
    pub fn insert_model_provider(&self, provider: &ProviderRow) -> Result<()> {
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO model_providers
                 (id, preset, label, wire, base_url, created_at, last_probe_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                provider.id,
                provider.preset,
                provider.label,
                provider.wire,
                provider.base_url,
                provider.created_at.to_rfc3339(),
                provider.last_probe_json,
            ],
        )?;
        if inserted == 0 {
            return Err(crate::Error::Invalid(format!(
                "a provider '{}' already exists",
                provider.id
            )));
        }
        Ok(())
    }

    /// Every provider, oldest first.
    pub fn model_providers(&self) -> Result<Vec<ProviderRow>> {
        self.query_list(
            "SELECT id, preset, label, wire, base_url, created_at, last_probe_json
             FROM model_providers ORDER BY created_at, id",
            [],
            provider_from_row,
        )
    }

    pub fn model_provider(&self, id: &str) -> Result<Option<ProviderRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, preset, label, wire, base_url, created_at, last_probe_json
                 FROM model_providers WHERE id = ?1",
                [id],
                provider_from_row,
            )
            .optional()?)
    }

    /// Record the last "Test" of a provider (`NotFound` if there is none).
    pub fn set_model_provider_probe(&self, id: &str, probe_json: Option<&str>) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE model_providers SET last_probe_json = ?2 WHERE id = ?1",
            params![id, probe_json],
        )?;
        expect_changed(changed, "model provider", id)
    }

    /// Delete a provider row; `false` if there was none.
    pub fn delete_model_provider(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM model_providers WHERE id = ?1", [id])?
            > 0)
    }

    // ----- question (b) ------------------------------------------------------------------------

    /// The student's answer for a course (`NotFound` if there is no such course). Sync upserts
    /// never touch it.
    pub fn set_course_material_sharing(
        &self,
        course_id: &str,
        answer: MaterialSharing,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE courses SET material_sharing = ?2 WHERE id = ?1",
            params![course_id, answer.as_str()],
        )?;
        expect_changed(changed, "course", course_id)
    }

    // ----- usage ledger ------------------------------------------------------------------------

    pub fn record_ai_usage(&self, record: &UsageRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO ai_usage (at, backend, model, feature, input_uncached, cache_read,
                                   cache_write, output, reasoning, micro_usd, cost_basis,
                                   estimated, outcome)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                record.at.to_rfc3339(),
                record.backend,
                record.model,
                record.feature.as_str(),
                saturate(record.input_uncached),
                saturate(record.cache_read),
                saturate(record.cache_write),
                saturate(record.output),
                record.reasoning.map(saturate),
                record.micro_usd.map(saturate),
                record.cost_basis,
                record.estimated,
                record.outcome,
            ],
        )?;
        Ok(())
    }

    /// Usage rows with `from <= at < to`, oldest first.
    pub fn ai_usage_between(&self, from: Timestamp, to: Timestamp) -> Result<Vec<UsageRecord>> {
        self.query_list(
            "SELECT at, backend, model, feature, input_uncached, cache_read, cache_write, output,
                    reasoning, micro_usd, cost_basis, estimated, outcome
             FROM ai_usage WHERE at >= ?1 AND at < ?2 ORDER BY at, id",
            params![from.to_rfc3339(), to.to_rfc3339()],
            usage_from_row,
        )
    }

    /// Delete usage rows older than `before` (the 13-month retention); returns how many.
    pub fn prune_ai_usage(&self, before: Timestamp) -> Result<u32> {
        let removed = self
            .conn
            .execute("DELETE FROM ai_usage WHERE at < ?1", [before.to_rfc3339()])?;
        Ok(u32::try_from(removed).unwrap_or(u32::MAX))
    }

    // ----- remove all ---------------------------------------------------------------------------

    /// Delete providers, generations and the usage ledger, and the students' plan provenance of
    /// generated plans, in one transaction. (Keys, AI settings and the backup file are the
    /// facade's part.) Must not be called inside `in_transaction`.
    pub fn remove_all_ai_data(&self) -> Result<AiDataRemoved> {
        self.in_transaction(|store| {
            let count = |table: &str| -> Result<u32> {
                let n: i64 =
                    store
                        .conn
                        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
                Ok(u32::try_from(n).unwrap_or(u32::MAX))
            };
            let removed = AiDataRemoved {
                providers: count("model_providers")?,
                generations: count("generations")?,
                usage_rows: count("ai_usage")?,
            };
            store.conn.execute_batch(
                "UPDATE course_calendars SET generation_id = NULL WHERE generation_id IS NOT NULL;
                 UPDATE study_plans SET generation_id = NULL WHERE generation_id IS NOT NULL;
                 DELETE FROM generations;
                 DELETE FROM ai_usage;
                 DELETE FROM model_providers;",
            )?;
            Ok(removed)
        })
    }
}

fn saturate(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

fn unsigned(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

fn provider_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProviderRow> {
    Ok(ProviderRow {
        id: row.get("id")?,
        preset: row.get("preset")?,
        label: row.get("label")?,
        wire: row.get("wire")?,
        base_url: row.get("base_url")?,
        created_at: get_value(row, "created_at")?,
        last_probe_json: row.get("last_probe_json")?,
    })
}

fn usage_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UsageRecord> {
    Ok(UsageRecord {
        at: get_value(row, "at")?,
        backend: row.get("backend")?,
        model: row.get("model")?,
        feature: get_value(row, "feature")?,
        input_uncached: unsigned(row.get("input_uncached")?),
        cache_read: unsigned(row.get("cache_read")?),
        cache_write: unsigned(row.get("cache_write")?),
        output: unsigned(row.get("output")?),
        reasoning: row.get::<_, Option<i64>>("reasoning")?.map(unsigned),
        micro_usd: row.get::<_, Option<i64>>("micro_usd")?.map(unsigned),
        cost_basis: row.get("cost_basis")?,
        estimated: row.get("estimated")?,
        outcome: row.get("outcome")?,
    })
}
