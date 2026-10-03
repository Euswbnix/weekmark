import { TriangleAlert } from "lucide-react";
import { type ReactNode, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  useForgetRemovedCourse,
  usePurgeRemovedCourses,
  useRemovedCourses,
  useRestoreCourse,
} from "@/api/removalQueries";
import type { RemovedCourse } from "@/api/types";
import { ErrorState } from "@/components/common/ErrorState";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { useReturnFocus } from "@/lib/focus";
import { formatDate } from "@/lib/format";
import { useApiErrorText } from "@/lib/useApiErrorText";
import { SettingsSection } from "../../settings/SettingsSection";

/**
 * Settings → "Removed courses" (calendar design §8.5): each removed course with Undo and
 * "Delete now" while its data waits the 7 days, then Restore (syncs it again) and Forget. When
 * moving its files to the Trash failed: "Try again", and "Delete files permanently" with its
 * own confirmation, the only way files are deleted for good.
 */
export function RemovedCoursesSection() {
  const { t } = useTranslation("removal");
  const removed = useRemovedCourses();
  // When a row goes away (Undo, Restore, Forget), focus continues from the list.
  const listRef = useRef<HTMLDivElement>(null);
  const focusList = () => requestAnimationFrame(() => listRef.current?.focus());

  return (
    <SettingsSection title={t("removed.title")} description={t("removed.description")}>
      <div ref={listRef} tabIndex={-1} className="outline-none">
        {removed.isPending ? (
          <Skeleton className="h-16" />
        ) : removed.isError ? (
          <ErrorState error={removed.error} onRetry={() => void removed.refetch()} />
        ) : removed.data.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("removed.empty")}</p>
        ) : (
          <ul aria-label={t("removed.listLabel")} className="divide-y border-y">
            {removed.data.map((course) => (
              <RemovedRow key={course.removed_id} course={course} onGone={focusList} />
            ))}
          </ul>
        )}
      </div>
    </SettingsSection>
  );
}

