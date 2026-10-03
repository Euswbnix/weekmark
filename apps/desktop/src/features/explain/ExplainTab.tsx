import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { type EstimateRequest, materialSharing } from "@/api/ai";
import type { WeeklyExplanation } from "@/api/explain";
import { useWeekMaterials } from "@/api/queries";
import type { CourseOverview } from "@/api/types";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { GenerateButton } from "@/features/ai/GenerateButton";
import { MaterialSharingReminder } from "@/features/ai/MaterialSharingNotices";
import { useAiErrorText } from "@/features/ai/useAiErrorText";
import { formatDateTime } from "@/lib/format";
import { useFocusOnMount } from "@/lib/useFocusOnMount";
import { DeleteExplanation } from "./DeleteExplanation";
import { ExplanationView } from "./ExplanationView";
import {
  type ExplainRunState,
  useDeleteExplanation,
  useExplanation,
  useSavedExplanations,
} from "./useExplanation";

/** The week picker's "Recent materials" (no week). */
const RECENT = "recent";

/**
 * Course → Explain (design §5.2, §7): a week, "≈ $x" and Explain, the run's stages with Stop
 * (no text arrives before the end), the explanation with its citations, and the last 5 of the
 * week. Disabled with the course's reason when its materials aren't readable. Every run starts
 * from "≈ $x" for exactly what it sends: Explain from the week's; Include from its own (the
 * explanation's include plus what the facade brings back); Regenerate of an explanation whose
 * run included materials from its own (the same include again).
 */
