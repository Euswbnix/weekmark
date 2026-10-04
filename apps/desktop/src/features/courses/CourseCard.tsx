import { CalendarClock, ChevronRight, EyeOff, TriangleAlert } from "lucide-react";
import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import type { CourseSummary, SourceErrorKind } from "@/api/types";
import { AiMaterialsStatus } from "@/components/common/AiMaterialsStatus";
import { PastCourseBadge } from "@/components/common/PastCourseBadge";
import { PolicyBadge } from "@/components/common/PolicyBadge";
import { SentenceWithTime, WHEN, WHEN_2 } from "@/components/common/SentenceWithTime";
import { WeekLabel } from "@/components/common/WeekLabel";
import { Badge } from "@/components/ui/badge";
import { KeepCurrentCardButton } from "@/features/course/lifecycle/KeepCurrentCardButton";
import { formatIsoDate } from "@/lib/format";
import { deadlinesReadSince } from "@/lib/freshness";
import { paths } from "@/lib/routes";
import { cn } from "@/lib/utils";
import { deadlineTime } from "./lib/thisWeek";
import { ShowInListButton } from "./ShowInListButton";

interface CourseCardProps {
  summary: CourseSummary;
  /** Why this course's source last failed to sync, if it did. */
  sourceError: SourceErrorKind | null;
  /** h4 when the row sits under a group heading (Current, Upcoming, Past). */
  headingLevel?: "h3" | "h4";
}

/**
 * One course as a line of a book's contents page (docs/design/macos-shell.md §6.4):
 * "DEMO205  Foundations of Sample Data ········· Week 4", then what's next, the AI policy, how
 * much of it the AI app can read and where the data comes from. The title link is stretched over
 * the whole row, so the row is clickable while the page keeps one link per course; the few
 * controls (show again, still taking this) sit above it.
 */
export function CourseCard({ summary, sourceError, headingLevel = "h3" }: CourseCardProps) {
  const Heading = headingLevel;
  const { t } = useTranslation("courses");
  const { t: tc } = useTranslation();
  const { t: tcal, i18n } = useTranslation("calendar");
  const { course, timeline, lifecycle, counts, next_deadline: next } = summary;
  const deadlinesAt = deadlinesReadSince(summary.last_synced_at, summary.deadlines_synced_at);
  const nextWhen = next ? deadlineTime(next) : null;
  const past = lifecycle.group === "past";
  // The "Set the first day of classes" hint is for the current group only. (Dates can still
  // give an inactive or upcoming course a week; its Week tab offers that.)
  const weekUnknown =
    lifecycle.group === "current" && timeline.phase === "unknown" && timeline.current_week == null;
  const linkRef = useRef<HTMLAnchorElement>(null);

  return (
    <article
      className={cn(
        "relative -mx-3 rounded-row px-3 py-3 text-sm transition-colors",
        // An outline, not a ring: it survives Windows contrast themes (forced colors).
        "hover:bg-muted has-[a:focus-visible]:outline-2 has-[a:focus-visible]:outline-offset-2 has-[a:focus-visible]:outline-ring",
        course.hidden && "text-muted-foreground",
      )}
    >
      <div className="flex items-baseline gap-3">
        <Heading className="min-w-0 shrink">
          <Link
            ref={linkRef}
            to={paths.course(course.id)}
            data-course-id={course.id}
            className="flex min-w-0 items-baseline gap-2 outline-hidden after:absolute after:inset-0 after:rounded-row"
          >
            <span className="shrink-0 text-subheadline font-semibold">
              {course.code ?? course.name}
            </span>{" "}
            {course.code ? (
              <span className="min-w-0 truncate text-foreground" title={course.name}>
                {course.name}
              </span>
            ) : null}
          </Link>
        </Heading>
        <span aria-hidden className="pl-leader min-w-6 flex-1" />
        <span className="shrink-0 text-right text-title-3 tabular-nums">
          {/* A past course's week doesn't matter; say what the lifecycle concluded instead. */}
          {past ? (
            <span className="text-muted-foreground">{tcal(`status.state.${lifecycle.state}`)}</span>
          ) : (
            <WeekLabel timeline={timeline} lifecycle={lifecycle} />
          )}
        </span>
        <ChevronRight className="size-4 shrink-0 self-center text-muted-foreground" aria-hidden />
      </div>

      <div className="mt-1.5 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
        {weekUnknown ? <span>{tcal("card.setDates")}</span> : null}
        {lifecycle.kept_current_until ? (
          <span>
            {tcal("card.keptUntil", {
              date: formatIsoDate(lifecycle.kept_current_until, i18n.language),
            })}
          </span>
        ) : null}
        <span className="inline-flex items-center gap-1.5">
          <CalendarClock className="size-3.5 shrink-0" aria-hidden />
          {next && nextWhen ? (
            <SentenceWithTime
              text={t("card.next", { title: next.title, when: WHEN })}
              iso={nextWhen}
            />
          ) : (
            t("card.noDeadlines")
          )}
        </span>
        <PolicyBadge plain policy={course.ai_policy} />
        {course.ai_policy === "unknown" ? <span>{t("card.setPolicy")}</span> : null}
        {/* Not read yet isn't "0 of 0 readable": the line below says what is missing. */}
        {summary.structure_pending ? null : (
          <AiMaterialsStatus
            state={summary.ai_materials}
            indexed={counts.indexed_materials}
            total={counts.materials}
            className="items-center gap-1.5 [&>svg]:mt-0 [&>svg]:size-3.5"
          />
        )}
      </div>

      <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <span>
          {/* "Synced" is the source's last full sync. A course found since by a lighter
              automatic sync has deadlines and announcements only; and deadlines read since
              then have their own time. */}
          {summary.structure_pending && summary.deadlines_synced_at ? (
            <SentenceWithTime
              text={t("card.pending", { source: summary.source_label, when: WHEN })}
              iso={summary.deadlines_synced_at}
            />
          ) : summary.last_synced_at && deadlinesAt ? (
            <SentenceWithTime
              text={t("card.syncedAndDeadlines", {
                source: summary.source_label,
                when: WHEN,
                deadlines: WHEN_2,
              })}
              iso={summary.last_synced_at}
              iso2={deadlinesAt}
            />
          ) : summary.last_synced_at ? (
            <SentenceWithTime
              text={t("card.synced", { source: summary.source_label, when: WHEN })}
              iso={summary.last_synced_at}
            />
          ) : (
            t("card.notSynced", { source: summary.source_label })
          )}
        </span>
        {sourceError ? (
          <span className="inline-flex items-center gap-1 text-foreground">
            <TriangleAlert className="size-3.5 text-warning" aria-hidden />
            {tc(`sourceError.${sourceError}`)}
          </span>
        ) : null}
        <PastCourseBadge lifecycle={lifecycle} />
        {/* Controls sit above the stretched link. */}
        {past ? (
          <span className="relative z-10">
            <KeepCurrentCardButton courseId={course.id} courseName={course.code ?? course.name} />
          </span>
        ) : null}
        {course.hidden ? (
          <>
            <Badge variant="outline">
              <EyeOff aria-hidden />
              {t("card.hidden")}
            </Badge>
            <span className="relative z-10">
              <ShowInListButton
                courseId={course.id}
                courseName={course.code ?? course.name}
                linkRef={linkRef}
              />
            </span>
          </>
        ) : null}
      </div>
    </article>
  );
}
