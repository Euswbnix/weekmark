import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { CodexLoginMethod, LoginEvent } from "@/api/ai";
import { aiKeys } from "@/api/ai-queries";
import { useApi } from "@/api/context";
import { type ApiError, toApiError } from "@/api/errors";
import { CopyButton } from "@/components/common/CopyButton";
import { ExternalLink } from "@/components/common/ExternalLink";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Spinner } from "@/components/ui/spinner";
import { useAiErrorText } from "@/features/ai/useAiErrorText";
import { useReturnFocus } from "@/lib/focus";

type Stage =
  | { kind: "choose" }
  | { kind: "running"; event: LoginEvent | null }
  | { kind: "failed"; error: ApiError };

/**
 * "Sign in with ChatGPT": Codex runs its own sign-in (browser or device code); PageLamp only shows
 * what Codex asks the student to do and never sees credentials. The device code lives only in
 * this dialog's state; closing the dialog cancels the sign-in.
 */
export function CodexSignInDialog({
  open,
  onOpenChange,
  onSignedIn,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Codex is signed in: the parent shows the disclosure sheet next. */
  onSignedIn: () => void;
}) {
  const returnFocus = useReturnFocus();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md" {...returnFocus}>
        {open ? (
          <SignIn
            onDone={() => onOpenChange(false)}
            onSignedIn={() => {
              onOpenChange(false);
              onSignedIn();
            }}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

function SignIn({ onDone, onSignedIn }: { onDone: () => void; onSignedIn: () => void }) {
  const { t } = useTranslation("ai");
  const { t: tc } = useTranslation();
  const api = useApi();
  const client = useQueryClient();
  const errorText = useAiErrorText();
  const [method, setMethod] = useState<CodexLoginMethod>("browser");
  const [stage, setStage] = useState<Stage>({ kind: "choose" });
  const ids = { method: useId(), options: useId() };
  // Closing while Codex waits for the student cancels the sign-in.
  const running = useRef(false);
  useEffect(
    () => () => {
      if (running.current) void api.cancelCodexLogin();
    },
    [api],
  );

  async function start() {
    running.current = true;
    setStage({ kind: "running", event: null });
    try {
      await api.codexLogin(method, (event) => {
        // "waiting" and "done" add nothing to show; keep the browser or code step on screen.
        if (event.type === "browser_opened" || event.type === "device_code") {
          setStage({ kind: "running", event });
        }
      });
      running.current = false;
      await client.invalidateQueries({ queryKey: aiKeys.all });
      toast.success(t("codex.signInDialog.done"));
      onSignedIn();
    } catch (error) {
      running.current = false;
      const e = toApiError(error);
      if (e.kind === "cancelled") return;
      setStage({ kind: "failed", error: e });
    }
  }

  function cancel() {
    if (running.current) {
      running.current = false;
      void api.cancelCodexLogin();
    }
    onDone();
  }

  return (
    <>
      <DialogHeader>
        <DialogTitle>{t("codex.signInDialog.title")}</DialogTitle>
        <DialogDescription>{t("codex.signInDialog.description")}</DialogDescription>
      </DialogHeader>

      {stage.kind === "choose" ? (
        <div className="space-y-2">
          <p id={ids.method} className="text-sm font-medium">
            {t("codex.signInDialog.method")}
          </p>
          <RadioGroup
            aria-labelledby={ids.method}
            className="gap-0 divide-y"
            value={method}
            onValueChange={(value) => setMethod(value as CodexLoginMethod)}
          >
            {(["browser", "device_code"] as const).map((option) => {
              const id = `${ids.options}-${option}`;
              const key = option === "browser" ? "browser" : "device";
              return (
                <div
                  key={option}
                  className="relative flex items-start gap-3 rounded-row px-3 py-3 transition-colors hover:bg-muted has-data-checked:bg-ink/9"
                >
                  <RadioGroupItem
                    id={id}
                    value={option}
                    aria-describedby={`${id}-hint`}
                    className="mt-0.5"
                  />
                  <div className="grid gap-1">
                    <Label htmlFor={id} className="cursor-pointer after:absolute after:inset-0">
                      {t(`codex.signInDialog.${key}`)}
                    </Label>
                    <p id={`${id}-hint`} className="text-sm text-muted-foreground">
                      {t(`codex.signInDialog.${key}Hint`)}
                    </p>
                  </div>
                </div>
              );
            })}
          </RadioGroup>
        </div>
      ) : null}

      {stage.kind === "running" ? <Waiting event={stage.event} /> : null}

      {stage.kind === "failed" ? (
        <div role="alert" className="space-y-1 text-sm text-destructive">
          <p className="font-medium">{t("codex.signInDialog.failed")}</p>
          <p>{errorText(stage.error)}</p>
        </div>
      ) : null}

      <DialogFooter>
        <Button type="button" variant="outline" onClick={cancel}>
          {tc("actions.cancel")}
        </Button>
        {stage.kind === "choose" ? (
          <Button type="button" onClick={() => void start()}>
            {t("codex.signInDialog.start")}
          </Button>
        ) : null}
        {stage.kind === "failed" ? (
          <Button type="button" onClick={() => setStage({ kind: "choose" })}>
            {t("codex.signInDialog.tryAgain")}
          </Button>
        ) : null}
      </DialogFooter>
    </>
  );
}

function Waiting({ event }: { event: LoginEvent | null }) {
  const { t } = useTranslation("ai");
  if (event?.type === "device_code") {
    return (
      <div className="space-y-3 text-sm">
        <p>{t("codex.signInDialog.deviceStep")}</p>
        <p>
          <ExternalLink href={event.verification_url}>{event.verification_url}</ExternalLink>
        </p>
        <div className="flex items-center gap-3">
          <span className="sr-only">{t("codex.signInDialog.codeLabel")}</span>
          <code className="pl-concentric bg-muted px-3 py-1.5 font-mono text-lg tracking-widest [--pl-pad:1rem]">
            {event.user_code}
          </code>
          <CopyButton text={event.user_code} />
        </div>
        {event.expires_in_secs ? (
          <p className="text-muted-foreground">
            {t("codex.signInDialog.expires", {
              minutes: Math.round(event.expires_in_secs / 60),
            })}
          </p>
        ) : null}
        <p role="status" className="flex items-center gap-2 text-muted-foreground">
          <Spinner aria-hidden />
          {t("codex.signInDialog.waiting")}
        </p>
      </div>
    );
  }
  return (
    <div className="space-y-3 text-sm">
      <p role="status" className="flex items-center gap-2">
        <Spinner aria-hidden />
        {t("codex.signInDialog.waitingBrowser")}
      </p>
      {event?.type === "browser_opened" && event.url ? (
        <p>
          <ExternalLink href={event.url}>{t("codex.signInDialog.openAgain")}</ExternalLink>
        </p>
      ) : null}
    </div>
  );
}
