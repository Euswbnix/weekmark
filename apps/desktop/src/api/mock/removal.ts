// Mock course lifecycle and removal (calendar design §8; F2): the lifecycle summary and its
// banner, removal suggestions and their snoozes, the removal preview, the two removal stages,
// the removed list, restore and forget. A simplified stand-in for pagelamp-app's course module;
// it keeps its own state beside the mock database.

import type { PageLampApi } from "../client";
import { ApiError } from "../errors";
import type {
  CourseLifecycle,
  LifecycleSummary,
  RemovalPreviewItem,
  RemovalReason,
  RemovedCourse,
  SourceKind,
} from "../types";
import { addDays, isoOf, keptCurrent } from "./calendar";
import type { MockCourse, MockDb, MockScenario } from "./fixtures";

/** A removed course and what it looked like, so Undo and restore can put it back. */
interface MockRemoved {
  record: RemovedCourse;
  course: MockCourse;
  /** "Also delete the pre-update backup": done with this removal's purge. */
  deleteBackup: boolean;
}

type LifecycleApi = Pick<
  PageLampApi,
  | "lifecycleSummary"
  | "snoozeLifecycleBanner"
  | "snoozeRemovalSuggestions"
  | "clearRemovalSnooze"
  | "removalPreview"
  | "removeCourses"
  | "removedCourses"
  | "restoreCourse"
  | "purgeRemovedCourses"
  | "forgetRemovedCourse"
>;

const KEEP_FOREVER = "9999-12-31";
const DAY_MS = 86_400_000;
/** Courses Canvas restricts access to (synthetic): a sync can't bring them back. */
const ACCESS_RESTRICTED = ["DEM101", "DEM150"];
const restricted = (code: string | null | undefined) =>
  ACCESS_RESTRICTED.some((prefix) => code?.startsWith(prefix));
const MB = 1024 * 1024;

