// Study plans written by PageLamp (v0.3 M3; design §5.1): the facade's types (generated.ts) and the
// code lists the UI translates (a test checks every code has words in en and zh-CN).

import type { PlanWarningCode, UnscheduledReason } from "./generated";

export type {
  ContextCourse,
  ContextSummary,
  DayOfWeek,
  GeneratedStudyPlan,
  GenerationMeta,
  LeftOutMaterial,
  LeftOutReason,
  PlanLimits,
  PlanOrigin,
  PlanWarning,
  PlanWarningCode,
  StudyPlanRequest,
  UnscheduledReason,
  UnscheduledTask,
} from "./generated";

// Records, so a code the facade adds fails the type check until it is listed (and translated).
const UNSCHEDULED: Record<UnscheduledReason, true> = {
  outside_horizon: true,
  no_study_days: true,
  no_time_before_latest: true,
  too_many_items: true,
};
export const UNSCHEDULED_REASONS = Object.keys(UNSCHEDULED) as UnscheduledReason[];

const WARNINGS: Record<PlanWarningCode, true> = {
  graded_work_left_out: true,
  unknown_materials_dropped: true,
};
export const PLAN_WARNING_CODES = Object.keys(WARNINGS) as PlanWarningCode[];
