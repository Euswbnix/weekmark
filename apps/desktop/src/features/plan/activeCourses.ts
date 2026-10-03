import type { CourseSummary } from "@/api/types";

/**
 * The courses a plan covers by default: visible and active (the facade's lifecycle `is_active`:
 * Current, Finishing, Unknown, or Upcoming within 14 days), the same set generate_study_plan
 * plans when given none.
 */
export function activeCourses(list: CourseSummary[]): CourseSummary[] {
  return list.filter(({ course, lifecycle }) => !course.hidden && lifecycle.is_active);
}
