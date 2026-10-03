import { LoaderCircle } from "lucide-react";
import { useEffect, useId, useRef } from "react";
import { useTranslation } from "react-i18next";
import type { AvailableUpdate } from "@/api/client";
import { useActivity, useUpdaterStatus } from "@/api/queries";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { useStopSync, useSyncActivity, useSyncStore } from "@/stores/sync";
import { type InstallState, useInstallUpdate, useUpdateStore } from "@/stores/updates";

/** Work that holds "Install and restart" back (App::activity, and the CLI's sync). */
type Cause = "sync" | "other_sync" | "generation" | "codex_install";

const AVAILABLE_AFTER = {
  sync: "install.availableAfterSync",
  other_sync: "install.availableAfterOtherSync",
  generation: "install.availableAfterGeneration",
  codex_install: "install.availableAfterCodexInstall",
} as const satisfies Record<Cause, string>;

/** After the download: what the install goes on after. */
const HELD = {
  sync: "install.held",
  other_sync: "install.held",
  generation: "install.heldGeneration",
  codex_install: "install.heldCodexInstall",
} as const satisfies Record<Cause, string>;

/**
 * "Install PageLamp x.y.z?": the only way an update gets installed (decision D2: always ask).
 * Waits while anything runs that the restart would kill: a sync (the app's or the CLI's), an AI
 * reading or a Codex download (App::activity). On Windows it says PageLamp will close.
 * Once installing, it can't be dismissed: the app restarts at the end. Work that started during
 * the download holds the install back; while the dialog stays open, the install goes on when
 * that work finishes.
 */
export function InstallUpdateDialog({
  update,
  open,
  onOpenChange,
}: {
  update: AvailableUpdate;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation("updates");
  const { t: tc } = useTranslation();
  const status = useUpdaterStatus();
  const install = useUpdateStore((s) => s.install);
  const start = useInstallUpdate();
  const sync = useSyncActivity();
  const activity = useActivity(open);
  const running = activity.data?.items ?? [];
  // What holds the install back, a sync first: it can be stopped from here.
  const cause: Cause | null = sync.external
    ? "other_sync"
    : sync.busy
      ? "sync"
      : running.some((item) => item.kind === "generation")
        ? "generation"
        : running.some((item) => item.kind === "codex_install")
          ? "codex_install"
          : null;
  const busy = cause !== null;
  const stopping = useSyncStore((s) => s.stopping);
  const stopSync = useStopSync();
  const hintId = useId();
  const working =
    install.phase === "downloading" ||
    install.phase === "installing" ||
    install.phase === "restarting";
  const blocked = busy || working;

  const wasBusy = useRef(busy);
  useEffect(() => {
    if (open && wasBusy.current && !busy && useUpdateStore.getState().install.phase === "held") {
      void start();
    }
    wasBusy.current = busy;
  }, [busy, open, start]);

  const openChange = (next: boolean) => {
    if (working) return;
    // Closing is "not now": nothing installs behind the student's back.
    if (!next && install.phase === "held") useUpdateStore.setState({ install: { phase: "idle" } });
    onOpenChange(next);
  };

  return (
    <AlertDialog open={open} onOpenChange={openChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("install.title", { version: update.version })}</AlertDialogTitle>
          <AlertDialogDescription asChild>
            <div className="space-y-2">
              <p>{t("install.body")}</p>
              {status.data?.platform === "windows" ? (
                <p className="font-medium text-foreground">{t("install.windows")}</p>
              ) : null}
              <p>{t("install.aiApp")}</p>
            </div>
          </AlertDialogDescription>
        </AlertDialogHeader>
        {install.phase === "idle" ? null : (
          <InstallProgress install={install} heldId={hintId} heldText={t(HELD[cause ?? "sync"])} />
        )}
        <AlertDialogFooter className="items-center">
          {busy && !working && install.phase !== "held" ? (
            <span id={hintId} className="text-xs text-muted-foreground sm:mr-auto">
              {cause ? t(AVAILABLE_AFTER[cause]) : null}
            </span>
          ) : null}
          {/* This window's sync can be stopped from here (design §7); the CLI's can't. */}
          {sync.busy && !sync.external && !working ? (
            <Button
              type="button"
              variant="outline"
              onClick={() => void stopSync()}
              aria-disabled={stopping || undefined}
              className="aria-disabled:opacity-50"
            >
              {stopping ? tc("sync.stopping") : t("install.stopSync")}
            </Button>
          ) : null}
          <AlertDialogCancel disabled={working}>{tc("actions.cancel")}</AlertDialogCancel>
          <Button
            type="button"
            aria-disabled={blocked || undefined}
            aria-describedby={busy && !working ? hintId : undefined}
            className="aria-disabled:opacity-50"
            onClick={() => {
              if (!blocked) void start();
            }}
          >
            {working ? <LoaderCircle className="animate-spin" aria-hidden /> : null}
            {install.phase === "failed" ? t("install.tryAgain") : t("install.confirm")}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/**
 * `heldId`: the held message describes the Install button, as the hint does otherwise.
 * `heldText` names what the install waits for.
 */
function InstallProgress({
  install,
  heldId,
  heldText,
}: {
  install: InstallState;
  heldId: string;
  heldText: string;
}) {
  const { t } = useTranslation("updates");
  const { t: tc } = useTranslation();
  if (install.phase === "held") {
    return (
      <p id={heldId} role="status" className="text-sm text-muted-foreground">
        {heldText}
      </p>
    );
  }
  if (install.phase === "failed") {
    return (
      <div role="alert" className="space-y-1 text-sm text-destructive">
        <p className="font-medium">{t("install.failed")}</p>
        <p>{tc(`errors.${install.error.kind}`)}</p>
        {install.error.message ? (
          <p lang="en" className="text-xs opacity-80">
            {install.error.message}
          </p>
        ) : null}
      </div>
    );
  }
  const percent =
    install.phase === "downloading" && install.total
      ? Math.min(100, Math.round((install.downloaded / install.total) * 100))
      : null;
  const text =
    install.phase === "downloading"
      ? percent === null
        ? t("install.downloadingUnknown")
        : t("install.downloading", { percent })
      : install.phase === "installing"
        ? t("install.installing")
        : t("install.restarting");
  return (
    <div className="space-y-2">
      <Progress
        aria-label={t("install.progressLabel")}
        value={install.phase === "downloading" ? (percent ?? 0) : 100}
      />
      <p role="status" className="text-sm text-muted-foreground">
        {text}
      </p>
    </div>
  );
}