export function ExplainTab({
  overview,
  onOpenPolicy,
}: {
  overview: CourseOverview;
  onOpenPolicy: () => void;
}) {
  const { t, i18n } = useTranslation("explain");
  const { course, timeline } = overview;
  const current = timeline.current_week ?? null;
  const [picked, setPicked] = useState<number | null>(null);
  const weeks = useWeekMaterials(course.id, null);
  const defaultWeek = timeline.default_week ?? current;
  const week = picked ?? defaultWeek;
  const saved = useSavedExplanations(course.id, week);
  const run = useExplanation(course.id);
  const remove = useDeleteExplanation(course.id);
  const [shownId, setShownId] = useState<string | null>(null);
  const [reminderClosed, setReminderClosed] = useState<string | null>(null);
  const [deleted, setDeleted] = useState<ReadonlySet<string>>(new Set());
  /** Runs started: each one's estimates start again (going over the budget is per run). */
  const [runs, setRuns] = useState(0);
  const weekLabelId = useId();
  const resultRef = useRef<HTMLDivElement>(null);
  const titleRef = useRef<HTMLHeadingElement>(null);
  /** After the shown explanation changes or goes: its region, else the tab's heading. */
  const focusResult = () =>
    requestAnimationFrame(() => (resultRef.current ?? titleRef.current)?.focus());
  const { state, stop, sentInclude } = run;
  // A run's explanation replaces its progress (and Stop): the focus goes to the result.
  useEffect(() => {
    if (state.phase === "done") resultRef.current?.focus();
  }, [state.phase]);

  const blocked = course.hidden
    ? "course_hidden"
    : overview.ai_materials !== "readable"
      ? overview.ai_materials
      : null;
  // Hidden, AI access turned off, or sharing with AI services answered "not allowed" for a run
  // that isn't on this computer, while it goes on (from another tab): the student's choice ends
  // it (the blocked view below has no Stop).
  const running = state.phase === "running";
  const sharingRefused =
    materialSharing(course) === "not_allowed" && running && state.onDevice === false;
  useEffect(() => {
    if ((blocked || sharingRefused) && running) void stop();
  }, [blocked, sharingRefused, running, stop]);
  if (blocked) {
    return (
      <div className="space-y-3">
        <h3 className="text-sm font-medium">{t("title")}</h3>
        <p className="text-sm">{t(`blocked.${blocked}`)}</p>
        {blocked !== "course_hidden" ? (
          <Button type="button" variant="outline" size="sm" onClick={onOpenPolicy}>
            {t("openPolicy")}
          </Button>
        ) : null}
      </div>
    );
  }

  // Deleted ones leave at once, the run's own result included.
  const list = (saved.data ?? []).filter((e) => !deleted.has(e.meta.generation_id));
  const fresh =
    state.phase === "done" && !deleted.has(state.explanation.meta.generation_id)
      ? state.explanation
      : null;
  const shown: WeeklyExplanation | null =
    list.find((e) => e.meta.generation_id === shownId) ??
    (fresh && fresh.week === week ? fresh : null) ??
    list[0] ??
    null;
  const weekOptions = weeks.data?.available_weeks ?? (week ? [week] : []);
  // No week (weeks unknown, before or after the teaching weeks): the facade explains the
  // materials of the last 14 days.
  const request: EstimateRequest = { feature: "weekly_explanation", course: course.id, week };
  const start = (include: string[], overrideBudget = false) => {
    // One run at a time: nothing changes for a click while one goes on.
    if (running) return;
    setShownId(null);
    setRuns((n) => n + 1);
    void run.start(week, { overrideBudget, include, uiLanguage: i18n.language });
  };
  /**
   * A GenerateButton's key: it starts again with each run and with each request (its tick
   * answers one "≈ $x"; a history pick or another week is another request).
   */
  const keyOf = (req: EstimateRequest) => `${runs}|${JSON.stringify(req)}`;
  // What the shown explanation's run included (one written here), and what Include sends: that,
  // plus the left-out materials the facade brings back (sending only the new ones would drop the
  // first include's materials back to "left out").
  const again = shown ? sentInclude(shown.meta.generation_id) : [];
  const includable = shown
    ? shown.left_out.filter((m) => m.includable).map((m) => m.material_id)
    : [];
  const include =
    includable.length > 0 ? [...again, ...includable.filter((id) => !again.includes(id))] : [];
  const withInclude = (ids: string[]): EstimateRequest => ({
    feature: "weekly_explanation",
    course: course.id,
    week,
    include: ids,
  });

  return (
    <div className="space-y-6">
      <div className="space-y-3">
        <div className="space-y-1">
          <h3 ref={titleRef} tabIndex={-1} className="text-sm font-medium outline-none">
            {t("title")}
          </h3>
          <p className="text-sm text-muted-foreground">{t("hint")}</p>
        </div>
        {weekOptions.length > 0 ? (
          <div className="space-y-1">
            <p id={weekLabelId} className="text-xs font-medium">
              {t("week")}
            </p>
            <Select
              value={week === null ? RECENT : String(week)}
              onValueChange={(value) => {
                setPicked(value === RECENT ? null : Number(value));
                setShownId(null);
              }}
              disabled={state.phase === "running"}
            >
              <SelectTrigger aria-labelledby={weekLabelId} className="w-48">
                <SelectValue placeholder={t("recent")} />
              </SelectTrigger>
              <SelectContent>
                {/* No week now (before or after the teaching weeks): the way back to it. Once
                    the course has a week (dates set, a sync), the facade explains that week for
                    no week, so recent-materials explanations aren't offered any more (kept,
                    unreachable here, until the course has no week again). */}
                {defaultWeek === null ? (
                  <SelectItem value={RECENT}>{t("recent")}</SelectItem>
                ) : null}
                {weekOptions.map((w) => (
                  <SelectItem key={w} value={String(w)}>
                    {w === current ? t("thisWeek", { week: w }) : t("weekOption", { week: w })}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        ) : weeks.isPending || weeks.isError ? null : (
          <p className="text-sm text-muted-foreground">{t("weeksUnknown")}</p>
        )}
        {state.phase === "running" ? (
          <ExplainProgress state={state} onStop={() => void run.stop()} />
        ) : (
          <GenerateButton
            key={keyOf(request)}
            request={request}
            label={week === null ? t("generateRecent") : t("generate", { week })}
            onGenerate={({ overrideBudget }) => start([], overrideBudget)}
          />
        )}
        <ExplainOutcome state={state} />
      </div>

      {shown ? (
        <section
          ref={resultRef}
          tabIndex={-1}
          aria-label={
            shown.week === null || shown.week === undefined
              ? t("result.regionLabelRecent")
              : t("result.regionLabel", { week: shown.week })
          }
          className="space-y-4 border-t pt-4 outline-none"
        >
          {shown.stale ? (
            // Regenerate sends what the explanation's run included (none for one saved before),
            // from "≈ $x" for exactly that; not offered while a run goes on.
            <div className="space-y-2 text-sm">
              <p>{t("result.stale")}</p>
              {running ? null : (
                <GenerateButton
                  key={keyOf(again.length > 0 ? withInclude(again) : request)}
                  request={again.length > 0 ? withInclude(again) : request}
                  label={t("result.regenerate")}
                  variant="outline"
                  onGenerate={({ overrideBudget }) => start(again, overrideBudget)}
                />
              )}
            </div>
          ) : null}
          <ExplanationView
            explanation={shown}
            include={
              include.length > 0 && !running
                ? {
                    request: withInclude(include),
                    resetKey: keyOf(withInclude(include)),
                    onInclude: ({ overrideBudget }) => start(include, overrideBudget),
                  }
                : undefined
            }
            actions={
              <DeleteExplanation
                deleting={remove.isPending}
                onDelete={() =>
                  remove.mutate(shown.meta.generation_id, {
                    onSuccess: () => {
                      const id = shown.meta.generation_id;
                      setDeleted((before) => new Set([...before, id]));
                      setShownId(null);
                      toast.success(t("result.deleted"));
                      // Delete… went with it: the next one, else the heading.
                      focusResult();
                    },
                    onError: () => toast.error(t("result.deleteFailed")),
                  })
                }
              />
            }
          />
          {shown.sharing_reminder && reminderClosed !== shown.meta.generation_id ? (
            <MaterialSharingReminder
              courseId={course.id}
              courseName={course.code ?? course.name}
              service={shown.meta.backend_label}
              onClose={() => setReminderClosed(shown.meta.generation_id)}
            />
          ) : null}
        </section>
      ) : null}

      {list.length > 1 ? (
        <section className="space-y-2">
          <h4 className="text-sm font-medium">{t("history.title")}</h4>
          <ul className="divide-y border-y text-sm">
            {list.map((item) => {
              const isShown = item.meta.generation_id === shown?.meta.generation_id;
              return (
                <li
                  key={item.meta.generation_id}
                  className="flex items-center justify-between gap-3 py-1.5"
                >
                  <span>
                    {formatDateTime(item.meta.created_at, i18n.language)}
                    <span className="text-muted-foreground"> · {item.meta.model}</span>
                  </span>
                  {isShown ? (
                    <span className="text-xs text-muted-foreground">{t("history.showing")}</span>
                  ) : (
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      aria-label={`${t("history.show")} ${formatDateTime(item.meta.created_at, i18n.language)}`}
                      onClick={() => {
                        setShownId(item.meta.generation_id);
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
  );
}

/** "Writing with <backend> · <model>", how many materials are read, the stage, and Stop. */
function ExplainProgress({
  state,
  onStop,
}: {
  state: Extract<ExplainRunState, { phase: "running" }>;
  onStop: () => void;
}) {
  const { t } = useTranslation("explain");
  // The button that started the run is gone: Stop takes the focus.
  const stopRef = useFocusOnMount<HTMLButtonElement>();
  const leaveId = useId();
  return (
    <div className="space-y-2 text-sm">
      <p className="font-medium">
        {state.backend && state.model
          ? t("running.withModel", { backend: state.backend, model: state.model })
          : t("running.starting")}
      </p>
      {state.materials !== null ? (
        <p className="text-muted-foreground">{t("running.reading", { count: state.materials })}</p>
      ) : null}
      {state.stage ? (
        <p className="text-muted-foreground" aria-hidden>
          {t(`running.stage.${state.stage}`)}
        </p>
      ) : null}
      <Button
        ref={stopRef}
        type="button"
        size="sm"
        variant="outline"
        onClick={onStop}
        aria-disabled={state.stopping || undefined}
        aria-describedby={leaveId}
        className="aria-disabled:opacity-50"
      >
        {state.stopping ? t("running.stopping") : t("running.stop")}
      </Button>
      <p id={leaveId} className="text-xs text-muted-foreground">
        {t("running.leaveHint")}
      </p>
    </div>
  );
}

/** The live region: stages and how the run ended (never text); a failure is an alert. */
function ExplainOutcome({ state }: { state: ExplainRunState }) {
  const { t } = useTranslation("explain");
  const errorText = useAiErrorText();
  const statusRef = useRef<HTMLParagraphElement>(null);
  const alertRef = useRef<HTMLDivElement>(null);
  // A stop or a failure brings Explain back: the focus goes to what happened.
  useEffect(() => {
    if (state.phase === "failed") alertRef.current?.focus();
    else if (state.phase === "stopped") statusRef.current?.focus();
  }, [state.phase]);
  const text =
    state.phase === "running"
      ? state.stage
        ? t(`running.stage.${state.stage}`)
        : t("running.writingNow")
      : state.phase === "done"
        ? t("result.done")
        : state.phase === "stopped"
          ? t("result.stopped")
          : "";
  return (
    <>
      <p
        ref={statusRef}
        tabIndex={-1}
        role="status"
        className={state.phase === "stopped" ? "text-sm outline-none" : "sr-only"}
      >
        {text}
      </p>
      {state.phase === "failed" ? (
        <div ref={alertRef} tabIndex={-1} role="alert" className="text-sm outline-none">
          <p className="font-medium">{t("result.failed")}</p>
          <p className="text-muted-foreground">{errorText(state.error)}</p>
        </div>
      ) : null}
    </>
  );
}
