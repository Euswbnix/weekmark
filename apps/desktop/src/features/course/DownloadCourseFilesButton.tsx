import { CloudDownload } from "lucide-react";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { Course } from "@/api/types";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { useApiErrorText } from "@/lib/useApiErrorText";
import { useDownloadCourseFiles, useSyncActivity, useSyncStore } from "@/stores/sync";

// Warnings listed in the toast; the rest are summed up as "…and N more".
const SHOWN_WARNINGS = 3;

/**
 * "Download files…" for a Canvas course: the whole course, and only after the dialog says a
 * download through Canvas can count as viewing the file (required copy). Shown in the course
 * header; the caller decides whether there is anything to download.
 */
export function DownloadCourseFilesButton({
  course,
  size = "sm",
}: {
  course: Course;
  size?: "xs" | "sm" | "default";
}) {
  const { t } = useTranslation("course");
  const { t: tc } = useTranslation();
  const errorText = useApiErrorText();
  const download = useDownloadCourseFiles();
  const hintId = useId();
  const [open, setOpen] = useState(false);
  // "Downloading…" only while THIS course's files download. During any other run (a sync, the
  // first sync, another course's download, the CLI) the label stays, the button waits and says
  // why: a greyed "Downloading…" would suggest files are being fetched (Canvas may count views).
  const downloadingThis = useSyncStore((s) => s.running && s.downloadCourseId === course.id);
  const { busy } = useSyncActivity();
  const waiting = busy && !downloadingThis;

  async function start() {
    const ran = await download(course.id);
    if (!ran) {
      // This window's own run is in the way (possibly one PageLamp started itself).
      toast.info(tc("errors.busy"));
      return;
    }
    const { lastSummary, runError } = useSyncStore.getState();
    const result = lastSummary?.results[0];
    // Files that were skipped (too large, locked) are only explained in the run's warnings.
    const warnings = result?.warnings ?? [];
    const description =
      warnings.length > 0 ? (
        <ul className="mt-1 space-y-0.5">
          {warnings.slice(0, SHOWN_WARNINGS).map((warning) => (
            <li key={warning} lang="en">
              {warning}
            </li>
          ))}
          {warnings.length > SHOWN_WARNINGS ? (
            <li>{t("download.moreWarnings", { count: warnings.length - SHOWN_WARNINGS })}</li>
          ) : null}
        </ul>
      ) : undefined;
    if (runError) toast.error(errorText(runError));
    else if (result && !result.ok) toast.error(tc(`sourceError.${result.error_kind ?? "other"}`));
    else if (result?.files_downloaded) {
      toast.success(t("download.done", { count: result.files_downloaded }), { description });
    } else if (warnings.length > 0) toast.warning(t("download.someSkipped"), { description });
    else toast.success(t("download.doneNone"));
  }

  return (
    <AlertDialog open={open} onOpenChange={(next) => setOpen(next && !busy)}>
      {waiting ? (
        <span id={hintId} className="text-xs text-muted-foreground">
          {t("download.availableAfterSync")}
        </span>
      ) : null}
      <AlertDialogTrigger asChild>
        <Button
          size={size}
          variant="outline"
          aria-disabled={busy || undefined}
          aria-describedby={waiting ? hintId : undefined}
          className="aria-disabled:opacity-50"
        >
          <CloudDownload aria-hidden />
          {downloadingThis ? t("download.running") : t("download.action")}
        </Button>
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("download.dialogTitle")}</AlertDialogTitle>
          <AlertDialogDescription asChild>
            <div className="space-y-2">
              <p className="font-medium text-foreground">{t("download.viewingNotice")}</p>
              <p>{t("download.dialogDetail")}</p>
            </div>
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{tc("actions.cancel")}</AlertDialogCancel>
          <AlertDialogAction disabled={busy} onClick={() => void start()}>
            {t("download.confirm")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
