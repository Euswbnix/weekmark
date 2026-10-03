import { Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
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

/** "Delete…" with a confirmation: the explanation and its text go, with no undo (design §7). */
export function DeleteExplanation({
  onDelete,
  deleting,
}: {
  onDelete: () => void;
  deleting: boolean;
}) {
  const { t } = useTranslation("explain");
  const { t: tc } = useTranslation();
  return (
    <AlertDialog>
      <AlertDialogTrigger asChild>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          // Not `disabled`: the dialog gives the focus back here while the deletion runs.
          aria-disabled={deleting || undefined}
          className="aria-disabled:opacity-50"
          onClick={(event) => {
            if (deleting) event.preventDefault();
          }}
        >
          <Trash2 aria-hidden />
          {t("result.delete")}
        </Button>
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("result.deleteTitle")}</AlertDialogTitle>
          <AlertDialogDescription>{t("result.deleteBody")}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{tc("actions.cancel")}</AlertDialogCancel>
          <AlertDialogAction onClick={onDelete}>{t("result.deleteConfirm")}</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
