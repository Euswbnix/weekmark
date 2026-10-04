import { CircleAlert, UserRound } from "lucide-react";
import { type FormEvent, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { type ApiError, toApiError } from "@/api/errors";
import { useUpdateSourceSecret } from "@/api/queries";
import type { SourceRecord } from "@/api/types";
import { SecretInput } from "@/components/common/SecretInput";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field, FieldError, FieldLabel } from "@/components/ui/field";
import { Spinner } from "@/components/ui/spinner";
import { useReturnFocus } from "@/lib/focus";
import { useStartSync } from "@/stores/sync";
import { describedBy, useAddErrorText } from "./add/AddErrorMessage";

interface ReplaceSecretDialogProps {
  /** The Canvas or calendar-feed source to fix; null keeps the dialog closed. */
  source: SourceRecord | null;
  onClose: () => void;
}

/** Replace an expired Canvas token or a changed feed address without removing the source. */
export function ReplaceSecretDialog({ source, onClose }: ReplaceSecretDialogProps) {
  const returnFocus = useReturnFocus();
  return (
    <Dialog
      open={source !== null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="sm:max-w-md" {...returnFocus}>
        {/* Content unmounts when the dialog closes, which discards the typed secret. */}
        {source ? <ReplaceSecretForm source={source} onDone={onClose} /> : null}
      </DialogContent>
    </Dialog>
  );
}

function ReplaceSecretForm({ source, onDone }: { source: SourceRecord; onDone: () => void }) {
  const { t } = useTranslation("sources");
  const { t: tc } = useTranslation();
  const update = useUpdateSourceSecret();
  const startSync = useStartSync();
  const errorText = useAddErrorText();
  const id = useId();
  const canvas = source.kind === "canvas";

  // Secret: only in this state, cleared on success.
  const [secret, setSecret] = useState("");
  const [missing, setMissing] = useState(false);
  const [error, setError] = useState<ApiError | null>(null);
  const [pending, setPending] = useState(false);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending) return;
    const value = secret.trim();
    setMissing(value === "");
    if (value === "") return;
    setError(null);
    setPending(true);
    try {
      await update.mutateAsync({ sourceId: source.id, secret: value });
      setSecret("");
      const sourceId = source.id;
      toast.success(t(canvas ? "replace.doneToken" : "replace.doneFeed", { label: source.label }), {
        description: t("replace.syncHint"),
        action: {
          label: tc("actions.syncNow"),
          onClick: () =>
            void startSync(sourceId).then((ran) => {
              // This window's own run is in the way (possibly one PageLamp started itself).
              if (!ran) toast.info(tc("errors.busy"));
            }),
        },
      });
      onDone();
    } catch (err) {
      setError(toApiError(err));
      document.getElementById(inputId)?.focus();
    } finally {
      setPending(false);
      // Drop the mutation (and the secret in its variables) from the mutation cache now.
      update.reset();
    }
  }

  const inputId = `${id}-secret`;
  const missingId = `${id}-missing`;
  const errorId = `${id}-error`;

  return (
    <form onSubmit={submit} noValidate aria-busy={pending} className="grid gap-4">
      <DialogHeader>
        <DialogTitle>{t(canvas ? "replace.titleToken" : "replace.titleFeed")}</DialogTitle>
        <DialogDescription>
          {t(canvas ? "replace.descriptionToken" : "replace.descriptionFeed", {
            label: source.label,
          })}
        </DialogDescription>
      </DialogHeader>

      {canvas ? (
        <Alert role="note">
          <UserRound aria-hidden />
          <AlertDescription>{tc("canvasNotice")}</AlertDescription>
        </Alert>
      ) : null}

      <Field data-invalid={missing || error ? true : undefined}>
        <FieldLabel htmlFor={inputId}>
          {t(canvas ? "replace.tokenLabel" : "replace.feedLabel")}
        </FieldLabel>
        <SecretInput
          id={inputId}
          value={secret}
          onChange={(e) => setSecret(e.target.value)}
          aria-invalid={missing || error ? true : undefined}
          aria-describedby={describedBy(missing && missingId, error && errorId)}
        />
        {missing ? (
          <FieldError id={missingId}>
            {t(canvas ? "replace.requiredToken" : "replace.requiredFeed")}
          </FieldError>
        ) : null}
      </Field>

      {error ? (
        <Alert variant="destructive" id={errorId}>
          <CircleAlert aria-hidden />
          <AlertTitle>{errorText(canvas ? "token" : "feed", error)}</AlertTitle>
          {error.message ? <AlertDescription>{error.message}</AlertDescription> : null}
        </Alert>
      ) : null}

      <div className="flex justify-end gap-2">
        <DialogClose asChild>
          <Button type="button" variant="outline">
            {tc("actions.cancel")}
          </Button>
        </DialogClose>
        <Button type="submit" disabled={pending} aria-busy={pending}>
          {pending ? <Spinner aria-hidden /> : null}
          {pending ? t("replace.submitting") : t("replace.submit")}
        </Button>
      </div>
      <span className="sr-only" aria-live="polite">
        {pending ? t("replace.submitting") : ""}
      </span>
    </form>
  );
}
