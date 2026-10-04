import { CircleAlert, KeyRound, LockKeyhole, RefreshCw } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useStatus } from "@/api/queries";
import type { SourceRecord } from "@/api/types";
import { ExternalLink } from "@/components/common/ExternalLink";
import { RelativeTime } from "@/components/common/RelativeTime";
import { SentenceWithTime, WHEN } from "@/components/common/SentenceWithTime";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { formatIsoDate } from "@/lib/format";
import { deadlinesReadSince } from "@/lib/freshness";
import { useStartSync, useSyncActivity, useSyncStore } from "@/stores/sync";
import { RemoveSourceButton } from "./RemoveSourceButton";
import { SourceStatusBadge } from "./SourceStatusBadge";
import { configString, hasSecret, SOURCE_ICON } from "./sourceMeta";
import { TechnicalDetails } from "./TechnicalDetails";

interface SourceCardProps {
  source: SourceRecord;
  /** Opens the replace-token / replace-feed-address dialog for this source. */
  onReplaceSecret: (source: SourceRecord) => void;
}

/** One configured source: what it is, its non-secret settings, status and actions. */
export function SourceCard({ source, onReplaceSecret }: SourceCardProps) {
  const { t } = useTranslation("sources");
  const { t: tc } = useTranslation();
  const startSync = useStartSync();
  const { running, busy } = useSyncActivity();
  const live = useSyncStore((s) => s.bySource[source.id]);
  const syncing = running && live !== undefined && live.result === null;
  const Icon = SOURCE_ICON[source.kind];
  const expired = source.last_error_kind === "auth_expired_or_revoked";
  const canvas = source.kind === "canvas";
  // Canvas: the account's display name from when the token was checked (never an email or id).
  const accountName = canvas ? configString(source, "account_name") : null;
  const replaceLabel = t(canvas ? "actions.replaceToken" : "actions.replaceFeed");
  // Unique per card for screen readers ("Replace token for Demo Canvas"); starts with the
  // visible text so voice control users can say what they see.
  const replaceName = t(canvas ? "actions.replaceTokenFor" : "actions.replaceFeedFor", {
    label: source.label,
  });

  return (
    <Card>
      <CardHeader>
        {/* A long label wraps (any character may break it), so the status badge stays on the card. */}
        <CardTitle className="flex min-w-0 items-start gap-2">
          <Icon className="mt-0.75 size-4 shrink-0 text-muted-foreground" aria-hidden />
          <h2 className="min-w-0 [overflow-wrap:anywhere]">{source.label}</h2>
        </CardTitle>
        <CardDescription>
          {accountName
            ? t("card.kindConnectedAs", {
                kind: tc(`sourceKind.${source.kind}`),
                name: accountName,
              })
            : tc(`sourceKind.${source.kind}`)}
        </CardDescription>
        <CardAction>
          <SourceStatusBadge source={source} syncing={syncing} />
        </CardAction>
      </CardHeader>

      <CardContent className="space-y-4">
        <SourceDetails source={source} />

        {source.last_error_kind ? (
          <Alert role="status" variant="destructive">
            {expired ? <KeyRound aria-hidden /> : <CircleAlert aria-hidden />}
            <AlertTitle>{expired ? t("problem.expiredTitle") : t("problem.title")}</AlertTitle>
            <AlertDescription className="space-y-2">
              {expired ? (
                <p className="font-medium text-foreground">
                  {t(canvas ? "problem.expiredCanvas" : "problem.expiredFeed")}
                </p>
              ) : null}
              {source.last_error ? (
                <TechnicalDetails lines={[source.last_error]} subject={source.label} />
              ) : null}
              {expired && hasSecret(source.kind) ? (
                <Button size="sm" onClick={() => onReplaceSecret(source)} aria-label={replaceName}>
                  <KeyRound aria-hidden />
                  {replaceLabel}
                </Button>
              ) : null}
            </AlertDescription>
          </Alert>
        ) : null}
      </CardContent>

      <CardFooter className="gap-2">
        <Button
          size="sm"
          variant="outline"
          onClick={() => {
            if (!busy) void startSync(source.id);
          }}
          aria-disabled={busy || undefined}
          className="aria-disabled:opacity-50"
          aria-label={t("actions.syncSource", { label: source.label })}
        >
          <RefreshCw className={syncing ? "animate-spin" : undefined} aria-hidden />
          {t("actions.sync")}
        </Button>
        {hasSecret(source.kind) && !expired ? (
          <Button
            size="sm"
            variant="ghost"
            onClick={() => onReplaceSecret(source)}
            aria-label={replaceName}
          >
            <KeyRound aria-hidden />
            {replaceLabel}
          </Button>
        ) : null}
        <span className="flex-1" />
        {/* Not while any sync runs, here or from the CLI: it may be syncing this source. */}
        <RemoveSourceButton source={source} disabled={busy} />
      </CardFooter>
    </Card>
  );
}

/** Non-secret configuration + freshness, as a definition list. Never shows a secret. */
function SourceDetails({ source }: { source: SourceRecord }) {
  const { t, i18n } = useTranslation("sources");
  const { t: tc } = useTranslation();
  const baseUrl = configString(source, "base_url");
  const path = configString(source, "path");
  const termStart = configString(source, "term_start");
  // "Last synced" is the last full sync. An automatic sync that only read the deadlines and
  // announcements since then gets its own line, so neither time is taken for the other.
  const status = useStatus();
  const deadlinesAt = deadlinesReadSince(
    source.last_synced_at,
    status.data?.deadlines_synced_at[source.id],
  );

  return (
    <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1.5 text-sm">
      {source.kind === "canvas" && baseUrl ? (
        <Row term={t("card.canvasAddress")}>
          <ExternalLink href={baseUrl}>{baseUrl}</ExternalLink>
        </Row>
      ) : null}
      {source.kind === "folder" && path ? (
        <Row term={t("card.folderPath")}>
          <span className="font-mono text-[13px] break-all">{path}</span>
        </Row>
      ) : null}
      {source.kind === "folder" && termStart ? (
        <Row term={t("card.termStart")}>{formatIsoDate(termStart, i18n.language)}</Row>
      ) : null}
      {source.kind === "ical" ? (
        <Row term={t("card.feed")}>
          <span className="inline-flex items-center gap-1.5 text-muted-foreground">
            <LockKeyhole className="size-3.5 shrink-0" aria-hidden />
            {t("card.feedPrivate")}
          </span>
        </Row>
      ) : null}
      <Row term={t("card.lastSynced")}>
        {source.last_synced_at ? <RelativeTime iso={source.last_synced_at} /> : tc("sync.never")}
        {deadlinesAt ? (
          <span className="block text-xs text-muted-foreground">
            <SentenceWithTime text={t("card.deadlinesChecked", { when: WHEN })} iso={deadlinesAt} />
          </span>
        ) : null}
      </Row>
    </dl>
  );
}

function Row({ term, children }: { term: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-muted-foreground">{term}</dt>
      <dd className="min-w-0">{children}</dd>
    </>
  );
}