function RemovedRow({ course, onGone }: { course: RemovedCourse; onGone: () => void }) {
  const { t, i18n } = useTranslation("removal");
  const errorText = useApiErrorText();
  const restore = useRestoreCourse();
  const purge = usePurgeRemovedCourses();
  const forget = useForgetRemovedCourse();
  const name = course.code ?? course.name;
  const busy = restore.isPending || purge.isPending || forget.isPending;

  async function restoreIt() {
    if (busy) return;
    try {
      const outcome = await restore.mutateAsync({ removedId: course.removed_id });
      if (outcome.restored) {
        toast.success(t("removed.restored", { course: name }));
        onGone();
      } else {
        toast.error(t(`removed.restoreFailed.${outcome.failure ?? "other"}`, { course: name }));
      }
    } catch (error) {
      toast.error(errorText(error));
    }
  }

  /**
   * Stage 2 for this course. `permanent` is true only from "Delete files permanently", after
   * the Trash failed; "Delete now" and "Try again" never delete files for good.
   */
  async function purgeIt(permanent: boolean, done: string, stillPending: string) {
    try {
      const report = await purge.mutateAsync({
        removedIds: [course.removed_id],
        permanentIfNoTrash: permanent,
      });
      if (report.files_pending.includes(course.removed_id)) toast.warning(stillPending);
      else toast.success(done);
      if (report.backup_failed) toast.warning(t("toast.backupFailed"));
    } catch (error) {
      toast.error(errorText(error));
    }
  }

  async function run(action: () => Promise<unknown>, message: string, gone: boolean) {
    try {
      await action();
      toast.success(message);
      if (gone) onGone();
    } catch (error) {
      toast.error(errorText(error));
    }
  }

  // A restore that isn't running here and didn't finish (another window, the CLI, or an
  // interrupted one): the next sync settles it, or the student restores it again.
  const stuck = course.state === "restoring" && !restore.isPending;
  const status = restore.isPending
    ? t("removed.restoring")
    : stuck
      ? t("removed.restoreStuck")
      : course.state === "pending"
        ? t("removed.pending", {
            date: formatDate(course.removed_at, i18n.language),
            count: course.purge_in_days ?? 0,
          })
        : t("removed.purged", {
            date: formatDate(course.purged_at ?? course.removed_at, i18n.language),
          });

  return (
    <li className="flex flex-wrap items-start justify-between gap-3 py-3 text-sm">
      <div className="min-w-0 space-y-0.5">
        <p className="font-medium">
          {name}
          {course.code ? (
            <span className="font-normal text-muted-foreground"> {course.name}</span>
          ) : null}
        </p>
        <p className="flex items-center gap-1.5 text-muted-foreground" aria-live="polite">
          {restore.isPending ? <Spinner aria-hidden /> : null}
          {status}
        </p>
        {course.keep_files ? (
          <p className="text-muted-foreground">{t("removed.keptFiles")}</p>
        ) : null}
        {course.files_pending ? (
          <p className="flex items-center gap-1">
            <TriangleAlert className="size-3.5 shrink-0 text-warning" aria-hidden />
            {t("removed.filesPending")}
          </p>
        ) : null}
      </div>
      <div className="flex flex-wrap gap-2">
        {course.state === "pending" ? (
          <>
            <ActionButton onClick={() => void restoreIt()} busy={busy}>
              {t("removed.undo")}
            </ActionButton>
            <ConfirmButton
              label={t("removed.deleteNow")}
              title={t("removed.confirmDeleteNowTitle", { course: name })}
              body={t("removed.confirmDeleteNowBody")}
              busy={busy}
              onConfirm={() =>
                purgeIt(
                  false,
                  t("removed.deleted", { course: name }),
                  t("removed.deletedFilesPending", { course: name }),
                )
              }
            />
          </>
        ) : course.state === "purged" ? (
          <>
            {/* Only when the Trash failed, and only with its own confirmation (design §8.3). */}
            {course.files_pending ? (
              <>
                <ActionButton
                  onClick={() =>
                    void purgeIt(
                      false,
                      t("removed.movedToTrash", { course: name }),
                      t("removed.stillPending", { course: name }),
                    )
                  }
                  busy={busy}
                  label={t("removed.tryAgainLabel", { course: name })}
                >
                  {t("removed.tryAgain")}
                </ActionButton>
                <ConfirmButton
                  label={t("removed.deletePermanently")}
                  title={t("removed.confirmPermanentTitle", { course: name })}
                  body={t("removed.confirmPermanentBody")}
                  busy={busy}
                  onConfirm={() =>
                    purgeIt(
                      true,
                      t("removed.deletedPermanently", { course: name }),
                      t("removed.stillPending", { course: name }),
                    )
                  }
                />
              </>
            ) : null}
            <ActionButton onClick={() => void restoreIt()} busy={busy}>
              {t("removed.restore")}
            </ActionButton>
            {/* Forgetting waits until the files are in the Trash or deleted (Invalid before). */}
            {course.files_pending ? null : (
              <ConfirmButton
                label={t("removed.forget")}
                title={t("removed.confirmForgetTitle", { course: name })}
                body={t("removed.confirmForgetBody")}
                busy={busy}
                onConfirm={() =>
                  run(
                    () => forget.mutateAsync({ removedId: course.removed_id }),
                    t("removed.forgotten", { course: name }),
                    true,
                  )
                }
              />
            )}
          </>
        ) : stuck ? (
          <ActionButton onClick={() => void restoreIt()} busy={busy}>
            {t("removed.restoreAgain")}
          </ActionButton>
        ) : null}
      </div>
    </li>
  );
}

function ActionButton({
  onClick,
  busy,
  label,
  children,
}: {
  onClick: () => void;
  busy: boolean;
  /** An accessible name that starts with the visible text and says which course. */
  label?: string;
  children: ReactNode;
}) {
  return (
    <Button
      size="sm"
      variant="outline"
      aria-label={label}
      aria-disabled={busy || undefined}
      className="aria-disabled:opacity-50"
      onClick={() => {
        if (!busy) onClick();
      }}
    >
      {children}
    </Button>
  );
}

/** A button whose action asks first (Delete now, Forget, Delete permanently). */
function ConfirmButton({
  label,
  title,
  body,
  busy,
  onConfirm,
}: {
  label: string;
  title: string;
  body: string;
  busy: boolean;
  onConfirm: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const returnFocus = useReturnFocus();
  const [open, setOpen] = useState(false);
  return (
    <>
      <ActionButton onClick={() => setOpen(true)} busy={busy}>
        {label}
      </ActionButton>
      <AlertDialog open={open} onOpenChange={setOpen}>
        <AlertDialogContent {...returnFocus}>
          <AlertDialogHeader>
            <AlertDialogTitle>{title}</AlertDialogTitle>
            <AlertDialogDescription>{body}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("actions.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                setOpen(false);
                void onConfirm();
              }}
            >
              {label}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
