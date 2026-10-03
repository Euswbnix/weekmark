import { Info } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { CodexStatus } from "@/api/ai";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { InstallUpdateDialog } from "@/features/updates/InstallUpdateDialog";
import { useCodexStore, useInstallCodex } from "@/stores/codex";
import { useCheckForUpdate, useUpdateStore } from "@/stores/updates";
import { CodexInstallProgress } from "./CodexInstall";

/**
 * What a RuntimeOutdated error means right now (design §2.3; the facade decides, rule 12):
 * - install_pin: this PageLamp pins a newer Codex than the one installed, so it installs it
 *   ("updating…"), with progress and Cancel;
 * - update_pagelamp: the Codex is already the pinned one, so only a PageLamp update helps:
 *   check for updates and offer to install.
 */
export function RuntimeOutdatedNotice({ status }: { status: CodexStatus }) {
  if (status.outdated_action === "install_pin") return <InstallPin />;
  if (status.outdated_action === "update_pagelamp") return <UpdatePageLamp />;
  return null;
}

function InstallPin() {
  const { t } = useTranslation("ai");
  const install = useCodexStore((s) => s.install);
  const start = useInstallCodex();
  // PageLamp installs the pinned version on its own; the student already chose this backend.
  useEffect(() => {
    if (useCodexStore.getState().install.phase === "idle") void start();
  }, [start]);
  return (
    <div className="space-y-2 rounded-row bg-muted p-3 text-sm">
      <p className="flex items-center gap-2 font-medium">
        <Info className="size-4 shrink-0" aria-hidden />
        {t("codex.outdated.installPin")}
      </p>
      <CodexInstallProgress />
      {install.phase === "failed" ? (
        <Button type="button" size="sm" onClick={() => void start()}>
          {t("codex.download.tryAgain")}
        </Button>
      ) : null}
    </div>
  );
}

function UpdatePageLamp() {
  const { t } = useTranslation("ai");
  const check = useCheckForUpdate();
  const checking = useUpdateStore((s) => s.checking);
  const checked = useUpdateStore((s) => s.checked);
  const available = useUpdateStore((s) => s.available);
  const [open, setOpen] = useState(false);
  return (
    <div className="space-y-2 rounded-row bg-muted p-3 text-sm">
      <p className="flex items-center gap-2 font-medium">
        <Info className="size-4 shrink-0" aria-hidden />
        {t("codex.outdated.updateApp")}
      </p>
      {available ? (
        <>
          <Button type="button" size="sm" onClick={() => setOpen(true)}>
            {t("codex.outdated.install", { version: available.version })}
          </Button>
          <InstallUpdateDialog update={available} open={open} onOpenChange={setOpen} />
        </>
      ) : (
        <>
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={() => void check()}
            aria-disabled={checking || undefined}
            className="aria-disabled:opacity-50"
          >
            {checking ? <Spinner aria-hidden /> : null}
            {t("codex.outdated.checkUpdates")}
          </Button>
          {checked && !checking ? (
            <p role="status" className="text-muted-foreground">
              {t("codex.outdated.noUpdateYet")}
            </p>
          ) : null}
        </>
      )}
    </div>
  );
}
