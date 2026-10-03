import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import type { EstimateRequest } from "@/api/ai";
import type { DayOfWeek, PlanLimits, StudyPlanRequest } from "@/api/plan";
import { useCourses, usePlanLimits } from "@/api/queries";
import { WEEKDAYS } from "@/api/reminders";
import type { CourseSummary } from "@/api/types";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { GenerateButton } from "@/features/ai/GenerateButton";
import { paths } from "@/lib/routes";
import { useApiErrorText } from "@/lib/useApiErrorText";
import { activeCourses } from "./activeCourses";

/**
 * What to plan (design §5.1): days from today, hours per week, days off, which courses (the
 * current ones to start with), and a note; then "≈ $x" and Write my plan. The limits are the
 * facade's (`plan_limits`); the form says what's wrong before anything is sent.
 */
export function PlanForm({
  initial,
  onGenerate,
}: {
  /** The last run's request: Write again after an error starts from it. */
  initial: StudyPlanRequest | null;
  onGenerate: (request: StudyPlanRequest) => void;
}) {
  const limits = usePlanLimits();
  const errorText = useApiErrorText();
  if (limits.isPending) return <Skeleton className="h-64" />;
  if (limits.isError) {
    return (
      <p role="alert" className="text-sm">
        {errorText(limits.error)}
      </p>
    );
  }
  return <PlanFields limits={limits.data} initial={initial} onGenerate={onGenerate} />;
}

