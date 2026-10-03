import { CircleCheck, CircleDashed, TriangleAlert } from "lucide-react";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { type AiBackendStatus, backendKey, type ModelProviderRecord } from "@/api/ai";
import { useRemoveModelProvider } from "@/api/ai-queries";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { DataPolicyLine } from "./DataPolicyLine";
import { useAiErrorText } from "./useAiErrorText";

/**
 * One configured backend: its name, kind and state, problems, the key's last 4 characters, the
 * data-policy line, and what can be done with it. The state and problems come from the facade.
 */
export function BackendRow({
  status,
  provider,
  onShowDisclosure,
  onReplaceKey,
}: {
  status: AiBackendStatus;
  /** The record behind an API-key or local backend (ai_status lists them apart). */
  provider: ModelProviderRecord | null;
  onShowDisclosure: () => void;
  onReplaceKey: (provider: ModelProviderRecord) => void;
}) {
  const { t } = useTranslation("ai");
  const headingId = useId();
  const StateIcon =
    status.state === "ready"
      ? CircleCheck
      : status.state === "needs_disclosure"
        ? CircleDashed
        : TriangleAlert;
  return (
    // Focusable from code only: after a new backend is turned on, focus lands on its row.
    <li
      tabIndex={-1}
      data-backend={backendKey(status.backend)}
      className="space-y-2 py-4 outline-hidden focus-visible:rounded-row focus-visible:ring-3 focus-visible:ring-ring"
    >
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <h3 id={headingId} className="font-medium">
          {status.label}
        </h3>
        <Badge variant="outline">{t(`backend.kind.${status.kind}`)}</Badge>
        <span className="inline-flex items-center gap-1 text-sm text-muted-foreground">
          <StateIcon className="size-4" aria-hidden />
          {t(`backend.state.${status.state}`)}
        </span>
      </div>
      {provider?.key_last4 ? (
        <p className="text-sm text-muted-foreground">
          {t("backend.keyEnds", { last4: provider.key_last4 })}
        </p>
      ) : null}
      <DataPolicyLine facts={status.disclosure} name={status.label} />
      {status.problems.length > 0 ? (
        <ul className="space-y-1 text-sm">
          {status.problems.map((problem) => (
            <li key={problem}>{t(`backend.problem.${problem}`)}</li>
          ))}
        </ul>
      ) : null}
      <div className="flex flex-wrap gap-2 pt-1">
        {status.state === "needs_disclosure" ? (
          <Button type="button" size="sm" onClick={onShowDisclosure} aria-describedby={headingId}>
            {t("backend.turnOn")}
          </Button>
        ) : (
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={onShowDisclosure}
            aria-describedby={headingId}
          >
            {t("backend.whatsShared")}
          </Button>
        )}
        {provider && status.kind === "api_key" ? (
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={() => onReplaceKey(provider)}
            aria-describedby={headingId}
          >
            {t("backend.replaceKey")}
          </Button>
        ) : null}
        {provider ? <RemoveBackendButton provider={provider} describedBy={headingId} /> : null}
      </div>
    </li>
  );
}

function RemoveBackendButton({
  provider,
  describedBy,
}: {
  provider: ModelProviderRecord;
  describedBy: string;
}) {
  const { t } = useTranslation("ai");
  const { t: tc } = useTranslation();
  const remove = useRemoveModelProvider();
  const errorText = useAiErrorText();
  const [open, setOpen] = useState(false);
  const name = provider.label;

  async function confirm() {
    try {
      await remove.mutateAsync(provider.provider_id);
      toast.success(t("backend.removed", { name }));
      setOpen(false);
    } catch {
      // Shown in the dialog (remove.error).
    }
  }

  return (
    <AlertDialog open={open} onOpenChange={setOpen}>
      <Button
        type="button"
        size="sm"
        variant="ghost"
        onClick={() => setOpen(true)}
        aria-describedby={describedBy}
      >
        {t("backend.remove")}
      </Button>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("backend.removeTitle", { name })}</AlertDialogTitle>
          <AlertDialogDescription>{t("backend.removeBody", { name })}</AlertDialogDescription>
        </AlertDialogHeader>
        {remove.error ? (
          <p role="alert" className="text-sm text-destructive">
            {errorText(remove.error)}
          </p>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel>{tc("actions.cancel")}</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            onClick={(event) => {
              // Keep the dialog open until the removal is done (or failed).
              event.preventDefault();
              void confirm();
            }}
          >
            {t("backend.remove")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
