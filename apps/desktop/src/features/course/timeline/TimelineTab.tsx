import { useRef } from "react";
import type { Course, CourseLifecycle, CourseTimeline } from "@/api/types";
import { Separator } from "@/components/ui/separator";
import { CALENDAR_UI } from "../proposals/availability";
import { ProposalsSection } from "../proposals/ProposalsSection";
import { SyllabusSection } from "../proposals/SyllabusSection";
import { DATES_V2_UI } from "./availability";
import { CheckDatesPrompt } from "./CheckDatesPrompt";
import { CourseDatesForm } from "./CourseDatesForm";
import { TermDatesForm } from "./TermDatesForm";
import { WhereCourseIsCard } from "./WhereCourseIsCard";

/**
 * Where the course is now and why we think so, its status, and the course dates form
 * (calendar design §7.13). Dates kept from version 0.1 get a one-time "Check this course's
 * dates" prompt first.
 */
export function TimelineTab({
  course,
  timeline,
  lifecycle,
}: {
  course: Course;
  timeline: CourseTimeline;
  lifecycle: CourseLifecycle;
}) {
  const headingRef = useRef<HTMLHeadingElement>(null);
  const startRef = useRef<HTMLInputElement>(null);
  const legacy = timeline.term.anchor_origin === "legacy";

  return (
    <div className="max-w-2xl space-y-8">
      {legacy ? (
        <CheckDatesPrompt
          courseId={course.id}
          onEdit={() => startRef.current?.focus()}
          onDone={() => requestAnimationFrame(() => headingRef.current?.focus())}
        />
      ) : null}

      {CALENDAR_UI ? (
        <ProposalsSection
          course={course}
          timeline={timeline}
          onEmptied={() => headingRef.current?.focus()}
        />
      ) : null}

      <WhereCourseIsCard
        courseId={course.id}
        timeline={timeline}
        lifecycle={lifecycle}
        headingRef={headingRef}
      />

      {CALENDAR_UI ? (
        <>
          <Separator />
          <SyllabusSection course={course} />
        </>
      ) : null}

      <Separator />

      {/* Form v2 (breaks, exams, second part) needs set_course_dates (alpha.2). */}
      {DATES_V2_UI ? (
        <CourseDatesForm course={course} timeline={timeline} startRef={startRef} />
      ) : (
        <TermDatesForm course={course} timeline={timeline} startRef={startRef} />
      )}
    </div>
  );
}
