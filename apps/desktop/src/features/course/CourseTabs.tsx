import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { CourseOverview } from "@/api/types";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ExplainTab } from "@/features/explain/ExplainTab";
import { DeadlinesTab } from "./deadlines/DeadlinesTab";
import { PolicyTab } from "./policy/PolicyTab";
import { SettingsTab } from "./settings/SettingsTab";
import { TimelineTab } from "./timeline/TimelineTab";
import { COURSE_TABS, isCourseTab, useCourseTab } from "./useCourseParams";
import { WeekTab } from "./week/WeekTab";

// Radix makes each panel focusable (Tab from the tab list lands on it), so show where focus is.
const PANEL = "rounded-lg pt-4 outline-hidden focus-visible:ring-[3px] focus-visible:ring-ring";

/**
 * The course tabs (Explain, M3, only where the AI screens are built). The selected tab is kept in `?tab=` (see useCourseParams).
 * Tabs with a form stay mounted while hidden, so unsaved edits survive switching tabs.
 */
export function CourseTabs({ overview }: { overview: CourseOverview }) {
  const { t } = useTranslation("course");
  const [tab, setTab] = useCourseTab();
  const { course } = overview;

  // "Set term dates" (This week tab) disappears with its tab. Move focus to the Timeline
  // panel it opens, so keyboard and screen-reader users aren't dropped at the top of the page.
  // Likewise Explain's "Open the AI policy" (shown while the course blocks it): the Policy panel.
  const timelinePanel = useRef<HTMLDivElement>(null);
  const policyPanel = useRef<HTMLDivElement>(null);
  const focusPanel = useRef<"timeline" | "policy" | null>(null);
  useEffect(() => {
    if (tab !== focusPanel.current) return;
    focusPanel.current = null;
    (tab === "timeline" ? timelinePanel : policyPanel).current?.focus();
  }, [tab]);
  const [explainOpened, setExplainOpened] = useState(false);
  const keepExplain = explainOpened || tab === "explain";
  useEffect(() => {
    if (tab === "explain") setExplainOpened(true);
  }, [tab]);

  function openTermDates() {
    focusPanel.current = "timeline";
    setTab("timeline");
  }
  function openPolicy() {
    focusPanel.current = "policy";
    setTab("policy");
  }

  return (
    <Tabs value={tab} onValueChange={(value) => isCourseTab(value) && setTab(value)}>
      <TabsList aria-label={t("tabs.label")} className="max-w-full overflow-x-auto">
        {COURSE_TABS.map((value) => (
          <TabsTrigger key={value} value={value} className="px-3">
            {t(`tabs.${value}`)}
          </TabsTrigger>
        ))}
      </TabsList>
      <TabsContent value="week" className={PANEL}>
        <WeekTab overview={overview} onSetTermDates={openTermDates} />
      </TabsContent>
      {COURSE_TABS.includes("explain") ? (
        // Once opened, kept mounted while hidden: a run keeps its progress and Stop when the
        // student looks at another tab.
        <TabsContent
          value="explain"
          className={PANEL}
          forceMount={keepExplain || undefined}
          hidden={tab !== "explain"}
        >
          <ExplainTab overview={overview} onOpenPolicy={openPolicy} />
        </TabsContent>
      ) : null}
      <TabsContent
        ref={timelinePanel}
        value="timeline"
        className={PANEL}
        forceMount
        hidden={tab !== "timeline"}
      >
        <TimelineTab course={course} timeline={overview.timeline} lifecycle={overview.lifecycle} />
      </TabsContent>
      <TabsContent value="deadlines" className={PANEL}>
        <DeadlinesTab courseId={course.id} />
      </TabsContent>
      <TabsContent
        ref={policyPanel}
        value="policy"
        className={PANEL}
        forceMount
        hidden={tab !== "policy"}
      >
        <PolicyTab course={course} aiMaterials={overview.ai_materials} />
      </TabsContent>
      <TabsContent value="settings" className={PANEL}>
        <SettingsTab course={course} lifecycle={overview.lifecycle} />
      </TabsContent>
    </Tabs>
  );
}