function PlanFields({
  limits,
  initial,
  onGenerate,
}: {
  limits: PlanLimits;
  initial: StudyPlanRequest | null;
  onGenerate: (request: StudyPlanRequest) => void;
}) {
  const { t, i18n } = useTranslation("plan");
  const courses = useCourses();
  // Only active courses, the facade's own scope (none to plan for: blocked no_course_to_plan).
  const listed = activeCourses(courses.data ?? []);
  const [horizon, setHorizon] = useState(
    String(initial?.horizon_days ?? limits.default_horizon_days),
  );
  const [hours, setHours] = useState(
    String(initial?.hours_per_week ?? limits.default_hours_per_week),
  );
  const [daysOff, setDaysOff] = useState<DayOfWeek[]>(initial?.days_off ?? []);
  const [picked, setPicked] = useState<string[] | null>(initial?.courses ?? null);
  const [note, setNote] = useState(initial?.note ?? "");
  const chosen = picked ?? listed.map((c) => c.course.id);

  const horizonLimits = { min: limits.min_horizon_days, max: limits.max_horizon_days };
  const hoursLimits = { min: limits.min_hours_per_week, max: limits.max_hours_per_week };
  const horizonDays = wholeNumber(horizon, horizonLimits);
  const hoursPerWeek = wholeNumber(hours, hoursLimits);
  const problem =
    horizonDays === null
      ? t("form.invalidHorizon")
      : hoursPerWeek === null
        ? t("form.invalidHours")
        : daysOff.length >= 7
          ? t("form.noStudyDays")
          : chosen.length === 0
            ? t("form.noCourses")
            : null;
  const request: StudyPlanRequest | null =
    problem === null && horizonDays !== null && hoursPerWeek !== null
      ? {
          horizon_days: horizonDays,
          hours_per_week: hoursPerWeek,
          days_off: daysOff,
          courses: chosen,
          note: note.trim() === "" ? null : note.trim(),
        }
      : null;
  const estimate: EstimateRequest | null = request
    ? { feature: "study_plan", courses: chosen, horizon_days: horizonDays }
    : null;

  const ids = {
    horizon: useId(),
    horizonHint: useId(),
    hours: useId(),
    hoursHint: useId(),
    daysOff: useId(),
    daysOffHint: useId(),
    courses: useId(),
    note: useId(),
    noteHint: useId(),
    problem: useId(),
  };
  const dayName = (index: number) =>
    new Intl.DateTimeFormat(i18n.language, { weekday: "short", timeZone: "UTC" }).format(
      // 2024-01-01 was a Monday.
      Date.UTC(2024, 0, 1 + index),
    );

  if (courses.data && listed.length === 0) {
    return (
      <div className="space-y-2 text-sm">
        <p>{t("form.nothingToPlan")}</p>
        <Link to={paths.courses} className="underline underline-offset-2">
          {t("form.seeCourses")}
        </Link>
      </div>
    );
  }

  return (
    <form
      className="space-y-6"
      onSubmit={(event) => {
        event.preventDefault();
      }}
    >
      <div className="flex flex-wrap gap-6">
        <NumberField
          id={ids.horizon}
          hintId={ids.horizonHint}
          label={t("form.horizon")}
          hint={t("form.horizonHint")}
          value={horizon}
          onChange={setHorizon}
          limits={horizonLimits}
          invalid={horizonDays === null}
        />
        <NumberField
          id={ids.hours}
          hintId={ids.hoursHint}
          label={t("form.hours")}
          hint={t("form.hoursHint")}
          value={hours}
          onChange={setHours}
          limits={hoursLimits}
          invalid={hoursPerWeek === null}
        />
      </div>

      <div className="space-y-2">
        <p id={ids.daysOff} className="text-sm font-medium">
          {t("form.daysOff")}
        </p>
        <ToggleGroup
          type="multiple"
          variant="outline"
          spacing={0}
          value={daysOff}
          aria-labelledby={ids.daysOff}
          aria-describedby={ids.daysOffHint}
          onValueChange={(value) =>
            setDaysOff(WEEKDAYS.filter((day) => (value as string[]).includes(day)))
          }
        >
          {WEEKDAYS.map((day, index) => (
            <ToggleGroupItem key={day} value={day} className="px-3">
              {dayName(index)}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <p id={ids.daysOffHint} className="text-xs text-muted-foreground">
          {t("form.daysOffHint")}
        </p>
      </div>

      <fieldset className="space-y-2">
        <legend className="text-sm font-medium">{t("form.courses")}</legend>
        <p className="text-xs text-muted-foreground">{t("form.coursesHint")}</p>
        <ul className="divide-y border-y empty:hidden">
          {listed.map((summary) => (
            <CourseChoice
              key={summary.course.id}
              summary={summary}
              checked={chosen.includes(summary.course.id)}
              onChange={(on) =>
                setPicked(
                  on
                    ? [...chosen, summary.course.id]
                    : chosen.filter((id) => id !== summary.course.id),
                )
              }
            />
          ))}
        </ul>
      </fieldset>

      <div className="space-y-2">
        <Label htmlFor={ids.note}>{t("form.note")}</Label>
        <Textarea
          id={ids.note}
          value={note}
          maxLength={limits.student_note_max_chars}
          aria-describedby={ids.noteHint}
          onChange={(event) => setNote(event.target.value)}
          rows={2}
        />
        <p id={ids.noteHint} className="text-xs text-muted-foreground">
          {t("form.noteHint")}
        </p>
      </div>

      {/* What's still missing, as a hint: nothing was sent, so it isn't an error. */}
      {problem && listed.length > 0 ? (
        <p id={ids.problem} className="text-sm text-muted-foreground">
          {problem}
        </p>
      ) : null}
      <GenerateButton
        request={estimate}
        label={t("form.generate")}
        describedBy={problem && listed.length > 0 ? ids.problem : undefined}
        onGenerate={({ overrideBudget }) => {
          if (request) onGenerate({ ...request, override_budget: overrideBudget });
        }}
      />
    </form>
  );
}

function CourseChoice({
  summary,
  checked,
  onChange,
}: {
  summary: CourseSummary;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  const id = useId();
  return (
    <li className="flex items-center gap-3 py-2">
      <Checkbox id={id} checked={checked} onCheckedChange={(value) => onChange(value === true)} />
      <Label htmlFor={id} className="font-normal">
        {summary.course.code ? (
          <>
            <span className="font-medium">{summary.course.code}</span>
            <span className="text-muted-foreground"> · {summary.course.name}</span>
          </>
        ) : (
          summary.course.name
        )}
      </Label>
    </li>
  );
}

function NumberField({
  id,
  hintId,
  label,
  hint,
  value,
  onChange,
  limits,
  invalid,
}: {
  id: string;
  hintId: string;
  label: string;
  hint: string;
  value: string;
  onChange: (value: string) => void;
  limits: { min: number; max: number };
  invalid: boolean;
}) {
  return (
    <div className="space-y-1">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        type="number"
        inputMode="numeric"
        min={limits.min}
        max={limits.max}
        step={1}
        value={value}
        className="w-28"
        aria-invalid={invalid || undefined}
        aria-describedby={hintId}
        onChange={(event) => onChange(event.target.value)}
      />
      <p id={hintId} className="text-xs text-muted-foreground">
        {hint}
      </p>
    </div>
  );
}

/** A whole number within the limits, else null. */
function wholeNumber(text: string, limits: { min: number; max: number }): number | null {
  if (!/^\d+$/.test(text.trim())) return null;
  const n = Number(text);
  return n >= limits.min && n <= limits.max ? n : null;
}
