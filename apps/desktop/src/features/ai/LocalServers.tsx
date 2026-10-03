import { useId } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { LocalServer, ModelProviderRecord } from "@/api/ai";
import { useAddModelProvider, useLocalServers } from "@/api/ai-queries";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { useAiErrorText } from "./useAiErrorText";

/**
 * "Models on this computer": Ollama and LM Studio found on loopback (asked again each time the
 * page opens, and on "Look again"). A running server can be added in one click; nothing is
 * downloaded. The facade says which preset to add it as and whether it's already added.
 */
export function LocalServers({ onAdded }: { onAdded: (record: ModelProviderRecord) => void }) {
  const { t } = useTranslation("ai");
  const servers = useLocalServers();
  const errorText = useAiErrorText();
  const headingId = useId();

  return (
    <section aria-labelledby={headingId} className="space-y-3">
      <div className="space-y-1">
        <h3 id={headingId} className="font-medium">
          {t("local.title")}
        </h3>
        <p className="text-sm text-muted-foreground">{t("local.hint")}</p>
      </div>
      {servers.isPending ? (
        <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
          <Spinner aria-hidden />
          {t("local.detecting")}
        </p>
      ) : servers.isError ? (
        <p role="alert" className="text-sm text-destructive">
          {t("local.detectFailed")} {errorText(servers.error)}
        </p>
      ) : (
        <ul className="divide-y border-y">
          {servers.data.map((server) => (
            <ServerRow
              key={server.kind}
              server={server}
              added={server.provider_id != null}
              onAdded={onAdded}
            />
          ))}
        </ul>
      )}
      <Button
        type="button"
        size="sm"
        variant="outline"
        onClick={() => void servers.refetch()}
        aria-disabled={servers.isFetching || undefined}
        className="aria-disabled:opacity-50"
      >
        {t("local.lookAgain")}
      </Button>
    </section>
  );
}

function ServerRow({
  server,
  added,
  onAdded,
}: {
  server: LocalServer;
  added: boolean;
  onAdded: (record: ModelProviderRecord) => void;
}) {
  const { t } = useTranslation("ai");
  const add = useAddModelProvider();
  const errorText = useAiErrorText();
  const name = t(`local.server.${server.kind}`);

  async function use() {
    if (add.isPending) return;
    try {
      const record = await add.mutateAsync({
        preset: server.preset,
        baseUrl: server.base_url,
        apiKey: null,
      });
      toast.success(t("addKey.added", { name: record.label }));
      onAdded(record);
    } catch {
      // Shown below (add.error).
    }
  }

  return (
    <li className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 py-3">
      <div className="min-w-0 space-y-0.5">
        <p className="font-medium">{name}</p>
        <p className="text-sm text-muted-foreground">
          {server.running
            ? t("local.running", { address: server.base_url })
            : t("local.notRunning")}
        </p>
        {add.error ? (
          <p role="alert" className="text-sm text-destructive">
            {errorText(add.error)}
          </p>
        ) : null}
      </div>
      {added ? (
        <span className="text-sm text-muted-foreground">{t("local.inUse")}</span>
      ) : server.running ? (
        <Button type="button" size="sm" onClick={() => void use()} disabled={add.isPending}>
          {add.isPending ? <Spinner aria-hidden /> : null}
          {t("local.use", { name })}
        </Button>
      ) : null}
    </li>
  );
}
