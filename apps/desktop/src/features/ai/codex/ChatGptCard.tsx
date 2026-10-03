import { TriangleAlert } from "lucide-react";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { toast } from "sonner";
import type { AiBackendStatus, CodexStatus } from "@/api/ai";
import {
  useCodexLogout,
  useCodexStatus,
  useRemoveCodex,
  useSetCodexSource,
} from "@/api/ai-queries";
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
import { Skeleton } from "@/components/ui/skeleton";
import { DataPolicyLine } from "@/features/ai/DataPolicyLine";
import { useAiErrorText } from "@/features/ai/useAiErrorText";
import { paths } from "@/lib/routes";
import { isInstalling, useCodexStore } from "@/stores/codex";
import { CodexDownload, CodexInstallProgress } from "./CodexInstall";
import { CodexSignInDialog } from "./CodexSignInDialog";
import { RuntimeOutdatedNotice } from "./RuntimeOutdatedNotice";
import { WeeklyCapField } from "./WeeklyCapField";

/**
 * "Use my ChatGPT plan (runs OpenAI Codex)", mode A (design §2.3, §7): download the pinned Codex,
 * sign in through Codex, then the disclosure sheet; the plan type and its warnings, what an
 * outdated Codex means, the weekly run cap, sign out and remove. No OpenAI logos; the title is a
 * single string key (its wording is one of the questions to OpenAI).
 */
export function ChatGptCard({
  backend,
  onShowDisclosure,
}: {
  /** The `codex` entry of ai_status, once Codex is installed. */
  backend: AiBackendStatus | null;
  /** `signedIn`: the sheet follows a sign-in (focus lands on this card afterwards). */
  onShowDisclosure: (signedIn: boolean) => void;
}) {
  const { t } = useTranslation("ai");
  const status = useCodexStatus();
  const headingId = useId();
  return (
    <section
      aria-labelledby={headingId}
      // Focusable from code only: after signing in and turning it on, focus lands here.
      tabIndex={-1}
      data-backend="codex"
      className="space-y-3 border-b pb-5 outline-hidden focus-visible:rounded-row focus-visible:ring-3 focus-visible:ring-ring"
    >
      <div className="space-y-1">
        <h3 id={headingId} className="font-medium">
          {t("codex.title")}
        </h3>
        <p className="text-sm text-muted-foreground">{t("codex.description")}</p>
      </div>
      {status.isPending ? (
        <Skeleton className="h-10 w-full" />
      ) : status.isError ? null : (
        <CardBody status={status.data} backend={backend} onShowDisclosure={onShowDisclosure} />
      )}
    </section>
  );
}

function CardBody({
  status,
  backend,
  onShowDisclosure,
}: {
  status: CodexStatus;
  backend: AiBackendStatus | null;
  onShowDisclosure: (signedIn: boolean) => void;
}) {
  const { t } = useTranslation("ai");
  const install = useCodexStore((s) => s.install);
  const [signingIn, setSigningIn] = useState(false);
  const runtime = status.runtime;

  if (runtime.state === "unsupported_platform") {
    return <p className="text-sm">{t("codex.download.unsupported")}</p>;
  }
  const untested = runtime.untested_platform ? (
    <p className="text-sm text-muted-foreground">{t("codex.download.untested")}</p>
  ) : null;
  if (runtime.state === "not_installed") {
    return (
      <>
        <CodexDownload />
        {untested}
      </>
    );
  }

  const login = status.login;
  const signedIn = login.state !== "signed_out";
  const plan = login.plan_type && login.plan_type !== "unknown" ? login.plan_type : null;
  return (
    <div className="space-y-3">
      <p className="text-sm text-muted-foreground">
        {t("codex.installed", { version: runtime.installed_version ?? runtime.pinned_version })}
      </p>
      {untested}
      <RuntimeOutdatedNotice status={status} />
      {status.outdated_action === "none" && isInstalling(install) ? <CodexInstallProgress /> : null}

      {login.state === "signed_out" ? (
        <Button type="button" onClick={() => setSigningIn(true)}>
          {t("codex.signIn")}
        </Button>
      ) : null}

      {login.state === "chatgpt" ? (
        <p className="text-sm font-medium">
          {plan ? t("codex.signedInPlan", { plan: t(`codex.plan.${plan}`) }) : t("codex.signedIn")}
        </p>
      ) : null}
      {backend?.disclosure.admin_visibility === "yes" ? (
        <Warning>{t("codex.adminWarning")}</Warning>
      ) : backend?.disclosure.admin_visibility === "unknown" ? (
        <Warning>{t("codex.adminUnknown")}</Warning>
      ) : null}
      {login.state === "api_key" ? <Warning>{t("codex.apiKeyWarning")}</Warning> : null}
      {login.state === "chatgpt" && status.exec_available === false ? (
        // D10: the plan doesn't let other apps run Codex; point to what still works.
        <p className="text-sm">
          {t("codex.noExec")}{" "}
          <Link to={paths.connect} className="underline underline-offset-4">
            {t("codex.noExecConnect")}
          </Link>
        </p>
      ) : null}

      {backend && login.state === "chatgpt" && status.exec_available !== false ? (
        <>
          <DataPolicyLine facts={backend.disclosure} name={backend.disclosure.recipient.name} />
          {backend.state === "needs_disclosure" ? (
            <Button type="button" size="sm" onClick={() => onShowDisclosure(false)}>
              {t("backend.turnOn")}
            </Button>
          ) : (
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => onShowDisclosure(false)}
            >
              {t("backend.whatsShared")}
            </Button>
          )}
        </>
      ) : null}

      {login.state === "chatgpt" ? (
        <WeeklyCapField cap={status.weekly_cap ?? null} runs={status.runs_this_week} />
      ) : null}

      <SystemCodex status={status} />

      <div className="flex flex-wrap gap-2 pt-1">
        {signedIn ? <SignOutButton /> : null}
        <RemoveCodexButton />
      </div>

      <CodexSignInDialog
        open={signingIn}
        onOpenChange={setSigningIn}
        onSignedIn={() => onShowDisclosure(true)}
      />
    </div>
  );
}

