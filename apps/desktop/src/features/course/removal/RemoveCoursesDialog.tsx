import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { toApiError } from "@/api/errors";
import {
  useRemovalPreview,
  useRemoveCourses,
  useSnoozeRemovalSuggestions,
  useUndoRemoval,
} from "@/api/removalQueries";
import type {
  CourseLifecycleEntry,
  LostAfterPurge,
  RemovalPreview,
  RemovalReport,
} from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Spinner } from "@/components/ui/spinner";
import { useReturnFocus } from "@/lib/focus";
import { useApiErrorText } from "@/lib/useApiErrorText";
import { evidenceText } from "../timeline/evidence";
import { formatBytes } from "./format";

const LOST_ORDER: LostAfterPurge[] = [
  "old_announcements",
  "locked_files",
  "whole_course",
  "redownload_counts_as_viewing",
];

/**
 * Remove courses from PageLamp (calendar design §8.7). "review" is the checklist opened from the
 * banner or the Past group: suggested courses ticked, each with its evidence and a "Keep". "one"
 * is a single course (course → Settings). Both spell out what is deleted, what stays, the undo
 * window and what a later sync can't bring back, and offer the three options. Stage 1 happens
 * at once; the toast offers Undo (not after "Delete now").
 */
export function RemoveCoursesDialog({
  open,
  onOpenChange,
  mode,
  candidates,
  initiallySelected,
  onRemoved,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  mode: "review" | "one";
  candidates: CourseLifecycleEntry[];
  initiallySelected: string[];
  /** After a successful removal (e.g. leave the removed course's page). */
  onRemoved?: (report: RemovalReport) => void;
}) {
  const { t } = useTranslation("removal");
  const { t: tc } = useTranslation();
  const returnFocus = useReturnFocus();
  const errorText = useApiErrorText();
  const remove = useRemoveCourses();
  const snooze = useSnoozeRemovalSuggestions();
  const undo = useUndoRemoval();
  const ids = { options: useId(), list: useId() };

  const [kept, setKept] = useState<string[]>([]);
  const [selected, setSelected] = useState<string[]>(initiallySelected);
  const [keepFiles, setKeepFiles] = useState(false);
  const [purgeNow, setPurgeNow] = useState(false);
  const [deleteBackup, setDeleteBackup] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);

  // Each opening starts from the suggestions again.
  const [openedWith, setOpenedWith] = useState<string | null>(null);
  const key = open ? initiallySelected.join("|") : null;
  if (key !== openedWith) {
    setOpenedWith(key);
    if (open) {
      setSelected(initiallySelected);
      setKept([]);
      setKeepFiles(false);
      setPurgeNow(false);
      setDeleteBackup(null);
      setBusy(false);
      remove.reset();
    }
  }

  const rows = candidates.filter((c) => !kept.includes(c.course_id));
  const chosen = selected.filter((id) => rows.some((r) => r.course_id === id));
  const preview = useRemovalPreview(chosen, open);
  const backup = preview.data?.backup ?? null;
  // The backup box starts as the facade suggests, until the student changes it.
  const backupChecked = deleteBackup ?? backup?.delete_by_default ?? false;

  useEffect(() => {
    if (!open) setOpenedWith(null);
  }, [open]);

  function toggle(id: string, on: boolean) {
    setSelected((list) => (on ? [...list, id] : list.filter((x) => x !== id)));
  }

  async function keep(entry: CourseLifecycleEntry) {
    const name = entry.code ?? entry.name;
    try {
      await snooze.mutateAsync({ courseIds: [entry.course_id], kind: "keep" });
      setKept((list) => [...list, entry.course_id]);
      toast.success(t("dialog.kept", { course: name }));
    } catch (error) {
      toast.error(errorText(error));
    }
  }

  async function confirm() {
    if (remove.isPending || chosen.length === 0) return;
    setBusy(false);
    try {
      const report = await remove.mutateAsync({
        courseIds: chosen,
        options: {
          reason: null,
          keep_downloaded_files: keepFiles,
          purge_now: purgeNow,
          delete_pre_update_backup: !!backup && backupChecked,
        },
      });
      onOpenChange(false);
      announce(report);
      onRemoved?.(report);
    } catch (error) {
      if (toApiError(error).kind === "busy") setBusy(true);
      else toast.error(errorText(error));
    }
  }

  function announce(report: RemovalReport) {
    const count = report.removed.length;
    const course = report.removed[0]?.code ?? report.removed[0]?.name ?? "";
    // The courses are removed either way; a backup that wouldn't go stays in the data folder.
    if (report.backup_failed) toast.warning(t("toast.backupFailed"));
    if (report.purged_now) {
      // The data is gone either way; files the Trash refused wait in "Removed courses".
      if (report.removed.some((r) => r.files_pending))
        toast.warning(t("toast.deletedFilesPending"));
      else toast.success(t("toast.deleted", { count, course }));
      return;
    }
    const removedIds = report.removed.map((r) => r.removed_id);
    toast.success(t("toast.removed", { count, course }), {
      action: {
        label: t("toast.undo"),
        onClick: () => {
          undo(removedIds).then(
            () => toast.success(t("toast.undone")),
            (error) => toast.error(errorText(error)),
          );
        },
      },
    });
  }

  const single = mode === "one" ? rows[0] : undefined;
  const title =
    mode === "one" && single
      ? t("dialog.titleOne", { course: single.code ?? single.name })
      : t("dialog.titleReview");

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent {...returnFocus} className="max-h-[85vh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{t("dialog.description")}</DialogDescription>
        </DialogHeader>

        {mode === "review" ? (
          <fieldset className="space-y-2">
            <legend id={ids.list} className="mb-2 text-sm font-medium">
              {t("dialog.listLabel")}
            </legend>
            <ul className="divide-y border-y">
              {rows.map((entry) => (
                <CandidateRow
                  key={entry.course_id}
                  entry={entry}
                  checked={selected.includes(entry.course_id)}
                  onCheckedChange={(on) => toggle(entry.course_id, on)}
                  onKeep={() => void keep(entry)}
                  keeping={snooze.isPending}
                />
              ))}
            </ul>
          </fieldset>
        ) : single ? (
          <p className="text-sm text-muted-foreground">
            <EvidenceLine entry={single} />
          </p>
        ) : null}

        {chosen.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("dialog.noneSelected")}</p>
        ) : (
          <PreviewDetails preview={preview.data} loading={preview.isPending} />
        )}

        <fieldset id={ids.options} className="space-y-3">
          <Option
            checked={keepFiles}
            onCheckedChange={setKeepFiles}
            label={t("dialog.keepFiles")}
            hint={t("dialog.keepFilesHint")}
          />
          <Option
            checked={purgeNow}
            onCheckedChange={setPurgeNow}
            label={t("dialog.purgeNow")}
            hint={t("dialog.purgeNowHint")}
          />
          {backup ? (
            <Option
              checked={backupChecked}
              onCheckedChange={setDeleteBackup}
              label={t("dialog.backup")}
              hint={
                backup.delete_by_default
                  ? t("dialog.backupOld", { days: backup.age_days })
                  : t("dialog.backupRecent", { days: backup.age_days })
              }
              note={t("dialog.backupWithData")}
            />
          ) : null}
        </fieldset>

        {busy ? (
          <p role="alert" className="text-sm text-destructive">
            {t("dialog.busy")}
          </p>
        ) : null}

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {tc("actions.cancel")}
          </Button>
          <Button
            variant="destructive"
            aria-disabled={chosen.length === 0 || remove.isPending || undefined}
            aria-busy={remove.isPending || undefined}
            className="aria-disabled:opacity-50"
            onClick={() => void confirm()}
          >
            {remove.isPending ? <Spinner aria-hidden /> : null}
            {remove.isPending
              ? t("dialog.removing")
              : t("dialog.confirm", { count: chosen.length })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function CandidateRow({
  entry,
  checked,
  onCheckedChange,
  onKeep,
  keeping,
}: {
  entry: CourseLifecycleEntry;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  onKeep: () => void;
  keeping: boolean;
}) {
  const { t } = useTranslation("removal");
  const id = useId();
  const name = entry.code ?? entry.name;
  return (
    <li className="flex items-start gap-3 py-2.5">
      <Checkbox
        id={id}
        checked={checked}
        onCheckedChange={(value) => onCheckedChange(value === true)}
        aria-describedby={`${id}-evidence`}
        className="mt-0.5"
      />
      <div className="min-w-0 flex-1">
        <Label htmlFor={id} className="flex flex-wrap items-center gap-2 font-medium">
          {name}
          {entry.code ? (
            <span className="font-normal text-muted-foreground">{entry.name}</span>
          ) : null}
          {entry.hidden ? <Badge variant="outline">{t("dialog.hidden")}</Badge> : null}
        </Label>
        <p id={`${id}-evidence`} className="text-xs text-muted-foreground">
          <EvidenceLine entry={entry} />
        </p>
      </div>
      <Button
        size="xs"
        variant="ghost"
        aria-label={t("dialog.keepLabel", { course: name })}
        aria-disabled={keeping || undefined}
        onClick={() => {
          if (!keeping) onKeep();
        }}
      >
        {t("dialog.keep")}
      </Button>
    </li>
  );
}

/** The lifecycle's evidence as one line: "The session ended … · Nothing new since …". */
function EvidenceLine({ entry }: { entry: CourseLifecycleEntry }) {
  const { t, i18n } = useTranslation("calendar");
  const lines = entry.lifecycle.evidence_items.map((item) => evidenceText(item, t, i18n.language));
  return <>{[t(`status.state.${entry.lifecycle.state}`), ...lines].join(" · ")}</>;
}

function PreviewDetails({ preview, loading }: { preview?: RemovalPreview; loading: boolean }) {
  const { t, i18n } = useTranslation("removal");
  if (!preview) {
    return loading ? (
      <p className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
        <Spinner aria-hidden />
        {t("dialog.loading")}
      </p>
    ) : null;
  }
  const sum = (pick: (item: RemovalPreview["items"][number]) => number) =>
    preview.items.reduce((n, item) => n + pick(item), 0);
  const files = sum((i) => i.downloaded_files);
  const generated = sum((i) => i.generated_items);
  const custom = preview.items.filter((i) => i.custom_settings).length;
  const deleted = [
    t("dialog.materials", { count: sum((i) => i.materials) }),
    files > 0
      ? t("dialog.files", {
          count: files,
          size: formatBytes(
            sum((i) => i.downloaded_bytes),
            i18n.language,
          ),
        })
      : null,
    t("dialog.deadlines", { count: sum((i) => i.deadlines) }),
    generated > 0 ? t("dialog.generated", { count: generated }) : null,
    custom > 0 ? t("dialog.customSettings", { count: custom }) : null,
  ].filter((line): line is string => line !== null);
  const lost = LOST_ORDER.filter(
    (reason) =>
      reason !== "whole_course" && preview.items.some((i) => i.lost_after_purge.includes(reason)),
  );
  const cannotSync = preview.items.filter((i) => i.cannot_sync_again);
  const folders = preview.items.some((i) => i.own_folder_untouched);

  return (
    <div className="pl-callout space-y-3 px-3 py-2.5 text-sm">
      <Section title={t("dialog.deletedTitle")}>
        <ul className="list-disc space-y-0.5 pl-5">
          {deleted.map((line) => (
            <li key={line}>{line}</li>
          ))}
        </ul>
      </Section>
      <Section title={t("dialog.keptTitle")}>
        <ul className="list-disc space-y-0.5 pl-5">
          {folders ? <li>{t("dialog.keptFolders")}</li> : null}
          <li>{t("dialog.keptNames")}</li>
        </ul>
      </Section>
      <Section title={t("dialog.undoTitle")}>
        <p>{t("dialog.undoBody")}</p>
        <ul className="list-disc space-y-0.5 pl-5">
          {lost.map((reason) => (
            <li key={reason}>{t(`lost.${reason}`)}</li>
          ))}
          {cannotSync.map((item) => (
            <li key={item.course_id}>
              {t("dialog.cannotSync", { course: item.code ?? item.name })}
            </li>
          ))}
        </ul>
      </Section>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  const id = useId();
  return (
    <section aria-labelledby={id} className="space-y-1">
      <h3 id={id} className="font-medium">
        {title}
      </h3>
      <div className="space-y-1 text-muted-foreground">{children}</div>
    </section>
  );
}

function Option({
  checked,
  onCheckedChange,
  label,
  hint,
  note,
}: {
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  label: string;
  hint: string;
  /** A second line under the hint, read with it. */
  note?: string;
}) {
  const id = useId();
  return (
    <div className="flex items-start gap-3">
      <Checkbox
        id={id}
        checked={checked}
        onCheckedChange={(value) => onCheckedChange(value === true)}
        aria-describedby={note ? `${id}-hint ${id}-note` : `${id}-hint`}
        className="mt-0.5"
      />
      <div className="space-y-0.5">
        <Label htmlFor={id}>{label}</Label>
        <p id={`${id}-hint`} className="text-xs text-muted-foreground">
          {hint}
        </p>
        {note ? (
          <p id={`${id}-note`} className="text-xs text-muted-foreground">
            {note}
          </p>
        ) : null}
      </div>
    </div>
  );
}
