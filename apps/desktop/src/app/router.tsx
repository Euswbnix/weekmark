import { createHashRouter, Navigate, type RouteObject } from "react-router";
import { AppShell } from "@/components/layout/AppShell";
import { ConnectPage } from "@/features/connect/ConnectPage";
import { CourseDetailPage } from "@/features/course/CourseDetailPage";
import { CoursesPage } from "@/features/courses/CoursesPage";
import { OnboardingPage } from "@/features/onboarding/OnboardingPage";
import { PlanPage } from "@/features/plan/PlanPage";
import { SettingsPage } from "@/features/settings/SettingsPage";
import { SourcesPage } from "@/features/sources/SourcesPage";
import { AI_SETUP_ENABLED } from "@/lib/features";
import { paths } from "@/lib/routes";
import { RouteError } from "./RouteError";
import { StartGate } from "./StartGate";

// Adding a screen: create src/features/<name>/<Name>Page.tsx, add a route here, a path in
// src/lib/routes.ts, a nav entry in components/layout/Sidebar.tsx (if it belongs in the
// sidebar) and a locales/<lang>/<name>.json namespace (+ a line in i18n/i18next.d.ts).
export const routes: RouteObject[] = [
  { index: true, element: <StartGate />, errorElement: <RouteError /> },
  { path: paths.welcome, element: <OnboardingPage />, errorElement: <RouteError /> },
  {
    element: <AppShell />,
    errorElement: <RouteError />,
    children: [
      { path: paths.courses, element: <CoursesPage /> },
      { path: "/courses/:courseId", element: <CourseDetailPage /> },
      { path: paths.sources, element: <SourcesPage /> },
      { path: paths.connect, element: <ConnectPage /> },
      { path: paths.settings, element: <SettingsPage /> },
      // M3: PageLamp writes study plans (mock only until the AI commands ship, like AI setup).
      ...(AI_SETUP_ENABLED ? [{ path: paths.plan, element: <PlanPage /> }] : []),
      { path: "*", element: <Navigate to={paths.courses} replace /> },
    ],
  },
];

/** Hash routing: works the same under Vite and Tauri's custom protocol. */
export function createAppRouter() {
  return createHashRouter(routes);
}
