//! The usage page and the budget: token counts and estimated cost per month (never content).

use std::collections::BTreeMap;

use chrono::{Datelike, Local, NaiveDate, TimeZone, Utc};
use pagelamp_core::ai::UsageRecord;
use pagelamp_core::model::Timestamp;
use pagelamp_core::store::Store;

use super::{BUDGET_WARN_AT_PERCENT, BudgetStatus, CostBasis, UsageRow, UsageSummary, settings};
use crate::{App, AppError, AppErrorKind, Result};

/// The first day of `date`'s month.
fn month_start(date: NaiveDate) -> NaiveDate {
    date.with_day(1).expect("day 1 exists")
}

/// The local month starting on `first` as a UTC range.
fn month_range(first: NaiveDate) -> Result<(Timestamp, Timestamp)> {
    let next = first
        .checked_add_months(chrono::Months::new(1))
        .ok_or_else(|| AppError::new(AppErrorKind::Invalid, "month out of range"))?;
    let local = |day: NaiveDate| {
        Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).expect("midnight exists"))
            .earliest()
            .map(|t| t.with_timezone(&Utc))
            .ok_or_else(|| AppError::new(AppErrorKind::Invalid, "month out of range"))
    };
    Ok((local(first)?, local(next)?))
}

/// Estimated spend of the current local month (priced rows only: what the budget counts).
pub(crate) fn spent_this_month(store: &Store) -> Result<u64> {
    let (from, to) = month_range(month_start(Local::now().date_naive()))?;
    Ok(store
        .ai_usage_between(from, to)?
        .iter()
        .filter_map(|record| record.micro_usd)
        .sum())
}

pub(crate) fn budget_status(store: &Store) -> Result<BudgetStatus> {
    Ok(BudgetStatus {
        monthly_micro_usd: settings::monthly_budget(store)?,
        spent_micro_usd: spent_this_month(store)?,
        warn_at_percent: BUDGET_WARN_AT_PERCENT,
    })
}

fn cost_basis(text: &str) -> CostBasis {
    match text {
        "free_on_device" => CostBasis::FreeOnDevice,
        "unpriced" => CostBasis::Unpriced,
        "plan" => CostBasis::Plan,
        _ => CostBasis::Priced,
    }
}

impl App {
    pub(crate) fn usage(&self, month: Option<NaiveDate>) -> Result<UsageSummary> {
        let first = month_start(month.unwrap_or_else(|| Local::now().date_naive()));
        let (from, to) = month_range(first)?;
        let store = self.read_store()?;
        let labels: BTreeMap<String, String> = store
            .model_providers()?
            .into_iter()
            .map(|p| (format!("provider:{}", p.id), p.label))
            .collect();
        let mut grouped: BTreeMap<(String, String, String), Vec<UsageRecord>> = BTreeMap::new();
        for record in store.ai_usage_between(from, to)? {
            let key = (
                record.backend.clone(),
                record.model.clone(),
                record.feature.as_str().to_string(),
            );
            grouped.entry(key).or_default().push(record);
        }
        let mut rows = Vec::new();
        for ((backend, model, _), records) in grouped {
            let basis = cost_basis(&records[0].cost_basis);
            let priced: Vec<u64> = records.iter().filter_map(|r| r.micro_usd).collect();
            rows.push(UsageRow {
                backend_label: labels.get(&backend).cloned().unwrap_or(backend),
                model,
                feature: records[0].feature,
                runs: u32::try_from(records.len()).unwrap_or(u32::MAX),
                input_tokens: records
                    .iter()
                    .map(|r| r.input_uncached + r.cache_read + r.cache_write)
                    .sum(),
                output_tokens: records.iter().map(|r| r.output).sum(),
                reasoning_tokens: records.iter().filter_map(|r| r.reasoning).sum(),
                cost_basis: basis,
                micro_usd: match basis {
                    CostBasis::Priced | CostBasis::FreeOnDevice => Some(priced.iter().sum()),
                    CostBasis::Unpriced | CostBasis::Plan => None,
                },
                estimated: records.iter().any(|r| r.estimated),
            });
        }
        let total_micro_usd = rows
            .iter()
            .filter(|r| r.cost_basis == CostBasis::Priced)
            .filter_map(|r| r.micro_usd)
            .sum();
        Ok(UsageSummary {
            month: first,
            rows,
            total_micro_usd,
            budget: budget_status(&store)?,
            mode_a: self.mode_a_usage(&store)?,
        })
    }
}
