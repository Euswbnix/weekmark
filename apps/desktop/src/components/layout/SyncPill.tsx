import { CircleAlert, CircleCheck, CircleDashed, History, LoaderCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { useStatus } from "@/api/queries";
import type { AutoSync } from "@/api/types";
import { formatRelative } from "@/lib/format";
import { paths } from "@/lib/routes";
import { useMinute } from "@/lib/useMinute";
import { cn } from "@/lib/utils";
import { useSyncStore } from "@/stores/sync";

const HOUR = 60 * 60 * 1000;

/**
 * How old the last sync may be before it stops looking fresh: the interval the student chose
 * for automatic sync, or a day when that is off.
 */
const FRESH_FOR: Record<AutoSync, number> = {
  off: 24 * HOUR,
  daily: 24 * HOUR,
  twice_daily: 12 * HOUR,
};

/**
 * Always-visible sync status at the bottom of the sidebar: "Synced 2h ago", "Syncing…" (another
 * process, e.g. the CLI) or "Needs attention". Links to Sources & sync. This window's own runs
 * show their progress in the accessory bar; meanwhile the pill keeps the last state.
 *
 * The pill speaks for every source, so it shows the OLDEST one: its last full sync. One source
 * synced a minute ago doesn't make the others fresh (a source that quietly fails to sync for
 * days, or the student syncing a single one by hand). Old data doesn't get the check mark: when
 * any source has never synced, or its last full sync is older than the chosen interval, the
 * same "Synced 5 days ago" shows with a plain history icon. Not a warning (nothing is wrong,
 * and "Needs attention" is for problems), just no longer a sign that everything is current.
 *
 * A lighter automatic sync (deadlines and announcements only) doesn't move this clock; when it
 * last read a source is on that source's card.
 */
export function SyncPill() {
  const { t, i18n } = useTranslation();
  const status = useStatus();
  const running = useSyncStore((s) => s.running);
  useMinute();

  const sources = status.data?.sources ?? [];
  const failing = sources.some((s) => s.last_error_kind);
  const externalSync = !running && (status.data?.sync_in_progress ?? false);

  let icon = <CircleDashed className="size-4" aria-hidden />;
  let text = t("sync.never");
  let tone = "text-muted-foreground";

  if (externalSync) {
    icon = <LoaderCircle className="size-4 animate-spin" aria-hidden />;
    text = t("sync.syncing");
    tone = "text-foreground";
  } else if (failing) {
    icon = <CircleAlert className="size-4" aria-hidden />;
    text = t("sync.needsAttention");
    tone = "text-destructive";
  } else if (status.data && sources.some((s) => s.last_synced_at)) {
    const synced = sources
      .map((s) => s.last_synced_at)
      .filter((at): at is string => !!at)
      .sort();
    const oldest = synced[0] ?? "";
    const old =
      synced.length < sources.length ||
      Date.now() - Date.parse(oldest) > FRESH_FOR[status.data.auto_sync];
    icon = old ? (
      <History className="size-4" aria-hidden data-sync="old" />
    ) : (
      <CircleCheck className="size-4" aria-hidden data-sync="fresh" />
    );
    text = t("sync.syncedAgo", { when: formatRelative(oldest, i18n.language) });
  }

  return (
    // No live region here: the accessory bar announces this window's runs, the Courses banner
    // another process's. The link's name says both the status and where it goes.
    <Link
      to={paths.sources}
      aria-label={t("sync.pillLabel", { status: text })}
      className={cn(
        "flex items-center gap-2 rounded-md px-3 py-2 text-xs transition-colors hover:bg-sidebar-accent",
        tone,
      )}
    >
      {icon}
      <span>{text}</span>
    </Link>
  );
}
