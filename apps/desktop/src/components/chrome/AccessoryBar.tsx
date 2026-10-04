import { CircleAlert, CircleCheck, CircleX, LoaderCircle, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link, useLocation } from "react-router";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { SyncProgressRow } from "@/features/sources/SyncProgressRow";
import { type SyncOutcome, useSyncOutcome } from "@/features/sources/useSyncOutcome";
import { paths } from "@/lib/routes";
import { useStopSync, useSyncCounts, useSyncStore } from "@/stores/sync";
import { ProgressRing } from "./ProgressRing";

/** How long "Sync finished" stays (§6.2). */
export const FINISHED_MS = 4000;

type Ending = Extract<SyncOutcome, "done" | "doneWithErrors" | "failed">;
type Capsule = { kind: "running" } | { kind: Ending };

const HEADLINE = {
  done: "sync.done",
  doneWithErrors: "sync.doneWithErrors",
  failed: "sync.failed",
} as const satisfies Record<Ending, string>;

/**
 * The window's one floating accessory bar (docs/design/macos-shell.md §6.2, §8), for now only
 * this window's sync runs: "Syncing 2 of 3 · Canvas" with a ring while one runs, "Sync
 * finished" for 4 s, or a problem until it is dismissed (× or Esc) or the next run starts. A
 * click opens per-source progress. Screen readers hear the start, the end and a problem, not
 * every step.
 *
 * A sync PageLamp started by itself looks the same while it runs and when it finishes. When it
 * goes wrong it just leaves (the store drops the run), unless the student is in the capsule or
 * its details; and it never takes focus from the page.
 */
export function AccessoryBar() {
  const { t } = useTranslation("chrome");
  const { t: tc } = useTranslation();
  const outcome = useSyncOutcome();
  const running = outcome === "running";
  const { done, total } = useSyncCounts();
  const current = useSyncStore((s) =>
    s.order.map((id) => s.bySource[id]).find((p) => p && !p.result),
  );
  const [ending, setEnding] = useState<Ending | null>(null);
  const [open, setOpen] = useState(false);
  const [announcement, setAnnouncement] = useState("");
  const wasRunning = useRef(running);
  const detailsRef = useRef<HTMLDivElement>(null);
  const capsuleRef = useRef<HTMLDivElement>(null);
  // Pointer over it or focus in it: "Sync finished" waits (like when its details are open).
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  // The run this capsule is about was started by PageLamp itself, and whether the student has
  // been in the capsule or its details since it appeared.
  const automaticRun = useRef(false);
  const touched = useRef(false);

  // A run starting or ending decides what the capsule says afterwards; a stopped run leaves.
  useEffect(() => {
    if (running === wasRunning.current) return;
    wasRunning.current = running;
    if (running) {
      automaticRun.current = useSyncStore.getState().automatic !== null;
      // Still in the capsule of the run before: that counts as being in this one too.
      touched.current = useSyncStore.getState().watched;
      setEnding(null);
      setAnnouncement(tc("sync.syncing"));
      return;
    }
    const next =
      outcome === "done" || outcome === "doneWithErrors" || outcome === "failed" ? outcome : null;
    setEnding(next);
    // A stopped run leaves quietly on screen, but screen readers hear that it stopped. So does an
    // automatic run that found something only the student can fix: the pill now says "Needs
    // attention", and the pill isn't a live region.
    setAnnouncement(
      next
        ? tc(HEADLINE[next])
        : outcome === "stopped"
          ? tc("sync.stopped")
          : useSyncStore.getState().automaticProblem
            ? tc("sync.doneWithErrors")
            : "",
    );
  }, [running, outcome, tc]);

  // "Sync finished" leaves after 4 s, unless its details are open or the student is on it.
  useEffect(() => {
    if (ending !== "done" || open || hovered || focused) return;
    const timer = setTimeout(() => setEnding(null), FINISHED_MS);
    return () => clearTimeout(timer);
  }, [ending, open, hovered, focused]);

  const capsule: Capsule | null = running ? { kind: "running" } : ending ? { kind: ending } : null;
  const attention = capsule?.kind === "doneWithErrors" || capsule?.kind === "failed";

  // The store keeps an automatic run's problem on screen only while the student is here.
  const watched = capsule !== null && (open || focused);
  useEffect(() => {
    useSyncStore.setState({ watched });
    if (watched) touched.current = true;
  }, [watched]);

  // Esc dismisses a problem (§6.2), unless a dialog or popover takes it first.
  useEffect(() => {
    if (!attention || open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      if (document.querySelector('[role="dialog"], [role="alertdialog"]')) return;
      setEnding(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [attention, open]);

  const visible = capsule !== null;

  // When it leaves (timed out, ×, Esc, a run stopped from its details), focus that was in it
  // or in its details (or already fell to <body> as it went inert) goes to the page, not to
  // the top of the window. After a sync the student didn't start, only if they were in it:
  // focus resting on <body> stays where it is.
  const visibleRef = useRef(visible);
  const stranded = useCallback((active: Element | null) => {
    return (
      !active ||
      active === document.body ||
      !!capsuleRef.current?.contains(active) ||
      !!detailsRef.current?.contains(active)
    );
  }, []);
  useEffect(() => {
    const was = visibleRef.current;
    visibleRef.current = visible;
    if (was && !visible) {
      if (stranded(document.activeElement) && (touched.current || !automaticRun.current)) {
        focusPage();
      }
      setHovered(false);
      setFocused(false);
      // Its details go with it. Left "open", they would open by themselves with the next run's
      // capsule and take the focus, also for a sync the student didn't start.
      setOpen(false);
    }
  }, [visible, stranded]);
  // While it fades out the capsule keeps what it showed last: by then the store may hold no run
  // at all (an automatic sync that went wrong), and the text must not fall back to "Syncing…".
  const shown = useRef<{ capsule: Capsule; text: string; done: number; total: number | null }>({
    capsule: { kind: "done" },
    text: "",
    done: 0,
    total: null,
  });
  if (capsule) {
    shown.current = {
      capsule,
      text:
        capsule.kind === "running"
          ? total && current
            ? t("accessory.syncingOf", { done, total, source: current.label })
            : total
              ? tc("sync.syncingProgress", { done, total })
              : current
                ? t("accessory.syncingSource", { source: current.label })
                : tc("sync.syncing")
          : tc(HEADLINE[capsule.kind]),
      done,
      total,
    };
  }
  const { text } = shown.current;

  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-3 z-20 flex justify-center px-4">
      <p role="status" className="sr-only">
        {announcement}
      </p>
      <div
        ref={capsuleRef}
        className="pl-accessory pl-glass flex h-(--pl-size-accessory-height) max-w-(--pl-layout-capsule-max) min-w-0 items-center gap-0.5 rounded-full p-1 text-sm"
        data-state={visible ? "visible" : "hidden"}
        aria-hidden={visible ? undefined : true}
        inert={!visible}
        onPointerEnter={() => setHovered(true)}
        onPointerLeave={() => setHovered(false)}
        onFocus={() => setFocused(true)}
        onBlur={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget)) setFocused(false);
        }}
      >
        <Popover open={visible && open} onOpenChange={setOpen}>
          <PopoverTrigger asChild>
            <button
              type="button"
              className="flex h-7 min-w-0 items-center gap-2 rounded-full px-3 outline-hidden hover:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring"
            >
              <CapsuleIcon
                capsule={shown.current.capsule}
                done={shown.current.done}
                total={shown.current.total}
              />
              <span className="truncate">{text}</span>
            </button>
          </PopoverTrigger>
          <PopoverContent
            ref={detailsRef}
            side="top"
            sideOffset={8}
            className="w-80 p-0"
            aria-label={t("accessory.details")}
            // Closed because the capsule left: its trigger is inert, so continue on the page.
            onCloseAutoFocus={(event) => {
              if (!visibleRef.current) {
                event.preventDefault();
                focusPage();
              }
            }}
            // Focus the details, not Stop: Enter or Space right after opening must not stop a sync.
            onOpenAutoFocus={(event) => {
              event.preventDefault();
              detailsRef.current?.focus();
            }}
          >
            <SyncDetails headline={text} running={running} onNavigate={() => setOpen(false)} />
          </PopoverContent>
        </Popover>
        {attention ? (
          <button
            type="button"
            className="grid size-7 shrink-0 place-items-center rounded-full outline-hidden hover:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring"
            aria-label={t("accessory.dismiss")}
            onClick={() => setEnding(null)}
          >
            <X className="size-4" aria-hidden />
          </button>
        ) : null}
      </div>
    </div>
  );
}

