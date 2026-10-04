import { FolderOpen, Info, Layers } from "lucide-react";
import { type ReactNode, useId } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { useWeekMaterials } from "@/api/queries";
import type { CourseLifecycle, CourseOverview, WeekMaterials } from "@/api/types";
import { ErrorState } from "@/components/common/ErrorState";
import { Alert, AlertAction, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { outsideWeekViews } from "@/lib/phase";
import { paths } from "@/lib/routes";
import { cn } from "@/lib/utils";
import { useSelectedWeek } from "../useCourseParams";
import { AnnouncementList } from "./AnnouncementList";
import { DownloadFilesCallout } from "./DownloadFilesCallout";
import { MaterialList } from "./MaterialList";
import { WeekSwitcher } from "./WeekSwitcher";

/** "This week": a week switcher, that week's modules and materials, recent announcements. */
export function WeekTab({
  overview,
  onSetTermDates,
}: {
  overview: CourseOverview;
  /** Opens the Timeline tab, where the term dates are set. */
  onSetTermDates: () => void;
}) {
  const { t } = useTranslation("course");
  const [selectedWeek, setSelectedWeek] = useSelectedWeek();
  const query = useWeekMaterials(overview.course.id, selectedWeek);
  // The week the facade shows by default (during a break, the week before it).
  const currentWeek = overview.timeline.default_week ?? overview.timeline.current_week ?? null;

  // Selecting the default week clears ?week=, so the URL and cache stay canonical.
  const selectWeek = (week: number | null) => setSelectedWeek(week === currentWeek ? null : week);

  let body: ReactNode;
  if (query.isError) {
    body = (
      <ErrorState
        error={query.error}
        title={t("week.loadError")}
        onRetry={() => void query.refetch()}
      />
    );
  } else if (query.data) {
    body = (
      <WeekView
        data={query.data}
        lifecycle={overview.lifecycle}
        pending={overview.structure_pending}
        onSelectWeek={selectWeek}
        onSetTermDates={onSetTermDates}
        // Previous week's list stays visible (dimmed) while the next one loads.
        stale={query.isPlaceholderData}
      />
    );
  } else {
    body = <WeekSkeleton />;
  }

  return (
    <div className="space-y-10">
      {body}
      <AnnouncementList announcements={overview.recent_announcements} />
    </div>
  );
}

function WeekView({
  data,
  lifecycle,
  pending,
  onSelectWeek,
  onSetTermDates,
  stale,
}: {
  data: WeekMaterials;
  lifecycle: CourseLifecycle;
  /** No full sync has read this course's modules and materials yet: they are missing, not none. */
  pending: boolean;
  onSelectWeek: (week: number | null) => void;
  onSetTermDates: () => void;
  stale: boolean;
}) {
  const { t } = useTranslation("course");
  const week = data.week ?? null;
  // "No materials this week" is already what the empty state below says.
  const showNote = !!data.note && data.note_kind !== "no_materials_this_week";
  // A course that is over, inactive or not started has no current week: the note gives that
  // reason. Term dates can't bring a week back for a course that has ended. They can for an
  // inactive one (with dates it is current again) and for one that hasn't started (its own
  // dates count), so those two keep the "Set term dates" button.
  const noCurrentWeek =
    data.note_kind === "outside_term" && outsideWeekViews(data.timeline, lifecycle)
      ? lifecycle.state
      : null;
  return (
    <div className={cn("space-y-6 transition-opacity", stale && "opacity-60")} aria-busy={stale}>
      <WeekSwitcher
        week={week}
        currentWeek={data.timeline.default_week ?? data.timeline.current_week ?? null}
        availableWeeks={data.available_weeks}
        onSelect={onSelectWeek}
      />
      {showNote ? (
        <Alert role="status">
          <Info aria-hidden />
          <AlertDescription>
            {/* Localised by code; a note without a known code is the backend's English text. */}
            {noCurrentWeek === "ended" ||
            noCurrentWeek === "inactive" ||
            noCurrentWeek === "upcoming" ? (
              <>
                {t(`week.note.no_current_week.${noCurrentWeek}`)}
                {/* Only when there is a week to pick: a site with no week-numbered material has none. */}
                {data.available_weeks.length > 0 ? (
                  <> {t("week.note.no_current_week.pick_week")}</>
                ) : null}
              </>
            ) : data.note_kind ? (
              t(`week.note.${data.note_kind}`)
            ) : (
              <span lang="en">{data.note}</span>
            )}
          </AlertDescription>
          {noCurrentWeek !== "ended" && (week === null || data.note_kind === "outside_term") ? (
            <AlertAction>
              <Button size="xs" variant="outline" onClick={onSetTermDates}>
                {t("week.setTermDates")}
              </Button>
            </AlertAction>
          ) : null}
        </Alert>
      ) : null}
      {data.modules.length > 0 ? <ModuleList modules={data.modules} /> : null}
      <DownloadFilesCallout course={data.course} materials={data.materials} />
      {data.materials.length > 0 ? (
        <MaterialList materials={data.materials} aiMaterials={data.ai_materials} />
      ) : (
        <EmptyWeek week={week} pending={pending} />
      )}
    </div>
  );
}

function ModuleList({ modules }: { modules: WeekMaterials["modules"] }) {
  const { t } = useTranslation("course");
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="space-y-2">
      <h3 id={headingId} className="text-sm font-medium text-muted-foreground">
        {t("week.modules")}
      </h3>
      <ul className="flex flex-wrap gap-2">
        {modules.map((module) => (
          <li
            key={module.id}
            className="inline-flex items-center gap-1.5 rounded-md border bg-card px-2.5 py-1 text-sm"
          >
            <Layers className="size-4 text-muted-foreground" aria-hidden />
            {module.name}
          </li>
        ))}
      </ul>
    </section>
  );
}

function EmptyWeek({ week, pending }: { week: number | null; pending: boolean }) {
  const { t } = useTranslation("course");
  return (
    <Empty className="border">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <FolderOpen aria-hidden />
        </EmptyMedia>
        {/* A course a light automatic sync found: nothing was looked for yet, so "none found"
            would be wrong. */}
        <EmptyTitle>
          {pending
            ? t("week.pending.title")
            : week !== null
              ? t("week.empty.title", { week })
              : t("week.empty.titleRecent")}
        </EmptyTitle>
        <EmptyDescription>
          {pending ? t("week.pending.description") : t("week.empty.description")}
        </EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Button asChild variant="outline">
          <Link to={paths.sources}>{t("week.empty.action")}</Link>
        </Button>
      </EmptyContent>
    </Empty>
  );
}

const ROWS = ["a", "b", "c", "d"];

function WeekSkeleton() {
  const { t: tc } = useTranslation();
  return (
    <div className="space-y-4" aria-busy="true">
      <span className="sr-only" role="status">
        {tc("states.loading")}
      </span>
      <Skeleton className="h-8 w-56" />
      <Skeleton className="h-7 w-72" />
      <div className="space-y-px overflow-hidden rounded-lg border">
        {ROWS.map((row) => (
          <Skeleton key={row} className="h-16 w-full rounded-none" />
        ))}
      </div>
    </div>
  );
}
