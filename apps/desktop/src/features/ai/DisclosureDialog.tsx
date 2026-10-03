import { type ReactNode, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { AiBackendStatus, DisclosureFacts } from "@/api/ai";
import { useAcknowledgeAiDisclosure } from "@/api/ai-queries";
import { ExternalLink } from "@/components/common/ExternalLink";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { useAiErrorText } from "./useAiErrorText";

/**
 * The disclosure sheet before a backend's first use (design §7; Canvas API Policy §2E, all six
 * items): generative AI use and labelling, what is sent and to whom, training and storage, who
 * else can see it, cost, age, limitations and risks, and ownership. Rendered only from the
 * facade's facts; acknowledging records the facts' version, so a change asks again. Never
 * collapsed: every section is always shown in full.
 */
export function DisclosureDialog({
  status,
  open,
  onOpenChange,
  returnFocus,
}: {
  status: AiBackendStatus;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /**
   * Where focus goes on close, when the element that opened the sheet is gone (e.g. the add
   * dialog's submit button). Default: back to that element.
   */
  returnFocus?: () => HTMLElement | null;
}) {
  const content = useRef<HTMLDivElement>(null);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        ref={content}
        className="max-h-[85vh] overflow-y-auto sm:max-w-xl"
        // Start at the top (title, then every section) rather than on the first link, which
        // would scroll the sheet past what it says first.
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          content.current?.focus();
        }}
        onCloseAutoFocus={(event) => {
          const target = returnFocus?.();
          if (target) {
            event.preventDefault();
            target.focus();
          }
        }}
      >
        {/* Remounts on open, so the confirmations start unticked every time. */}
        <DisclosureBody status={status} onDone={() => onOpenChange(false)} />
      </DialogContent>
    </Dialog>
  );
}