function CapsuleIcon({
  capsule,
  done,
  total,
}: {
  capsule: Capsule;
  done: number;
  total: number | null;
}) {
  switch (capsule.kind) {
    case "running":
      return total ? (
        <ProgressRing value={done / total} />
      ) : (
        <LoaderCircle className="size-4 animate-spin" aria-hidden />
      );
    case "done":
      return <CircleCheck className="size-4 text-success" aria-hidden />;
    case "doneWithErrors":
      return <CircleAlert className="size-4 text-warning" aria-hidden />;
    case "failed":
      return <CircleX className="size-4 text-destructive" aria-hidden />;
  }
}

/**
 * The capsule's popover: one row per source, Stop while running, and the way to Sources & sync
 * (unless already there).
 */
function SyncDetails({
  headline,
  running,
  onNavigate,
}: {
  headline: string;
  running: boolean;
  onNavigate: () => void;
}) {
  const { t } = useTranslation("chrome");
  const { t: tc } = useTranslation();
  const order = useSyncStore((s) => s.order);
  const bySource = useSyncStore((s) => s.bySource);
  const runError = useSyncStore((s) => s.runError);
  const stopping = useSyncStore((s) => s.stopping);
  const stop = useStopSync();
  const onSources = useLocation().pathname === paths.sources;
  return (
    <div className="text-sm">
      <div className="flex min-h-12 items-center justify-between gap-3 border-b px-4 py-2">
        <h2 className="font-semibold">{headline}</h2>
        {running ? (
          <Button
            size="sm"
            variant="outline"
            onClick={() => void stop()}
            aria-disabled={stopping || undefined}
            className="aria-disabled:opacity-50"
          >
            {stopping ? tc("sync.stopping") : tc("sync.stop")}
          </Button>
        ) : null}
      </div>
      {runError ? (
        <p className="border-b px-4 py-3 text-muted-foreground">
          {runError.kind === "busy" ? tc("sync.busy") : tc(`errors.${runError.kind}`)}
        </p>
      ) : null}
      {order.length > 0 ? (
        <ul className="max-h-72 divide-y overflow-y-auto px-4">
          {order.map((id) => {
            const progress = bySource[id];
            return progress ? <SyncProgressRow key={id} progress={progress} showFixLink /> : null;
          })}
        </ul>
      ) : null}
      {onSources ? null : (
        <div className="border-t px-2 py-1.5">
          <Button asChild variant="ghost" size="sm">
            <Link to={paths.sources} onClick={onNavigate}>
              {t("accessory.open")}
            </Link>
          </Button>
        </div>
      )}
    </div>
  );
}

/** The page's main region (AppShell's <main tabIndex={-1}>), without scrolling it. */
function focusPage() {
  document.getElementById("main")?.focus({ preventScroll: true });
}
