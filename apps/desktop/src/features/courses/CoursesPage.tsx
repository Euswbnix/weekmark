import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useCourses } from "@/api/queries";
import { ErrorState } from "@/components/common/ErrorState";
import { PageHeader } from "@/components/common/PageHeader";
import { CALENDAR_UI } from "@/features/course/proposals/availability";
import { ReadSyllabiBatch } from "@/features/course/proposals/ReadSyllabiBatch";
import { REMOVAL_UI } from "@/features/course/removal/availability";
import { LifecycleBanner } from "@/features/course/removal/LifecycleBanner";
import { WeeklyNoteCard } from "@/features/weekly-note/WeeklyNoteCard";
import { CourseList } from "./CourseList";
import { CoursesEmpty } from "./CoursesEmpty";
import { CoursesPageSkeleton } from "./Skeletons";
import { SourceProblems } from "./SourceProblems";
import { StudyPlanCard } from "./StudyPlanCard";
import { SyncBanner } from "./SyncBanner";
import { SyncNowButton } from "./SyncNowButton";
import { ThisWeek } from "./ThisWeek";

/**
 * Home screen: this week's deadlines, the latest study plan and every course with where it is
 * this week. Everything is read-only here; course settings live on the course page.
 */
export function CoursesPage() {
  const { t } = useTranslation("courses");
  const courses = useCourses();
  const noCourses = courses.isSuccess && courses.data.length === 0;

  let body: ReactNode;
  if (courses.isPending) {
    body = <CoursesPageSkeleton />;
  } else if (courses.isError) {
    body = (
      <ErrorState
        error={courses.error}
        title={t("list.errorTitle")}
        onRetry={() => void courses.refetch()}
      />
    );
  } else if (noCourses) {
    body = <CoursesEmpty />;
  } else {
    body = (
      <div className="space-y-10">
        {REMOVAL_UI ? <LifecycleBanner /> : null}
        <ThisWeek />
        <WeeklyNoteCard />
        <StudyPlanCard />
        {CALENDAR_UI ? <ReadSyllabiBatch /> : null}
        <CourseList courses={courses.data} />
      </div>
    );
  }

  return (
    <>
      <PageHeader
        title={t("title")}
        description={t("description")}
        // The empty state has its own "Sync now"; don't show two.
        actions={noCourses ? null : <SyncNowButton />}
      />
      <SyncBanner />
      <SourceProblems />
      {body}
    </>
  );
}
