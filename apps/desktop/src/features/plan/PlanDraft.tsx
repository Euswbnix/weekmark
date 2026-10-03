import { TriangleAlert } from "lucide-react";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import type { EstimateRequest } from "@/api/ai";
import type { GeneratedStudyPlan } from "@/api/plan";
import { useCourses } from "@/api/queries";
import { AiGeneratedLabel } from "@/components/common/AiGeneratedLabel";
import { Button } from "@/components/ui/button";
import { GenerateButton } from "@/features/ai/GenerateButton";
import { courseLabelFor } from "@/features/courses/lib/courses";
import { groupPlanByDate } from "@/features/courses/lib/plan";
import { formatIsoDate, formatIsoDay } from "@/lib/format";
import { useFocusOnMount } from "@/lib/useFocusOnMount";

/**
 * The draft to review (design §5.1, §7): by day and course, with what PageLamp left out and why,
 * which courses were planned from structure only, and the AI-generated line. Accept saves it as
 * the study plan; Write again runs the same request, with its "≈ $x" like the first run; Discard
 * drops it.
 */
export function PlanDraft({
  draft,
  regenerate,
  onAccept,
  onRegenerate,
  onDiscard,
  accepting,
}: {
  draft: GeneratedStudyPlan;
  /** What Write again would send, for its estimate. */
  regenerate: EstimateRequest;
  onAccept: () => void;
  onRegenerate: (overrideBudget: boolean) => void;
  onDiscard: () => void;
  accepting: boolean;
}) {
  const { t, i18n } = useTranslation("plan");
  const courses = useCourses();
  const headingId = useId();
  // Cells name their day and column explicitly (`headers`): rowgroup scope isn't read everywhere.
  const tableId = useId();
  const col = (name: string) => `${tableId}-${name}`;
  const dayId = (date: string) => `${tableId}-day-${date}`;
  // The draft replaces the progress (and its Stop): the focus goes to its heading.
  const headingRef = useFocusOnMount<HTMLHeadingElement>();
  const { plan, unscheduled, warnings, meta } = draft;
  const days = groupPlanByDate(plan.items);
  const label = (id: string | null | undefined) => courseLabelFor(id, courses.data);
  const structureOnly = meta.context.courses
    .filter((c) => !c.text_included)
    .map((c) => label(c.course_id))
    .filter((name): name is string => name !== null);

  return (
    <section aria-labelledby={headingId} className="space-y-5">
      <div className="space-y-1">
        <h2
          id={headingId}
          ref={headingRef}
          tabIndex={-1}
          className="font-heading text-lg font-medium outline-none"
        >
          {t("draft.title")}
        </h2>
        <p className="text-sm text-muted-foreground">
          {t("draft.summary", {
            count: plan.items.length,
            start: formatIsoDate(plan.horizon_start, i18n.language),
            end: formatIsoDate(plan.horizon_end, i18n.language),
          })}
        </p>
        <AiGeneratedLabel meta={meta} />
      </div>

      {warnings.length > 0 || structureOnly.length > 0 ? (
        <ul className="space-y-1.5 text-sm">
          {warnings.map((warning) => (
            <li key={warning.code} className="flex items-start gap-2">
              <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden />
              {t(`warnings.${warning.code}`, { count: warning.count })}
            </li>
          ))}
          {structureOnly.length > 0 ? (
            <li className="text-muted-foreground">
              {t("structureOnly", {
                courses: new Intl.ListFormat(i18n.language).format(structureOnly),
              })}
            </li>
          ) : null}
        </ul>
      ) : null}

      {days.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("draft.noItems")}</p>
      ) : (
        <table className="w-full border-y text-sm">
          <caption className="sr-only">{t("draft.title")}</caption>
          <thead className="text-left text-xs text-muted-foreground">
            <tr className="border-b">
              <th id={col("day")} scope="col" className="py-2 pr-3 font-medium">
                {t("draft.day")}
              </th>
              <th id={col("course")} scope="col" className="py-2 pr-3 font-medium">
                {t("draft.course")}
              </th>
              <th id={col("task")} scope="col" className="py-2 pr-3 font-medium">
                {t("draft.task")}
              </th>
              <th id={col("time")} scope="col" className="py-2 text-right font-medium">
                {t("draft.time")}
              </th>
            </tr>
          </thead>
          {days.map((day) => (
            <tbody key={day.date} className="border-b last:border-b-0">
              {day.entries.map(({ item, position }, index) => (
                <tr key={position} className="align-top">
                  {index === 0 ? (
                    <th
                      id={dayId(day.date)}
                      headers={col("day")}
                      scope="rowgroup"
                      rowSpan={day.entries.length}
                      className="py-2 pr-3 text-left font-medium whitespace-nowrap"
                    >
                      {formatIsoDay(day.date, i18n.language)}
                    </th>
                  ) : null}
                  <td
                    headers={`${dayId(day.date)} ${col("course")}`}
                    className="py-2 pr-3 text-muted-foreground"
                  >
                    {label(item.course_id)}
                  </td>
                  <td headers={`${dayId(day.date)} ${col("task")}`} className="py-2 pr-3">
                    <span className="font-medium">{item.title}</span>
                    {item.description ? (
                      <span className="block text-xs text-muted-foreground">
                        {item.description}
                      </span>
                    ) : null}
                  </td>
                  <td
                    headers={`${dayId(day.date)} ${col("time")}`}
                    className="py-2 text-right text-muted-foreground whitespace-nowrap"
                  >
                    {item.minutes ? t("draft.minutes", { count: item.minutes }) : null}
                  </td>
                </tr>
              ))}
            </tbody>
          ))}
        </table>
      )}

      {unscheduled.length > 0 ? (
        <div className="space-y-2">
          <h3 className="text-sm font-medium">{t("unscheduled.title")}</h3>
          <ul className="space-y-1 text-sm">
            {unscheduled.map((task, index) => (
              // Tasks have no id; the list is fixed for this draft.
              // biome-ignore lint/suspicious/noArrayIndexKey: a draft's list never changes
              <li key={index}>
                {label(task.course_id) ? (
                  <span className="text-muted-foreground">{label(task.course_id)} · </span>
                ) : null}
                {task.title}
                <span className="text-muted-foreground">
                  {" "}
                  ({t(`unscheduled.reason.${task.reason}`)})
                </span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      <div className="space-y-3">
        <div className="flex flex-wrap gap-2">
          <Button type="button" onClick={onAccept} disabled={accepting}>
            {t("draft.accept")}
          </Button>
          <Button type="button" variant="ghost" onClick={onDiscard} disabled={accepting}>
            {t("draft.discard")}
          </Button>
        </div>
        <GenerateButton
          request={accepting ? null : regenerate}
          label={t("draft.regenerate")}
          variant="outline"
          onGenerate={({ overrideBudget }) => onRegenerate(overrideBudget)}
        />
      </div>
    </section>
  );
}
