// The backend ⇄ frontend contract, as TypeScript.
//
// All shapes come from `generated.ts`, which `pnpm gen:types` produces from the Rust facade's
// JSON Schema (crates/pagelamp-app). Never hand-write a contract type here — change the Rust
// type and regenerate. This file only re-exports them and adds a few UI conveniences.
//
// Conventions (serde on the Rust side):
// - Option<T> fields are `field?: T | null`; serde always sends null, but handle both (`??`).
// - Instants are RFC 3339 strings; calendar dates are "YYYY-MM-DD" strings.

export type {
  AcceptedCalendar,
  Activity,
  ActivityItem,
  ActivityKind,
  AiLabel,
  AiMaterialsState,
  AiPolicy,
  AlternativeDate,
  AppError,
  AppErrorKind,
  AppStatus,
  BackupInfo,
  BreakInput,
  BreakKind,
  CalendarBatchEvent,
  CalendarBreak,
  CalendarCandidate,
  CalendarChange,
  CalendarConflict,
  CalendarOrigin,
  CalendarProposal,
  CalendarRunOutcome,
  CalendarStatus,
  CandidateLeftOut,
  CandidateReason,
  ChangeCode,
  Confidence,
  ConflictCode,
  Course,
  CourseCalendar,
  CourseCalendarView,
  CourseCounts,
  CourseDatesInput,
  CourseGroup,
  CourseLifecycle,
  CourseLifecycleEntry,
  CourseOverview,
  CoursePhase,
  CourseSummary,
  CourseTimeline,
  CrashReport,
  DateEvidence,
  DateKind,
  DateSpan,
  Deadline,
  DoctorReport,
  DownloadBlock,
  DropCount,
  DropReason,
  EventKind,
  EvidenceCode,
  EvidenceItem,
  EvidenceParam,
  EvidenceSignal,
  ExtractWorkerCheck,
  ExtractWorkerStatus,
  InstallKind,
  LifecycleState,
  LifecycleSummary,
  LostAfterPurge,
  MaterialKind,
  MaterialView,
  McpClient,
  McpClientConfig,
  McpLaunch,
  McpNoteCode,
  Module,
  ProposedDate,
  PurgeReport,
  ReadCalendarOptions,
  RejectedDates,
  RejectReason,
  RemovalPreview,
  RemovalPreviewItem,
  RemovalReason,
  RemovalReport,
  RemovedCourse,
  RemoveOptions,
  RestoreFailure,
  RestoreOutcome,
  SearchHit,
  SegmentInput,
  SnoozeKind,
  SourceErrorKind,
  SourceKind,
  SourceRecord,
  SourceSyncResult,
  StartupTasks,
  StoreCounts,
  StoredStudyPlan,
  StudyPlan,
  StudyPlanItem,
  SyllabusOffer,
  SyncEvent,
  SyncRequest,
  SyncStage,
  SyncSummary,
  TeachingSegment,
  TemporaryLocation,
  TermAnchorSource,
  TermResolution,
  TermSource,
  TextErrorKind,
  TextProblem,
  TextStatus,
  TombstoneState,
  UnreadableFiles,
  UpdateChannel,
  UpdateCheckOutcome,
  UpdateCheckRecord,
  UpdatePrefs,
  WeekMaterials,
  WeekNoteKind,
  WhatsNew,
  WhatsNewTopic,
} from "./generated";

import type { AiMaterialsState, AiPolicy, Course } from "./generated";

/** RFC 3339 instant, e.g. "2026-09-25T14:03:00Z". */
export type Timestamp = string;
/** Calendar date, "YYYY-MM-DD". */
export type IsoDate = string;

/** All AI policies in the order the policy editor lists them. */
export const AI_POLICIES: readonly AiPolicy[] = [
  "unknown",
  "prohibited",
  "learning_aid",
  "allowed_with_citation",
  "unrestricted",
];

/**
 * Effective AI access to a course's material text (§3 rule 8): "No AI" wins over the switch,
 * then the switch, else readable ("Not set" counts as readable). The backend computes this;
 * the mock uses this function so both agree.
 */
export function aiMaterialsState(
  course: Pick<Course, "ai_policy" | "ai_access">,
): AiMaterialsState {
  if (course.ai_policy === "prohibited") return "withheld_by_policy";
  if (!course.ai_access) return "turned_off";
  return "readable";
}
