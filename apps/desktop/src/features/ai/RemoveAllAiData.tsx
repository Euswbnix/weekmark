import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useCodexStatus, useRemoveAllAiData } from "@/api/ai-queries";
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
import { Button } from "@/components/ui/button";
import { useAiErrorText } from "./useAiErrorText";

/**
 * "Remove all AI data" (design §8 "Your controls"): keys, choices, ledger, generated content.
 * The dialog names the ChatGPT sign-out only when there is one to undo: while the ChatGPT plan
 * isn't offered, no copy mentions it.
 */
export function RemoveAllAiData() {
  const { t } = useTranslation("ai");
  const { t: tc } = useTranslation();
  const remove = useRemoveAllAiData();
  const codex = useCodexStatus();
  const signedInToChatGpt = codex.data !== undefined && codex.data.login.state !== "signed_out";
  const errorText = useAiErrorText();
  const [open, setOpen] = useState(false);

  async function confirm() {
    try {
      await remove.mutateAsync();
      toast.success(t("removeAll.done"));
      setOpen(false);
    } catch {
      // Shown in the dialog (remove.error).
    }
  }

  return (
    <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-2 border-t pt-4">
      <p className="min-w-0 flex-1 basis-64 text-sm text-muted-foreground">{t("removeAll.hint")}</p>
      <AlertDialog open={open} onOpenChange={setOpen}>
        <Button type="button" variant="destructive" onClick={() => setOpen(true)}>
          {t("removeAll.button")}
        </Button>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("removeAll.title")}</AlertDialogTitle>
            <AlertDialogDescription>
              {signedInToChatGpt ? t("removeAll.bodyCodex") : t("removeAll.body")}
            </AlertDialogDescription>
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
                event.preventDefault();
                void confirm();
              }}
            >
              {t("removeAll.confirm")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