export function createLifecycleMock(deps: {
  db: MockDb;
  scenario: MockScenario;
  now: () => Date;
  respond: <T>(value: T | (() => T), extraLatency?: number) => Promise<T>;
  findCourse: (courseId: string) => MockCourse;
}) {
  const { db, now, respond, findCourse } = deps;
  const today = () => isoOf(now());
  /** Course id → "Not now" / "Keep" date. */
  const snoozes = new Map<string, string>();
  let banner: { until: string; courses: string[] } | null = null;
  const removed: MockRemoved[] = [];

  const sourceKind = (sourceId: string): SourceKind =>
    db.sources.find((s) => s.id === sourceId)?.kind ?? "folder";

  /** The lifecycle as the facade reports it: "I'm still taking this" and snoozes applied. */
  function lifecycleOf(c: MockCourse): CourseLifecycle {
    const kept =
      c.keptCurrentUntil && c.keptCurrentUntil >= today()
        ? keptCurrent(c.lifecycle, c.keptCurrentUntil)
        : c.lifecycle;
    const snoozed = (snoozes.get(c.course.id) ?? "") >= today();
    return snoozed && kept.suggest_removal ? { ...kept, suggest_removal: false } : kept;
  }

  function suggested(): string[] {
    return db.courses.filter((c) => lifecycleOf(c).suggest_removal).map((c) => c.course.id);
  }

  function summary(): LifecycleSummary {
    const ids = suggested();
    const snoozeActive = banner !== null && banner.until >= today();
    const covered = new Set(snoozeActive ? banner?.courses : []);
    return {
      courses: db.courses.map((c) => ({
        course_id: c.course.id,
        code: c.course.code ?? null,
        name: c.course.name,
        hidden: c.course.hidden,
        lifecycle: lifecycleOf(c),
      })),
      suggested: ids,
      show_banner: ids.some((id) => !covered.has(id)),
      banner_snoozed_until: snoozeActive ? (banner?.until ?? null) : null,
    };
  }

  function previewItem(c: MockCourse): RemovalPreviewItem {
    const kind = sourceKind(c.course.source_id);
    const downloaded =
      kind === "canvas"
        ? c.materials.filter((m) => m.kind === "file" && m.text_status === "ok")
        : [];
    const lifecycle = lifecycleOf(c);
    return {
      course_id: c.course.id,
      code: c.course.code ?? null,
      name: c.course.name,
      source_kind: kind,
      lifecycle,
      materials: c.materials.length,
      downloaded_files: downloaded.length,
      downloaded_bytes: downloaded.length * Math.round(1.6 * MB),
      deadlines: c.deadlines.length,
      generated_items: 0,
      custom_settings:
        c.course.ai_policy !== "unknown" ||
        !c.course.ai_access ||
        c.course.term_source === "user" ||
        !!c.keptCurrentUntil,
      own_folder_untouched: kind === "folder",
      cannot_sync_again: restricted(c.course.code),
      lost_after_purge:
        kind === "canvas"
          ? ["old_announcements", "locked_files", "redownload_counts_as_viewing"]
          : [],
    };
  }

  function reasonFor(c: MockCourse): RemovalReason {
    const state = lifecycleOf(c).state;
    return state === "ended" ? "ended" : state === "inactive" ? "inactive" : "other";
  }

  function removeOne(
    c: MockCourse,
    reason: RemovalReason,
    purgeNow: boolean,
    keepFiles: boolean,
    deleteBackup = false,
  ) {
    const at = now().toISOString();
    const record: RemovedCourse = {
      // Like the facade's tombstone, keyed by the course id.
      removed_id: c.course.id,
      source_id: c.course.source_id,
      source_kind: sourceKind(c.course.source_id),
      external_id: c.course.external_id,
      course_id: c.course.id,
      code: c.course.code ?? null,
      name: c.course.name,
      reason,
      state: purgeNow ? "purged" : "pending",
      removed_at: at,
      // An instant, like the tombstone's (removed_at + 7 days).
      purge_after: purgeNow ? null : new Date(now().getTime() + 7 * DAY_MS).toISOString(),
      purged_at: purgeNow ? at : null,
      purge_in_days: purgeNow ? null : 7,
      keep_files: keepFiles,
      files_pending: false,
    };
    removed.unshift({ record, course: c, deleteBackup: deleteBackup && !purgeNow });
    db.courses = db.courses.filter((x) => x !== c);
    return record;
  }

  function findRemoved(removedId: string): MockRemoved {
    const found = removed.find((r) => r.record.removed_id === removedId);
    if (!found) throw new ApiError("not_found", `No removed course ${removedId}`);
    return found;
  }

  function busyCheck() {
    if (db.externalSyncRunning) {
      throw new ApiError("busy", "A sync is running. Try again when it finishes.");
    }
  }

  // Scenario "removed": courses removed a few days ago, in every state "Removed courses" shows:
  // pending, purged with its files waiting for the Trash, purged with its files kept, and a
  // restore that didn't finish.
  if (deps.scenario === "removed") {
    const [a, b, c] = db.courses.filter((x) => x.lifecycle.group === "past");
    if (a) {
      const r = removeOne(a, "ended", false, false);
      r.removed_at = new Date(now().getTime() - 4 * 86_400_000).toISOString();
      r.purge_after = new Date(now().getTime() + 3 * DAY_MS).toISOString();
      r.purge_in_days = 3;
    }
    if (b) {
      const r = removeOne(b, "ended", true, false);
      r.files_pending = true;
    }
    if (c) removeOne(c, "inactive", true, true);
    const [d] = db.courses.filter((x) => x.lifecycle.group === "past");
    if (d) {
      const r = removeOne(d, "ended", true, false);
      r.state = "restoring";
    }
  }

  const api: LifecycleApi = {
    lifecycleSummary: () => respond(summary),

    snoozeLifecycleBanner: () =>
      respond(() => {
        banner = { until: addDays(today(), 14), courses: suggested() };
      }),

    snoozeRemovalSuggestions: (courseIds, kind) =>
      respond(() => {
        for (const id of courseIds) {
          findCourse(id);
          snoozes.set(id, kind === "keep" ? KEEP_FOREVER : addDays(today(), 14));
        }
      }),

    clearRemovalSnooze: (courseIds) =>
      respond(() => {
        for (const id of courseIds) snoozes.delete(id);
      }),

    removalPreview: (courseIds) =>
      respond(() => ({
        items: courseIds.map((id) => previewItem(findCourse(id))),
        backup:
          deps.scenario === "phases"
            ? { age_days: 3, delete_by_default: false, reason_code: "backup_recent" }
            : { age_days: 21, delete_by_default: true, reason_code: "backup_old" },
      })),

    removeCourses: (courseIds, options) =>
      respond(() => {
        busyCheck();
        if (courseIds.length === 0) throw new ApiError("invalid", "No courses to remove.");
        const courses = courseIds.map(findCourse);
        const records = courses.map((c) =>
          removeOne(
            c,
            options.reason ?? reasonFor(c),
            options.purge_now,
            options.keep_downloaded_files,
            options.delete_pre_update_backup,
          ),
        );
        return {
          removed: records,
          purged_now: options.purge_now,
          // Like the facade: the backup goes with the purge, never at stage 1.
          backup_deleted: options.delete_pre_update_backup && options.purge_now,
          backup_failed: false,
        };
      }, 150),

    removedCourses: () => respond(() => removed.map((r) => r.record)),

    restoreCourse: (removedId) =>
      respond(() => {
        busyCheck();
        const entry = findRemoved(removedId);
        const { record } = entry;
        // A purged course, or a restore that didn't finish, comes back by syncing its source.
        const synced = record.state !== "pending";
        if (synced && restricted(record.code)) {
          // Canvas restricts access to this one: a sync can't bring it back.
          return { restored: false, course_id: null, failure: "access_restricted" as const };
        }
        const course = entry.course;
        if (synced) {
          // Synced again: Canvas files come back as "not downloaded".
          course.materials = course.materials.map((m) =>
            m.kind === "file" && record.source_kind === "canvas"
              ? { ...m, text_status: "not_downloaded" as const, chunk_count: 0 }
              : m,
          );
        }
        db.courses.push(course);
        removed.splice(removed.indexOf(entry), 1);
        return { restored: true, course_id: course.course.id, failure: null };
      }, 400),

    purgeRemovedCourses: (removedIds, permanentIfNoTrash) =>
      respond(() => {
        busyCheck();
        // Without ids: every due removal, and the purged ones whose files wait for the Trash. A
        // restore in progress is skipped either way.
        const due = (removedIds ? removedIds.map(findRemoved) : removed).filter(
          (r) => r.record.state !== "restoring",
        );
        const targets = removedIds
          ? due
          : due.filter(
              (r) =>
                (r.record.state === "pending" &&
                  Date.parse(r.record.purge_after ?? "") <= now().getTime()) ||
                (r.record.state === "purged" && r.record.files_pending),
            );
        const purged: string[] = [];
        const filesPending: string[] = [];
        let backupDeleted = false;
        for (const r of targets) {
          if (r.record.state === "pending") {
            r.record.state = "purged";
            r.record.purged_at = now().toISOString();
            r.record.purge_after = null;
            r.record.purge_in_days = null;
            purged.push(r.record.removed_id);
            if (r.deleteBackup) backupDeleted = true;
            r.deleteBackup = false;
          }
          // The mock's Trash keeps failing for a course whose move already failed: only "Delete
          // permanently" (permanentIfNoTrash) clears it.
          if (r.record.files_pending && permanentIfNoTrash) r.record.files_pending = false;
          if (r.record.files_pending) filesPending.push(r.record.removed_id);
        }
        return {
          purged,
          files_pending: filesPending,
          backup_deleted: backupDeleted,
          backup_failed: false,
        };
      }),

    forgetRemovedCourse: (removedId) =>
      respond(() => {
        busyCheck();
        const entry = findRemoved(removedId);
        if (entry.record.files_pending) {
          throw new ApiError(
            "invalid",
            "Its files are still waiting for the Trash: try again or delete them permanently first.",
          );
        }
        if (entry.record.state !== "purged") {
          throw new ApiError(
            "invalid",
            "Only a course whose data is already deleted can be forgotten.",
          );
        }
        removed.splice(removed.indexOf(entry), 1);
      }),
  };

  return {
    api,
    lifecycleOf,
    removedCount: () => removed.length,
    removedIds: () => removed.map((r) => r.course.course.id),
  };
}
