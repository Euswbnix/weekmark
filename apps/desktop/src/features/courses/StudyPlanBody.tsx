import { ChevronDown, ChevronUp, Circle, CircleCheck } from "lucide-react";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import type { CourseSummary, StudyPlan, StudyPlanItem } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { formatIsoDay } from "@/lib/format";
import { useToday } from "@/lib/useToday";
import { cn } from "@/lib/utils";
import { courseLabelFor } from "./lib/courses";
import { addDays, groupPlanByDate, isInFocus, type PlanDay } from "./lib/plan";

interface StudyPlanBodyProps {
  plan: StudyPlan;
  courses: CourseSummary[] | undefined;
  /** M3: tick an item off (its position in the saved plan); without it the plan is read-only. */
  onToggle?: (position: number, done: boolean) => void;
}

/**
 * Notes plus the plan's items grouped by day. Collapsed, it shows today and the next 2 days;
 * "Show full plan" reveals every day (including past ones).
 */
export function StudyPlanBody({ plan, courses, onToggle }: StudyPlanBodyProps) {
  const { t } = useTranslation("courses");
  const [expanded, setExpanded] = useState(false);
  const listId = useId();
  const today = useToday();

  const days = groupPlanByDate(plan.items);
  const focus = days.filter((day) => isInFocus(day.date, today));
  const shown = expanded ? days : focus;
  const canExpand = days.length > focus.length;

  return (
    <div className="space-y-4">
      {plan.notes ? (
        <div className="pl-callout p-3 text-sm">
          <p className="text-xs font-medium text-muted-foreground">{t("plan.notesLabel")}</p>
          <p className="mt-1 whitespace-pre-line">{plan.notes}</p>
        </div>
      ) : null}

      {days.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("plan.noItems")}</p>
      ) : (
        <div id={listId}>
          {shown.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("plan.nothingSoon")}</p>
          ) : (
            <ol className="space-y-3">
              {shown.map((day) => (
                <PlanDayGroup
                  key={day.date}
                  day={day}
                  today={today}
                  courses={courses}
                  onToggle={onToggle}
                />
              ))}
            </ol>
          )}
        </div>
      )}

      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-xs text-muted-foreground">
          {onToggle ? t("plan.tickHint") : t("plan.readOnly")}
        </p>
        {canExpand ? (
          <Button
            variant="ghost"
            size="sm"
            aria-expanded={expanded}
            aria-controls={listId}
            onClick={() => setExpanded((v) => !v)}
          >
            {expanded ? <ChevronUp aria-hidden /> : <ChevronDown aria-hidden />}
            {expanded ? t("plan.showLess") : t("plan.showFull")}
          </Button>
        ) : null}
      </div>
    </div>
  );
}

function PlanDayGroup({
  day,
  today,
  courses,
  onToggle,
}: {
  day: PlanDay;
  today: string;
  courses: CourseSummary[] | undefined;
  onToggle?: (position: number, done: boolean) => void;
}) {
  const { t: tc, i18n } = useTranslation();
  const isToday = day.date === today;
  const date = formatIsoDay(day.date, i18n.language);
  const relative = isToday
    ? tc("time.today")
    : day.date === addDays(today, 1)
      ? tc("time.tomorrow")
      : null;

  return (
    <li
      className={cn(
        // Today gets a quiet row fill (an ink wash, radius.row), not an accent box.
        "rounded-row px-3 py-2",
        isToday && "bg-muted",
      )}
    >
      <h3 className="text-sm font-medium">
        {relative ?? date}
        {relative ? <span className="font-normal text-muted-foreground"> · {date}</span> : null}
      </h3>
      <ul className="mt-1">
        {day.entries.map(({ item, position }) => (
          <PlanItemRow
            key={position}
            item={item}
            courseLabel={courseLabelFor(item.course_id, courses)}
            onToggle={onToggle ? (done) => onToggle(position, done) : undefined}
          />
        ))}
      </ul>
    </li>
  );
}

/**
 * One task. Done is shown with a check, a "Done" label and quieter text; with `onToggle` the
 * check is a checkbox named after the task.
 */
function PlanItemRow({
  item,
  courseLabel,
  onToggle,
}: {
  item: StudyPlanItem;
  courseLabel: string | null;
  onToggle?: (done: boolean) => void;
}) {
  const { t } = useTranslation("courses");
  const done = item.done === true;
  const name = courseLabel ? `${courseLabel} · ${item.title}` : item.title;
  return (
    <li className={cn("flex items-start gap-2.5 py-1.5", done && "text-muted-foreground")}>
      {onToggle ? (
        <Checkbox
          checked={done}
          aria-label={name}
          className="mt-0.5"
          onCheckedChange={(value) => onToggle(value === true)}
        />
      ) : done ? (
        <CircleCheck className="mt-0.5 size-4 shrink-0 text-success" aria-hidden />
      ) : (
        <Circle className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden />
      )}
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-baseline gap-x-2">
          {courseLabel ? (
            <span className="text-xs font-medium text-muted-foreground">{courseLabel}</span>
          ) : null}
          <span className={cn("font-medium", done && "line-through decoration-1")}>
            {item.title}
          </span>
          {done ? (
            <span className="text-xs">{t("plan.done")}</span>
          ) : (
            <span className="sr-only">{t("plan.notDone")}</span>
          )}
        </div>
        {item.description ? (
          <p className="text-xs text-muted-foreground">{item.description}</p>
        ) : null}
      </div>
      {item.minutes ? (
        <span className="shrink-0 text-xs text-muted-foreground">
          {t("plan.minutes", { count: item.minutes })}
        </span>
      ) : null}
    </li>
  );
}
