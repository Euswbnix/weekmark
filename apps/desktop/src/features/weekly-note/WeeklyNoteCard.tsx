import { TriangleAlert } from "lucide-react";
import { type ReactNode, type RefObject, useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { toast } from "sonner";
import { useCourses } from "@/api/queries";
import type { WeeklyNote } from "@/api/weeklyNote";
import { AiGeneratedLabel, aiGeneratedLabelText } from "@/components/common/AiGeneratedLabel";
import { CopyButton } from "@/components/common/CopyButton";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { GenerateButton } from "@/features/ai/GenerateButton";
import { useAiErrorText } from "@/features/ai/useAiErrorText";
import { Section } from "@/features/courses/parts/Section";
import { AI_SETUP_ENABLED } from "@/lib/features";
import { formatDateTime, formatIsoDate } from "@/lib/format";
import { paths } from "@/lib/routes";
import { useFocusOnMount } from "@/lib/useFocusOnMount";
import {
  type NoteRunState,
  useDeleteWeeklyNote,
  useNoteRunStore,
  useWeeklyNoteRun,
  useWeeklyNotes,
} from "./useWeeklyNote";

/** The Monday of the week `date` is in, as YYYY-MM-DD (local time). */
function thisMonday(date: Date): string {
  const back = (date.getDay() + 6) % 7;
  const monday = new Date(date.getFullYear(), date.getMonth(), date.getDate() - back);
  const m = String(monday.getMonth() + 1).padStart(2, "0");
  const d = String(monday.getDate()).padStart(2, "0");
  return `${monday.getFullYear()}-${m}-${d}`;
}

/**
 * Courses → Weekly note (design §5.3): between the week's facts and the study plan, the AI's few
 * sentences on the week and what to focus on, from structure only. "≈ $x" before Write; the run's
 * stages and Stop (the app's one note run, a click's or Monday's); the note with its focus list,
 * the AI-generated line, Copy and Delete; the earlier notes. Only where the AI screens are built.
 */
export function WeeklyNoteCard() {
  if (!AI_SETUP_ENABLED) return null;
  return <NoteCard />;
}

function NoteCard() {
  const { t, i18n } = useTranslation("weeklyNote");
  const notes = useWeeklyNotes();
  const { run, start, stop } = useWeeklyNoteRun();
  const automaticProblem = useNoteRunStore((s) => s.automaticProblem);
  const remove = useDeleteWeeklyNote();
  const errorText = useAiErrorText();
  const [shownId, setShownId] = useState<string | null>(null);
  const [deleted, setDeleted] = useState<ReadonlySet<string>>(new Set());
  const resultRef = useRef<HTMLElement>(null);
  const controlsRef = useRef<HTMLDivElement>(null);
  const historyId = useId();

  // A click's note replaces its progress (and Stop): the focus goes to the note. Monday's note
  // arrives on its own and moves nothing.
  const clickDone = run.phase === "done" && !run.automatic;
  useEffect(() => {
    if (clickDone) resultRef.current?.focus();
  }, [clickDone]);
  /** After the shown note changes or goes: that note, else the controls. */
  const focusResult = () =>
    requestAnimationFrame(() => (resultRef.current ?? controlsRef.current)?.focus());

  const list = (notes.data ?? []).filter((n) => !deleted.has(n.meta.generation_id));
  const fresh = run.phase === "done" && !deleted.has(run.note.meta.generation_id) ? run.note : null;
  const shown: WeeklyNote | null =
    list.find((n) => n.meta.generation_id === shownId) ?? fresh ?? list[0] ?? null;
  const write = (overrideBudget: boolean) => {
    setShownId(null);
    void start({ automatic: false, overrideBudget, uiLanguage: i18n.language });
  };

  return (
    <Section
      title={t("title")}
      description={
        shown ? (
          <>
            {t("weekOf", { date: formatIsoDate(shown.week_of, i18n.language) })}
            {shown.automatic ? <> · {t("automatic")}</> : null}
          </>
        ) : null
      }
    >
      <div className="space-y-4">
        {shown ? (
          <NoteView
            note={shown}
            regionRef={resultRef}
            actions={
              <DeleteNote
                deleting={remove.isPending}
                onDelete={() =>
                  remove.mutate(shown.meta.generation_id, {
                    onSuccess: () => {
                      const id = shown.meta.generation_id;
                      setDeleted((before) => new Set([...before, id]));
                      setShownId(null);
                      toast.success(t("deleted"));
                      // Delete… went with it: the next note, else the controls.
                      focusResult();
                    },
                    onError: () => toast.error(t("deleteFailed")),
                  })
                }
              />
            }
          />
        ) : notes.isPending ? null : (
          <p className="text-sm text-muted-foreground">{t("empty")}</p>
        )}

        {automaticProblem ? (
          <p className="text-sm text-muted-foreground">
            {t("automaticProblem", { reason: errorText(automaticProblem) })}
          </p>
        ) : null}

        <div ref={controlsRef} tabIndex={-1} className="space-y-2 outline-none">
          {run.phase === "running" ? (
            <NoteProgress run={run} onStop={() => void stop()} />
          ) : (
            <GenerateButton
              request={{ feature: "weekly_note" }}
              label={shown ? t("writeAgain") : t("write")}
              onGenerate={({ overrideBudget }) => write(overrideBudget)}
            />
          )}
          <NoteOutcome run={run} />
        </div>

        {list.length > 1 ? (
          <section aria-labelledby={historyId} className="space-y-2">
            <h3 id={historyId} className="text-sm font-medium">
              {t("history.title")}
            </h3>
            <ul className="divide-y border-y text-sm">
              {list.map((note) => {
                const isShown = note.meta.generation_id === shown?.meta.generation_id;
                const when = formatDateTime(note.meta.created_at, i18n.language);
                return (
                  <li
                    key={note.meta.generation_id}
                    className="flex items-center justify-between gap-3 py-1.5"
                  >
                    <span>
                      {when}
                      <span className="text-muted-foreground"> · {note.meta.model}</span>
                    </span>
                    {isShown ? (
                      <span className="text-xs text-muted-foreground">{t("history.showing")}</span>
                    ) : (
                      <Button
                        type="button"
                        size="sm"
                        variant="ghost"
                        aria-label={`${t("history.show")} ${when}`}
                        onClick={() => {
                          setShownId(note.meta.generation_id);
                          // "Show" becomes "Showing": the focus goes to what it shows.
                          focusResult();
                        }}
                      >
                        {t("history.show")}
                      </Button>
                    )}
                  </li>
                );
              })}
            </ul>
          </section>
        ) : null}
      </div>
    </Section>
  );
}

/** One note: its sentences, what to focus on (with the course), what was left out, the label. */
function NoteView({
  note,
  regionRef,
  actions,
}: {
  note: WeeklyNote;
  regionRef: RefObject<HTMLElement | null>;
  actions: ReactNode;
}) {
  const { t, i18n } = useTranslation("weeklyNote");
  const { t: tai } = useTranslation("ai");
  const courses = useCourses();
  const focusId = useId();
  const codeOf = (id: string | null | undefined) => {
    const c = id ? courses.data?.find((s) => s.course.id === id) : undefined;
    return c ? (c.course.code ?? c.course.name) : null;
  };
  const date = formatIsoDate(note.week_of, i18n.language);
  const label = aiGeneratedLabelText(note.meta, tai, i18n.language);
  const copy = [
    note.text,
    note.focus.length > 0
      ? `${t("focusTitle")}\n${note.focus
          .map(
            (f, i) => `${i + 1}. ${codeOf(f.course_id) ? `${codeOf(f.course_id)}: ` : ""}${f.text}`,
          )
          .join("\n")}`
      : null,
    label,
  ]
    .filter(Boolean)
    .join("\n\n");

  return (
    <article
      ref={regionRef}
      tabIndex={-1}
      aria-label={t("regionLabel", { date })}
      className="space-y-3 outline-none"
    >
      <p className="pl-prose text-sm">{note.text}</p>
      {note.focus.length > 0 ? (
        <div className="space-y-1.5">
          <h3 id={focusId} className="text-sm font-medium">
            {t("focusTitle")}
          </h3>
          <ol aria-labelledby={focusId} className="list-decimal space-y-1 pl-5 text-sm">
            {note.focus.map((focus, i) => {
              const code = codeOf(focus.course_id);
              return (
                // A note never changes; its focus items have no ids.
                // biome-ignore lint/suspicious/noArrayIndexKey: fixed list
                <li key={i}>
                  {code && focus.course_id ? (
                    <>
                      <Link
                        to={paths.course(focus.course_id)}
                        className="font-medium underline-offset-4 hover:underline"
                      >
                        {code}
                      </Link>
                      {": "}
                    </>
                  ) : null}
                  {focus.text}
                </li>
              );
            })}
          </ol>
        </div>
      ) : null}
      {note.graded_work_left_out > 0 ? (
        <p className="flex items-start gap-2 text-sm text-muted-foreground">
          <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden />
          {t("gradedLeftOut", { count: note.graded_work_left_out })}
        </p>
      ) : null}
      {note.week_of !== thisMonday(new Date()) ? (
        <p className="text-sm text-muted-foreground">{t("earlierWeek")}</p>
      ) : null}
      <div className="flex flex-wrap items-start justify-between gap-2">
        <AiGeneratedLabel meta={note.meta} />
        <div className="flex items-center gap-1">
          <CopyButton text={copy} label={t("copy")} />
          {actions}
        </div>
      </div>
    </article>
  );
}

/** "Delete…" with a confirmation: the note goes, with no undo. */
function DeleteNote({ onDelete, deleting }: { onDelete: () => void; deleting: boolean }) {
  const { t } = useTranslation("weeklyNote");
  const { t: tc } = useTranslation();
  return (
    <AlertDialog>
      <AlertDialogTrigger asChild>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          // Not `disabled`: the dialog gives the focus back here while the deletion runs.
          aria-disabled={deleting || undefined}
          className="aria-disabled:opacity-50"
          onClick={(event) => {
            if (deleting) event.preventDefault();
          }}
        >
          {t("delete")}
        </Button>
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("deleteTitle")}</AlertDialogTitle>
          <AlertDialogDescription>{t("deleteBody")}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{tc("actions.cancel")}</AlertDialogCancel>
          <AlertDialogAction onClick={onDelete}>{t("deleteConfirm")}</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/** "Writing with <backend> · <model>" (or Monday's), the stage, and Stop. */
function NoteProgress({
  run,
  onStop,
}: {
  run: Extract<NoteRunState, { phase: "running" }>;
  onStop: () => void;
}) {
  const { t } = useTranslation("weeklyNote");
  const leaveId = useId();
  // A click's Write is gone: Stop takes the focus. Monday's run moves nothing.
  const stopRef = useFocusOnMount<HTMLButtonElement>(!run.automatic);
  return (
    <div className="space-y-2 text-sm">
      <p className="font-medium">
        {run.automatic
          ? t("running.automatic")
          : run.backend && run.model
            ? t("running.withModel", { backend: run.backend, model: run.model })
            : t("running.starting")}
      </p>
      {run.stage ? (
        <p className="text-muted-foreground" aria-hidden>
          {t(`running.stage.${run.stage}`)}
        </p>
      ) : null}
      <Button
        ref={stopRef}
        type="button"
        size="sm"
        variant="outline"
        onClick={onStop}
        aria-disabled={run.stopping || undefined}
        aria-describedby={leaveId}
        className="aria-disabled:opacity-50"
      >
        {run.stopping ? t("running.stopping") : t("running.stop")}
      </Button>
      <p id={leaveId} className="text-xs text-muted-foreground">
        {t("running.leaveHint")}
      </p>
    </div>
  );
}

/** The live region: stages and how a run ended (never text); a click's failure is an alert. */
function NoteOutcome({ run }: { run: NoteRunState }) {
  const { t } = useTranslation("weeklyNote");
  const errorText = useAiErrorText();
  const statusRef = useRef<HTMLParagraphElement>(null);
  const alertRef = useRef<HTMLDivElement>(null);
  const stoppedByClick = run.phase === "stopped" && !run.automatic;
  // A stop or a failure brings Write back: the focus goes to what happened.
  useEffect(() => {
    if (run.phase === "failed") alertRef.current?.focus();
    else if (stoppedByClick) statusRef.current?.focus();
  }, [run.phase, stoppedByClick]);
  const text =
    run.phase === "running"
      ? run.stage
        ? t(`running.stage.${run.stage}`)
        : t("running.writingNow")
      : run.phase === "done"
        ? t("result.done")
        : run.phase === "stopped"
          ? t("result.stopped")
          : "";
  return (
    <>
      <p
        ref={statusRef}
        tabIndex={-1}
        role="status"
        className={run.phase === "stopped" ? "text-sm outline-none" : "sr-only"}
      >
        {text}
      </p>
      {run.phase === "failed" ? (
        <div ref={alertRef} tabIndex={-1} role="alert" className="text-sm outline-none">
          <p className="font-medium">{t("result.failed")}</p>
          <p className="text-muted-foreground">{errorText(run.error)}</p>
        </div>
      ) : null}
    </>
  );
}
