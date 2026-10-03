import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useLocation } from "react-router";
import { PageHeader } from "@/components/common/PageHeader";
import { AiModelsSection } from "@/features/ai/AiModelsSection";
import { UsageSection } from "@/features/ai/UsageSection";
import { REMOVAL_UI } from "@/features/course/removal/availability";
import { RemovedCoursesSection } from "@/features/course/removal/RemovedCoursesSection";
import { OutputLanguageSection } from "@/features/explain/OutputLanguageSection";
import { REMINDERS_UI } from "@/features/reminders/availability";
import { RemindersSection } from "@/features/reminders/RemindersSection";
import { WeeklyNoteSettingsSection } from "@/features/weekly-note/WeeklyNoteSettingsSection";
import { AI_SETUP_ENABLED } from "@/lib/features";
import { AboutSection } from "./AboutSection";
import { AppearanceSection } from "./AppearanceSection";
import { DataSection } from "./DataSection";
import { HelpSection } from "./HelpSection";
import { PrivacySection } from "./PrivacySection";
import { UpdatesSection } from "./UpdatesSection";

/**
 * Settings: appearance, data, reminders (M3), privacy, AI models and usage (M1), updates, help &
 * feedback and about.
 */
export function SettingsPage() {
  const { t } = useTranslation("settings");
  useFocusLinkedSection();
  return (
    <div className="max-w-3xl">
      <PageHeader title={t("title")} />
      <div className="space-y-6">
        <AppearanceSection />
        <DataSection />
        {REMOVAL_UI ? <RemovedCoursesSection /> : null}
        {REMINDERS_UI ? <RemindersSection /> : null}
        <PrivacySection />
        {AI_SETUP_ENABLED ? (
          <>
            <AiModelsSection />
            <OutputLanguageSection />
            <WeeklyNoteSettingsSection />
            <UsageSection />
          </>
        ) : null}
        <UpdatesSection />
        <HelpSection />
        <AboutSection />
      </div>
    </div>
  );
}

/**
 * `/settings#<section>` (onboarding's "Set up a model"): focus that section's heading, which
 * scrolls it into view. After a frame, so the shell's scroll-to-top on navigation comes first.
 */
function useFocusLinkedSection() {
  const { hash } = useLocation();
  useEffect(() => {
    const id = decodeURIComponent(hash.slice(1));
    if (!id) return;
    const frame = requestAnimationFrame(() => {
      const heading = document.getElementById(id)?.querySelector<HTMLElement>("h2");
      if (!heading) return;
      heading.tabIndex = -1;
      heading.classList.add("outline-none");
      heading.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [hash]);
}
