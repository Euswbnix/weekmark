import { useQueryClient } from "@tanstack/react-query";
import { FolderSync, Plus, RefreshCw } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { queryKeys, useSources, useStatus } from "@/api/queries";
import type { SourceRecord } from "@/api/types";
import { AiDisclosure } from "@/components/common/AiDisclosure";
import { ErrorState } from "@/components/common/ErrorState";
import { PageHeader } from "@/components/common/PageHeader";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Spinner } from "@/components/ui/spinner";
import { useStartSync, useSyncActivity, useSyncStore } from "@/stores/sync";
import { AddSourceDialog } from "./AddSourceDialog";
import { AutoSyncSetting } from "./AutoSyncSetting";
import { BusyBanner } from "./BusyBanner";
import { ReplaceSecretDialog } from "./ReplaceSecretDialog";
import { SourceCard } from "./SourceCard";
import { SourcesSkeleton } from "./SourcesSkeleton";
import { SyncProgressPanel } from "./SyncProgressPanel";

/** Sources & sync: every configured source, its health, and sync controls. */
export function SourcesPage() {
  const { t } = useTranslation("sources");
  const sources = useSources();
  const status = useStatus();
  const startSync = useStartSync();
  const queryClient = useQueryClient();
  const { running, busy } = useSyncActivity();
  const runError = useSyncStore((s) => s.runError);
  const [addOpen, setAddOpen] = useState(false);
  // Only the id is kept, so the dialog always shows the latest record.
  const [replaceId, setReplaceId] = useState<string | null>(null);

  const list = sources.data ?? [];
  const replacing = list.find((s) => s.id === replaceId) ?? null;
  // Another process holds the sync lock. While *we* run, sync_in_progress is our own run;
  // a failed run with kind "busy" is explained inside the progress panel instead.
  const externalBusy =
    !running && runError?.kind !== "busy" && status.data?.sync_in_progress === true;

  return (
    <>
      <PageHeader
        title={t("title")}
        description={t("description")}
        actions={
          <>
            <Button variant="outline" onClick={() => setAddOpen(true)}>
              <Plus aria-hidden />
              {t("actions.add")}
            </Button>
            <Button
              onClick={() => {
                if (!busy && list.length > 0) void startSync();
              }}
              aria-disabled={busy || list.length === 0 || undefined}
              aria-busy={running}
              className="aria-disabled:opacity-50"
            >
              {running ? <Spinner aria-hidden /> : <RefreshCw aria-hidden />}
              {t("actions.syncAll")}
            </Button>
          </>
        }
      />

      <div className="space-y-6">
        {list.length > 0 ? <AutoSyncSetting /> : null}

        {externalBusy ? (
          <BusyBanner
            onCheckAgain={() => void queryClient.invalidateQueries({ queryKey: queryKeys.all })}
            checking={status.isFetching}
          />
        ) : null}

        <SyncProgressPanel
          onRetry={() => void startSync()}
          onDismiss={() => useSyncStore.getState().hideRun()}
          announce={false}
        />

        <SourceList
          sources={sources}
          onAdd={() => setAddOpen(true)}
          onReplaceSecret={(source) => setReplaceId(source.id)}
        />

        <AiDisclosure variant="short" />
      </div>

      <AddSourceDialog open={addOpen} onOpenChange={setAddOpen} />
      <ReplaceSecretDialog source={replacing} onClose={() => setReplaceId(null)} />
    </>
  );
}

function SourceList({
  sources,
  onAdd,
  onReplaceSecret,
}: {
  sources: ReturnType<typeof useSources>;
  onAdd: () => void;
  onReplaceSecret: (source: SourceRecord) => void;
}) {
  const { t } = useTranslation("sources");

  if (sources.isPending) return <SourcesSkeleton />;
  if (sources.isError) {
    return <ErrorState error={sources.error} onRetry={() => void sources.refetch()} />;
  }
  if (sources.data.length === 0) {
    return (
      <Empty className="border">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <FolderSync aria-hidden />
          </EmptyMedia>
          <EmptyTitle>{t("empty.title")}</EmptyTitle>
          <EmptyDescription>{t("empty.description")}</EmptyDescription>
        </EmptyHeader>
        <EmptyContent>
          <Button onClick={onAdd}>
            <Plus aria-hidden />
            {t("actions.addFirst")}
          </Button>
        </EmptyContent>
      </Empty>
    );
  }
  return (
    <ul className="space-y-4">
      {sources.data.map((source) => (
        <li key={source.id}>
          <SourceCard source={source} onReplaceSecret={onReplaceSecret} />
        </li>
      ))}
    </ul>
  );
}