function DisclosureBody({ status, onDone }: { status: AiBackendStatus; onDone: () => void }) {
  const { t } = useTranslation("ai");
  const errorText = useAiErrorText();
  const acknowledge = useAcknowledgeAiDisclosure();
  const facts = status.disclosure;
  // The title and button name the backend as the student set it up; the facts name who
  // receives the data (e.g. "OpenAI" for the ChatGPT plan through Codex).
  const name = status.label;
  const who = facts.recipient.name;
  const needsAge = !!facts.min_age;
  const needsFreeTier = facts.training.kind === "may_train_free_tier";
  const [ageOk, setAgeOk] = useState(false);
  const [freeTierOk, setFreeTierOk] = useState(false);
  const acknowledged = status.disclosure_acknowledged === facts.version;
  const changed = status.problems.includes("disclosure_changed");
  const canAccept = (!needsAge || ageOk) && (!needsFreeTier || freeTierOk);
  const ids = { age: useId(), freeTier: useId(), confirmFirst: useId() };

  async function accept() {
    if (!canAccept || acknowledge.isPending) return;
    try {
      await acknowledge.mutateAsync({ backend: status.backend, version: facts.version });
      toast.success(t("disclosure.accepted", { name }));
      onDone();
    } catch {
      // Shown below (acknowledge.error).
    }
  }

  return (
    <>
      <DialogHeader>
        <DialogTitle>{t("disclosure.title", { name })}</DialogTitle>
        <DialogDescription>{t("disclosure.intro")}</DialogDescription>
        {changed ? <p className="text-sm font-medium">{t("disclosure.changed")}</p> : null}
      </DialogHeader>

      <div className="space-y-4">
        <Item heading={t("disclosure.gai.heading")}>
          <p>{t("disclosure.gai.body")}</p>
        </Item>
        <Item heading={t("disclosure.sent.heading")}>
          <ul className="list-disc space-y-1 pl-5">
            {facts.sends.map((kind) => (
              <li key={kind}>{t(`disclosure.sent.${kind}`)}</li>
            ))}
          </ul>
          <p>
            {facts.on_device
              ? t("disclosure.sent.onDevice", { name: who })
              : status.kind === "codex"
                ? t("disclosure.sent.codex", { name: who })
                : t("disclosure.sent.cloud", { name: who })}
          </p>
          {facts.recipient.terms_url ? (
            <p>
              <ExternalLink href={facts.recipient.terms_url}>
                {t("disclosure.sent.terms", { name: who })}
              </ExternalLink>
            </p>
          ) : null}
        </Item>
        <Item heading={t("disclosure.training.heading")}>
          <Training facts={facts} name={who} />
          <p>
            {facts.retention.kind === "stored_days"
              ? t("disclosure.retention.stored_days", { name: who, count: facts.retention.days })
              : t(`disclosure.retention.${facts.retention.kind}`, { name: who })}
          </p>
        </Item>
        {facts.admin_visibility !== "no" ? (
          <Item heading={t("disclosure.admin.heading")}>
            <p>{t(`disclosure.admin.${facts.admin_visibility}`)}</p>
          </Item>
        ) : null}
        <Item heading={t("disclosure.cost.heading")}>
          <p>{t(`disclosure.cost.${facts.cost}`, { name: who })}</p>
        </Item>
        {facts.min_age ? (
          <Item heading={t("disclosure.age.heading")}>
            <p>
              {facts.guardian_permission
                ? t("disclosure.age.guardian", { name: who, age: facts.min_age })
                : t("disclosure.age.body", { name: who, age: facts.min_age })}
            </p>
          </Item>
        ) : null}
        <Item heading={t("disclosure.limits.heading")}>
          <p>{t("disclosure.limits.body")}</p>
        </Item>
        <Item heading={t("disclosure.ownership.heading")}>
          <p>{t("disclosure.ownership.body")}</p>
        </Item>
      </div>

      {acknowledged ? null : (
        <div className="space-y-3 border-t pt-4">
          {needsAge ? (
            <div className="flex items-start gap-3">
              <Checkbox
                id={ids.age}
                checked={ageOk}
                onCheckedChange={(v) => setAgeOk(v === true)}
              />
              <Label htmlFor={ids.age} className="leading-snug font-normal">
                {t("disclosure.age.confirm", { name: who })}
              </Label>
            </div>
          ) : null}
          {needsFreeTier ? (
            <div className="flex items-start gap-3">
              <Checkbox
                id={ids.freeTier}
                checked={freeTierOk}
                onCheckedChange={(v) => setFreeTierOk(v === true)}
              />
              <Label htmlFor={ids.freeTier} className="leading-snug font-normal">
                {t("disclosure.freeTier.confirm", { name: who })}
              </Label>
            </div>
          ) : null}
          {canAccept ? null : (
            <p id={ids.confirmFirst} className="text-sm text-muted-foreground">
              {t("disclosure.confirmFirst")}
            </p>
          )}
          {acknowledge.error ? (
            <p role="alert" className="text-sm text-destructive">
              {errorText(acknowledge.error)}
            </p>
          ) : null}
        </div>
      )}

      <DialogFooter>
        <Button type="button" variant="outline" onClick={onDone}>
          {acknowledged ? t("disclosure.close") : t("disclosure.later")}
        </Button>
        {acknowledged ? null : (
          <Button
            type="button"
            aria-disabled={!canAccept || acknowledge.isPending || undefined}
            aria-describedby={canAccept ? undefined : ids.confirmFirst}
            className="aria-disabled:opacity-50"
            onClick={() => void accept()}
          >
            {t("disclosure.accept", { name })}
          </Button>
        )}
      </DialogFooter>
    </>
  );
}

function Item({ heading, children }: { heading: string; children: ReactNode }) {
  return (
    <section className="space-y-1.5">
      <h3 className="font-medium text-foreground">{heading}</h3>
      <div className="space-y-1.5 text-muted-foreground">{children}</div>
    </section>
  );
}

function Training({ facts, name }: { facts: DisclosureFacts; name: string }) {
  const { t } = useTranslation("ai");
  const training = facts.training;
  if (training.kind === "may_train") {
    return (
      <p>
        {t("disclosure.training.may_train", { name })}{" "}
        {training.how_to_turn_off_url ? (
          <ExternalLink href={training.how_to_turn_off_url}>
            {t("disclosure.training.turnOff")}
          </ExternalLink>
        ) : null}
      </p>
    );
  }
  return <p>{t(`disclosure.training.${training.kind}`, { name })}</p>;
}
