import { ArrowLeft, ArrowRight, BookOpenText, Cable, Download } from "lucide-react";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import type { SyncSummary } from "@/api/types";
import { PageHeader } from "@/components/common/PageHeader";
import { Button } from "@/components/ui/button";
import { TemporaryLocationWarning } from "@/features/connect/TemporaryLocationWarning";
import { REMINDERS_UI } from "@/features/reminders/availability";
import { RemindMeCard } from "@/features/reminders/RemindMeCard";
import { SyncProgressPanel } from "@/features/sources/SyncProgressPanel";
import { useSyncOutcome } from "@/features/sources/useSyncOutcome";
import { AI_SETUP_ENABLED } from "@/lib/features";
import { paths } from "@/lib/routes";
import { useStartSync, useSyncStore } from "@/stores/sync";
import { AiOfferCard } from "./AiOfferCard";

const COPY = {
  running: { title: "sync.title", description: "sync.description" },
  done: { title: "sync.doneTitle", description: "sync.doneDescription" },
  doneWithErrors: { title: "sync.problemsTitle", description: "sync.problemsDescription" },
  failed: { title: "sync.failedTitle", description: "sync.failedDescription" },
  stopped: { title: "sync.stoppedTitle", description: "sync.stoppedDescription" },
} as const;

/** Step 3: sync everything once, show progress, then point to "Connect your AI app". */
export function FirstSyncStep({ onBack }: { onBack: () => void }) {
  const { t } = useTranslation("onboarding");
  const { t: tc } = useTranslation();
  const startSync = useStartSync();
  const outcome = useSyncOutcome();
  const summary = useSyncStore((s) => s.lastSummary);
  // True once *our* run has ended, so an older run's result is never shown as this one's.
  const [finished, setFinished] = useState(false);
  // Another run in this window was active when we tried: start ours once it ends.
  const [waiting, setWaiting] = useState(false);
  const running = useSyncStore((s) => s.running);
  const started = useRef(false);

  const run = useCallback(() => {
    setFinished(false);
    void startSync().then((ran) => (ran ? setFinished(true) : setWaiting(true)));
  }, [startSync]);

  useEffect(() => {
    if (waiting && !running) {
      setWaiting(false);
      run();
    }
  }, [waiting, running, run]);

  // Start automatically, once per visit to this step (the ref also covers StrictMode).
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    run();
  }, [run]);

  const view = !finished || outcome === "running" || outcome === "idle" ? "running" : outcome;
  const copy = COPY[view];
  // Canvas files aren't downloaded by a sync (a download can count as viewing them), so after
  // a Canvas sync "everything is on this computer" would be wrong.
  const canvasSynced =
    (view === "done" || view === "doneWithErrors") &&
    !!summary?.results.some((r) => r.kind === "canvas" && r.ok);
  const description =
    view === "done" && canvasSynced ? t("sync.doneDescriptionCanvas") : t(copy.description);

  return (
    <div>
      <PageHeader title={t(copy.title)} description={description} />
      <div className="space-y-6">
        <SyncProgressPanel onRetry={run} showFixLink />

        {REMINDERS_UI ? <RemindMeCard /> : null}

        {/* Once the courses are in: what PageLamp could write about them. */}
        {AI_SETUP_ENABLED && (view === "done" || view === "doneWithErrors") ? (
          <AiOfferCard />
        ) : null}

        {(view === "done" || view === "doneWithErrors") && summary ? (
          <SummaryStats summary={summary} />
        ) : null}

        {/* Before "Connect your AI app": a setup copied from here would break later. */}
        {view === "running" ? null : <TemporaryLocationWarning />}

        {canvasSynced ? (
          <p className="flex items-start gap-2 text-sm text-muted-foreground">
            <Download className="mt-0.5 size-4 shrink-0" aria-hidden />
            {t("sync.canvasFilesNote")}
          </p>
        ) : null}

        <div className="flex flex-wrap items-center justify-between gap-3">
          <Button type="button" variant="ghost" onClick={onBack} disabled={view === "running"}>
            <ArrowLeft aria-hidden />
            {tc("actions.back")}
          </Button>
          {view === "running" && waiting ? null : view === "running" ? (
            // A long first Canvas sync shouldn't trap the student here (Back is disabled):
            // the sync lives in the store and keeps going; the sidebar shows its progress.
            // Not while `waiting` for another run to end: this screen starts ours then, so
            // leaving would silently drop it.
            <div className="flex flex-wrap items-center justify-end gap-x-3 gap-y-1">
              <span className="text-sm text-muted-foreground">{t("sync.backgroundHint")}</span>
              <Button asChild variant="outline">
                <Link to={paths.courses}>
                  {t("sync.continueInBackground")}
                  <ArrowRight aria-hidden />
                </Link>
              </Button>
            </div>
          ) : (
            <div className="flex flex-wrap gap-3">
              <Button asChild size="lg" variant="outline">
                <Link to={paths.courses}>
                  <BookOpenText aria-hidden />
                  {t("sync.goToCourses")}
                </Link>
              </Button>
              <Button asChild size="lg">
                <Link to={paths.connect}>
                  <Cable aria-hidden />
                  {t("sync.connect")}
                </Link>
              </Button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/** Totals of what the run found, plus a nudge when no course turned up. */
function SummaryStats({ summary }: { summary: SyncSummary }) {
  const { t, i18n } = useTranslation("onboarding");
  const headingId = useId();
  const number = new Intl.NumberFormat(i18n.language);
  const sum = (pick: (r: SyncSummary["results"][number]) => number) =>
    summary.results.reduce((total, r) => total + pick(r), 0);
  const courses = sum((r) => r.courses);
  const stats = [
    { key: "courses", label: t("sync.courses"), value: courses },
    { key: "materials", label: t("sync.materials"), value: sum((r) => r.materials) },
    { key: "events", label: t("sync.events"), value: sum((r) => r.events) },
  ];

  return (
    <section aria-labelledby={headingId} className="space-y-3">
      <h2 id={headingId} className="text-sm font-medium">
        {t("sync.summaryLabel")}
      </h2>
      <dl className="grid grid-cols-3 gap-3">
        {stats.map((stat) => (
          <div key={stat.key} className="rounded-lg border bg-card p-3">
            <dt className="text-xs text-muted-foreground">{stat.label}</dt>
            <dd className="font-heading text-2xl font-semibold tabular-nums">
              {number.format(stat.value)}
            </dd>
          </div>
        ))}
      </dl>
      {courses === 0 ? (
        <p className="text-sm text-muted-foreground">{t("sync.noCourses")}</p>
      ) : null}
    </section>
  );
}