function Warning({ children }: { children: string }) {
  return (
    <p className="flex gap-2 text-sm">
      <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden />
      <span>{children}</span>
    </p>
  );
}

/** D12: an installed `codex` in the tested range can be used instead of PageLamp's own. */
function SystemCodex({ status }: { status: CodexStatus }) {
  const { t } = useTranslation("ai");
  const setSource = useSetCodexSource();
  const errorText = useAiErrorText();
  const system = status.system_codex;
  if (!system) return null;
  if (status.runtime.source === "system") {
    return (
      <div className="flex flex-wrap items-center gap-3 text-sm">
        <p>{t("codex.system.inUse")}</p>
        <Button type="button" size="sm" variant="ghost" onClick={() => setSource.mutate("managed")}>
          {t("codex.system.useManaged")}
        </Button>
      </div>
    );
  }
  return (
    <div className="space-y-1 text-sm text-muted-foreground">
      <p>
        {t("codex.system.found", { version: system.version })}{" "}
        {system.in_tested_range ? null : t("codex.system.outOfRange")}
      </p>
      {system.in_tested_range ? (
        <Button type="button" size="sm" variant="ghost" onClick={() => setSource.mutate("system")}>
          {t("codex.system.use")}
        </Button>
      ) : null}
      {setSource.error ? (
        <p role="alert" className="text-destructive">
          {errorText(setSource.error)}
        </p>
      ) : null}
    </div>
  );
}

function SignOutButton() {
  const { t } = useTranslation("ai");
  const logout = useCodexLogout();
  const errorText = useAiErrorText();
  return (
    <>
      <Button
        type="button"
        size="sm"
        variant="outline"
        disabled={logout.isPending}
        onClick={() =>
          logout.mutate(undefined, { onSuccess: () => toast.success(t("codex.signOutDone")) })
        }
      >
        {t("codex.signOut")}
      </Button>
      {logout.error ? (
        <p role="alert" className="text-sm text-destructive">
          {errorText(logout.error)}
        </p>
      ) : null}
    </>
  );
}

function RemoveCodexButton() {
  const { t } = useTranslation("ai");
  const { t: tc } = useTranslation();
  const remove = useRemoveCodex();
  const errorText = useAiErrorText();
  const [open, setOpen] = useState(false);

  async function confirm() {
    try {
      await remove.mutateAsync();
      toast.success(t("codex.removed"));
      setOpen(false);
    } catch {
      // Shown in the dialog (remove.error).
    }
  }

  return (
    <AlertDialog open={open} onOpenChange={setOpen}>
      <Button type="button" size="sm" variant="ghost" onClick={() => setOpen(true)}>
        {t("codex.remove")}
      </Button>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("codex.removeTitle")}</AlertDialogTitle>
          <AlertDialogDescription>{t("codex.removeBody")}</AlertDialogDescription>
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
            {t("codex.remove")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
