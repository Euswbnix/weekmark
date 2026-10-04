import { ExternalLink as ExternalLinkIcon, Eye, EyeOff, LoaderCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useSources } from "@/api/queries";
import type { Course, CourseOverview } from "@/api/types";
import { PageHeader } from "@/components/common/PageHeader";
import { PastCourseBadge } from "@/components/common/PastCourseBadge";
import { PolicyBadge } from "@/components/common/PolicyBadge";
import { SentenceWithTime, WHEN, WHEN_2 } from "@/components/common/SentenceWithTime";
import { useOpenExternal } from "@/components/common/useOpenExternal";
import { WeekLabel } from "@/components/common/WeekLabel";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { focusPageHeading } from "@/lib/focus";
import { deadlinesReadSince } from "@/lib/freshness";
import { isHttpUrl } from "@/lib/url";
import { useSyncStore } from "@/stores/sync";
import { BackToCourses } from "../BackToCourses";
import { DownloadCourseFilesButton } from "../DownloadCourseFilesButton";
import { useCourseHidden } from "../useCourseHidden";
import { SourceAlert } from "./SourceAlert";

/** Back link, code + name (the page's h1), freshness, policy/week badges, source problems. */
export function CourseHeader({ overview }: { overview: CourseOverview }) {
  const { t } = useTranslation("course");
  const { course, timeline } = overview;
  return (
    <div className="pb-6">
      <PageHeader
        leading={<BackToCourses />}
        eyebrow={course.code ?? undefined}
        title={course.name}
        description={<Freshness overview={overview} />}
        actions={
          <>
            <DownloadAction overview={overview} />
            {isHttpUrl(course.url) ? <OpenWebsiteButton url={course.url} /> : null}
          </>
        }
      />
      <ul
        aria-label={t("header.statusLabel")}
        className="-mt-3 flex flex-wrap items-center gap-x-3 gap-y-2 text-sm"
      >
        <li>
          <PolicyBadge policy={course.ai_policy} />
        </li>
        <li>
          <WeekLabel timeline={timeline} lifecycle={overview.lifecycle} />
        </li>
        {overview.lifecycle.group === "past" ? (
          <li>
            <PastCourseBadge lifecycle={overview.lifecycle} />
          </li>
        ) : null}
        {course.hidden ? (
          <li className="flex items-center gap-2">
            <HiddenNotice course={course} />
          </li>
        ) : null}
      </ul>
      <SourceAlert sourceId={course.source_id} />
    </div>
  );
}

/**
 * "Data from Course folder · synced 2 hours ago", plus "Syncing now…" while its source syncs.
 * "Synced" is the source's last full sync: deadlines and announcements a lighter automatic sync
 * has read since get their own time, and a course only such a sync has found says that its
 * materials haven't been read yet.
 */
function Freshness({ overview }: { overview: CourseOverview }) {
  const { t } = useTranslation("course");
  const sourceId = overview.course.source_id;
  const syncing = useSyncStore((s) => {
    const progress = s.bySource[sourceId];
    return s.running && !!progress && !progress.result;
  });
  const source = overview.source_label;
  const deadlinesAt = deadlinesReadSince(overview.last_synced_at, overview.deadlines_synced_at);
  return (
    <>
      {overview.structure_pending && overview.deadlines_synced_at ? (
        <SentenceWithTime
          text={t("header.freshnessPending", { source, when: WHEN })}
          iso={overview.deadlines_synced_at}
        />
      ) : overview.last_synced_at && deadlinesAt ? (
        <SentenceWithTime
          text={t("header.freshnessWithDeadlines", { source, when: WHEN, deadlines: WHEN_2 })}
          iso={overview.last_synced_at}
          iso2={deadlinesAt}
        />
      ) : overview.last_synced_at ? (
        <SentenceWithTime
          text={t("header.freshness", { source, when: WHEN })}
          iso={overview.last_synced_at}
        />
      ) : (
        t("header.freshnessNever", { source })
      )}
      <span aria-live="polite">
        {syncing ? (
          <span className="ml-2 inline-flex items-center gap-1 text-foreground">
            <LoaderCircle className="size-3.5 animate-spin" aria-hidden />
            {t("header.syncing")}
          </span>
        ) : null}
      </span>
    </>
  );
}

/**
 * Canvas files are listed but downloaded only when the student asks (a download can count as
 * viewing them). Offered here, for the whole course, whenever any file could be downloaded,
 * whichever week or tab is on screen.
 */
function DownloadAction({ overview }: { overview: CourseOverview }) {
  const sources = useSources();
  const isCanvas = sources.data?.find((s) => s.id === overview.course.source_id)?.kind === "canvas";
  if (!isCanvas || overview.downloadable_files === 0) return null;
  return <DownloadCourseFilesButton course={overview.course} size="default" />;
}

function OpenWebsiteButton({ url }: { url: string }) {
  const { t } = useTranslation("course");
  const openExternal = useOpenExternal();
  return (
    <Button asChild variant="outline">
      <a
        href={url}
        target="_blank"
        rel="noreferrer noopener"
        onClick={(event) => {
          // The desktop webview never navigates away; the link opens in the browser.
          event.preventDefault();
          openExternal(url);
        }}
      >
        <ExternalLinkIcon aria-hidden />
        {t("header.openWebsite")}
      </a>
    </Button>
  );
}

function HiddenNotice({ course }: { course: Course }) {
  const { t } = useTranslation("course");
  const { setHidden, isPending } = useCourseHidden(course);
  return (
    <>
      <Badge variant="secondary">
        <EyeOff aria-hidden />
        {t("header.hidden")}
      </Badge>
      <Button
        size="xs"
        variant="outline"
        aria-disabled={isPending || undefined}
        className="aria-disabled:opacity-50"
        onClick={async () => {
          if (isPending) return;
          // The button goes away once the course is visible again; continue from the heading.
          if (await setHidden(false)) focusPageHeading();
        }}
      >
        <Eye aria-hidden />
        {t("header.showInList")}
      </Button>
    </>
  );
}
